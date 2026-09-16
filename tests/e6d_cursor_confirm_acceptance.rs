//! E6d.3 --- confirm the cursor before the slow work.
//!
//! The GPU's optimistic floor waits for the `CursorByte` that confirms
//! a keystroke's predicted caret, for up to `FLOOR_CONFIRM_TIMEOUT`
//! (500 ms), and on timing out drops to round-trip input and releases
//! the keys it held. The daemon used to emit that `CursorByte` once per
//! tick, after the edit's `buffer.after-edit` fan-out, the render pass
//! and everything both cost on a large file; it now emits it the
//! moment the edit applies, before the fan-out. The witness makes the
//! fan-out slow on purpose --- a `buffer.after-edit` hook that spins
//! for 700 ms on the first edit --- and drives the GPU's production
//! dispatch through the latency probe: the first keystroke confirms in
//! milliseconds, where the tick's copy would have arrived after the
//! hook and the floor would have released.
//!
//! The second row probes the fallback itself, as the row asks
//! regardless. The GPU checks the floor's age only when a daemon
//! message arrives, so a daemon that says nothing releases nothing; the
//! fallback fires when the daemon speaks again with an unconfirmed
//! floor older than 500 ms, which is what a slow render pass produces:
//! the frame's messages come out after the edits queued behind it, and
//! their confirmations follow. The fixture spins 700 ms inside a
//! statusline provider on the first frame after the first edit, three
//! characters go at once, an 800 ms pause lets that frame land and
//! release the floor, and three more are typed after. The text must
//! carry every character once, in order, and the run must have
//! released a floor for the row to say anything.

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

/// Where the fixture spins: inside the first `buffer.after-edit`
/// fan-out (the edit path), or inside the statusline provider on the
/// first frame after the first edit (the render pass).
#[derive(Clone, Copy)]
enum SpinIn {
    AfterEditHook,
    RenderPass,
}

/// A daemon visiting a generated Rust file with no server, spinning
/// `spin_ms` once at `spin_in`, and the scratch buffer killed so one
/// CRDT buffer is on offer (the E6b.3 fixture's reason).
fn daemon_with_slow_first_edit(spin_ms: u64, spin_in: SpinIn) -> (TestDaemon, tempfile::TempDir) {
    let fixture_dir = tempfile::TempDir::new().expect("fixture tempdir");
    let fixture = fixture_dir.path().join("a.rs");
    let mut body = String::new();
    for i in 0..200 {
        let _ = writeln!(body, "fn f{i}() -> u32 {{\n    {i}\n}}\n");
    }
    std::fs::write(&fixture, body).expect("write a.rs");
    let spin = match spin_in {
        SpinIn::AfterEditHook => format!(
            "pmacs.hook.add('buffer.after-edit', function()\n\
               if spun then return end\n\
               spun = true\n\
               local t0 = pmacs.editor.monotonic_ms()\n\
               while pmacs.editor.monotonic_ms() - t0 < {spin_ms} do end\n\
             end)\n"
        ),
        SpinIn::RenderPass => format!(
            "local edited = false\n\
             pmacs.hook.add('buffer.after-edit', function() edited = true end)\n\
             pmacs.statusline.register {{\n\
               name = 'e6d-spin', side = 'right', priority = 0, face = 'ui.modeline.lsp',\n\
               fn = function()\n\
                 if edited and not spun then\n\
                   spun = true\n\
                   local t0 = pmacs.editor.monotonic_ms()\n\
                   while pmacs.editor.monotonic_ms() - t0 < {spin_ms} do end\n\
                 end\n\
                 return 'spin'\n\
               end,\n\
             }}\n"
        ),
    };
    let fixture = fixture.display().to_string();
    let init_lua = format!(
        "pmacs.lsp.config = {{}}\n\
         local spun = false\n\
         {spin}\
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

/// The first keystroke's caret is confirmed before the after-edit
/// fan-out runs: with the fan-out spinning 700 ms, the confirming
/// `CursorByte` still arrives inside the 500 ms floor, and no floor
/// releases.
#[test]
fn the_first_keystroke_confirms_before_a_slow_after_edit_hook() {
    let (daemon, _dir) = daemon_with_slow_first_edit(700, SpinIn::AfterEditHook);
    let Some(facts) = run_latency_probe(
        &daemon,
        "confirm.txt",
        &[("PMACS_GPU_PROBE_TYPE_TEXT", "x")],
    ) else {
        return;
    };
    assert_eq!(field(&facts, "samples"), "1", "one sample was taken");
    assert_eq!(
        field(&facts, "failure"),
        "",
        "the sample settled and restored"
    );
    let cursor_ms: u64 = sample_field(&facts, "cursor")
        .parse()
        .expect("the confirming CursorByte arrived");
    assert!(
        cursor_ms < 500,
        "the caret confirmed inside the floor, before the 700 ms hook: {cursor_ms} ms"
    );
    assert_eq!(
        field(&facts, "fallbacks"),
        "0",
        "no floor released unconfirmed"
    );
}

/// Typing through a floor timeout keeps every character once, in
/// order: `abc` typed at once with a 700 ms render pass behind the
/// first, an 800 ms pause in which that frame lands and releases the
/// floor, then `def` typed after the transition; the mirror ends with
/// `abcdef` at the typed byte and the run restores.
#[test]
fn typing_through_a_floor_timeout_keeps_every_character_once_in_order() {
    let (daemon, _dir) = daemon_with_slow_first_edit(700, SpinIn::RenderPass);
    let Some(facts) = run_latency_probe(
        &daemon,
        "fallback.txt",
        &[
            ("PMACS_GPU_PROBE_TYPE_TEXT", "abcdef"),
            ("PMACS_GPU_PROBE_KEY_GAPS_MS", "50,50,800,50,50"),
        ],
    ) else {
        return;
    };
    assert_eq!(field(&facts, "samples"), "1", "one sample was taken");
    assert_eq!(
        field(&facts, "fallbacks"),
        "1",
        "the floor released while the fan-out held the daemon (the positive control)"
    );
    let fallback_ms: u64 = sample_field(&facts, "fallback")
        .parse()
        .expect("a fallback time");
    assert!(
        (500..1000).contains(&fallback_ms),
        "the floor released when the slow frame landed, during the pause: {fallback_ms} ms"
    );
    assert_eq!(
        field(&facts, "failure"),
        "",
        "the text settled to abcdef once, in order, and the deletes restored it"
    );
    assert!(
        sample_field(&facts, "text").parse::<u64>().is_ok(),
        "abcdef landed at the typed byte"
    );
    assert_eq!(
        field(&facts, "final_text_matches_original"),
        "true",
        "the restore left the document as it was"
    );
}
