//! Full-grid resync acceptance — FG-INV at the real PTY boundary.
//!
//! A `CellDelta` with `full_grid: true` carries only the frame's
//! **non-default** cells: the producer diffs against a blank grid, so a
//! cell that should be blank produces no span. A consumer that applies
//! those spans to a surface it has not blanked keeps whatever was
//! underneath every blank cell.
//!
//! The grid TUI ignored the flag entirely. That was correct for exactly
//! one frame — the fresh-attach frame, which follows `Frontend::new`'s
//! `Clear` — and wrong for every resize after it, which follows
//! nothing. Zooming a terminal font left the reflowed previous frame
//! showing through.
//!
//! # Why this test is suffix-scoped, and why it settles first
//!
//! Startup emits its own `Clear(ClearType::All)`. A test that searches
//! the whole output therefore finds a clear **whether or not resize
//! handling works** — it passes against the broken build and proves
//! nothing. So the assertion is confined to bytes emitted strictly
//! after a mark.
//!
//! Taking that mark is the subtle half. Marking while startup is still
//! in flight puts startup's own clear *into the suffix* and recreates
//! the same false pass one step later. The mark is therefore anchored
//! to CONTENT, not to time: it sits just past the first painted byte of
//! the fixture, and both of startup's clears provably precede that —
//! `Frontend::new` clears before any frame exists, and the first frame
//! is itself a resync whose clear precedes its own spans.
//!
//! A time-based settle was tried first and does not work: a settled
//! pmacs screen emits per-frame bytes indefinitely, so "output stopped
//! growing" never becomes true.
//!
//! # What this proves, and what it does not
//!
//! The vterm suites assert on raw output bytes; there is no screen
//! model and no `vt100` / `termwiz` / `vte` dependency in the
//! workspace. This proves **pmacs emitted a blanking sequence at the
//! right moment**, not that the screen ended up correct. Closing that
//! gap needs a terminal emulator in the test dependencies, which is
//! deliberately out of this lane's scope.

use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

use common::pty::{PmacsPty, spawn_pmacs_in_pty};

/// CSI 2 J — erase the whole display.
const CLEAR_ALL: &[u8] = b"\x1b[2J";
/// SGR 0 — reset colors and attributes. `Clear` paints with the
/// *current* background, so a resync taken mid-style would otherwise
/// wash the screen in it.
const SGR_RESET: &[u8] = b"\x1b[0m";

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Wait until `needle` has been painted, and return the index just past
/// its first occurrence.
///
/// **That index is the mark, and it is anchored to CONTENT rather than
/// to time.** Startup's blanking provably precedes it: `Frontend::new`
/// clears before any frame exists, and the first frame is itself a
/// resync whose own clear precedes its own spans — so both clears are
/// behind the first byte of painted content, always, with no timing
/// assumption at all.
///
/// A time-based settle was tried first and is wrong here: a settled
/// pmacs screen keeps emitting per-frame bytes indefinitely (the vterm
/// suite records the same thing — "a settled screen emits empty diffs
/// forever"), so "output stopped growing" never becomes true and the
/// wait cannot distinguish a finished startup from a live one.
fn mark_after_first_paint(pty: &PmacsPty, needle: &[u8], timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let out = pty.output();
        if let Some(at) = find(&out, needle) {
            let mark = at + needle.len();
            assert!(
                contains(&out[..mark], CLEAR_ALL),
                "premise: startup DOES blank the host before painting — \
                 which is exactly why the assertion below must not be \
                 allowed to see startup's bytes"
            );
            return mark;
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

/// Wait for a blanking sequence to appear strictly after `mark`, and
/// return the suffix containing it.
///
/// Waiting for the *clear specifically* rather than for "any new
/// output" is what makes the timeout meaningful: pmacs emits per-frame
/// bytes regardless, so "output grew" would be satisfied instantly by
/// noise and the assertion would race the resize it is meant to
/// observe.
fn suffix_with_blank_after(pty: &PmacsPty, mark: usize, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        let out = pty.output();
        if out.len() > mark && contains(&out[mark..], CLEAR_ALL) {
            // Let the rest of the frame land so the ordering assertions
            // see the whole repaint, not its first fragment.
            std::thread::sleep(Duration::from_millis(200));
            return pty.output()[mark..].to_vec();
        }
        if Instant::now() >= deadline {
            let out = pty.output();
            let suffix = &out[mark.min(out.len())..];
            panic!(
                "FG-INV: the post-resize resync must blank the host, and \
                 no CSI 2 J appeared in the {} bytes emitted after the \
                 first painted frame. Suffix head: {:?}",
                suffix.len(),
                String::from_utf8_lossy(&suffix[..suffix.len().min(400)])
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Resizing a real PTY makes pmacs blank the host surface before it
/// repaints.
///
/// This is the user-visible bug: `Ctrl +/-` in a terminal changes the
/// font, the terminal reflows and emits `SIGWINCH`, and pmacs repaints
/// only its non-blank cells over content it never cleared.
#[test]
fn a_pty_resize_blanks_the_host_before_repainting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("marker.txt");
    // Distinctive, and long enough that the shrink below genuinely
    // changes what fits.
    std::fs::write(&file, "ZQXMARKERQZ\n".repeat(40)).expect("write fixture");

    let mut pty = spawn_pmacs_in_pty(
        &[file.to_str().expect("utf-8 path")],
        &[("HOME", dir.path()), ("XDG_CONFIG_HOME", dir.path())],
        24,
        80,
    );

    let mark = mark_after_first_paint(&pty, b"ZQXMARKERQZ", Duration::from_secs(20));

    // The zoom: same window, different cell geometry.
    pty.resize(12, 40).expect("resize the host PTY");

    let suffix = suffix_with_blank_after(&pty, mark, Duration::from_secs(20));

    let clear = find(&suffix, CLEAR_ALL).expect("waited for it above");
    let reset = find(&suffix, SGR_RESET).expect("the blank is style-reset first");
    assert!(
        reset < clear,
        "the reset must precede the clear — `Clear` paints with the \
         CURRENT background, so a resync taken mid-style washes the \
         screen in it"
    );

    // …and the blank is followed by the repaint it exists to precede.
    //
    // Scoped to AFTER the clear on purpose. The mark sits just past the
    // *first* painted marker line and the fixture repeats it, so the
    // suffix still opens with the tail of startup's own frame —
    // comparing the clear against those would compare it against paints
    // it was never supposed to precede.
    assert!(
        contains(&suffix[clear..], b"ZQXMARKERQZ"),
        "the resync must repaint after blanking. Without this the test \
         would pass on a resize that cleared the screen and painted \
         nothing — which is the other way to have a broken frame"
    );

    let _ = pty.write_input(b"\x18\x03"); // C-x C-c
    let _ = pty.wait_for_exit(Duration::from_secs(5));
}
