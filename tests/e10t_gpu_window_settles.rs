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

use pmacs_protocol::message::{InstanceMessage, PROTOCOL_VERSION};
use pmacs_protocol::{BufferId, Key, Modifiers};

use common::daemon::TestDaemon;
use common::oversize::oversize_text;
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
    if let Err(said) = settled(probe, name, AT_MOST, what) {
        panic!("{said}");
    }
}

/// Whether `probe` settled on `name`: ready, the same buffer named `name`
/// at its `early` and `late` counts, the count unchanged across the hold
/// and at most `at_most`; what it read when it did not.
fn settled(probe: &Probe, name: &str, at_most: u64, what: &str) -> Result<(), String> {
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
    if probe.fact("ready") != "true" {
        return Err(format!("not ready; {said}"));
    }
    if probe.fact("early.name") != name || probe.fact("late.name") != name {
        return Err(format!("not on {name:?}; {said}"));
    }
    if probe.fact("early.buffer") != probe.fact("late.buffer") {
        return Err(format!("the buffer changed across the hold; {said}"));
    }
    if early != late {
        return Err(format!("the count rose across the hold; {said}"));
    }
    if late > at_most {
        return Err(format!(
            "more snapshots than a window that settles is sent, {at_most}; {said}"
        ));
    }
    Ok(())
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
/// that settled elsewhere would carry its buffer into the rest. Those
/// three take the restored branch, which bootstrapped on one buffer before
/// E10t too; the row below makes all four fresh.
#[test]
fn e10t_2_ii_four_attaches_settle_on_the_file_the_daemon_shows() {
    four_attaches_settle_on_one_rs("");
}

/// E10t.2 (ii) as #326 ran it (review 1's Low 1): four attaches, each a
/// fresh no-target attach, since `session.reopen-desktop` is off and so no
/// attach reopens the desktop the one before it left. At the base each
/// was sent every buffer and could settle anywhere, as #326's four did
/// (`two.rs`, `two.rs`, `one.rs`, `one.rs`, 2026-10-07).
#[test]
fn e10t_2_ii_four_fresh_attaches_settle_with_the_desktop_off() {
    four_attaches_settle_on_one_rs("pmacs.config.set('session.reopen-desktop', false)\n");
}

/// #326's run: `one.rs`, `two.rs` and `three.rs` visited, `*scratch*` kept,
/// `one.rs` shown, `extra` after, and four attaches one after another, each
/// of which must settle on `one.rs`.
fn four_attaches_settle_on_one_rs(extra: &str) {
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
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&files, true, extra));
    // Every attach is read, and every one that did not settle named, so
    // a failure says which of the four did not (at the base a fresh attach
    // was sent every buffer, a restored one only its own).
    let mut unsettled = Vec::new();
    for attach in 1..=4 {
        let Some(probe) = run_probe(&daemon, &format!("ii-{attach}"), &settle_steps()) else {
            return;
        };
        if let Err(said) = settled(&probe, "one.rs", AT_MOST, &format!("attach {attach} of 4")) {
            unsettled.push(said);
        }
    }
    assert!(
        unsettled.is_empty(),
        "{} of 4 attaches did not settle on one.rs:\n{}",
        unsettled.len(),
        unsettled.join("\n")
    );
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

/// `one.log` and `two.log` of `lines` each, `one.log` shown, as
/// [`daemon_with_two`], with `extra` after the init, its `@one@` and
/// `@two@` replaced by the files' paths.
fn daemon_with_pair_and(lines: usize, extra: &str) -> TestDaemon {
    let dir = files_dir();
    let one = dir.join("one.log");
    let two = dir.join("two.log");
    std::fs::write(&one, fixture(lines)).expect("write one.log");
    std::fs::write(&two, fixture(lines)).expect("write two.log");
    let extra = extra
        .replace("@one@", &one.display().to_string())
        .replace("@two@", &two.display().to_string());
    TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&[one, two], false, &extra))
}

/// `C-c 8` switches the window to `two.log` `at_ms` after the key, from the
/// tick hook, so the probe's next step (each key step pumps 250 ms) lands
/// while `two.log`'s snapshot is in flight.
fn late_switch(at_ms: u64) -> String {
    format!(
        "local at = nil\n\
         pmacs.command.define {{ name = 'e10t.late', description = 'switch to two later',\n\
           fn = function() at = pmacs.editor.monotonic_ms() + {at_ms} end }}\n\
         pmacs.keymap.bind {{ scope = 'global', sequence = 'C-c 8', command = 'e10t.late' }}\n\
         pmacs.hook.add('process.after-tick', function()\n\
           if at and pmacs.editor.monotonic_ms() >= at then at = nil; pmacs.buffer.find_or_open('@two@') end\n\
         end)\n"
    )
}

/// `C-c 9` switches the window to `two.log` at once and back to `one.log`
/// `gap_ms` later: two `C-x b` with both snapshots in flight.
fn flip(gap_ms: u64) -> String {
    format!(
        "local back = nil\n\
         pmacs.command.define {{ name = 'e10t.flip', description = 'two, then one',\n\
           fn = function() pmacs.buffer.find_or_open('@two@'); back = pmacs.editor.monotonic_ms() + {gap_ms} end }}\n\
         pmacs.keymap.bind {{ scope = 'global', sequence = 'C-c 9', command = 'e10t.flip' }}\n\
         pmacs.hook.add('process.after-tick', function()\n\
           if back and pmacs.editor.monotonic_ms() >= back then back = nil; pmacs.buffer.find_or_open('@one@') end\n\
         end)\n"
    )
}

/// From a switch to the first count. Longer than [`LEAD_MS`]: a debug
/// daemon had not yet named a 12.5 MB buffer it switched to 3 s after the
/// switch, though its snapshot had landed (the control below, at 8bf9b16).
const SWITCH_LEAD_MS: u64 = 10_000;

/// The steps of a switch: [`LEAD_MS`] after readiness, then `chord`, then
/// `then` (a step or nothing), and two counts, [`SWITCH_LEAD_MS`] after the
/// switch and [`HOLD_MS`] after that.
fn switch_steps(chord: &str, then: &str) -> String {
    format!(
        "quiet:{LEAD_MS} {chord} {then} quiet:{SWITCH_LEAD_MS} count:early quiet:{HOLD_MS} count:late"
    )
}

/// Review 1's Medium 1: a `C-x b` and a wheel turn while its snapshot is
/// in flight, which a trackpad's inertia delivers for about a second after
/// a gesture. Until the snapshot lands the GPU shows `one.log`, so the
/// wheel's declaration names it; that moved the daemon's window back, the
/// tick followed with `one.log`'s snapshot, and each applied snapshot's own
/// answer moved it again: #326's spin, one switch after the attach. At
/// 8bf9b16 it spun without end at both sizes, a key not ending it (9 of 9
/// release runs). A declaration naming the buffer the frontend showed
/// before a snapshot it has not answered is now stale and moves nothing.
#[test]
fn e10t_r1_a_switch_and_a_wheel_turn_in_flight_settle_on_the_switch() {
    for (lines, label) in [(179_000, "r1-wheel-179k"), (20_000, "r1-wheel-20k")] {
        let daemon = daemon_with_pair_and(lines, &late_switch(235));
        let Some(probe) = run_probe(&daemon, label, &switch_steps("key:C-c key:8", "wheel:3"))
        else {
            return;
        };
        // The attach's snapshot and the switch's.
        if let Err(said) = settled(&probe, "two.log", 2, label) {
            panic!("{said}");
        }
    }
}

/// The control for the row above: the same switch with no wheel turn
/// settles on `two.log`, at the reviewed head and after.
#[test]
fn e10t_r1_control_the_switch_alone_settles() {
    let daemon = daemon_with_pair_and(179_000, &late_switch(235));
    let Some(probe) = run_probe(
        &daemon,
        "r1-control",
        &switch_steps("key:C-c key:8", "quiet:250"),
    ) else {
        return;
    };
    if let Err(said) = settled(&probe, "two.log", 2, "the switch alone") {
        panic!("{said}");
    }
}

/// Review 1's Medium 1, its other route: two switches about 120 ms apart,
/// both snapshots in flight. The first's answer moved the window back to
/// `two.log` after the second had moved it to `one.log`, and the second's
/// moved it again (at 8bf9b16, 120 ms apart: 1, 52, 148 snapshots at 3, 7
/// and 12 s, release). The window now settles on the last switch.
#[test]
fn e10t_r1_two_switches_in_flight_settle_on_the_last() {
    let daemon = daemon_with_pair_and(179_000, &flip(120));
    let Some(probe) = run_probe(&daemon, "r1-flip", &switch_steps("key:C-c key:9", "")) else {
        return;
    };
    // The attach's snapshot and the two switches'.
    if let Err(said) = settled(&probe, "one.log", 3, "two switches 120 ms apart") {
        panic!("{said}");
    }
}

/// `C-c 6` holds the dispatcher `hold_ms`, as a slow command does, and then
/// shows `@two@` in the key's own handling. A declaration made in that
/// time waits behind the key and is read in the same dispatcher pass: after
/// the switch, before the tick writes the follow. (Review 2 timed the real
/// commands: a switch to an open 12.5 MB buffer held the dispatcher 11 ms
/// in release and 101 ms in debug, a cold open 13 and 109 ms, about a
/// touchpad's 8 to 16 ms between scroll events in release and under the
/// probe's 250 ms step in both, which is why the hold is here.)
fn slow_switch(hold_ms: u64) -> String {
    format!(
        "pmacs.command.define {{ name = 'e10t.slow', description = 'hold, then show two',\n\
           fn = function()\n\
             local until_ms = pmacs.editor.monotonic_ms() + {hold_ms}\n\
             while pmacs.editor.monotonic_ms() < until_ms do end\n\
             pmacs.buffer.find_or_open('@two@')\n\
           end }}\n\
         pmacs.keymap.bind {{ scope = 'global', sequence = 'C-c 6', command = 'e10t.slow' }}\n"
    )
}

/// Review 2's Medium 1: a declaration read in the same dispatcher pass as
/// the key that switched the window, before the follow is written, names
/// the buffer the frontend still shows. Fix round 1's record holds only
/// snapshots already written, so it found nothing in flight and called the
/// declaration real; aligning to it put the window back, no snapshot was
/// ever written, and the switch was gone under a status line that claimed
/// it (in a real window, a scroll under way when `RET` went). At 143920e:
/// `not on "two.log"; early 1 snapshots on "one.log"`, 2 of 2 release runs
/// and 1 of 1 debug, and the pre-round daemon the same. While the daemon
/// owes the session a follow, a declaration naming the buffer last sent to
/// it now moves nothing.
#[test]
fn review2_a_wheel_turn_read_in_the_switch_s_own_pass_does_not_undo_it() {
    let daemon = daemon_with_pair_and(179_000, &slow_switch(400));
    let Some(probe) = run_probe(
        &daemon,
        "r2-same-pass",
        &switch_steps("key:C-c key:6", "wheel:3"),
    ) else {
        return;
    };
    // The attach's snapshot and the switch's.
    if let Err(said) = settled(
        &probe,
        "two.log",
        2,
        "a wheel turn in the switch's own pass",
    ) {
        panic!("{said}");
    }
}

/// Its control: the same slow switch with no wheel turn settles on
/// `two.log`, at the reviewed head and after.
#[test]
fn review2_control_the_slow_switch_alone_settles() {
    let daemon = daemon_with_pair_and(179_000, &slow_switch(400));
    let Some(probe) = run_probe(
        &daemon,
        "r2-same-pass-control",
        &switch_steps("key:C-c key:6", "quiet:250"),
    ) else {
        return;
    };
    if let Err(said) = settled(&probe, "two.log", 2, "the slow switch alone") {
        panic!("{said}");
    }
}

/// `C-c 3` kills the shown buffer 235 ms after the key, from the tick hook,
/// so the window falls to `two.log` and the probe's wheel turn declares a
/// buffer that no longer exists while `two.log`'s snapshot is in flight.
fn late_kill() -> String {
    "local at = nil\n\
     pmacs.command.define { name = 'e10t.latekill', description = 'kill one later',\n\
       fn = function() at = pmacs.editor.monotonic_ms() + 235 end }\n\
     pmacs.keymap.bind { scope = 'global', sequence = 'C-c 3', command = 'e10t.latekill' }\n\
     pmacs.hook.add('process.after-tick', function()\n\
       if at and pmacs.editor.monotonic_ms() >= at then\n\
         at = nil\n\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == 'one.log' then pcall(pmacs.buffer.kill, b) end\n\
         end\n\
       end\n\
     end)\n"
        .to_owned()
}

/// Review 2's guard, passing at 143920e (2 / 2 / 2 on `two.log`, release)
/// and after: the shown buffer is killed with a wheel turn in flight, and
/// the record reads the declaration of the buffer that is gone as made
/// before the snapshot that replaced it.
#[test]
fn review2_a_killed_buffer_and_a_wheel_turn_in_flight_settle() {
    let daemon = daemon_with_pair_and(179_000, &late_kill());
    let Some(probe) = run_probe(
        &daemon,
        "r2-kill",
        &switch_steps("key:C-c key:3", "wheel:3"),
    ) else {
        return;
    };
    if let Err(said) = settled(
        &probe,
        "two.log",
        2,
        "the shown buffer killed, a wheel turn in flight",
    ) {
        panic!("{said}");
    }
}

/// [`WireSession::wait`] with `patience` in place of its ten seconds: a
/// debug daemon upgrading and encoding a 24 MiB buffer on its main thread
/// writes nothing meanwhile (#434 is E9.5's row timing out at ten under
/// load).
fn wait_within<T>(
    session: &mut WireSession,
    what: &str,
    patience: Duration,
    mut pick: impl FnMut(&InstanceMessage) -> Option<T>,
) -> T {
    let deadline = Instant::now() + patience;
    let mut seen = 0_usize;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out after {patience:?} waiting for {what}; the daemon wrote {seen} other messages"
        );
        match session
            .rx
            .recv_timeout(remaining.min(Duration::from_millis(200)))
        {
            Ok(message) => {
                if let Some(found) = pick(&message) {
                    return found;
                }
                seen += 1;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the daemon closed the session while waiting for {what}")
            }
        }
    }
}

/// `C-c 6` and, written behind it at once, a declaration of the buffer the
/// session shows: the declaration waits in the dispatcher's queue while
/// the command holds it, and is read in the key's own pass.
fn slow_switch_and_declare(session: &mut WireSession, shown: BufferId) {
    session.key(Key::Char('c'), Modifiers::CTRL);
    session.key(Key::Char('6'), Modifiers::NONE);
    session.declare_viewport(shown);
}

/// Review 2's Medium 1 on the wire, where the pass is forced and no GPU is
/// needed: the key that switches the window and, written behind it, a
/// declaration of the buffer the session was last sent. The window goes to
/// `two.log`, the tick follows with its snapshot, and the next key lands
/// there. At 143920e the declaration put the window back and nothing
/// followed: `timed out waiting for two.log's snapshot, the follow`.
#[test]
fn e10t_r2_a_declaration_read_in_the_switch_s_own_pass_does_not_undo_it() {
    let daemon = daemon_with_pair_and(20, &slow_switch(400));
    let mut session = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    let (one, _) = session.where_am_i();
    assert_eq!(
        session.bootstrap_snapshots(),
        [one],
        "bootstrapped on the window's buffer alone"
    );
    // The frontend's answer to its bootstrap snapshot, as `pmacs-gpu`'s is.
    session.declare_viewport(one);

    slow_switch_and_declare(&mut session, one);
    let two = session.wait("two.log's snapshot, the follow", |m| match m {
        InstanceMessage::BufferSnapshot { buffer_id, .. } if *buffer_id != one => Some(*buffer_id),
        _ => None,
    });
    session.declare_viewport(two);
    session.type_str("k");
    session.wait("the key's echo on two.log", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == two)
            .then_some(())
    });
}

/// The same pass when the follow it leaves owed is then refused (E9.5):
/// the switch is to a buffer whose snapshot is past the frame. The owner's
/// ruling on review 2 read the code as putting the window back on the last
/// buffer sent, so that no follow stays owed there, and asked for the run.
/// The refusal is said on the status line; the window is back on the
/// buffer the session mirrors, where the next key lands; and the daemon
/// refused once and not on every tick after, as it would with a follow
/// still owed.
/// At 143920e the declaration put the window back before the tick could
/// refuse anything, so the user was told nothing: `timed out ... waiting
/// for the refusal on the status line`.
#[test]
fn e10t_r2_a_follow_refused_after_such_a_declaration_is_said_and_leaves_none_owed() {
    let dir = files_dir();
    let (one, big) = (dir.join("one.log"), dir.join("big.bin"));
    std::fs::write(&one, fixture(20)).expect("write one.log");
    std::fs::write(&big, oversize_text(24 * 1024 * 1024)).expect("write big.bin");
    // Visited one, big; `*scratch*` killed; `one.log` shown. `C-c 6`
    // holds, then shows `big.bin`.
    let extra = slow_switch(400).replace("@two@", &big.display().to_string());
    let daemon =
        TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&[one, big], false, &extra));
    let mut session = WireSession::semantic_with_patience(
        &daemon,
        PROTOCOL_VERSION,
        None,
        None,
        Duration::from_mins(2),
    );
    let (shown, _) = session.where_am_i();
    assert_eq!(session.bootstrap_snapshots(), [shown]);
    session.declare_viewport(shown);

    slow_switch_and_declare(&mut session, shown);
    let message = wait_within(
        &mut session,
        "the refusal on the status line",
        Duration::from_mins(1),
        |m| match m {
            InstanceMessage::StatusFacts {
                message: Some(message),
                ..
            } if message.contains("a frontend can mirror") => Some(message.clone()),
            InstanceMessage::BufferSnapshot { buffer_id, .. } => {
                panic!(
                    "a snapshot of {buffer_id:?} was sent; the switch was to a buffer past the frame"
                )
            }
            _ => None,
        },
    );
    assert!(
        message.contains("big.bin is 24.0 MiB") && message.contains("this window stays on one.log"),
        "the status names what weighed too much and where the window stayed: {message:?}"
    );
    // Back on the buffer the session mirrors: the next key lands there.
    session.type_str("k");
    session.wait("the key's echo on one.log", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == shown)
            .then_some(())
    });
    // No follow stays owed: a window left on `big.bin` would be refused
    // again on every tick. A second of ticks, named, then the count.
    std::thread::sleep(Duration::from_secs(1));
    let log = daemon.stderr_log();
    assert_eq!(
        log.matches("this window stays on one.log").count(),
        1,
        "the daemon refused the switch once; its log:\n{log}"
    );
}

/// A daemon whose window shows a file too large to mirror: `big.bin`,
/// E9.5's fixture (24 MiB of pseudo-random printable text, its snapshot
/// past the 16 MiB frame), visited first, then `one.log` of 10,000 lines
/// and `two.log` of 20,000; `*scratch*` killed; `big.bin` shown; `extra`
/// after, its `@two@` replaced by `two.log`'s path.
fn daemon_showing_oversize(extra: &str) -> TestDaemon {
    let dir = files_dir();
    let (big, one, two) = (
        dir.join("big.bin"),
        dir.join("one.log"),
        dir.join("two.log"),
    );
    std::fs::write(&big, oversize_text(24 * 1024 * 1024)).expect("write big.bin");
    std::fs::write(&one, fixture(10_000)).expect("write one.log");
    std::fs::write(&two, fixture(20_000)).expect("write two.log");
    let extra = extra.replace("@two@", &two.display().to_string());
    TestDaemon::spawn_with_env_and_init(&[], &init_visiting(&[big, one, two], false, &extra))
}

/// Review 2's Medium 2: a GPU attaching with no target to a daemon whose
/// window shows a buffer too large to mirror. The daemon refuses it, puts
/// the window on the fallback and says so (`this window stays on
/// one.log`). Until E10t's fix round 2 the attach then swept every other
/// buffer, as a fresh semantic window's did before E10t: each answer moved
/// the window and the last one won, `two.log`, at 3 or 4 snapshots, 9 runs
/// of 9 across two reviews, under a status line naming `one.log`. The
/// session now bootstraps on the fallback's one snapshot.
#[test]
fn review2_an_oversize_window_s_gpu_settles_where_the_daemon_says() {
    let daemon = daemon_showing_oversize("");
    let Some(probe) = run_probe(&daemon, "r2-oversize", &settle_steps()) else {
        return;
    };
    assert_settled(
        &probe,
        "one.log",
        "a window on a buffer past the frame, sent to the fallback",
    );
    let log = daemon.stderr_log();
    assert!(
        log.contains("this window stays on one.log"),
        "the daemon said where the window stays; its log:\n{log}"
    );
}

/// Medium 2 on the wire, where no GPU is needed: the refused window's
/// session is sent one snapshot in its bootstrap, the fallback's, the
/// status line names that buffer, and the window is on it. At 143920e the
/// bootstrap carried every other buffer's snapshot, `one.log`'s and
/// `two.log`'s.
#[test]
fn e10t_r2_an_oversize_window_bootstraps_on_the_fallback_s_one_snapshot() {
    let daemon = daemon_showing_oversize("");
    let mut session = WireSession::semantic_with_patience(
        &daemon,
        PROTOCOL_VERSION,
        None,
        None,
        Duration::from_mins(2),
    );
    let sent = session.bootstrap_snapshots();
    assert_eq!(
        sent.len(),
        1,
        "the bootstrap carries the fallback's snapshot alone: {sent:?}"
    );
    // The frontend's answer to it, as `pmacs-gpu`'s is; the status line
    // comes with the frames for the declared buffer.
    session.declare_viewport(sent[0]);
    let (name, status) = wait_within(
        &mut session,
        "the first status",
        Duration::from_mins(1),
        |m| match m {
            InstanceMessage::StatusFacts {
                buffer_id,
                name,
                message: Some(message),
                ..
            } if *buffer_id == sent[0] => Some((name.clone(), message.clone())),
            _ => None,
        },
    );
    assert_eq!(name, "one.log", "the snapshot sent is the fallback's");
    assert!(
        status.contains("big.bin is 24.0 MiB") && status.contains("this window stays on one.log"),
        "the status names what weighed too much and where the window stays: {status:?}"
    );
    assert_eq!(
        session.where_am_i().0,
        sent[0],
        "the window is on the buffer the session was sent"
    );
}

/// Review 2's guard for the record after the oversize attach, restated for
/// the bootstrap that attach now has: the window starts on `one.log`, the
/// fallback, and a switch to `two.log` with a wheel turn in flight settles
/// there at two snapshots, the attach's and the switch's. The attach's one
/// snapshot is in the session's record, so the wheel's declaration of
/// `one.log` is read as made before `two.log`'s landed. (At 143920e the
/// sweep had left the window on `two.log` already, at 3 or 4 snapshots the
/// record never held; the review's probe switched back to `one.log` and
/// read 5 / 5 / 5.)
#[test]
fn review2_a_switch_after_the_oversize_fallback_settles() {
    let daemon = daemon_showing_oversize(&late_switch(235));
    let Some(probe) = run_probe(
        &daemon,
        "r2-oversize-switch",
        &switch_steps("key:C-c key:8", "wheel:3"),
    ) else {
        return;
    };
    if let Err(said) = settled(
        &probe,
        "two.log",
        2,
        "a switch with a wheel turn in flight, after the oversize attach",
    ) {
        panic!("{said}");
    }
}
