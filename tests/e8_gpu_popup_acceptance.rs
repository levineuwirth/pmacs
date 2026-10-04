// tests/e8_gpu_popup_acceptance.rs --- E8.3, the popup on the GPU.

//! The hover and signature popup on the GPU frontend, through the
//! production dispatch against a real daemon: `pmacs-gpu
//! --headless-probe` with `PMACS_GPU_PROBE_ACTION=popup` presses keys
//! through `App::apply_keyboard`, turns the wheel and clicks through
//! `App::apply_wheel` and `App::apply_left_button`, and reports the
//! popup as the next frame paints it, a background pixel read back from
//! that frame included. The probe asserts nothing; these rows read its
//! report.
//!
//! CRDT-only, as every suite that drives the GPU probe is: the probe is
//! a semantic replica, which a daemon can host only with the feature.
#![cfg(feature = "crdt")]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

mod common;

use common::daemon::TestDaemon;

/// A daemon visiting `a.rs` holding `source` in a cargo project, the
/// rust server pointed at the fake in `mode` with `env`, `*scratch*`
/// killed so the attaching frontend lands on the file, `extra` last.
fn daemon_on(
    source: &str,
    mode: &str,
    env: &[(&str, &str)],
    extra: &str,
) -> (TestDaemon, tempfile::TempDir) {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let file = td.path().join("a.rs");
    std::fs::write(&file, source).expect("write rs");
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut env_lua = format!("PMACS_FAKE_LSP_MODE = '{mode}',");
    for (k, v) in env {
        let _ = write!(env_lua, " {k} = '{v}',");
    }
    let init_lua = format!(
        "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, env = {{ {env_lua} }} }} }}\n\
         pmacs.buffer.find_or_open({:?})\n\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n\
         end\n\
         {extra}\n",
        file.display().to_string()
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

/// Run the popup probe with `script`; `None` when skipped (no
/// `pmacs-gpu` binary or no wgpu adapter), which panics under
/// `PMACS_REQUIRE_GPU` so the gate's GPU leg never passes vacuously.
fn run_popup_probe(
    daemon: &TestDaemon,
    script: &str,
    name: &str,
) -> Option<HashMap<String, String>> {
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
            "skipping the GPU popup probe: {} is not built",
            binary.display()
        );
        return None;
    }
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join(name);
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", script)
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "40000")
        .output()
        .expect("run the headless GPU popup probe");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU popup probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU popup probe: no wgpu adapter available");
        return None;
    }
    let text = std::fs::read_to_string(&report).expect("read the probe report");
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
}

fn fact<'a>(facts: &'a HashMap<String, String>, key: &str) -> &'a str {
    facts.get(key).map_or_else(
        || panic!("the report has no {key}: {facts:?}"),
        String::as_str,
    )
}

fn num(facts: &HashMap<String, String>, key: &str) -> f32 {
    fact(facts, key)
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a number: {facts:?}"))
}

/// `x,y,w,h` of a report's box.
fn rect(facts: &HashMap<String, String>, label: &str) -> [f32; 4] {
    let v: Vec<f32> = fact(facts, &format!("{label}.box"))
        .split(',')
        .map(|n| n.parse().expect("box number"))
        .collect();
    [v[0], v[1], v[2], v[3]]
}

/// E8.3's gate row: a hover popup at the caret on the GPU carrying the
/// server's whole text --- two hundred lines, so it scrolls --- placed
/// below the caret's line inside the window; the wheel over it scrolls
/// it and nothing else; a click inside does not dismiss it; and motion
/// out of its range does.
#[test]
fn e8_3_the_gpu_shows_scrolls_keeps_on_click_and_closes_on_motion_the_hover() {
    let (daemon, _td) = daemon_on(
        "fn main() { let value = 1; }\n",
        "hover",
        &[("PMACS_FAKE_LSP_HOVER_LINES", "200")],
        "",
    );
    let Some(facts) = run_popup_probe(
        &daemon,
        "ask:h report:open wheel:3 report:scrolled click quiet:600 report:clicked \
         key:C-e closed report:moved",
        "popup.report",
    ) else {
        return;
    };
    assert_eq!(fact(&facts, "session_protocol_version"), "26");
    assert_eq!(fact(&facts, "open.open"), "true", "{facts:?}");
    assert_eq!(fact(&facts, "open.kind"), "hover");
    assert_eq!(fact(&facts, "open.lines"), "202", "the whole text arrives");
    assert_eq!(fact(&facts, "open.first"), "Some(\"fn hovered()\")");
    let rows = num(&facts, "open.rows");
    let shown = num(&facts, "open.shown");
    assert!(
        rows >= 202.0 && shown < rows,
        "it overflows, so it scrolls: {facts:?}"
    );
    assert_eq!(
        fact(&facts, "open.above"),
        "false",
        "room below: below the line"
    );
    let [x, y, w, h] = rect(&facts, "open");
    let (win_w, band_top) = (900.0, num(&facts, "open.band_top"));
    assert!(
        x >= 0.0 && x + w <= win_w + 0.5,
        "inside the window: {facts:?}"
    );
    assert!(
        y >= 0.0 && y + h <= band_top + 0.5,
        "above the status band: {facts:?}"
    );
    assert_eq!(fact(&facts, "open.scroll"), "0");

    assert_eq!(
        fact(&facts, "scrolled.scroll"),
        "9",
        "three notches, three rows each: {facts:?}"
    );
    assert_eq!(
        fact(&facts, "clicked.open"),
        "true",
        "a click inside keeps it"
    );
    assert_eq!(
        fact(&facts, "clicked.scroll"),
        "9",
        "and changes nothing in it"
    );
    assert_eq!(
        fact(&facts, "moved.open"),
        "false",
        "motion out of range closes it"
    );
    assert_eq!(fact(&facts, "disconnect"), "");
}

/// The one placement rule (E7d.3), inherited: with no room below the
/// caret's line the popup goes above it; with no room right of the
/// anchor it is pulled left to the window's edge.
#[test]
fn e8_3_the_gpu_popup_flips_above_near_the_bottom_and_clamps_right() {
    // Thirty-five lines, more than a 600-pixel window shows, so walking
    // to the last puts it at the bottom; the last line indents its word
    // fifty columns, right of centre but short of the wrap (the minimap
    // narrows the text area), and the fake's default hover is wide
    // enough that a box starting there would pass the window's edge.
    let mut source = String::new();
    for i in 0..34 {
        let _ = writeln!(source, "let line_{i} = {i};");
    }
    let _ = writeln!(source, "{}tail", " ".repeat(50));
    let (daemon, _td) = daemon_on(&source, "hover", &[], "");
    let mut script = String::new();
    for _ in 0..34 {
        script.push_str("key:C-n ");
    }
    script.push_str("key:C-e key:C-b ask:h report:bottom");
    let Some(facts) = run_popup_probe(&daemon, &script, "flip.report") else {
        return;
    };
    assert_eq!(fact(&facts, "bottom.open"), "true", "{facts:?}");
    let [x, y, w, h] = rect(&facts, "bottom");
    let anchor: Vec<f32> = fact(&facts, "bottom.anchor_px")
        .split(',')
        .map(|n| n.parse().expect("anchor number"))
        .collect();
    let band_top = num(&facts, "bottom.band_top");
    assert!(
        anchor[1] + anchor[2] + h > band_top,
        "the setup must leave no room below: {facts:?}"
    );
    assert_eq!(
        fact(&facts, "bottom.above"),
        "true",
        "flipped above: {facts:?}"
    );
    assert!(
        y + h <= anchor[1] + 0.5,
        "its bottom on the line's top: {facts:?}"
    );
    assert!(
        anchor[0] + w > 900.0,
        "the setup must leave no room right: {facts:?}"
    );
    assert!(
        (x + w - 900.0).abs() < 0.5,
        "pulled left to the edge: {facts:?}"
    );
}

/// Themed through `ui.popup`: a background the theme sets is the one
/// the frame paints, and with none set the frontend's own default is.
#[test]
fn e8_3_the_gpu_popup_paints_the_ui_popup_face() {
    let (daemon, _td) = daemon_on(
        "fn main() {}\n",
        "hover",
        &[],
        "pmacs.theme.set { [\"ui.popup\"] = { fg = { 255, 255, 255 }, bg = { 200, 0, 0 } } }",
    );
    let Some(themed) = run_popup_probe(&daemon, "ask:h report:themed", "themed.report") else {
        return;
    };
    let px: Vec<u32> = fact(&themed, "themed.pixel")
        .split(',')
        .map(|n| n.parse().expect("pixel"))
        .collect();
    assert!(
        px[0] > 150 && px[1] < 60 && px[2] < 60,
        "the ui.popup background paints the box: {themed:?}"
    );
    drop(daemon);

    let (daemon, _td) = daemon_on("fn main() {}\n", "hover", &[], "");
    let Some(plain) = run_popup_probe(&daemon, "ask:h report:plain", "plain.report") else {
        return;
    };
    let px: Vec<u32> = fact(&plain, "plain.pixel")
        .split(',')
        .map(|n| n.parse().expect("pixel"))
        .collect();
    assert!(
        px[0] < 150 && px[0].abs_diff(px[2]) < 40,
        "unset, the frontend's own dark default: {plain:?}"
    );
}

/// The signature popup on the GPU: `C-c s` inside a call opens it,
/// anchored at the call's `(`, carrying the signature's label.
#[test]
fn e8_3_the_gpu_shows_the_signature_popup() {
    let (daemon, _td) = daemon_on("fn main() { let value = 1; }\n", "sighelp", &[], "");
    let mut script = String::new();
    for _ in 0..8 {
        script.push_str("key:C-f ");
    }
    script.push_str("ask:s report:sig");
    let Some(facts) = run_popup_probe(&daemon, &script, "sig.report") else {
        return;
    };
    assert_eq!(fact(&facts, "sig.open"), "true", "{facts:?}");
    assert_eq!(fact(&facts, "sig.kind"), "signature");
    assert_eq!(
        fact(&facts, "sig.first"),
        "Some(\"fn echo(name: &str, count: usize) -> String\")"
    );
    assert_eq!(fact(&facts, "sig.anchor"), "7");
}
