//! E10k.1 --- typing at the end of a large buffer, timed (#413, #414).
//!
//! The endpoint is the roadmap's: one key's time runs from its arrival at
//! the frontend to the first frame painted with its character, optimistic
//! or confirmed; a burst is three keys sent together, timed until a frame
//! shows all three. Every row types at the end of the last line of a
//! 100-line buffer and of an 18 MiB one (269,634 lines of 70 bytes, a log
//! whose last line ends in the word `zq`): letter bursts (`kxv`), which
//! extend that word, so completion collects on every key and, matching
//! nothing, reads as much of the buffer as it reads; and punctuation
//! bursts (`.,;`), which start no completion. Each key goes once the
//! frontend has been quiet and the previous one is on screen; each burst
//! is deleted again before the next, so every sample starts from the same
//! text.
//!
//! Three routes, one row each:
//!
//! - **in process**: `pmacs <file>` in a PTY, the in-process TUI with
//!   completion on;
//! - **the attached TUI**: `pmacs --attach` in a PTY against a daemon
//!   holding the file, its `BufferMirror` deciding the optimistic paint;
//! - **the GPU**: `pmacs-gpu --headless-probe` with
//!   `PMACS_GPU_PROBE_ACTION=keypaint` against the same kind of daemon,
//!   its replica answering the key and each key's frame rendered
//!   offscreen and waited on (the windowed path without `present`).
//!
//! A TUI's screen is modelled from what it writes to its terminal: the
//! PTY's reads, stamped by the reader thread as they arrive
//! (`spawn_pmacs_in_pty_tapped`), fed through `pmacs::ansi::AnsiParser`
//! into a `pmacs::terminal::screen::TerminalScreen`, whose snapshot holds
//! a DEC 2026 synchronized frame until the frame ends, as a terminal
//! does. A key is painted at the arrival of the read after which the
//! snapshot shows its character in its cell.
//!
//! The rows are wall-clock budgets (D12): `#[ignore]`, run by exact name
//! by `scripts/perf-budgets`. Each prints its figures, per key and per
//! burst for each kind and size, with the load average beside them.
//! `PMACS_E10K_SAMPLES` (eight by default) and `PMACS_E10K_SIZES`
//! (`small,large` by default) narrow a run for a profile.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::pty::{OutputTap, PmacsPty, spawn_pmacs_in_pty_tapped};
use pmacs::ansi::{AnsiParser, AnsiParserProfile};
use pmacs::cell::{CellCoord, CellSize, Glyph};
use pmacs::terminal::screen::TerminalScreen;

/// Bytes per line of the fixture, its newline included.
const LINE_BYTES: usize = 70;
/// The small buffer's lines.
const SMALL_LINES: usize = 100;
/// The large buffer's lines: 18,874,380 bytes, 18 MiB, #413's file.
const LARGE_LINES: usize = 269_634;
/// A letter burst: extends `zq`, so every key's word is three letters or
/// more and matches nothing in the buffer.
const LETTERS: &str = "kxv";
/// A punctuation burst: no pair character, no completion.
const PUNCTUATION: &str = ".,;";
/// How long a TUI's screen must stand unchanged, or the GPU's caret
/// confirmed, before a key or a burst goes.
const GAP: Duration = Duration::from_millis(150);
/// How long a placement, a key or a restore may take before the row
/// calls it failed: generous, for a debug build at 18 MiB.
const STEP_LIMIT: Duration = Duration::from_mins(2);
/// The rows run one at a time: three routes timing at once would time
/// each other.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The fixture: `lines` log lines of [`LINE_BYTES`], the last ending in
/// the word `zq`, which nothing else in the file begins with.
fn fixture(lines: usize) -> String {
    let mut out = String::with_capacity(lines * LINE_BYTES);
    for i in 0..lines {
        let mut line = format!(
            "2026-10-27T12:{:02}:{:02}.{:03}Z INFO daemon: served request {i:07} in {:03}us",
            (i / 60_000) % 60,
            (i / 1000) % 60,
            i % 1000,
            i % 997
        );
        assert_eq!(line.len(), LINE_BYTES - 1, "fixture line {i}: {line:?}");
        if i + 1 == lines {
            line.replace_range(LINE_BYTES - 4.., " zq");
        }
        line.push('\n');
        out.push_str(&line);
    }
    out
}

/// One buffer the rows type into.
struct Size {
    label: &'static str,
    key: &'static str,
    lines: usize,
}

const SIZES: [Size; 2] = [
    Size {
        label: "100 lines",
        key: "small",
        lines: SMALL_LINES,
    },
    Size {
        label: "18 MiB",
        key: "large",
        lines: LARGE_LINES,
    },
];

/// The sizes this run types into: `PMACS_E10K_SIZES` (`small`, `large`,
/// or both, comma-separated), both by default.
fn sizes() -> Vec<&'static Size> {
    let wanted = std::env::var("PMACS_E10K_SIZES").unwrap_or_else(|_| "small,large".into());
    SIZES
        .iter()
        .filter(|s| wanted.split(',').any(|w| w.trim() == s.key))
        .collect()
}

fn samples() -> usize {
    std::env::var("PMACS_E10K_SAMPLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8)
        .max(1)
}

/// Write `size`'s fixture as `buffer.log` in `dir`.
fn write_fixture(dir: &Path, size: &Size) -> std::path::PathBuf {
    let path = dir.join("buffer.log");
    let text = fixture(size.lines);
    assert_eq!(text.len(), size.lines * LINE_BYTES);
    std::fs::write(&path, text).expect("write the fixture");
    path
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|l| {
            let mut f = l.split_whitespace();
            Some(format!("{} {} {}", f.next()?, f.next()?, f.next()?))
        })
        .unwrap_or_else(|| "unknown".into())
}

fn kind_of(burst: &str) -> &'static str {
    if burst == LETTERS {
        "letters"
    } else {
        "punctuation"
    }
}

/// What one route measured at one size: each timed key and burst, with
/// its kind.
#[derive(Default)]
struct Figures {
    keys: Vec<(&'static str, Duration)>,
    bursts: Vec<(&'static str, Duration)>,
}

/// `n p50 p95 max` of `values`, in milliseconds.
fn summary(values: &mut [Duration]) -> String {
    values.sort_unstable();
    let n = values.len();
    if n == 0 {
        return "n=0".into();
    }
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    format!(
        "n={n} p50={:.1} p95={:.1} max={:.1} ms",
        ms(values[n / 2]),
        ms(values[(n * 95 / 100).min(n - 1)]),
        ms(values[n - 1])
    )
}

impl Figures {
    fn of(list: &[(&'static str, Duration)], kind: &str) -> Vec<Duration> {
        list.iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, d)| *d)
            .collect()
    }

    /// Print the figures, and check the run measured what it says.
    fn report(&self, route: &str, size: &Size, load_before: &str, load_after: &str) {
        let n = samples();
        for kind in ["letters", "punctuation"] {
            let mut keys = Self::of(&self.keys, kind);
            let mut bursts = Self::of(&self.bursts, kind);
            assert_eq!(
                keys.len(),
                n * 3,
                "{route} {}: {kind} keys timed",
                size.label
            );
            assert_eq!(
                bursts.len(),
                n,
                "{route} {}: {kind} bursts timed",
                size.label
            );
            eprintln!(
                "e10k {route} {}: {kind} per keystroke {}; per burst {}; load {load_before} -> {load_after}",
                size.label,
                summary(&mut keys),
                summary(&mut bursts),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A TUI in a PTY, its screen modelled from what it writes
// ---------------------------------------------------------------------------

struct Tui {
    pty: PmacsPty,
    tap: OutputTap,
    parser: AnsiParser,
    screen: TerminalScreen,
}

impl Tui {
    fn new((pty, tap): (PmacsPty, OutputTap)) -> Self {
        Self {
            pty,
            tap,
            // The full-screen profile: the default one is a compile
            // buffer's, which keeps no cursor.
            parser: AnsiParser::with_profile(AnsiParserProfile::FullScreen),
            screen: TerminalScreen::new(CellSize::new(ROWS.into(), COLS.into()), 0)
                .expect("a valid screen"),
        }
    }

    /// Apply `bytes`; whether the published screen or caret changed. A
    /// TUI writes a frame every tick whether or not anything moved, so
    /// a write is not a change.
    fn feed(&mut self, bytes: &[u8]) -> bool {
        let before = self.screen.snapshot();
        for event in self.parser.feed(bytes) {
            let _ = self.screen.apply_event(event);
        }
        let after = self.screen.snapshot();
        before.cells != after.cells || before.cursor != after.cursor
    }

    /// The published character at `(row, col)`.
    fn char_at(&self, row: u32, col: u32) -> Option<char> {
        let snap = self.screen.snapshot();
        let cell = snap.cells.get((row * snap.size.cols + col) as usize)?;
        match cell.glyph {
            Glyph::Char(c) => Some(c),
            _ => None,
        }
    }

    fn cursor(&self) -> Option<CellCoord> {
        self.screen.snapshot().cursor
    }

    /// Whether `text` stands at `(row, col)` onwards.
    fn shows(&self, row: u32, col: u32, text: &str) -> bool {
        text.chars()
            .zip(col..)
            .all(|(ch, c)| self.char_at(row, c) == Some(ch))
    }

    /// The published screen's rows as text.
    fn rows(&self) -> Vec<String> {
        let snap = self.screen.snapshot();
        snap.cells
            .chunks(snap.size.cols as usize)
            .map(|row| {
                row.iter()
                    .map(|c| match c.glyph {
                        Glyph::Char(ch) => ch,
                        _ => ' ',
                    })
                    .collect()
            })
            .collect()
    }

    /// Any row holds `needle`.
    fn screen_contains(&self, needle: &str) -> bool {
        self.rows().iter().any(|row| row.contains(needle))
    }

    /// The column the modeline reports (`L<line>:C<col>`): the editor's
    /// own word for where its caret is, which an attached TUI hears from
    /// the daemon, so it confirms what the TUI painted on its own.
    fn modeline_col(&self) -> Option<u32> {
        self.rows().iter().rev().find_map(|row| {
            let (_, after) = row.split_once(":C")?;
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
    }

    /// What the row saw, for a failure: the screen, the caret, and the
    /// last of what the TUI wrote.
    fn describe(&self) -> String {
        let out = self.pty.output();
        let tail = &out[out.len().saturating_sub(600)..];
        let dump = std::env::temp_dir().join(format!("e10k-tui-{}.out", std::process::id()));
        let _ = std::fs::write(&dump, &out);
        format!(
            "caret {:?}, {} bytes written (all of them in {}); screen:\n{}\nlast bytes: {:?}",
            self.cursor(),
            out.len(),
            dump.display(),
            self.rows().join("\n"),
            String::from_utf8_lossy(tail)
        )
    }

    fn write(&mut self, bytes: &[u8]) {
        self.pty.write_input(bytes).expect("write to the PTY");
    }

    /// Read until `done` holds; the arrival of the read that made it
    /// hold. Panics after `limit`.
    fn wait_for(&mut self, limit: Duration, what: &str, done: &dyn Fn(&Self) -> bool) -> Instant {
        let start = Instant::now();
        if done(self) {
            return start;
        }
        loop {
            let received = limit
                .checked_sub(start.elapsed())
                .and_then(|left| self.tap.recv_timeout(left).ok());
            let Some((at, bytes)) = received else {
                panic!("{what}: not on screen after {limit:?}; {}", self.describe());
            };
            self.feed(&bytes);
            if done(self) {
                return at;
            }
        }
    }

    /// Read until `done` holds and the screen has not changed for `quiet`.
    fn settle(
        &mut self,
        quiet: Duration,
        limit: Duration,
        what: &str,
        done: &dyn Fn(&Self) -> bool,
    ) {
        let start = Instant::now();
        let mut last = Instant::now();
        loop {
            if done(self) && last.elapsed() >= quiet {
                return;
            }
            assert!(
                start.elapsed() < limit,
                "{what}: not settled after {limit:?}; {}",
                self.describe()
            );
            if let Ok((_, bytes)) = self.tap.recv_timeout(Duration::from_millis(10))
                && self.feed(&bytes)
            {
                last = Instant::now();
            }
        }
    }
}

/// Walk the caret to the end of the buffer's last line (`M->`, `C-b`)
/// and return where it stands on screen, the cell right of `zq`, with
/// the column the modeline gives it there (the line's 69 bytes, plus
/// one).
fn place_at_end(tui: &mut Tui) -> (CellCoord, u32) {
    tui.wait_for(STEP_LIMIT, "the document", &|t| {
        t.screen_contains("INFO daemon")
    });
    tui.settle(GAP, STEP_LIMIT, "the document", &|_| true);
    tui.write(b"\x1b>");
    tui.settle(GAP, STEP_LIMIT, "M->", &|t| t.screen_contains(" zq"));
    tui.write(b"\x02");
    let at_zq = |t: &Tui| {
        t.cursor().is_some_and(|c| {
            c.col >= 2
                && t.char_at(c.row, c.col - 2) == Some('z')
                && t.char_at(c.row, c.col - 1) == Some('q')
        })
    };
    tui.settle(GAP, STEP_LIMIT, "the caret right of `zq`", &|t| {
        at_zq(t) && t.modeline_col() == Some(LINE_BYTES as u32)
    });
    (tui.cursor().expect("a caret"), LINE_BYTES as u32)
}

/// Type `burst`'s keys one at a time and then all at once, `samples()`
/// times each, deleting them after every sample; each key timed from
/// its write to the read that put its character on screen.
fn type_into_tui(
    tui: &mut Tui,
    (at, col): (CellCoord, u32),
    burst: &'static str,
    figures: &mut Figures,
) {
    let kind = kind_of(burst);
    let n = burst.len() as u32;
    let restored = move |t: &Tui| {
        t.cursor() == Some(at)
            && t.modeline_col() == Some(col)
            && (0..n).all(|i| t.char_at(at.row, at.col + i) == Some(' '))
    };
    // Everything typed is framed by the editor too: an attached TUI's
    // optimistic paint is confirmed before the keys are deleted, or a
    // delete the daemon frames with the insert in one tick leaves the
    // last optimistic character on screen (#421).
    let confirmed =
        move |t: &Tui| t.modeline_col() == Some(col + n) && t.shows(at.row, at.col, burst);
    for _ in 0..samples() {
        let mut typed = String::new();
        for (i, ch) in burst.chars().enumerate() {
            let caret = CellCoord::new(at.row, at.col + i as u32);
            let so_far = typed.clone();
            let col_now = col + i as u32;
            tui.settle(GAP, STEP_LIMIT, "the caret before a key", &|t| {
                t.cursor() == Some(caret)
                    && t.modeline_col() == Some(col_now)
                    && t.shows(at.row, at.col, &so_far)
            });
            typed.push(ch);
            let mut buf = [0u8; 4];
            let t0 = Instant::now();
            tui.write(ch.encode_utf8(&mut buf).as_bytes());
            let painted = tui.wait_for(STEP_LIMIT, "a typed key", &|t| {
                t.char_at(at.row, at.col + i as u32) == Some(ch)
            });
            figures
                .keys
                .push((kind, painted.saturating_duration_since(t0)));
        }
        tui.settle(GAP, STEP_LIMIT, "the keys confirmed", &confirmed);
        tui.write("\x7f".repeat(burst.len()).as_bytes());
        tui.settle(GAP, STEP_LIMIT, "the keys deleted", &restored);
        let t0 = Instant::now();
        tui.write(burst.as_bytes());
        let painted = tui.wait_for(STEP_LIMIT, "a typed burst", &|t| {
            t.shows(at.row, at.col, burst)
        });
        figures
            .bursts
            .push((kind, painted.saturating_duration_since(t0)));
        tui.settle(GAP, STEP_LIMIT, "the burst confirmed", &confirmed);
        tui.write("\x7f".repeat(burst.len()).as_bytes());
        tui.settle(GAP, STEP_LIMIT, "the burst deleted", &restored);
    }
}

fn run_tui(route: &str, size: &Size, spawn: impl FnOnce() -> (PmacsPty, OutputTap)) {
    let load_before = loadavg();
    let mut tui = Tui::new(spawn());
    let at = place_at_end(&mut tui);
    let mut figures = Figures::default();
    for burst in [LETTERS, PUNCTUATION] {
        type_into_tui(&mut tui, at, burst, &mut figures);
    }
    figures.report(route, size, &load_before, &loadavg());
}

/// A daemon holding `file` as its one buffer, its scratch buffer killed
/// so an attach snapshots the file alone.
#[cfg(feature = "crdt")]
fn daemon_holding(file: &Path) -> common::daemon::TestDaemon {
    let init_lua = format!(
        "pmacs.buffer.find_or_open({:?})\nfor _, b in ipairs(pmacs.buffer.list()) do\n  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n",
        file.display().to_string()
    );
    common::daemon::TestDaemon::spawn_with_env_and_init(&[], &init_lua)
}

/// E10k.1, in process: `pmacs <file>` in a PTY, completion on.
#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn e10k_typing_at_the_end_of_a_large_buffer_in_process() {
    let _serial = one_at_a_time();
    for size in sizes() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = write_fixture(dir.path(), size);
        let file_arg = file.to_str().expect("UTF-8 path").to_owned();
        let home = dir.path().to_path_buf();
        run_tui("in process", size, || {
            spawn_pmacs_in_pty_tapped(&[&file_arg], &[("HOME", &home)], ROWS, COLS)
        });
    }
}

/// E10k.1, the attached TUI: `pmacs --attach` in a PTY against a daemon
/// holding the file.
#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn e10k_typing_at_the_end_of_a_large_buffer_attached() {
    let _serial = one_at_a_time();
    #[cfg(feature = "crdt")]
    for size in sizes() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = write_fixture(dir.path(), size);
        let daemon = daemon_holding(&file);
        let socket = daemon
            .socket_path()
            .to_str()
            .expect("UTF-8 socket")
            .to_owned();
        let home = daemon
            .socket_path()
            .parent()
            .expect("socket dir")
            .to_path_buf();
        eprintln!("e10k attached {}: daemon pid {}", size.label, daemon.pid());
        run_tui("attached", size, || {
            spawn_pmacs_in_pty_tapped(
                &["--attach", "--socket", &socket],
                &[("HOME", &home), ("XDG_CONFIG_HOME", &home)],
                ROWS,
                COLS,
            )
        });
    }
    #[cfg(not(feature = "crdt"))]
    eprintln!("skipping: the attached TUI's replica is a crdt build's");
}

/// E10k.1, the GPU: `pmacs-gpu --headless-probe`'s `keypaint` action
/// against a daemon holding the file.
#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn e10k_typing_at_the_end_of_a_large_buffer_gpu() {
    let _serial = one_at_a_time();
    #[cfg(feature = "crdt")]
    {
        let Some(binary) = common::gpu_probe::gpu_probe_binary() else {
            return;
        };
        for size in sizes() {
            let dir = tempfile::TempDir::new().expect("tempdir");
            let file = write_fixture(dir.path(), size);
            let daemon = daemon_holding(&file);
            eprintln!("e10k gpu {}: daemon pid {}", size.label, daemon.pid());
            let report = dir.path().join("keypaint.txt");
            let load_before = loadavg();
            let output = std::process::Command::new(&binary)
                .arg("--headless-probe")
                .arg(daemon.socket_path())
                .arg(&report)
                .env("PMACS_GPU_PROBE_ACTION", "keypaint")
                .env(
                    "PMACS_GPU_PROBE_TYPE_TEXT",
                    format!("{LETTERS}|{PUNCTUATION}"),
                )
                .env("PMACS_GPU_PROBE_SAMPLES", samples().to_string())
                .env("PMACS_GPU_PROBE_GAP_MS", GAP.as_millis().to_string())
                .env(
                    "PMACS_GPU_PROBE_SAMPLE_MS",
                    STEP_LIMIT.as_millis().to_string(),
                )
                .output()
                .expect("run the keypaint probe");
            let load_after = loadavg();
            let Some(facts) = common::gpu_probe::probe_facts(&output, &report, size.label) else {
                return;
            };
            assert!(
                !facts.contains_key("failure"),
                "the probe failed: {:?}",
                facts.get("failure")
            );
            let text_len: usize = common::gpu_probe::fact(&facts, "text_len")
                .parse()
                .expect("text_len");
            assert_eq!(
                text_len,
                size.lines * LINE_BYTES,
                "the GPU holds the whole file"
            );
            let mut figures = Figures::default();
            let us = |v: &str| Duration::from_micros(v.parse().expect("microseconds"));
            let keys: usize = common::gpu_probe::fact(&facts, "keys")
                .parse()
                .expect("keys");
            let mut optimistic = 0;
            for n in 0..keys {
                let line = common::gpu_probe::fact(&facts, &format!("key.{n}"));
                let parts: Vec<&str> = line.split('|').collect();
                assert_eq!(parts.len(), 6, "key.{n}: {line}");
                figures.keys.push((kind_of(parts[0]), us(parts[2])));
                optimistic += usize::from(parts[5] == "true");
            }
            let bursts: usize = common::gpu_probe::fact(&facts, "bursts")
                .parse()
                .expect("bursts");
            for n in 0..bursts {
                let line = common::gpu_probe::fact(&facts, &format!("burst.{n}"));
                let parts: Vec<&str> = line.split('|').collect();
                assert_eq!(parts.len(), 3, "burst.{n}: {line}");
                figures.bursts.push((kind_of(parts[0]), us(parts[1])));
            }
            eprintln!(
                "e10k gpu {}: {optimistic} of {keys} keys optimistic",
                size.label
            );
            figures.report("gpu", size, &load_before, &load_after);
        }
    }
    #[cfg(not(feature = "crdt"))]
    eprintln!("skipping: the GPU's replica is a crdt build's");
}
