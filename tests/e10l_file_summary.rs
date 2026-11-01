//! E10l.2 --- #439's file opened through the GPU's route reaches a frame
//! with its highlighting, and the daemon keeps answering.
//!
//! The route is the daemon's frame producer driven by a GPU peer: a
//! semantic session over the daemon's socket, attached with the
//! capabilities and at the version `pmacs-gpu` offers and declaring a
//! viewport as it does after the snapshot (`tests/common/wire.rs`), which
//! is what makes the daemon run the minimap's whole-file summary for it
//! (`scoped_file_summary`; the attached TUI's hello says
//! `semantic_render: false`, so it never does). The GPU's own paint of
//! the spans it is sent is unchanged by E10l and not timed here: until
//! E10l the frame carrying the viewport's `StyleSpans` was written only
//! after the summary returned, and the summary walked every span for
//! every line, so on #439's file no highlighting ever arrived and the
//! daemon's main thread, which serves every frontend and buffer, ran for
//! minutes.
//!
//! The file is #439's, byte for byte what its generator writes
//! (185,000 entries, 18,070,563 bytes, 1,665,002 lines) in a release
//! build; a debug build, the gate's `--perf` and CI's `Perf budgets
//! (debug)`, writes 10,000 of its entries (935,563 bytes, 90,002 lines),
//! where the walk it replaced is about 8 × 10^9 steps, some 80 s at the
//! 9.9 ns a step a debug build took on the laptop, so the daemon still
//! answers nothing within the deadline, while the worker's whole-file
//! read stays well inside its 2 s limit on a slower machine (0.78 s
//! here at 20,000). `PMACS_E10L_ENTRIES` sets another count.
//!
//! The row is a deadline at a size the gate's default sweep should not
//! carry, so it runs with the budgets (D12): `#[ignore]`, run by exact
//! name by `scripts/perf-budgets`.
//! It prints each time with the load average beside it, and the daemon's
//! main thread's CPU time when the deadline passes.

mod common;

#[cfg(feature = "crdt")]
use std::time::{Duration, Instant};

/// The entries of #439's file a row writes: the issue's 185,000 in a
/// release build, 10,000 in a debug one.
const ENTRIES: usize = if cfg!(debug_assertions) {
    10_000
} else {
    185_000
};

/// How long the frame with the viewport's highlighting, and the
/// summary, may take from the viewport's declaration: generous, for a
/// debug build under the gate's load (1.6 s at 20,000 entries in debug,
/// 3.0 to 3.6 s for #439's file in release, on the laptop).
#[cfg(feature = "crdt")]
const DEADLINE: Duration = Duration::from_secs(30);

/// The file #439 names, `entries` objects long, byte for byte what its
/// generator writes: `json.dumps([{"id": i, "name": f"item {i}", "tags":
/// ["a", "b"], "value": i * 0.5} …], indent=1)` and print's newline
/// (checked against it by sha256 at 185,000 entries). `src/semantic_render.rs`'s
/// tests carry the same generator.
fn issue_439_json(entries: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(entries * 98 + 4);
    out.push('[');
    for i in 0..entries {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        #[allow(clippy::cast_precision_loss, reason = "entries stay far below 2^52")]
        let value = i as f64 * 0.5;
        write!(
            out,
            " {{\n  \"id\": {i},\n  \"name\": \"item {i}\",\n  \"tags\": [\n   \"a\",\n   \"b\"\n  ],\n  \"value\": {value:?}\n }}"
        )
        .expect("write");
    }
    out.push_str("\n]\n");
    out
}

/// The 1, 5 and 15 minute load averages.
#[cfg(feature = "crdt")]
fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".to_owned(),
        |s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

/// The user CPU time of `pid`'s main thread, in clock ticks of 1/100 s
/// (`/proc/<pid>/task/<pid>/stat`'s fourteenth field; Linux's `USER_HZ`
/// is 100), or `None` where `/proc` cannot say.
#[cfg(feature = "crdt")]
fn main_thread_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/stat")).ok()?;
    // The command name is parenthesized and may hold spaces; the fields
    // after it are space-separated, `utime` the twelfth of them.
    let after = stat.rsplit_once(')')?.1;
    after.split_whitespace().nth(11)?.parse().ok()
}

/// What a GPU peer saw after declaring its viewport, timed from the
/// declaration: the first `StyleSpans` for the buffer carrying a styled
/// span, and the first `FileStyleSummary` for it with its length and
/// how many of its lines are styled.
#[cfg(feature = "crdt")]
struct Seen {
    highlighted: Duration,
    summarized: Duration,
    summary_lines: usize,
    styled: usize,
}

/// Read `gpu`'s messages until both the highlighted frame and the
/// summary for `buffer` have come, failing at [`DEADLINE`] with the
/// daemon's main thread's CPU time; `what` names the file in the
/// failure.
#[cfg(feature = "crdt")]
fn await_highlighting(
    gpu: &common::wire::WireSession,
    buffer: pmacs_protocol::BufferId,
    pid: u32,
    what: &str,
) -> Seen {
    use pmacs_protocol::message::InstanceMessage;

    let declared = Instant::now();
    let mut highlighted: Option<Duration> = None;
    let mut summarized: Option<(Duration, usize, usize)> = None;
    loop {
        if let (Some(highlighted), Some((summarized, summary_lines, styled))) =
            (highlighted, summarized)
        {
            return Seen {
                highlighted,
                summarized,
                summary_lines,
                styled,
            };
        }
        let remaining = DEADLINE.saturating_sub(declared.elapsed());
        assert!(
            !remaining.is_zero(),
            "{what}: no {} within {DEADLINE:?} of the viewport; the daemon's main thread \
             had used {} ticks of CPU time (1/100 s), load {}",
            if highlighted.is_none() {
                "frame with the viewport's highlighting"
            } else {
                "file summary"
            },
            main_thread_ticks(pid).map_or_else(|| "unknown".to_owned(), |t| t.to_string()),
            loadavg()
        );
        let msg = match gpu
            .rx
            .recv_timeout(remaining.min(Duration::from_millis(200)))
        {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("{what}: the daemon closed the session before the highlighted frame")
            }
        };
        match msg {
            InstanceMessage::StyleSpans {
                buffer_id,
                segments,
                ..
            } if buffer_id == buffer
                && segments.iter().any(|segment| {
                    segment
                        .spans
                        .iter()
                        .any(|span| span.style != pmacs::cell::Style::default())
                }) =>
            {
                highlighted.get_or_insert(declared.elapsed());
            }
            InstanceMessage::FileStyleSummary {
                buffer_id,
                lines: summary,
                ..
            } if buffer_id == buffer => {
                let styled = summary
                    .iter()
                    .filter(|style| **style != pmacs::cell::Style::default())
                    .count();
                summarized.get_or_insert((declared.elapsed(), summary.len(), styled));
            }
            _ => {}
        }
    }
}

/// E10l.2: the #439 file, opened through the GPU's route, reaches a
/// frame with the viewport's highlighting and the summary within the
/// deadline, and a key after them is answered.
#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn e10l_a_large_json_file_reaches_a_highlighted_frame_in_the_gpu_route() {
    #[cfg(feature = "crdt")]
    {
        use common::daemon::TestDaemon;
        use common::wire::WireSession;
        use pmacs_protocol::message::{Key, Modifiers, PROTOCOL_VERSION};

        let entries = std::env::var("PMACS_E10L_ENTRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(ENTRIES);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        let text = issue_439_json(entries);
        if entries == 185_000 {
            assert_eq!(text.len(), 18_070_563, "#439's file, byte for byte");
        }
        std::fs::write(&file, &text).expect("write the file");
        // One summary entry a line, as `line_start_offsets` counts them:
        // the text after the last newline is a line of its own.
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        drop(text);

        let daemon = TestDaemon::holding(&file);
        let pid = daemon.pid();
        let load_before = loadavg();
        let started = Instant::now();
        let mut gpu =
            WireSession::semantic_with_patience(&daemon, PROTOCOL_VERSION, None, None, DEADLINE);
        let buffer = *gpu
            .bootstrap_snapshots()
            .first()
            .expect("the bootstrap snapshots the file");
        let attached = started.elapsed();
        gpu.bootstrap.clear();
        gpu.declare_viewport(buffer);
        let what = format!("{entries} entries ({lines} lines)");
        let Seen {
            highlighted,
            summarized,
            summary_lines,
            styled,
        } = await_highlighting(&gpu, buffer, pid, &what);
        assert_eq!(summary_lines, lines, "the summary has one style a line");
        assert!(
            styled > lines / 2,
            "most of the summary's lines are styled ({styled} of {lines}): the whole-file \
             spans reached it"
        );

        // The daemon keeps answering: `C-n` from the top moves the cursor
        // to the second line, which starts at byte 2 (`[\n`).
        let key = Instant::now();
        gpu.key(Key::Char('n'), Modifiers::CTRL);
        gpu.cursor_at(buffer, 2);
        let answered = key.elapsed();
        let load_after = loadavg();
        eprintln!(
            "e10l gpu route, {entries} entries ({} bytes, {lines} lines): attached in {attached:?}; \
             from the viewport, highlighted frame {highlighted:?}, summary {summarized:?} \
             ({styled} of {summary_lines} lines styled); a key answered in {answered:?}; \
             main thread {} ticks of CPU; load {load_before} before, {load_after} after",
            std::fs::metadata(&file).map_or(0, |m| m.len()),
            main_thread_ticks(pid).map_or_else(|| "unknown".to_owned(), |t| t.to_string()),
        );
    }
    #[cfg(not(feature = "crdt"))]
    {
        let _ = ENTRIES;
        let _ = issue_439_json;
        eprintln!("skipping: a GPU session is a crdt build's");
    }
}
