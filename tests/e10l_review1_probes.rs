//! E10l review 1's probes: states a GUI user reaches on #439's file
//! that the handoff's witnesses did not choose, through the same route
//! as `tests/e10l_file_summary.rs` (the daemon's frame producer driven
//! by a peer attached with `pmacs-gpu`'s capabilities and version).
//! Committed by E10l's fix round 1, rustfmt aside as the review left
//! them, with what each found beside its question.
//!
//! - P1: typing into the file, then stopping. The handoff stated by
//!   reading that the summary is rebuilt a second after typing stops and
//!   at most ten seconds behind the first edit it has not described,
//!   holding the main thread about 1.5 s each time. Measured (review 1's
//!   Medium 1): no summary is rebuilt while typing, since every parse of
//!   this file runs cold in a fresh worker and the summary waits for an
//!   installed parse; the holds of 3.5 to 3.9 s while typing are the JSON
//!   server's completion answers, about 41.5 MB each, converted into Lua
//!   on the main thread; after typing stops the summary runs, about 1.3 s
//!   on the main thread, and again on each diagnostics publish.
//! - P2: a file whose whole-file span answer runs past `READ_HARD` (2 s).
//!   The handoff stated by reading that the read is abandoned, the unit
//!   buried, the summary plain, and that the next read the fetched spans
//!   do not cover starts a fresh unit and parses the whole text while
//!   the main thread waits. Measured: the main thread waits 2.00 s on
//!   the read and 2.13 s on the reestablish, whose parse runs in the
//!   fresh worker rather than on the main thread.
//! - P3: a second GUI window on the file. Each semantic session keeps
//!   its own summary cache (`SemanticRenderState::new(fid)` per session,
//!   `src/daemon.rs`), so each runs the summary. Measured: the second
//!   reuses the whole-file read the first fetched, paying the mapping,
//!   the flatten and the pass alone.
//! - P4: scrolling after the summary; P6: the GPU binary on the file.
//!
//! - P5: a file whose summary cannot fit the transport's frame (review
//!   1's Medium 2). Measured at `f0754a4`: computed on the main thread,
//!   refused by the transport, and recomputed after a pause, about 2 s
//!   each time. Since the fix the producer reads nothing for it, and P5
//!   is that fix's witness: it fails at `f0754a4` on the hold.
//!
//! Wall-clock probes: `#[ignore]`, run by hand in release (the file's
//! 185,000 entries, P2's 370,000 and P5's 270,000;
//! `PMACS_E10L_R1_ENTRIES` sets another count). Each prints its figures
//! with the load average beside them. The assertions are what the North
//! star promises, "the daemon keeps answering": every key answered
//! within `ANSWER_CEILING`; P5's, that no key after a pause waits past
//! `SUMMARY_HOLD`.

mod common;

#[cfg(feature = "crdt")]
use std::time::{Duration, Instant};

#[cfg(feature = "crdt")]
use pmacs_protocol::message::{FrontendEvent, InstanceMessage, Key, Modifiers, PROTOCOL_VERSION};

/// A key not answered within this is a daemon that stopped answering.
#[cfg(feature = "crdt")]
const ANSWER_CEILING: Duration = Duration::from_secs(8);

/// How long the first highlighted frame and summary may take.
#[cfg(feature = "crdt")]
const FIRST_FRAME: Duration = Duration::from_mins(1);

/// #439's generator, as `tests/e10l_file_summary.rs` carries it.
#[cfg(feature = "crdt")]
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

#[cfg(feature = "crdt")]
fn entries_or(default: usize) -> usize {
    std::env::var("PMACS_E10L_R1_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(feature = "crdt")]
fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".to_owned(),
        |s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

/// The daemon's `pmacs-parse-unit` children, by pid.
#[cfg(feature = "crdt")]
fn parse_units(daemon: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some((head, tail)) = stat.rsplit_once(')') else {
            continue;
        };
        let comm = head.split_once('(').map_or("", |(_, c)| c);
        let ppid: Option<u32> = tail.split_whitespace().nth(1).and_then(|p| p.parse().ok());
        if ppid == Some(daemon) && comm.starts_with("pmacs-parse") {
            out.push(pid);
        }
    }
    out.sort_unstable();
    out
}

/// One GPU-route session on the file, with a log of what it was sent.
#[cfg(feature = "crdt")]
struct Peer {
    gpu: common::wire::WireSession,
    buffer: pmacs_protocol::BufferId,
    epoch: Instant,
    /// `(since epoch, what)`: every `FileStyleSummary` (its styled lines)
    /// and every `StyleSpans` carrying a styled span (its first byte).
    log: Vec<(Duration, String)>,
    summaries: Vec<(Duration, usize, usize)>,
}

#[cfg(feature = "crdt")]
impl Peer {
    fn attach(daemon: &common::daemon::TestDaemon) -> Self {
        let mut gpu = common::wire::WireSession::semantic_with_patience(
            daemon,
            PROTOCOL_VERSION,
            None,
            None,
            FIRST_FRAME,
        );
        let buffer = *gpu
            .bootstrap_snapshots()
            .first()
            .expect("the bootstrap snapshots the file");
        gpu.bootstrap.clear();
        Self {
            gpu,
            buffer,
            epoch: Instant::now(),
            log: Vec::new(),
            summaries: Vec::new(),
        }
    }

    fn viewport(&mut self, start: u64, end: u64) {
        pmacs_protocol::transport::write_message(
            &mut self.gpu.stream,
            &FrontendEvent::Viewport {
                frontend_id: self.gpu.fid,
                buffer_id: self.buffer,
                visible: pmacs_protocol::ByteRange { start, end },
                generation: 0,
            },
        )
        .expect("declare a viewport");
    }

    /// Read one message, logging the ones this file's probes time.
    fn next(&mut self, wait: Duration) -> Option<InstanceMessage> {
        let msg = match self.gpu.rx.recv_timeout(wait) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the daemon closed the session")
            }
        };
        let at = self.epoch.elapsed();
        if std::env::var_os("E10L_R1_VERBOSE").is_some() {
            match &msg {
                InstanceMessage::StyleSpans {
                    buffer_id,
                    generation,
                    full,
                    segments,
                } => {
                    let styled: usize = segments
                        .iter()
                        .map(|s| {
                            s.spans
                                .iter()
                                .filter(|sp| sp.style != pmacs::cell::Style::default())
                                .count()
                        })
                        .sum();
                    let ranges: Vec<(u64, u64)> = segments
                        .iter()
                        .map(|s| (s.range.start, s.range.end))
                        .collect();
                    eprintln!(
                        "verbose {:.3}s StyleSpans buffer {buffer_id:?} gen {generation} full {full} \
                         segments {ranges:?} styled {styled}",
                        at.as_secs_f64()
                    );
                }
                InstanceMessage::FileStyleSummary { lines, .. } => {
                    eprintln!(
                        "verbose {:.3}s FileStyleSummary {} lines",
                        at.as_secs_f64(),
                        lines.len()
                    );
                }
                InstanceMessage::CursorByte { .. } | InstanceMessage::StatusFacts { .. } => {}
                other => {
                    let mut name = format!("{other:?}");
                    name.truncate(name.find([' ', '(', '{']).unwrap_or(name.len()));
                    eprintln!("verbose {:.3}s {name}", at.as_secs_f64());
                }
            }
        }
        match &msg {
            InstanceMessage::FileStyleSummary {
                buffer_id, lines, ..
            } if *buffer_id == self.buffer => {
                let styled = lines
                    .iter()
                    .filter(|s| **s != pmacs::cell::Style::default())
                    .count();
                self.summaries.push((at, lines.len(), styled));
                // The message's own frame, as the daemon wrote it: the
                // transport refuses a payload past `MAX_FRAME_BYTES`.
                let mut frame = Vec::new();
                let encoded = pmacs_protocol::transport::write_message(&mut frame, &msg)
                    .map_or_else(
                        |e| format!("not encodable: {e}"),
                        |()| frame.len().to_string(),
                    );
                self.log.push((
                    at,
                    format!(
                        "summary {styled}/{} styled, {encoded} bytes framed (cap {})",
                        lines.len(),
                        pmacs_protocol::transport::MAX_FRAME_BYTES
                    ),
                ));
            }
            InstanceMessage::StyleSpans {
                buffer_id,
                segments,
                ..
            } if *buffer_id == self.buffer => {
                if let Some(first) = segments
                    .iter()
                    .flat_map(|s| s.spans.iter())
                    .find(|s| s.style != pmacs::cell::Style::default())
                {
                    self.log
                        .push((at, format!("styled spans from {}", first.range.start)));
                }
            }
            _ => {}
        }
        Some(msg)
    }

    /// Until `pick` accepts a message or `ceiling` passes; the time taken,
    /// or `None` at the ceiling.
    fn until(
        &mut self,
        ceiling: Duration,
        mut pick: impl FnMut(&InstanceMessage) -> bool,
    ) -> Option<Duration> {
        let t0 = Instant::now();
        loop {
            let left = ceiling.saturating_sub(t0.elapsed());
            if left.is_zero() {
                return None;
            }
            if let Some(msg) = self.next(left.min(Duration::from_millis(100)))
                && pick(&msg)
            {
                return Some(t0.elapsed());
            }
        }
    }

    /// Send `key` and time the `CursorByte` at `byte` that answers it.
    fn key_to(&mut self, key: Key, mods: Modifiers, byte: u64) -> Duration {
        let buffer = self.buffer;
        let t0 = Instant::now();
        self.gpu.key(key, mods);
        let took = self.until(ANSWER_CEILING, |m| {
            matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos }
                if *buffer_id == buffer && *byte_pos == byte)
        });
        took.unwrap_or_else(|| {
            panic!(
                "a key was not answered within {ANSWER_CEILING:?} (to byte {byte}, sent at {:?}); load {}",
                t0.duration_since(self.epoch),
                loadavg()
            )
        })
    }

    /// The first highlighted frame and the first summary, from now.
    fn first_frame(&mut self) -> (Duration, Duration) {
        let t0 = Instant::now();
        let (mut highlighted, mut summarized) = (None, None);
        while highlighted.is_none() || summarized.is_none() {
            assert!(
                t0.elapsed() < FIRST_FRAME,
                "no highlighted frame and summary within {FIRST_FRAME:?}; load {}",
                loadavg()
            );
            let before = self.summaries.len();
            if let Some(msg) = self.next(Duration::from_millis(100)) {
                if self.summaries.len() > before {
                    summarized.get_or_insert(t0.elapsed());
                }
                if let InstanceMessage::StyleSpans { segments, .. } = &msg
                    && segments.iter().any(|s| {
                        s.spans
                            .iter()
                            .any(|sp| sp.style != pmacs::cell::Style::default())
                    })
                {
                    highlighted.get_or_insert(t0.elapsed());
                }
            }
        }
        (highlighted.expect("set"), summarized.expect("set"))
    }

    /// Walk the caret from byte 0 to byte 27, inside `"item 0"` on the
    /// fourth line, where a typed letter keeps the JSON valid.
    fn walk_into_the_first_name(&mut self) {
        for (byte, key) in [(2, 'n'), (5, 'n'), (16, 'n')] {
            self.key_to(Key::Char(key), Modifiers::CTRL, byte);
        }
        for byte in 17..=27 {
            self.key_to(Key::Char('f'), Modifiers::CTRL, byte);
        }
    }
}

/// Latencies over a window: the largest, how many past `over`, and the
/// times (since `epoch`) of those.
#[cfg(feature = "crdt")]
fn describe(samples: &[(Duration, Duration)], over: Duration) -> String {
    let max = samples.iter().map(|(_, l)| *l).max().unwrap_or_default();
    let mut sorted: Vec<Duration> = samples.iter().map(|(_, l)| *l).collect();
    sorted.sort_unstable();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
    let held: Vec<String> = samples
        .iter()
        .filter(|(_, l)| *l > over)
        .map(|(at, l)| format!("{:.2}s:{:.0}ms", at.as_secs_f64(), l.as_secs_f64() * 1e3))
        .collect();
    format!(
        "{} keys, median {:.2} ms, max {:.0} ms, {} past {over:?} [{}]",
        samples.len(),
        median.as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3,
        held.len(),
        held.join(" ")
    )
}

/// P1: typing into #439's file through the GPU route, a key every
/// 100 ms for fifteen seconds, then the caret moved every 100 ms for six
/// seconds after the last key. Measures what the handoff states by
/// reading: the summary rebuilt during typing (at most `SUMMARY_MAX_LAG`
/// behind) and a second after it stops (`SUMMARY_QUIET`).
#[test]
#[ignore = "review probe; wall-clock, run by hand in release"]
fn p1_typing_into_the_439_file_and_stopping() {
    #[cfg(feature = "crdt")]
    {
        let entries = entries_or(185_000);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        std::fs::write(&file, issue_439_json(entries)).expect("write");
        let daemon = common::daemon::TestDaemon::holding(&file);
        let mut peer = Peer::attach(&daemon);
        peer.viewport(0, 4096);
        let (highlighted, summarized) = peer.first_frame();
        eprintln!(
            "p1 {entries} entries: highlighted {highlighted:?}, summary {summarized:?} [{}]; load {}",
            peer.log
                .iter()
                .filter(|(_, w)| w.starts_with("summary"))
                .map(|(_, w)| w.as_str())
                .collect::<Vec<_>>()
                .join("; "),
            loadavg()
        );
        peer.walk_into_the_first_name();

        let load_before = loadavg();
        let typing_from = peer.epoch.elapsed();
        let mut typing = Vec::new();
        let mut caret = 27u64;
        let cadence = Duration::from_millis(100);
        let start = Instant::now();
        let mut n = 0u32;
        while start.elapsed() < Duration::from_secs(15) {
            let slot = start + cadence * n;
            if let Some(wait) = slot.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            caret += 1;
            let at = peer.epoch.elapsed();
            let took = peer.key_to(Key::Char('x'), Modifiers::NONE, caret);
            typing.push((at, took));
            n += 1;
        }
        let stopped_at = peer.epoch.elapsed();
        let mut quiet = Vec::new();
        let start = Instant::now();
        let mut m = 0u32;
        while start.elapsed() < Duration::from_secs(6) {
            let slot = start + cadence * m;
            if let Some(wait) = slot.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let (key, to) = if m.is_multiple_of(2) {
                ('b', caret - 1)
            } else {
                ('f', caret)
            };
            let at = peer.epoch.elapsed();
            let took = peer.key_to(Key::Char(key), Modifiers::CTRL, to);
            quiet.push((at, took));
            m += 1;
        }
        // Drain what is left for a moment, so a late summary is logged.
        let _ = peer.until(Duration::from_secs(2), |_| false);
        let summaries: Vec<String> = peer
            .summaries
            .iter()
            .map(|(at, lines, styled)| format!("{:.2}s ({styled}/{lines})", at.as_secs_f64()))
            .collect();
        eprintln!(
            "p1 typing from {:.2}s to {:.2}s: {}",
            typing_from.as_secs_f64(),
            stopped_at.as_secs_f64(),
            describe(&typing, Duration::from_millis(250))
        );
        eprintln!(
            "p1 after the last key: {}",
            describe(&quiet, Duration::from_millis(250))
        );
        eprintln!(
            "p1 summaries sent at [{}]; load {load_before} before, {} after",
            summaries.join(" "),
            loadavg()
        );
    }
}

/// P2: a file whose whole-file span answer runs past `READ_HARD`
/// (370,000 entries, 36.5 MB, by default): the first frame, the summary,
/// the parse units, then a viewport in the middle of the file, then one
/// typed letter and the six seconds after it.
#[test]
#[ignore = "review probe; wall-clock, run by hand in release"]
#[allow(
    clippy::too_many_lines,
    reason = "one probe, its stages printed as they go"
)]
fn p2_a_file_past_the_read_limit() {
    #[cfg(feature = "crdt")]
    {
        let entries = entries_or(370_000);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        let text = issue_439_json(entries);
        let mid = {
            let half = text.len() / 2;
            half + text[half..].find('\n').expect("a newline") + 1
        } as u64;
        std::fs::write(&file, &text).expect("write");
        drop(text);
        let daemon = common::daemon::TestDaemon::holding(&file);
        let pid = daemon.pid();
        let stderr_path = {
            let mut p = daemon.socket_path().as_os_str().to_os_string();
            p.push(".stderr.log");
            std::path::PathBuf::from(p)
        };
        let stderr = || std::fs::read_to_string(&stderr_path).unwrap_or_default();
        let mut peer = Peer::attach(&daemon);
        peer.viewport(0, 4096);
        let t0 = Instant::now();
        let highlighted = peer.until(FIRST_FRAME, |m| match m {
            InstanceMessage::StyleSpans { segments, .. } => segments.iter().any(|s| {
                s.spans
                    .iter()
                    .any(|sp| sp.style != pmacs::cell::Style::default())
            }),
            _ => false,
        });
        let units_at_highlight = parse_units(pid);
        // The summary runs in the same frame; give it the read limit and
        // more, then see whether the unit that answered the parse lived.
        let _ = peer.until(Duration::from_secs(8), |_| false);
        let units_after = parse_units(pid);
        let _ = t0;
        eprintln!(
            "p2 {entries} entries: highlighted {highlighted:?} after the viewport; summaries \
             {:?}; parse units {units_at_highlight:?} at the highlight, {units_after:?} eight \
             seconds on; the daemon's stderr: {:?}; load {}",
            peer.summaries,
            stderr()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>(),
            loadavg()
        );
        let c_n = peer.key_to(Key::Char('n'), Modifiers::CTRL, 2);
        eprintln!("p2 C-n after the summary: {c_n:?}");

        // A viewport in the middle of the file, and a key right behind it
        // (`M-<`, whose answer does not depend on where the viewport put
        // the caret).
        let t0 = Instant::now();
        peer.viewport(mid, mid + 4096);
        let key = peer.key_to(Key::Char('<'), Modifiers::ALT, 0);
        let styled_mid = peer
            .until(Duration::from_secs(30), |m| match m {
                InstanceMessage::StyleSpans { segments, .. } => segments.iter().any(|s| {
                    s.spans.iter().any(|sp| {
                        sp.range.start >= mid && sp.style != pmacs::cell::Style::default()
                    })
                }),
                _ => false,
            })
            .map(|_| t0.elapsed());
        eprintln!(
            "p2 viewport at byte {mid}: a key behind it answered in {key:?}; styled spans there \
             {:?} after the declaration (None: not within 30 s); parse units {:?}; load {}",
            styled_mid,
            parse_units(pid),
            loadavg()
        );

        // Back to the top, one letter typed, and the six seconds after.
        peer.viewport(0, 4096);
        peer.walk_into_the_first_name();
        let typed_at = peer.epoch.elapsed();
        let typed = peer.key_to(Key::Char('x'), Modifiers::NONE, 28);
        let mut quiet = Vec::new();
        let start = Instant::now();
        let mut m = 0u32;
        while start.elapsed() < Duration::from_secs(6) {
            let slot = start + Duration::from_millis(100) * m;
            if let Some(wait) = slot.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let (key, to) = if m.is_multiple_of(2) {
                ('b', 27)
            } else {
                ('f', 28)
            };
            let at = peer.epoch.elapsed();
            let took = peer.key_to(Key::Char(key), Modifiers::CTRL, to);
            quiet.push((at, took));
            m += 1;
        }
        let _ = peer.until(Duration::from_secs(2), |_| false);
        let summaries: Vec<String> = peer
            .summaries
            .iter()
            .map(|(at, lines, styled)| format!("{:.2}s ({styled}/{lines})", at.as_secs_f64()))
            .collect();
        eprintln!(
            "p2 a letter typed at {:.2}s, answered in {typed:?}; after it: {}; summaries at [{}]; \
             parse units {:?}; the daemon's stderr now {:?}; load {}",
            typed_at.as_secs_f64(),
            describe(&quiet, Duration::from_millis(250)),
            summaries.join(" "),
            parse_units(pid),
            stderr()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>(),
            loadavg()
        );
    }
}

/// P4: scrolling #439's file after its summary. The summary's whole-file
/// read is kept among the handle's fetched pieces, and a viewport the
/// parse's own piece does not cover is served from the first piece that
/// covers it (`IsolatedHandle::spans_for`), which is then the whole-file
/// one, walked span by span to clip it (`grammar_scoped_style_spans`).
/// Thirty viewports down the file, each timed from its declaration to
/// the `StyleSpans` styling it; run at 185,000 and at 18,500 entries to
/// see whether a scroll's cost follows the file's size.
#[test]
#[ignore = "review probe; wall-clock, run by hand in release"]
fn p4_scrolling_the_439_file_after_its_summary() {
    #[cfg(feature = "crdt")]
    {
        let entries = entries_or(185_000);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        let text = issue_439_json(entries);
        let len = text.len() as u64;
        std::fs::write(&file, &text).expect("write");
        drop(text);
        let daemon = common::daemon::TestDaemon::holding(&file);
        let mut peer = Peer::attach(&daemon);
        peer.viewport(0, 4096);
        let (highlighted, summarized) = peer.first_frame();
        eprintln!(
            "p4 {entries} entries ({len} bytes): highlighted {highlighted:?}, summary \
             {summarized:?}; load {}",
            loadavg()
        );
        let _ = peer.until(Duration::from_millis(500), |_| false);
        let mut scrolls = Vec::new();
        for i in 1..=30u64 {
            let start = (len * i / 31).max(65_536);
            let t0 = Instant::now();
            let at = peer.epoch.elapsed();
            peer.viewport(start, start + 4096);
            let took = peer
                .until(Duration::from_secs(10), |m| match m {
                    InstanceMessage::StyleSpans { segments, .. } => segments.iter().any(|s| {
                        s.spans.iter().any(|sp| {
                            sp.range.start >= start && sp.style != pmacs::cell::Style::default()
                        })
                    }),
                    _ => false,
                })
                .unwrap_or_else(|| panic!("no styled spans at {start} within 10 s"));
            scrolls.push((at, took));
            let _ = t0;
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!(
            "p4 thirty viewports down the file, declaration to styled spans: {}; load {}",
            describe(&scrolls, Duration::from_millis(16)),
            loadavg()
        );
    }
}

/// The longest P5 lets a key wait after its typed letter. At `f0754a4`
/// the summary's recomputation held a key 1,845 to 2,141 ms at 250,000
/// entries (review 1); a key otherwise takes tens of milliseconds there.
#[cfg(feature = "crdt")]
const SUMMARY_HOLD: Duration = Duration::from_secs(1);

/// P5, Medium 2's witness: a JSON file of #439's shape whose summary
/// cannot fit the transport's 16 MiB frame whatever its lines' styles
/// (270,000 entries, 26.6 MB, 2,430,003 lines by default: at seven bytes
/// a line, `Style::default`'s, past 17 MB). At `f0754a4` the daemon
/// computed it on the main thread and the transport refused it (`frame
/// length … exceeds maximum` on the daemon's stderr), at the first frame
/// and again a second after a typed letter, holding the next key about
/// 2 s for a message never sent (review 1 measured 1,845 to 2,141 ms at
/// 250,000 entries, and its P2 the whole-file read abandoned at
/// `READ_HARD`, 2.00 s, past about 300,000). Since the fix the producer
/// reads nothing for a summary that cannot fit: after the letter no key
/// waits past [`SUMMARY_HOLD`], no summary arrives and the stderr names
/// no refusal. The first frame, the summaries and the refusals are
/// printed with the load.
#[test]
#[ignore = "review probe; wall-clock, run by hand in release"]
#[allow(
    clippy::too_many_lines,
    reason = "one probe, its stages printed as they go"
)]
fn p5_a_summary_past_the_frame() {
    #[cfg(feature = "crdt")]
    {
        let entries = entries_or(270_000);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        let text = issue_439_json(entries);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        let len = text.len();
        std::fs::write(&file, &text).expect("write");
        drop(text);
        // The precondition: a summary of this many lines, every line at
        // the smallest encoding a style has, is refused by the transport.
        let plain = InstanceMessage::FileStyleSummary {
            buffer_id: pmacs_protocol::BufferId::from_raw(1),
            generation: 0,
            lines: vec![pmacs::cell::Style::default(); lines],
        };
        assert!(
            matches!(
                pmacs_protocol::transport::write_message(&mut Vec::new(), &plain),
                Err(pmacs_protocol::TransportError::FrameTooLarge { .. })
            ),
            "{lines} lines' summary fits the frame unstyled: not P5's file"
        );
        drop(plain);
        let daemon = common::daemon::TestDaemon::holding(&file);
        let stderr_path = {
            let mut p = daemon.socket_path().as_os_str().to_os_string();
            p.push(".stderr.log");
            std::path::PathBuf::from(p)
        };
        let refusals = || {
            std::fs::read_to_string(&stderr_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains("exceeds maximum"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let mut peer = Peer::attach(&daemon);
        peer.viewport(0, 4096);
        let highlighted = peer.until(FIRST_FRAME, |m| match m {
            InstanceMessage::StyleSpans { segments, .. } => segments.iter().any(|s| {
                s.spans
                    .iter()
                    .any(|sp| sp.style != pmacs::cell::Style::default())
            }),
            _ => false,
        });
        let _ = peer.until(Duration::from_secs(5), |_| false);
        eprintln!(
            "p5 {entries} entries ({len} bytes, {lines} lines): highlighted {highlighted:?}; \
             summaries received {:?}; the daemon's refusals {:?}; load {}",
            peer.summaries,
            refusals(),
            loadavg()
        );
        assert!(
            highlighted.is_some(),
            "no highlighted frame within {FIRST_FRAME:?}; load {}",
            loadavg()
        );
        peer.walk_into_the_first_name();
        let typed_at = peer.epoch.elapsed();
        let typed = peer.key_to(Key::Char('x'), Modifiers::NONE, 28);
        let mut keys = vec![(typed_at, typed)];
        let start = Instant::now();
        let mut m = 0u32;
        while start.elapsed() < Duration::from_secs(6) {
            let slot = start + Duration::from_millis(100) * m;
            if let Some(wait) = slot.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let (key, to) = if m.is_multiple_of(2) {
                ('b', 27)
            } else {
                ('f', 28)
            };
            let at = peer.epoch.elapsed();
            keys.push((at, peer.key_to(Key::Char(key), Modifiers::CTRL, to)));
            m += 1;
        }
        let _ = peer.until(Duration::from_secs(2), |_| false);
        eprintln!(
            "p5 a letter typed at {:.2}s and the six seconds after: {}; summaries received {:?}; \
             the daemon's refusals now {:?}; load {}",
            typed_at.as_secs_f64(),
            describe(&keys, Duration::from_millis(250)),
            peer.summaries,
            refusals(),
            loadavg()
        );
        let (worst_at, worst) = keys
            .iter()
            .copied()
            .max_by_key(|(_, took)| *took)
            .expect("keys were sent");
        assert!(
            worst <= SUMMARY_HOLD,
            "a key after the typed letter waited {worst:?} (sent at {:.2}s): the main thread held \
             past {SUMMARY_HOLD:?}, as a summary the frame cannot carry being recomputed holds it; \
             the daemon's refusals {:?}; load {}",
            worst_at.as_secs_f64(),
            refusals(),
            loadavg()
        );
        assert!(
            peer.summaries.is_empty(),
            "a summary of {lines} lines arrived: {:?}",
            peer.summaries
        );
        assert!(
            refusals().is_empty(),
            "the transport refused a message: {:?}",
            refusals()
        );
    }
}

/// P6: the GPU binary itself on #439's file, headless: E10k.1's
/// `keypaint` probe (`pmacs-gpu --headless-probe`, the production `App`
/// fed by the daemon's messages, frames rendered offscreen), which the
/// handoff's GPU peer stands in for. Two samples of `kxv` at the end of
/// the last line; the whole run timed from the probe's start.
#[test]
#[ignore = "review probe; wall-clock, run by hand in release"]
fn p6_the_gpu_binary_on_the_439_file() {
    #[cfg(feature = "crdt")]
    {
        let Some(binary) = common::gpu_probe::gpu_probe_binary() else {
            return;
        };
        let entries = entries_or(185_000);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        std::fs::write(&file, issue_439_json(entries)).expect("write");
        let daemon = common::daemon::TestDaemon::holding(&file);
        let report = dir.path().join("keypaint.txt");
        let load_before = loadavg();
        let t0 = Instant::now();
        let output = std::process::Command::new(&binary)
            .arg("--headless-probe")
            .arg(daemon.socket_path())
            .arg(&report)
            .env("PMACS_GPU_PROBE_ACTION", "keypaint")
            .env("PMACS_GPU_PROBE_TYPE_TEXT", "kxv")
            .env("PMACS_GPU_PROBE_SAMPLES", "2")
            .env("PMACS_GPU_PROBE_GAP_MS", "150")
            .env("PMACS_GPU_PROBE_SAMPLE_MS", "60000")
            .output()
            .expect("run the keypaint probe");
        let took = t0.elapsed();
        let facts = common::gpu_probe::probe_facts(&output, &report, "p6").expect("an adapter");
        let mut keys: Vec<String> = facts
            .iter()
            .filter(|(k, _)| k.starts_with("key.") || k.starts_with("burst."))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        keys.sort();
        eprintln!(
            "p6 {entries} entries, the GPU binary's keypaint probe: {took:?} in all; failure {:?}; \
             text_len {:?}; {}; load {load_before} before, {} after",
            facts.get("failure"),
            facts.get("text_len"),
            keys.join(" "),
            loadavg()
        );
        assert!(
            !facts.contains_key("failure"),
            "the GPU binary did not settle and type on #439's file: {:?}",
            facts.get("failure")
        );
    }
}

/// P3: a second GUI window on #439's file. The first window's caret is
/// moved every 100 ms while the second attaches and receives its first
/// highlighted frame and summary; then the first types one letter and
/// both are watched for six seconds.
#[test]
#[ignore = "review probe; wall-clock, run by hand in release"]
fn p3_a_second_window_on_the_439_file() {
    #[cfg(feature = "crdt")]
    {
        let entries = entries_or(185_000);
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("big.json");
        std::fs::write(&file, issue_439_json(entries)).expect("write");
        let daemon = common::daemon::TestDaemon::holding(&file);
        let mut first = Peer::attach(&daemon);
        first.viewport(0, 4096);
        let (h1, s1) = first.first_frame();
        eprintln!(
            "p3 {entries} entries, first window: highlighted {h1:?}, summary {s1:?}; load {}",
            loadavg()
        );
        first.walk_into_the_first_name();

        // The second window attaches (its bootstrap snapshot, here), then
        // waits for its first frame on a thread while the first window's
        // caret keeps moving.
        let t0 = Instant::now();
        let mut second = Peer::attach(&daemon);
        let attached = t0.elapsed();
        let (second_times, first_keys) = std::thread::scope(|scope| {
            let second = scope.spawn(move || {
                let t0 = Instant::now();
                second.viewport(0, 4096);
                let (h, s) = second.first_frame();
                (attached, t0.elapsed().min(h.max(s)), (h, s), second)
            });
            let mut keys = Vec::new();
            let start = Instant::now();
            let mut m = 0u32;
            while !second.is_finished() || m < 10 {
                let slot = start + Duration::from_millis(100) * m;
                if let Some(wait) = slot.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
                let (key, to) = if m.is_multiple_of(2) {
                    ('b', 26)
                } else {
                    ('f', 27)
                };
                let at = first.epoch.elapsed();
                keys.push((at, first.key_to(Key::Char(key), Modifiers::CTRL, to)));
                m += 1;
            }
            (second.join().expect("the second window"), keys)
        });
        let (attached, _, (h2, s2), mut second) = second_times;
        eprintln!(
            "p3 second window: attached (to its token) in {attached:?}; from its viewport, \
             highlighted {h2:?}, summary {s2:?}; the first window's keys meanwhile: {}",
            describe(&first_keys, Duration::from_millis(100))
        );

        let typed = first.key_to(Key::Char('x'), Modifiers::NONE, 28);
        let mut quiet = Vec::new();
        let start = Instant::now();
        let mut m = 0u32;
        while start.elapsed() < Duration::from_secs(6) {
            let slot = start + Duration::from_millis(100) * m;
            if let Some(wait) = slot.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let (key, to) = if m.is_multiple_of(2) {
                ('b', 27)
            } else {
                ('f', 28)
            };
            let at = first.epoch.elapsed();
            quiet.push((at, first.key_to(Key::Char(key), Modifiers::CTRL, to)));
            m += 1;
        }
        let _ = first.until(Duration::from_secs(2), |_| false);
        let _ = second.until(Duration::from_secs(2), |_| false);
        let show = |p: &Peer| {
            p.summaries
                .iter()
                .map(|(at, lines, styled)| format!("{:.2}s ({styled}/{lines})", at.as_secs_f64()))
                .collect::<Vec<_>>()
                .join(" ")
        };
        eprintln!(
            "p3 one letter typed in the first window, answered in {typed:?}; its keys after: {}; \
             summaries, first window [{}], second window [{}] (each from its own attach); load {}",
            describe(&quiet, Duration::from_millis(250)),
            show(&first),
            show(&second),
            loadavg()
        );
    }
}
