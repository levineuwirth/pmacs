// tests/e7b_gpu_chord_acceptance.rs --- E7b.3, `C-M-i` reaches the daemon.

//! The GPU's `AltGr` rule (`is_layout_text`) treated any printable text
//! produced while Ctrl and Alt are both held as layout text and
//! stripped the modifiers --- right for `AltGr` producing `€` from `e`,
//! wrong for `C-M-i` producing `i` from `i`, which then arrived at the
//! daemon as the letter. E7b.3 refines the rule: layout text only when
//! the produced text differs from the key's own unmodified character.
//!
//! Witnessed through the production dispatch against a real daemon:
//! `pmacs-gpu --headless-probe` with `PMACS_GPU_PROBE_ACTION=chord`
//! presses one key through `App::apply_keyboard` with Ctrl and Alt
//! held, naming the produced text and the key's own character as winit
//! reports them, and reports what the session holds afterwards.
//!
//! * `i` from `i` (a US layout's `C-M-i`) reaches `completion.at-point`:
//!   the popup opens, fed by the fake server, and the text is unchanged.
//! * `€` from `e` (`AltGr`-e on a layout that yields the euro sign, Ctrl
//!   and Alt held as Windows reports `AltGr`) inserts `€`.
//!
//! The machine this was written on has one layout (US); the German and
//! other `AltGr` layouts are exercised by naming what winit reports for
//! them, not by pressing their keys --- said in the record.

use std::path::Path;

mod common;

use common::daemon::TestDaemon;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

/// A daemon visiting `a.rs` under a cargo project, the rust server
/// pointed at the fake (which answers every completion request with
/// items), `*scratch*` killed so the attaching frontend lands on the
/// file.
fn daemon_on_a_rust_file() -> (TestDaemon, tempfile::TempDir) {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let file = td.path().join("a.rs");
    std::fs::write(&file, b"fn main() {\n    prin\n}\n").expect("write rs");
    let fixture = file.display().to_string();
    let fake = fake_lsp_path();
    let init_lua = format!(
        "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, env = {{ PMACS_FAKE_LSP_MODE = 'e7b' }} }} }}\n\
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
    (daemon, td)
}

/// Run the chord probe once: `text` is what winit would report for the
/// press, `key` the key's own character. `None` when the run is
/// skipped (no `pmacs-gpu` binary, or no wgpu adapter); a skip panics
/// under `PMACS_REQUIRE_GPU`, so the gate's GPU leg never passes
/// vacuously.
fn run_chord_probe(
    daemon: &TestDaemon,
    text: &str,
    key: &str,
    report_name: &str,
) -> Option<std::collections::HashMap<String, String>> {
    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("test binary directory")
        .join("pmacs-gpu");
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!(
            "skipping the GPU chord probe: {} is not built",
            binary.display()
        );
        return None;
    }
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join(report_name);
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", text)
        .env("PMACS_GPU_PROBE_CHORD_KEY", key)
        .env("PMACS_GPU_PROBE_ACTION", "chord")
        .output()
        .expect("run the headless GPU chord probe");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU chord probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU chord probe: no wgpu adapter available");
        return None;
    }
    let report_text = std::fs::read_to_string(&report).expect("probe report");
    eprintln!("--- chord probe report ({text} from {key}) ---\n{report_text}--- end ---");
    Some(
        report_text
            .lines()
            .filter_map(|line| {
                let (k, v) = line.split_once('=')?;
                Some((k.to_owned(), v.to_owned()))
            })
            .collect(),
    )
}

/// `C-M-i` on a US layout: the text `i` from the key `i`, Ctrl and Alt
/// held. It leaves the GPU as the chord, the daemon runs
/// `completion.at-point`, the popup opens, and no `i` is inserted.
/// Bitten by reverting `is_layout_text` to the text-only rule: the
/// letter is inserted and no popup opens.
#[test]
fn e7b_3_c_m_i_on_a_us_layout_opens_the_completion_popup() {
    let (daemon, _td) = daemon_on_a_rust_file();
    let Some(facts) = run_chord_probe(&daemon, "i", "i", "chord-i.txt") else {
        return;
    };
    assert_eq!(
        facts.get("pressed_at_ms").map(String::as_str),
        Some(facts["pressed_at_ms"].as_str())
    );
    assert_ne!(facts["pressed_at_ms"], "none", "the key was pressed");
    assert_eq!(
        facts["text_after"], facts["text_before"],
        "C-M-i inserts nothing"
    );
    assert!(
        !facts["text_after"].contains("prini") && !facts["text_after"].contains("iprin"),
        "no letter i landed: {}",
        facts["text_after"]
    );
    assert_eq!(
        facts["popup_open"], "true",
        "the completion popup is open after C-M-i (opened at {} ms)",
        facts["popup_open_at_ms"]
    );
}

/// `AltGr`-e on a layout that yields `€`: the text `€` from the key `e`,
/// Ctrl and Alt held as Windows reports `AltGr`. It leaves the GPU as the
/// plain character and the mirror gains a `€`; no popup opens.
#[test]
fn e7b_3_altgr_e_still_inserts_the_euro_sign() {
    let (daemon, _td) = daemon_on_a_rust_file();
    let Some(facts) = run_chord_probe(&daemon, "€", "e", "chord-euro.txt") else {
        return;
    };
    assert_ne!(facts["pressed_at_ms"], "none", "the key was pressed");
    assert!(
        facts["text_after"].contains('€'),
        "AltGr-e inserts the euro sign: {}",
        facts["text_after"]
    );
    assert!(
        !facts["text_before"].contains('€'),
        "which was not there before: {}",
        facts["text_before"]
    );
    assert_eq!(facts["popup_open"], "false", "and opens no popup");
}
