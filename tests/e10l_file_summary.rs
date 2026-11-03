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
//!
//! Two rows of E10l's fix round 2 (the owner's GUI pass: the minimap
//! lost its colors on #439's file while the text kept its highlighting)
//! run in the default sweep on a small file of the same shape: a
//! whole-file read that fails after a good summary sends nothing, so the
//! frontend keeps that summary until a read succeeds; and a buffer with
//! no styles still gets its plain summary.

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

/// A daemon holding `file` alone, with no language server (so no
/// completion popup takes a key and no diagnostics publish moves the
/// summary's key), its parses traced to `trace` (`PMACS_E7I_TRACE`).
#[cfg(feature = "crdt")]
fn holding_traced(file: &std::path::Path, trace: &std::path::Path) -> common::daemon::TestDaemon {
    let init_lua = format!(
        "pmacs.lsp.config = {{}}\npmacs.buffer.find_or_open({:?})\nfor _, b in ipairs(pmacs.buffer.list()) do\n  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n",
        file.display().to_string()
    );
    common::daemon::TestDaemon::spawn_with_env_and_init(
        &[("PMACS_E7I_TRACE", trace.to_str().expect("a UTF-8 path"))],
        &init_lua,
    )
}

/// The worker of each parse the trace records, in order (`"unit":"pid N"`
/// on its `parsed` event).
#[cfg(feature = "crdt")]
fn parsed_units(trace: &std::path::Path) -> Vec<u32> {
    std::fs::read_to_string(trace)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("\"event\":\"parsed\""))
        .filter_map(|line| {
            line.split("\"unit\":\"pid ")
                .nth(1)?
                .split('"')
                .next()?
                .parse()
                .ok()
        })
        .collect()
}

/// `kill <sig> <pid>`; whether it was delivered (`-0`: whether `pid` lives).
#[cfg(feature = "crdt")]
fn signal(sig: &str, pid: u32) -> bool {
    std::process::Command::new("kill")
        .args([sig, &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Every `FileStyleSummary` for `buffer` that comes within `wait`, as
/// `(lines, styled lines)`, stopping at the first when `first` is set.
#[cfg(feature = "crdt")]
fn summaries_within(
    gpu: &common::wire::WireSession,
    buffer: pmacs_protocol::BufferId,
    wait: Duration,
    first: bool,
) -> Vec<(usize, usize)> {
    use pmacs_protocol::message::InstanceMessage;

    let started = Instant::now();
    let mut seen = Vec::new();
    while (seen.is_empty() || !first) && started.elapsed() < wait {
        match gpu.rx.recv_timeout(Duration::from_millis(20)) {
            Ok(InstanceMessage::FileStyleSummary {
                buffer_id, lines, ..
            }) if buffer_id == buffer => {
                let styled = lines
                    .iter()
                    .filter(|style| **style != pmacs::cell::Style::default())
                    .count();
                seen.push((lines.len(), styled));
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the daemon closed the session")
            }
        }
    }
    seen
}

/// Stop `worker` (`SIGSTOP`) and wait for the daemon to discard it, which
/// it does when a read of the tree it holds runs past `READ_HARD`; the
/// summaries for `buffer` sent until then and three seconds after, the
/// failed read's. Fails when nothing read from the worker within 20 s, so
/// nothing was made to fail.
#[cfg(feature = "crdt")]
fn stopped_until_discarded(
    gpu: &common::wire::WireSession,
    buffer: pmacs_protocol::BufferId,
    worker: u32,
) -> Vec<(usize, usize)> {
    assert!(signal("-STOP", worker), "stop the worker {worker}");
    let stopped = Instant::now();
    let mut after = Vec::new();
    let gone = loop {
        after.extend(summaries_within(
            gpu,
            buffer,
            Duration::from_millis(100),
            false,
        ));
        if !signal("-0", worker) {
            break true;
        }
        if stopped.elapsed() > Duration::from_secs(20) {
            break false;
        }
    };
    if !gone {
        signal("-CONT", worker);
    }
    assert!(
        gone,
        "the summary's read never reached the stopped worker {worker} within 20 s: \
         nothing was made to fail (summaries meanwhile {after:?})"
    );
    after.extend(summaries_within(gpu, buffer, Duration::from_secs(3), false));
    after
}

/// The caret from byte 0 into `"item 0"` on the fourth line (byte 27,
/// after its quote), where a typed letter keeps the JSON valid and every
/// line's style.
#[cfg(feature = "crdt")]
fn walk_into_the_first_name(gpu: &mut common::wire::WireSession, buffer: pmacs_protocol::BufferId) {
    use pmacs_protocol::message::{Key, Modifiers};

    for (key, byte) in [('n', 2), ('n', 5), ('n', 16)] {
        gpu.key(Key::Char(key), Modifiers::CTRL);
        gpu.cursor_at(buffer, byte);
    }
    for byte in 17..=27 {
        gpu.key(Key::Char('f'), Modifiers::CTRL);
        gpu.cursor_at(buffer, byte);
    }
}

/// E10l fix round 2 (the owner's GUI pass on #439's file): a whole-file
/// read that fails after a good summary sends nothing, so the frontend
/// keeps the last good summary; the read is not taken again until the
/// summary's key changes, and the next read that succeeds, after an edit,
/// sends the next. The read is made to fail as a loaded machine fails it:
/// the worker holding the edit's tree is stopped (`SIGSTOP`) before the
/// summary asks it for the whole file, so the read runs past `READ_HARD`
/// and the daemon discards the worker. Before the fix a failed read was a
/// summary of no styles, sent over the good one: the minimap lost its
/// colors while the text kept its own, whose spans came with the parse.
/// 300 of #439's entries (2,703 lines, 26,863 bytes) put the file past
/// what a parse returns spans for (the viewport and 4096 bytes on each
/// side).
#[test]
fn e10l_f2_a_failed_whole_file_read_keeps_the_last_good_summary() {
    #[cfg(feature = "crdt")]
    {
        use common::wire::WireSession;
        use pmacs_protocol::message::{Key, Modifiers, PROTOCOL_VERSION};

        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("small.json");
        let trace = dir.path().join("trace.jsonl");
        let text = issue_439_json(300);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        assert!(text.len() > 3 * 4096, "the file is past the parse's spans");
        std::fs::write(&file, &text).expect("write the file");

        let daemon = holding_traced(&file, &trace);
        let mut gpu =
            WireSession::semantic_with_patience(&daemon, PROTOCOL_VERSION, None, None, DEADLINE);
        let buffer = *gpu
            .bootstrap_snapshots()
            .first()
            .expect("the bootstrap snapshots the file");
        gpu.bootstrap.clear();
        gpu.declare_viewport(buffer);
        let good = await_highlighting(&gpu, buffer, daemon.pid(), "300 entries");
        assert_eq!(good.summary_lines, lines, "one style a line");
        assert!(
            good.styled > lines / 2,
            "the first summary is a good one ({} of {lines} lines styled)",
            good.styled
        );

        walk_into_the_first_name(&mut gpu, buffer);
        let parses = parsed_units(&trace).len();
        assert!(parses >= 1, "the trace records the file's parse");
        gpu.type_str("x");
        gpu.cursor_at(buffer, 28);

        // The edit's parse, and the worker holding its tree, stopped before
        // the summary reads it, a second after the parse installs.
        let asked = Instant::now();
        let worker = loop {
            if let Some(&pid) = parsed_units(&trace).get(parses) {
                break pid;
            }
            assert!(
                asked.elapsed() < Duration::from_secs(20),
                "no parse of the edit in the trace"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        let after = stopped_until_discarded(&gpu, buffer, worker);
        assert!(
            after.is_empty(),
            "after the whole-file read failed the daemon sent {after:?} ((lines, styled \
             lines)) over the good summary's {} styled: a failed read is not a file without \
             styles",
            good.styled
        );
        // Nor is the read retried with nothing changed: a read of that tree
        // would first re-parse its text in a fresh worker, which the trace
        // records.
        let traced = std::fs::read_to_string(&trace).unwrap_or_default();
        assert!(
            !traced.contains("\"event\":\"reestablished\""),
            "the failed read was taken again with nothing changed, its tree re-parsed in a \
             fresh worker: {}",
            traced
                .lines()
                .filter(|line| line.contains("reestablished"))
                .collect::<Vec<_>>()
                .join("; ")
        );

        // The next read that succeeds sends the next summary: a line opened
        // at the end of the fourth line, which parses in a fresh worker.
        gpu.key(Key::Char('e'), Modifiers::CTRL);
        gpu.cursor_at(buffer, 36);
        gpu.key(Key::Enter, Modifiers::NONE);
        let next = summaries_within(&gpu, buffer, DEADLINE, true);
        assert!(
            next.first()
                .is_some_and(|&(n, styled)| n == lines + 1 && styled > lines / 2),
            "after the failed read, the next edit's summary is read and sent with its \
             styles ({next:?}; {} lines, {} styled before)",
            lines,
            good.styled
        );
    }
}

/// E10l fix round 2: a buffer with no styles still gets its summary, every
/// line plain: a JSON file whose lines no capture styles, read from its
/// worker past the parse's own spans, and a text file with no grammar.
#[test]
fn e10l_f2_a_buffer_with_no_styles_still_gets_its_plain_summary() {
    #[cfg(feature = "crdt")]
    {
        use common::wire::WireSession;
        use pmacs_protocol::message::PROTOCOL_VERSION;

        let dir = tempfile::TempDir::new().expect("tempdir");
        for (name, text) in [
            ("brackets.json", format!("[{}]\n", "\n".repeat(12_000))),
            ("notes.txt", "plain words on a line\n".repeat(12_000)),
        ] {
            let file = dir.path().join(name);
            let trace = dir.path().join(format!("{name}.trace"));
            let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
            std::fs::write(&file, &text).expect("write the file");
            let daemon = holding_traced(&file, &trace);
            let mut gpu = WireSession::semantic_with_patience(
                &daemon,
                PROTOCOL_VERSION,
                None,
                None,
                DEADLINE,
            );
            let buffer = *gpu
                .bootstrap_snapshots()
                .first()
                .expect("the bootstrap snapshots the file");
            gpu.bootstrap.clear();
            gpu.declare_viewport(buffer);
            let seen = summaries_within(&gpu, buffer, DEADLINE, true);
            assert_eq!(
                seen,
                vec![(lines, 0)],
                "{name}: one summary of {lines} plain lines (lines, styled lines)"
            );
        }
    }
}
