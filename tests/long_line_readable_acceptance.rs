//! The originating report, at the real PTY boundary.
//!
//! > "long lines need to either wrap somehow or be scrollable. Haven't
//! > tried this in GUI, but in TUI, a line that extends off screen
//! > cannot be read in full in any way."
//!
//! Every other test in this lane checks a mechanism: that the mode
//! resolves, that it reaches the viewport, that the classifier agrees
//! across frontends. This one checks the **complaint** — that the end of
//! a long line reaches a terminal at all — and it is deliberately the
//! only test here that runs the shipped binary against a real PTY.
//!
//! # Why this is not redundant with `line_wrap_acceptance.rs`
//!
//! That suite reconstructs rows from emitted `CellDelta` spans, which is
//! the right granularity for asserting *where* text lands. But it drives
//! `RenderState` in-process, so everything between the grid and a
//! terminal — the frontend, the ANSI writer, startup, the real terminal
//! size — is assumed rather than exercised. The defect being fixed was
//! reported from a terminal, so at least one test should end in one.
//!
//! # What it asserts, and why that is the honest assertion
//!
//! The vterm suites here assert on raw output bytes; there is no screen
//! model and no `vt100` / `termwiz` / `vte` dependency in the workspace
//! (`full_grid_resync_acceptance.rs` records the same limit). So this
//! proves **the tail of the line was written to the terminal**, not that
//! it occupies the row a human would point at. That is nevertheless the
//! whole of the original report: under truncation those bytes are never
//! emitted at all, because there is no column past the edge to paint
//! them into and no horizontal scrolling to reveal them.
//!
//! Hence the `truncate` control below. Without it the wrap assertion
//! would be satisfied by anything that happened to echo the fixture, and
//! the pair is what makes the marker discriminating.

use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

use common::pty::{PmacsPty, spawn_pmacs_in_pty};

/// Painted first, well within any terminal width — the "pmacs got this
/// far" anchor that keeps an absence assertion from passing vacuously.
const HEAD: &[u8] = b"HEADZQX";
/// Painted only if something puts it on a row: it sits ~200 columns into
/// a single source line, past the right edge of the 80-column PTY below.
const TAIL: &[u8] = b"TAILZQX";

/// One source line, far wider than the terminal, marked at both ends.
fn fixture() -> String {
    format!(
        "{}{}{}\n",
        String::from_utf8_lossy(HEAD),
        "-".repeat(200),
        String::from_utf8_lossy(TAIL),
    )
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Block until `needle` appears in pmacs's output.
///
/// Content-anchored rather than timed, for the reason
/// `full_grid_resync_acceptance.rs` spells out: a settled pmacs screen
/// emits per-frame bytes forever, so "output stopped growing" never
/// becomes true and cannot mark the end of startup.
fn wait_for(pty: &PmacsPty, needle: &[u8], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if contains(&pty.output(), needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pmacs never painted {:?} within {timeout:?}; emitted {} bytes",
            String::from_utf8_lossy(needle),
            pty.output().len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Spawn pmacs over the fixture in an 80x24 PTY, with an isolated
/// config root that optionally carries an `init.lua`.
fn spawn(dir: &std::path::Path, init_lua: Option<&str>) -> PmacsPty {
    let file = dir.join("longline.txt");
    std::fs::write(&file, fixture()).expect("write fixture");
    if let Some(body) = init_lua {
        let cfg = dir.join("pmacs");
        std::fs::create_dir_all(&cfg).expect("create config dir");
        std::fs::write(cfg.join("init.lua"), body).expect("write init.lua");
    }
    spawn_pmacs_in_pty(
        &[file.to_str().expect("utf-8 path")],
        &[("HOME", dir), ("XDG_CONFIG_HOME", dir)],
        24,
        80,
    )
}

fn quit(pty: &mut PmacsPty) {
    let _ = pty.write_input(b"\x18\x03"); // C-x C-c
    let _ = pty.wait_for_exit(Duration::from_secs(5));
}

/// The report, closed: opening a file whose line runs off the right edge
/// puts the end of that line on the terminal.
#[test]
fn the_end_of_a_long_line_reaches_the_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut pty = spawn(dir.path(), None);

    wait_for(&pty, HEAD, Duration::from_secs(20));
    wait_for(&pty, TAIL, Duration::from_secs(20));

    quit(&mut pty);
}

/// The control that makes the marker above mean something: pinned to
/// `truncate`, the same fixture in the same terminal never emits the
/// tail.
///
/// This is also the honest statement of what `truncate` costs today.
/// Those bytes are not merely off-screen, they are unreachable — there
/// is no horizontal scrolling yet, which is why `wrap` is the default
/// and why `ui.toggle-line-wrap` says so when it turns wrapping off.
#[test]
fn truncate_leaves_the_end_of_the_line_unreachable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut pty = spawn(
        dir.path(),
        Some("pmacs.config.set('ui.line-wrap', 'truncate')\n"),
    );

    // The anchor first: without it, "TAIL never appeared" would also be
    // satisfied by pmacs failing to start.
    wait_for(&pty, HEAD, Duration::from_secs(20));
    // Head and tail would be painted in the SAME frame if wrapping were
    // on, so this settle is generous rather than load-bearing.
    std::thread::sleep(Duration::from_millis(750));

    assert!(
        !contains(&pty.output(), TAIL),
        "truncate must clip at the edge — emitting the tail would mean \
         the mode reached the resolver but not the renderer, which is \
         exactly the defect the rendered witnesses in \
         line_wrap_acceptance.rs guard from the other side"
    );

    quit(&mut pty);
}
