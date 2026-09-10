//! E3.1 --- the scrollbar's *gesture*, against a real daemon.
//!
//! Every other E3.1 row is a `State` row: the hit-test, the thumb's
//! interpolation, the page step, the clamp. Review round 1 measured
//! what that leaves uncovered by neutering the press dispatch in
//! `pmacs-gpu/src/main.rs` so `scrollbar_press_target`'s result could
//! never be `Some` --- the entire `pmacs-gpu` suite still reported
//! `363 passed; 0 failed`, and none of the binary's probe variables
//! could inject a pointer event, so no integration test could have
//! reached it either. E3.1's only user gesture had no witness at any
//! level.
//!
//! This file is one half of the answer and `pmacs-gpu`'s two
//! `e3_1_*_through_the_app` rows are the other. Here the document is a
//! real file opened by a real daemon, the pointer events go through the
//! production `App` dispatch inside `pmacs-gpu --headless-probe`, and
//! the viewport the gesture moved is read out of the probe's report.

mod common;

/// How many lines the fixture file carries. Far more than a 900x600
/// surface shows, so the thumb is small and the track above and below
/// it is bare --- which is what makes "press bare track" a press the
/// hit-test can classify as a page rather than a grab.
#[cfg(feature = "crdt")]
const FIXTURE_LINES: usize = 400;

/// E3.1 --- **click-to-page and thumb-drag, end to end.**
///
/// One real daemon opens a 400-line file in the attached frontend's own
/// window; `pmacs-gpu --headless-probe` then drives one gesture through
/// `route_event` and `apply_left_button` and reports the viewport's top
/// line before it, after it, and after the button came up and the
/// pointer moved again.
///
/// The third number is the one a reading cannot settle. `take()`
/// clearing `scrollbar_drag` and `is_some()` merely reporting it look
/// alike on the page and differ only in what the *next* motion does, so
/// the probe performs that next motion and the difference becomes a
/// number.
///
/// `crdt`-gated like every other probe acceptance in the tree: a daemon
/// built without it advertises neither `crdt_replica` nor
/// `semantic_render`, so the attach is refused for a reason that has
/// nothing to do with the scrollbar.
///
/// **`gpu_binary` lives INSIDE the gate, not beside it** --- the idiom
/// `tests/gui_desktop_basics_acceptance.rs:30` states, because a helper
/// at module scope whose only caller is gated is dead code in the
/// opt-out build and both CI legs that see that build deny warnings.
#[cfg(feature = "crdt")]
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real-daemon/real-document/real-dispatch scenario driven twice; \
              splitting it would give each gesture its own daemon for no gain"
)]
fn e3_1_a_scrollbar_press_and_a_thumb_drag_move_a_real_document() {
    use std::path::{Path, PathBuf};

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
            "skipping the GPU scrollbar probe: {} is not built",
            binary.display()
        );
        return;
    }

    // The fixture file lives in this test's own tempdir, not the
    // daemon's: the daemon's is `HOME` and `XDG_CONFIG_HOME` for the
    // run, and a document is not configuration.
    let fixture_dir = tempfile::TempDir::new().expect("fixture tempdir");
    let fixture = fixture_dir.path().join("tall.txt");
    let mut body = String::new();
    for line in 0..FIXTURE_LINES {
        use std::fmt::Write as _;
        let _ = writeln!(body, "scrollbar fixture line {line}");
    }
    std::fs::write(&fixture, &body).expect("write the fixture document");

    // Opened at daemon start, so the document is the instance's active
    // buffer before any frontend attaches and a fresh attach lands on
    // it. Asserted rather than assumed: the line-count row below fails
    // loudly if the frontend came up on a scratch buffer instead, which
    // is the one way this fixture could make the whole acceptance pass
    // vacuously --- a two-line buffer has no thumb, and a gesture that
    // presses an absent control moves nothing for a reason that has
    // nothing to do with the dispatch.
    let init_lua = format!("pmacs.buffer.find_or_open({fixture:?})\n");

    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_lua,
    );

    let run = |gesture: &str| -> Option<std::collections::HashMap<String, String>> {
        let report = daemon
            .socket_path()
            .parent()
            .expect("socket parent")
            .join(format!("gpu-scrollbar-{gesture}.txt"));
        let output = std::process::Command::new(&binary)
            .arg("--headless-probe")
            .arg(daemon.socket_path())
            .arg(&report)
            .env("PMACS_GPU_PROBE_SCROLLBAR", gesture)
            .output()
            .expect("run the headless GPU scrollbar probe");

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let no_adapter = output.status.code() == Some(3);
            assert!(
                no_adapter && !required,
                "headless GPU scrollbar probe failed (status {:?}):\n{stderr}",
                output.status.code()
            );
            eprintln!("skipping the GPU scrollbar probe: no wgpu adapter available");
            return None;
        }
        let text = std::fs::read_to_string(&report).expect("probe report");
        let facts = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        Some(facts)
    };

    // --- click-to-page ------------------------------------------------
    let Some(page) = run("page") else { return };
    let text = |facts: &std::collections::HashMap<String, String>| format!("{facts:#?}");
    let fact = |facts: &std::collections::HashMap<String, String>, key: &str| {
        facts.get(key).cloned().unwrap_or_default()
    };
    let number = |facts: &std::collections::HashMap<String, String>, key: &str| {
        fact(facts, key)
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("{key} is not a number in {}", text(facts)))
    };

    assert_eq!(
        fact(&page, "completion_observed"),
        "true",
        "a deadline-driven pass must not read as success:\n{}",
        text(&page)
    );
    assert_eq!(
        fact(&page, "scrollbar_gesture"),
        "page",
        "the probe must report which gesture the numbers belong to:\n{}",
        text(&page)
    );
    assert_eq!(
        fact(&page, "scrollbar_thumb_seen"),
        "true",
        "the fixture document must overflow the probe's surface, or the \
         gesture pressed an absent control:\n{}",
        text(&page)
    );
    // `>=`, not `==`: the file ends in a newline, so the frontend
    // counts one more line than the fixture writes. What this row is
    // for is the scratch buffer, which has two.
    assert!(
        number(&page, "scrollbar_line_count") >= FIXTURE_LINES,
        "the frontend must be on the fixture document, not a scratch \
         buffer:\n{}",
        text(&page)
    );
    assert_eq!(
        fact(&page, "scrollbar_pressed_below_thumb"),
        "true",
        "the press pixel must be bare track BELOW the thumb, or this row \
         measures a grab:\n{}",
        text(&page)
    );
    let visible = number(&page, "scrollbar_visible_lines");
    assert_eq!(
        number(&page, "scrollbar_top_before"),
        0,
        "a fresh attach starts at the top of the file:\n{}",
        text(&page)
    );
    assert_eq!(
        number(&page, "scrollbar_top_after"),
        visible,
        "a press on bare track below the thumb pages exactly one \
         screenful down:\n{}",
        text(&page)
    );
    assert_eq!(
        number(&page, "scrollbar_top_after_release"),
        visible,
        "click-to-page arms no drag, so the motion after the release \
         must move nothing:\n{}",
        text(&page)
    );

    // --- thumb drag ---------------------------------------------------
    let Some(drag) = run("drag") else { return };
    assert_eq!(
        fact(&drag, "completion_observed"),
        "true",
        "a deadline-driven pass must not read as success:\n{}",
        text(&drag)
    );
    assert_eq!(fact(&drag, "scrollbar_gesture"), "drag", "{}", text(&drag));
    let lines = number(&drag, "scrollbar_line_count");
    assert!(
        lines >= FIXTURE_LINES,
        "the frontend must be on the fixture document:\n{}",
        text(&drag)
    );
    assert_eq!(number(&drag, "scrollbar_top_before"), 0, "{}", text(&drag));
    assert_eq!(
        number(&drag, "scrollbar_top_after"),
        lines - 1,
        "dragging the thumb past the foot of the track scrolls to the \
         clamp --- `line_count - 1`, the same clamp the wheel reaches --- \
         and no further:\n{}",
        text(&drag)
    );
    assert_eq!(
        number(&drag, "scrollbar_top_after_release"),
        lines - 1,
        "the release must END the drag: a motion afterwards would drag \
         the viewport back toward the top of the file, and this number is \
         the only place that difference is observable:\n{}",
        text(&drag)
    );
}
