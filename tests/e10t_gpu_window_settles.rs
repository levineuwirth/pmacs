//! E10t --- a GPU window that settles (#326).
//!
//! A `pmacs-gpu` attaching with no target was sent every buffer, one
//! snapshot each, and answered each with a `Viewport`, which moves the
//! daemon's window to the buffer it names; the tick then followed the
//! window with a snapshot of that buffer, answered in turn. Two answers
//! reaching the dispatcher on separate passes kept each other going until
//! two reached it on one pass, which grew rarer as the snapshots grew: at
//! 10,000 + 20,000 lines the window alternated between the files about a
//! hundred times a second in release, and never stopped within a watch of
//! 16 s; in debug it never stopped at any size. Where it did stop, the
//! window stayed on whichever answer came last. A semantic window
//! with no target now bootstraps on the one buffer its window shows, as a
//! resumed (E9) or restored (E10.2) one does.
//!
//! **Settled** (the roadmap's definition): the expected buffer stays
//! selected, and the count of snapshots written to that frontend stops
//! increasing, both held across a stated interval. Here the interval is
//! [`HOLD_MS`], after [`LEAD_MS`] from the probe's readiness, read by the
//! production `App` in `pmacs-gpu --headless-probe`'s popup action, whose
//! `count:` step reports the snapshots it has applied and the band's name
//! for the buffer it shows. E10t.1 measured the base: wherever it spins, the
//! longest gap between two snapshots was 1.5 s (debug, 179,000 + 179,000
//! lines), so a hold of five seconds sees the count rise. A snapshot reaches the probe in order
//! on its one socket, so the count it applied once the interval is over is
//! the count written to it before the interval began. A window that
//! settles is sent its buffer once: [`AT_MOST`].
//!
//! Every probe here runs under a deadline enforced from outside its
//! process ([`OUTSIDE`]), killed and reported when it passes: until E10t.3
//! a probe's own limit was not always read.
#![cfg(feature = "crdt")]

mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pmacs_protocol::BufferId;
use pmacs_protocol::message::{InstanceMessage, PROTOCOL_VERSION};

use common::daemon::TestDaemon;
use common::wire::WireSession;

/// From the probe's readiness to the first count.
const LEAD_MS: u64 = 3_000;
/// The stated observation interval: the count and the buffer must hold
/// across it.
const HOLD_MS: u64 = 5_000;
/// How long the probe waits to be ready (a snapshot applied, the caret
/// known, the daemon idle) before it goes on with its steps regardless.
const READY_MS: u64 = 30_000;
/// The outside bound: the probe is killed from here past it.
const OUTSIDE: Duration = Duration::from_mins(2);
/// The snapshots a window that settles is sent: its own buffer, once.
const AT_MOST: u64 = 1;

const LINE_BYTES: usize = 70;

/// `lines` log lines of [`LINE_BYTES`], E10k's fixture.
fn fixture(lines: usize) -> String {
    let mut out = String::with_capacity(lines * LINE_BYTES);
    for i in 0..lines {
        let line = format!(
            "2026-10-27T12:{:02}:{:02}.{:03}Z INFO daemon: served request {i:07} in {:03}us\n",
            (i / 60_000) % 60,
            (i / 1000) % 60,
            i % 1000,
            i % 997
        );
        assert_eq!(line.len(), LINE_BYTES, "fixture line {i}: {line:?}");
        out.push_str(&line);
    }
    out
}

/// A directory the daemon outlives: leaked, as E9's note is.
fn files_dir() -> PathBuf {
    let dir = tempfile::TempDir::new().expect("tempdir for the files");
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);
    path
}

/// An init visiting `files` in order, killing `*scratch*` unless `keep`,
/// and then showing `files[0]`, so the buffer the window shows is never
/// the sweep's last, plus `extra`.
fn init_visiting(files: &[PathBuf], keep_scratch: bool, extra: &str) -> String {
    let mut init = String::new();
    for f in files {
        let _ = writeln!(
            init,
            "pmacs.buffer.find_or_open({:?})",
            f.display().to_string()
        );
    }
    if !keep_scratch {
        init.push_str(
            "for _, b in ipairs(pmacs.buffer.list()) do\n  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n",
        );
    }
    let _ = writeln!(
        init,
        "pmacs.buffer.find_or_open({:?})",
        files[0].display().to_string()
    );
    init.push_str(extra);
    init
}

/// Two log files of `a` and `b` lines, `one.log` and `two.log`, visited
/// in that order, `*scratch*` killed and `one.log` shown, on a fresh state
/// root (each `TestDaemon` has its own).
fn daemon_with_two(a: usize, b: usize) -> TestDaemon {
    let dir = files_dir();
    let one = dir.join("one.log");
    let two = dir.join("two.log");
    std::fs::write(&one, fixture(a)).expect("write one.log");
    std::fs::write(&two, fixture(b)).expect("write two.log");
    TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&[one, two], false, ""))
}

/// The probe's report, and how long it ran.
struct Probe {
    facts: HashMap<String, String>,
    took: Duration,
}

impl Probe {
    fn fact(&self, key: &str) -> &str {
        self.facts.get(key).map_or("", String::as_str)
    }

    fn count(&self, label: &str) -> u64 {
        self.fact(&format!("{label}.snapshots"))
            .parse()
            .unwrap_or_else(|_| panic!("no {label}.snapshots in {:?}", self.facts))
    }
}

/// `pmacs-gpu --headless-probe` with the popup action against `daemon`,
/// attaching with no target, running `steps`; killed from outside past
/// [`OUTSIDE`]. `None` when skipped (no `pmacs-gpu` built, or no wgpu
/// adapter), which panics under `PMACS_REQUIRE_GPU`.
fn run_probe(daemon: &TestDaemon, label: &str, steps: &str) -> Option<Probe> {
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
        eprintln!("skipping the GPU probe: {} is not built", binary.display());
        return None;
    }
    let dir = daemon.socket_path().parent().expect("socket parent");
    let report = dir.join(format!("{label}.report"));
    let stderr_path = dir.join(format!("{label}.stderr"));
    let started = Instant::now();
    let mut child = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", steps)
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", READY_MS.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&stderr_path).expect("probe stderr"))
        .spawn()
        .expect("spawn the headless GPU probe");
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the probe") {
            break status;
        }
        if started.elapsed() > OUTSIDE {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "{label}: the probe was still running at the outside bound, {OUTSIDE:?}, \
                 and was killed; its stderr ends {:?}",
                tail(&stderr_path)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let took = started.elapsed();
    if !status.success() {
        let no_adapter = status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "{label}: the probe failed ({status}); its stderr ends {:?}",
            tail(&stderr_path)
        );
        eprintln!("skipping the GPU probe: no wgpu adapter available");
        return None;
    }
    let text = std::fs::read_to_string(&report).expect("read the probe report");
    let facts = text
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    Some(Probe { facts, took })
}

fn tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(6)..].join(" | ")
}

/// The steps that read a settle: a count after [`LEAD_MS`], another
/// [`HOLD_MS`] later.
fn settle_steps() -> String {
    format!("quiet:{LEAD_MS} count:early quiet:{HOLD_MS} count:late")
}

/// Settled on `name`: ready, the same buffer named `name` at both counts,
/// and the count unchanged across the hold and at most [`AT_MOST`].
fn assert_settled(probe: &Probe, name: &str, what: &str) {
    let (early, late) = (probe.count("early"), probe.count("late"));
    let said = format!(
        "{what}: early {early} snapshots on {:?} ({}), late {late} on {:?} ({}), \
         ready={}, the probe ran {:?}",
        probe.fact("early.name"),
        probe.fact("early.buffer"),
        probe.fact("late.name"),
        probe.fact("late.buffer"),
        probe.fact("ready"),
        probe.took
    );
    assert_eq!(probe.fact("ready"), "true", "{said}");
    assert_eq!(probe.fact("early.name"), name, "{said}");
    assert_eq!(probe.fact("late.name"), name, "{said}");
    assert_eq!(
        probe.fact("early.buffer"),
        probe.fact("late.buffer"),
        "{said}"
    );
    assert_eq!(early, late, "the count rose across the hold; {said}");
    assert!(
        late <= AT_MOST,
        "more snapshots than a window that settles is sent; {said}"
    );
}

/// E10t.2 (i): two files of 10,000 + 20,000 lines on a fresh root. At the
/// base the window never settled there (E10t.1: about 105 snapshots a second
/// in release and 5 in debug, to the end of every watch).
#[test]
fn e10t_2_i_ten_and_twenty_thousand_lines_settle_on_the_shown_file() {
    let daemon = daemon_with_two(10_000, 20_000);
    let Some(probe) = run_probe(&daemon, "i", &settle_steps()) else {
        return;
    };
    assert_settled(&probe, "one.log", "10,000 + 20,000 lines");
}

/// E10t.2 (ii): #326's own run. A daemon whose init visits `one.rs`,
/// `two.rs` and `three.rs`, keeps `*scratch*` and shows `one.rs`; four
/// attaches one after another. The first is on a fresh root; each later
/// one finds the desktop the one before it left (E10.2), so a first attach
/// that settled elsewhere would carry its buffer into the rest.
#[test]
fn e10t_2_ii_four_attaches_settle_on_the_file_the_daemon_shows() {
    let dir = files_dir();
    let files: Vec<PathBuf> = ["one.rs", "two.rs", "three.rs"]
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(
                &path,
                format!("fn {}() {{}}\n", name.trim_end_matches(".rs")),
            )
            .expect("write a source file");
            path
        })
        .collect();
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&files, true, ""));
    for attach in 1..=4 {
        let Some(probe) = run_probe(&daemon, &format!("ii-{attach}"), &settle_steps()) else {
            return;
        };
        assert_settled(&probe, "one.rs", &format!("attach {attach} of 4"));
    }
}

/// E10t.2 (iii): a pair large enough to have run to the timeout at the
/// base, 179,000 + 179,000 lines (12.5 MB each): 4 runs of 4 spun to a
/// 400 s timeout (#326's comment), and E10t.1's 4 to the end of their
/// 30 s watches, about 20 snapshots a second in release and 1 in debug.
#[test]
fn e10t_2_iii_two_twelve_megabyte_files_settle_too() {
    let daemon = daemon_with_two(179_000, 179_000);
    let Some(probe) = run_probe(&daemon, "iii", &settle_steps()) else {
        return;
    };
    assert_settled(&probe, "one.log", "179,000 + 179,000 lines");
}

/// Guard: a real declaration still moves the window, before a follow as
/// well as after it, and the window's buffer then follows it. One session
/// learns the three buffers' ids through commands that put its window on
/// each; a second, attached fresh beside it, declares `two.log` before the
/// daemon has followed anything for it, and `three.log` after the follow
/// that answered that. Passes at the base and after.
#[test]
fn e10t_2_guard_a_declaration_moves_the_window_before_and_after_a_follow() {
    let dir = files_dir();
    let files: Vec<PathBuf> = ["one.log", "two.log", "three.log"]
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, fixture(20)).expect("write a log");
            path
        })
        .collect();
    let mut commands = String::new();
    for (name, path) in [("two", &files[1]), ("three", &files[2])] {
        let _ = writeln!(
            commands,
            "pmacs.command.define {{ name = 'e10t.{name}', description = 'show {name}',\n\
               fn = function() pmacs.buffer.find_or_open({:?}) end }}",
            path.display().to_string()
        );
    }
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&files, false, &commands));

    let mut learner = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    let (one, _) = learner.where_am_i();
    let reach = |s: &mut WireSession, command: &str, from: BufferId| {
        s.run_command(command);
        s.wait("the window to leave", |m| match m {
            InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id != from => Some(*buffer_id),
            _ => None,
        })
    };
    let two = reach(&mut learner, "e10t.two", one);
    let three = reach(&mut learner, "e10t.three", two);
    assert!(one != two && two != three && one != three);

    let mut window = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    assert_eq!(
        window.where_am_i().0,
        one,
        "a fresh window shows the daemon's"
    );
    // Before any follow: the declaration moves the window, and the tick
    // then sends the buffer it moved to.
    window.declare_viewport(two);
    window.wait("two's snapshot, the follow", |m| {
        matches!(m, InstanceMessage::BufferSnapshot { buffer_id, .. } if *buffer_id == two)
            .then_some(())
    });
    window.wait("the window on two", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id == two)
            .then_some(())
    });
    // After that follow: another declaration moves it again.
    window.declare_viewport(three);
    window.wait("three's snapshot, the follow", |m| {
        matches!(m, InstanceMessage::BufferSnapshot { buffer_id, .. } if *buffer_id == three)
            .then_some(())
    });
    window.wait("the window on three", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id == three)
            .then_some(())
    });
}
