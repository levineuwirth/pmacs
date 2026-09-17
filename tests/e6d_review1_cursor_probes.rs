// tests/e6d_review1_cursor_probes.rs --- E6d review 1: typing on
// through a daemon that is slow at the first edit.

//! Behavioral probes against E6d.3 through the GPU's production
//! dispatch, driven by the latency probe against a daemon whose
//! `buffer.after-edit` fan-out spins 700 ms on the first edit --- the
//! `e6d_cursor_confirm_acceptance` fixture --- in the state the phase's
//! witness left out: the user does not stop at the first keystroke.
//! The owner's cold-daemon case was `foo(` typed and typing on; the
//! first row types an opener and two characters at 50 ms, so the two
//! characters go out optimistically while the daemon is still inside
//! the opener's fan-out and the auto-pair closer that fan-out inserts
//! lands concurrently with them; the second row types three plain
//! characters the same way. The text must settle to what was typed,
//! the closer behind it, every character once, and the run must
//! restore.
//!
//! Helpers are the acceptance suite's, copied so the probe stands
//! alone.

#![cfg(feature = "crdt")]

use std::fmt::Write as _;
use std::path::Path;

#[path = "common/mod.rs"]
mod common;

use common::daemon::TestDaemon;

fn gpu_binary() -> std::path::PathBuf {
    Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("test binary directory")
        .join("pmacs-gpu")
}

/// A daemon visiting a generated Rust file with no server, its
/// `buffer.after-edit` fan-out spinning `spin_ms` once on the first
/// edit, and the scratch buffer killed so one CRDT buffer is on offer.
fn daemon_with_slow_first_edit(spin_ms: u64) -> (TestDaemon, tempfile::TempDir) {
    let fixture_dir = tempfile::TempDir::new().expect("fixture tempdir");
    let fixture = fixture_dir.path().join("a.rs");
    let mut body = String::new();
    for i in 0..200 {
        let _ = writeln!(body, "fn f{i}() -> u32 {{\n    {i}\n}}\n");
    }
    std::fs::write(&fixture, body).expect("write a.rs");
    let fixture = fixture.display().to_string();
    let init_lua = format!(
        "pmacs.lsp.config = {{}}\n\
         local spun = false\n\
         pmacs.hook.add('buffer.after-edit', function()\n\
           if spun then return end\n\
           spun = true\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           while pmacs.editor.monotonic_ms() - t0 < {spin_ms} do end\n\
         end)\n\
         pmacs.buffer.find_or_open({fixture:?})\n\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n\
         end\n"
    );
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_lua,
    );
    (daemon, fixture_dir)
}

/// Run the latency probe once against `daemon` with `envs`, returning
/// the report's facts, or `None` when no adapter is available and the
/// GPU is not required.
fn run_latency_probe(
    daemon: &TestDaemon,
    report_name: &str,
    envs: &[(&str, &str)],
) -> Option<std::collections::HashMap<String, String>> {
    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = gpu_binary();
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!(
            "skipping the latency probe: {} is not built",
            binary.display()
        );
        return None;
    }
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join(report_name);
    let mut cmd = std::process::Command::new(&binary);
    cmd.arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_ACTION", "latency")
        .env("PMACS_GPU_PROBE_SAMPLES", "1")
        .env("PMACS_GPU_PROBE_TYPE_AT", "0")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "30000");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run the latency probe");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "latency probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the latency probe: no wgpu adapter available");
        return None;
    }
    let text = std::fs::read_to_string(&report).expect("probe report");
    eprintln!("--- probe report ({report_name}) ---\n{text}--- end ---");
    Some(
        text.lines()
            .filter_map(|line| {
                let (k, v) = line.split_once('=')?;
                Some((k.to_owned(), v.to_owned()))
            })
            .collect(),
    )
}

fn field<'a>(facts: &'a std::collections::HashMap<String, String>, key: &str) -> &'a str {
    facts
        .get(key)
        .map_or_else(|| panic!("report carries no {key}"), String::as_str)
}

/// `sample.0`'s `name=value` fields.
fn sample_fields(facts: &std::collections::HashMap<String, String>) -> Vec<(String, String)> {
    field(facts, "sample.0")
        .split(' ')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect()
}

fn sample_field(facts: &std::collections::HashMap<String, String>, key: &str) -> String {
    sample_fields(facts)
        .into_iter()
        .find(|(k, _)| k == key)
        .map_or_else(|| panic!("sample.0 carries no {key}"), |(_, v)| v)
}

/// The probe's `landed` predicate is the typed text at the typed byte,
/// so a closer that lands between the typed characters (`(a)b`) or
/// ahead of them (`()ab`) never settles and the sample reports its
/// failure; the restore deletes what the run inserted and the mirror
/// must come back to the original, so nothing may have landed twice.
fn assert_typed_once_in_order(facts: &std::collections::HashMap<String, String>, what: &str) {
    assert_eq!(field(facts, "samples"), "1", "{what}: one sample was taken");
    assert_eq!(
        field(facts, "failure"),
        "",
        "{what}: the text settled to what was typed, in order, and the deletes restored it"
    );
    assert!(
        sample_field(facts, "text").parse::<u64>().is_ok(),
        "{what}: the typed text landed at the typed byte"
    );
    assert_eq!(
        field(facts, "final_text_matches_original"),
        "true",
        "{what}: the restore left the document as it was"
    );
}

/// An opener and two characters at 50 ms into a daemon that spins 700
/// ms inside the opener's fan-out: the characters go optimistically
/// while the daemon is still in the fan-out, and the closer that
/// fan-out inserts must still land behind them --- `(ab)`, not `()ab`.
#[test]
fn typing_on_through_a_slow_opener_keeps_the_closer_behind_the_typed_text() {
    let (daemon, _dir) = daemon_with_slow_first_edit(700);
    let Some(facts) = run_latency_probe(
        &daemon,
        "opener-then-typing.txt",
        &[
            ("PMACS_GPU_PROBE_TYPE_TEXT", "(ab"),
            ("PMACS_GPU_PROBE_KEY_GAPS_MS", "50,50"),
        ],
    ) else {
        return;
    };
    assert_typed_once_in_order(&facts, "(ab under a slow opener");
    // The opener's own caret is confirmed before the fan-out (the
    // phase's row); the two characters typed into the 700 ms stall
    // cannot be confirmed inside a 500 ms floor by any ordering, so
    // the report's `fallbacks` is read, not asserted: the row is about
    // the text.
    let cursor_ms: u64 = sample_field(&facts, "cursor")
        .parse()
        .expect("the confirming CursorByte arrived");
    eprintln!(
        "opener-then-typing: cursor {cursor_ms} ms, fallbacks {}",
        field(&facts, "fallbacks")
    );
}

/// Three plain characters at 50 ms into the same daemon: each goes
/// optimistically, the second and third while the first's fan-out
/// holds the dispatcher, and every one lands once, in order.
#[test]
fn typing_on_through_a_slow_first_edit_keeps_every_character_once_in_order() {
    let (daemon, _dir) = daemon_with_slow_first_edit(700);
    let Some(facts) = run_latency_probe(
        &daemon,
        "typing-on.txt",
        &[
            ("PMACS_GPU_PROBE_TYPE_TEXT", "abc"),
            ("PMACS_GPU_PROBE_KEY_GAPS_MS", "50,50"),
        ],
    ) else {
        return;
    };
    assert_typed_once_in_order(&facts, "abc under a slow first edit");
    eprintln!(
        "typing-on: cursor {} ms, fallbacks {}",
        sample_field(&facts, "cursor"),
        field(&facts, "fallbacks")
    );
}
