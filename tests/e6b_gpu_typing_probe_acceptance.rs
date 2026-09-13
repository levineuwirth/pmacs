//! E6b.3 --- the instrument, against a real daemon.
//!
//! Nothing in the tree drives a real window, and every witness before
//! this one read a settled state, so no probe could see a flash: the
//! semantic refinement vanishing from a GPU frame between a keystroke
//! and the server's answer, which is what the owner's pass on E6 saw.
//! `pmacs-gpu --headless-probe` with `PMACS_GPU_PROBE_TYPE_TEXT` types
//! through the production `App` dispatch and records the style spans
//! the GPU holds after every daemon message until the answer lands and
//! the stream is quiet; this file reads that trace.
//!
//! The daemon runs a fake server in `semantichold` mode: every word of
//! the document is one token in the marker face, and every answer is
//! held for a while, so the window a real server's round-trip opens is
//! open here for a known length and the frames inside it can be read.

mod common;

/// The marker face the fixture merges into the theme for the fake
/// server's token type; `pmacs-gpu`'s probe defaults to the same RGB.
#[cfg(feature = "crdt")]
const MARK_LUA: &str = "{ 0x7b, 0x1f, 0xa2 }";

/// How long the fake server holds each semantic-token answer.
#[cfg(feature = "crdt")]
const HOLD_MS: u64 = 400;

/// E6b.3 --- **no frame between the keystroke and the answer shows the
/// refinement absent where it stood before the edit, outside the
/// edited token.**
///
/// The document is `fn main() {\n    foo(bar);\n}\n`; the probe walks
/// the caret to byte 16, the start of `foo`, and types `x`. Before the
/// keystroke the marked spans are `fn` [0,2), `main` [3,7), `foo`
/// [16,19) and `bar` [20,23). Every frame from the keystroke to the
/// settle must carry `fn` and `main` where they were and `bar` one
/// byte to the right; `foo` is the edited token and is exempt. The
/// settle frame --- the server's answer for `xfoo` --- must mark
/// [16,20), which no shift can produce, so it is the answer and not a
/// translation that ends the window; and the trace must hold at least
/// one daemon `StyleSpans` frame between the keystroke and the settle,
/// the post-edit frame the flash lived in. Reverting E6b.2 fails the
/// first assertion on exactly that frame.
///
/// `crdt`-gated like every probe acceptance: a daemon built without it
/// advertises neither `crdt_replica` nor `semantic_render`. `gpu_binary`
/// lives inside the gate (`tests/gui_desktop_basics_acceptance.rs:30`'s
/// idiom) so the opt-out build carries no dead helper.
#[cfg(feature = "crdt")]
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real-daemon scenario: fixture, probe, and the trace read frame by frame"
)]
fn e6b_3_a_typed_character_never_blanks_the_refinement_on_the_gpu() {
    use std::path::{Path, PathBuf};

    /// One trace frame: `at_ms|origin|text_len|marked|spans`.
    struct Frame {
        at_ms: u64,
        origin: String,
        text_len: usize,
        marked: Vec<(u64, u64)>,
    }

    fn gpu_binary() -> PathBuf {
        Path::new(env!("CARGO_BIN_EXE_pmacs"))
            .parent()
            .expect("test binary directory")
            .join("pmacs-gpu")
    }

    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = gpu_binary();
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!(
            "skipping the GPU typing probe: {} is not built",
            binary.display()
        );
        return;
    }

    let fixture_dir = tempfile::TempDir::new().expect("fixture tempdir");
    let fixture = fixture_dir.path().join("a.rs");
    std::fs::write(&fixture, "fn main() {\n    foo(bar);\n}\n").expect("write a.rs");
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    // The scratch buffer is killed once the document is open, so the
    // daemon holds ONE CRDT-backed buffer when the probe attaches.
    // With two, the attach-time snapshot of each makes the frontend
    // declare a viewport for each, every declaration aligns the
    // daemon's window to the buffer it names, and the follow path
    // answers each alignment with a snapshot: a snapshot/viewport
    // ping-pong between the two buffers at hundreds of frames a
    // second, which no trace could be read through. Pre-existing and
    // reported in the handoff, not this phase's to fix.
    let init_lua = format!(
        "pmacs.lsp.config.rust = {{\n  command = {fake:?},\n  env = {{ PMACS_FAKE_LSP_MODE = 'semantichold', PMACS_FAKE_LSP_SEMANTIC_HOLD_MS = '{HOLD_MS}' }},\n}}\npmacs.theme.merge {{ namespace = {{ fg = {MARK_LUA} }} }}\npmacs.buffer.find_or_open({fixture:?})\nfor _, b in ipairs(pmacs.buffer.list()) do\n  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n"
    );
    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_lua,
    );

    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("gpu-typing.txt");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "x")
        .env("PMACS_GPU_PROBE_TYPE_AT", "16")
        .output()
        .expect("run the headless GPU typing probe");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU typing probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU typing probe: no wgpu adapter available");
        return;
    }
    let text = std::fs::read_to_string(&report).expect("probe report");
    eprintln!("--- probe report ---\n{text}--- end ---");
    let facts: std::collections::HashMap<String, String> = text
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect();
    let fact = |key: &str| -> &str {
        facts
            .get(key)
            .map_or_else(|| panic!("report carries no {key}"), String::as_str)
    };
    let frames: usize = fact("frames").parse().expect("frames");
    let trace: Vec<Frame> = (0..frames)
        .map(|i| {
            let line = fact(&format!("frame.{i}"));
            let parts: Vec<&str> = line.split('|').collect();
            assert_eq!(parts.len(), 5, "frame.{i} is malformed: {line}");
            let marked = if parts[3].is_empty() {
                Vec::new()
            } else {
                parts[3]
                    .split(';')
                    .map(|r| {
                        let (s, e) = r.split_once('-').expect("s-e");
                        (s.parse().expect("start"), e.parse().expect("end"))
                    })
                    .collect()
            };
            Frame {
                at_ms: parts[0].parse().expect("at_ms"),
                origin: parts[1].to_owned(),
                text_len: parts[2].parse().expect("text_len"),
                marked,
            }
        })
        .collect();
    let covers = |marked: &[(u64, u64)], start: u64, end: u64| -> bool {
        let mut at = start;
        for &(s, e) in marked {
            if s <= at && at < e {
                at = e;
                if at >= end {
                    return true;
                }
            }
        }
        at >= end
    };

    assert_eq!(
        fact("completion_observed"),
        "true",
        "the probe typed and saw the answer land"
    );
    assert_eq!(
        fact("typed_at_byte"),
        "16",
        "the caret walked to the start of `foo`"
    );
    let key_index = trace
        .iter()
        .position(|f| f.origin == "key")
        .expect("the trace holds the keystroke's frame");
    let before = &trace[key_index - 1];
    assert!(
        before.origin != "key",
        "the frame before the keystroke is a daemon frame"
    );
    assert_eq!(
        before.marked,
        vec![(0, 2), (3, 7), (16, 19), (20, 23)],
        "before the keystroke: fn, main, foo, bar in the marker face"
    );
    assert_eq!(before.text_len, 28);
    let typed_at_ms: u64 = fact("typed_at_ms").parse().expect("typed_at_ms");
    let settled_at_ms: u64 = fact("settled_at_ms").parse().expect("settled_at_ms");
    let settle_index = trace
        .iter()
        .position(|f| f.origin != "key" && f.at_ms >= settled_at_ms && covers(&f.marked, 16, 20))
        .expect("the trace holds the settle frame");

    // The acceptance: every frame from the keystroke through the settle
    // keeps the refinement outside the edited token.
    let expected_outside: [(u64, u64); 3] = [(0, 2), (3, 7), (21, 24)];
    for (i, frame) in trace
        .iter()
        .enumerate()
        .take(settle_index + 1)
        .skip(key_index)
    {
        assert_eq!(frame.text_len, 29, "frame {i} sees the typed byte");
        for &(s, e) in &expected_outside {
            assert!(
                covers(&frame.marked, s, e),
                "frame {i} ({} at {} ms) lost the refinement at [{s},{e}): {:?}",
                frame.origin,
                frame.at_ms,
                frame.marked
            );
        }
    }
    // The window was real: at least one daemon StyleSpans frame landed
    // between the keystroke and the settle, and the settle is the
    // server's answer (it marks `xfoo`, which no shift can).
    let daemon_frames_inside = trace[key_index + 1..settle_index]
        .iter()
        .filter(|f| f.origin == "StyleSpans")
        .count();
    assert!(
        daemon_frames_inside >= 1,
        "the post-edit StyleSpans frame must arrive before the answer; trace origins: {:?}",
        trace.iter().map(|f| f.origin.as_str()).collect::<Vec<_>>()
    );
    assert!(
        covers(&trace[settle_index].marked, 16, 20),
        "the settle frame marks `xfoo`"
    );
    assert!(
        settled_at_ms >= typed_at_ms + HOLD_MS / 2,
        "the answer was held: settled {settled_at_ms} ms, typed {typed_at_ms} ms"
    );
    eprintln!(
        "E6b.3 window: {} ms from the keystroke to the settle frame ({} frames inside, hold {HOLD_MS} ms)",
        fact("window_ms"),
        settle_index - key_index - 1
    );
}
