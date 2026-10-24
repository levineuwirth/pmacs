//! E10b review 1: the byte-ceiling row's liveness reading, its (b), asks
//! `kill(pid, 0)` before the row's first tick, and nothing reaps the
//! producer until a tick does (`poll_one`'s `try_wait`). A producer
//! that wrote its whole payload and exited is then an unreaped zombie,
//! which the kernel still answers for, so (b) passes whether the producer
//! was held back or finished. The first probe shows the instrument; the
//! second asks the producer itself, as (b) means to.
#![cfg(unix)]

mod common;

use std::time::Duration;

use common::ready::Probe;
use pmacs::process::{ProcessEventKind, ProcessSpec, ProcessState, ProcessSupervisor};

/// The row's payload: ten times the byte channel's 1 MiB ceiling.
const TOTAL: usize = 10 * 1024 * 1024;

/// The pid the supervisor reports for `id` right after its spawn.
fn pid_of(sup: &ProcessSupervisor, id: pmacs::process::ProcessId) -> u32 {
    match sup.state(id).expect("tracked") {
        ProcessState::Running { pid, .. } => *pid,
        s => panic!("expected Running immediately after spawn; got {s:?}"),
    }
}

/// `kill(pid, 0)`, the row's (b).
fn answers_kill_zero(pid: u32) -> bool {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(pid).expect("pid fits i32")),
        None,
    )
    .is_ok()
}

/// The process state `ps` reports for `pid` (`Z` for a zombie), empty
/// once nothing has that pid. `ps` reaps nothing, so reading it leaves
/// the supervisor's reap to its tick.
fn ps_state(pid: u32) -> String {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The instrument: a child that has exited, which the supervisor has not
/// ticked since its spawn and so has not reaped (`ps` reads it as a
/// zombie), still answers `kill(pid, 0)`; the first tick reaps it and
/// only then does it stop answering. So a liveness reading taken before
/// the first tick cannot fail for a producer that exited.
#[test]
fn e10b_review1_an_exited_child_answers_kill_zero_until_a_tick_reaps_it() {
    let mut sup = ProcessSupervisor::new();
    let mut spec = ProcessSpec::new("exits-at-once", "/bin/sh", "test process");
    spec.args = vec!["-c".into(), "exit 0".into()];
    let id = sup.spawn(spec).expect("spawn");
    let pid = pid_of(&sup, id);
    // No tick: the exit is read from `ps`.
    common::ready::expect_true(
        "the child to have exited, unreaped",
        Duration::from_secs(10),
        || ps_state(pid).starts_with('Z'),
        || format!("ps reads pid {pid} as {:?}", ps_state(pid)),
    );
    assert!(
        answers_kill_zero(pid),
        "an exited, unreaped child answers kill(pid, 0): the row's (b) reads it as alive"
    );
    // The first tick reaps it; then the kernel no longer answers for it.
    common::ready::expect(
        "the tick to reap the child",
        Duration::from_secs(10),
        || {
            sup.tick();
            if answers_kill_zero(pid) {
                Probe::Pending(format!("pid {pid} still answers after a tick"))
            } else {
                Probe::Ready(())
            }
        },
    );
}

/// The reading (b) means: a producer of `TOTAL` bytes that nothing
/// drains has not finished writing a second after it started, because
/// the supervisor holds it back. Its shell touches `started` before
/// `head` and `finished` after it, so the reading is the producer's own
/// and not its pid's. Draining it then delivers every byte and the
/// producer says it finished: the control that `finished` is written
/// when the write completes.
#[test]
fn e10b_review1_a_producer_nothing_drains_has_not_finished_its_write() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let started = dir.path().join("started");
    let finished = dir.path().join("finished");
    let mut sup = ProcessSupervisor::new();
    let mut spec = ProcessSpec::new("held-back", "/bin/sh", "test process");
    spec.args = vec![
        "-c".into(),
        format!(
            "touch {s}; head -c {TOTAL} /dev/zero; touch {f}",
            s = started.display(),
            f = finished.display()
        ),
    ];
    let id = sup.spawn(spec).expect("spawn");
    // No tick until the window has passed: a tick drains the channel.
    common::ready::expect_true(
        "the producer's shell to start",
        Duration::from_secs(10),
        || started.exists(),
        || format!("{} absent", started.display()),
    );
    // A window before a negative assertion, named with its length: one
    // second, far more than `head` needs to write 10 MiB unheld.
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !finished.exists(),
        "with nothing draining, the producer wrote its {TOTAL} bytes and \
         finished within 1 s of starting: nothing held it back"
    );
    // The control: drain, and the producer finishes and says so.
    let mut total = 0usize;
    common::ready::expect(
        "every byte drained and the producer finished",
        Duration::from_secs(15),
        || {
            sup.tick();
            for ev in sup.take_events(id) {
                if let ProcessEventKind::Stdout(b) = ev.kind {
                    total += b.len();
                }
            }
            if total == TOTAL && finished.exists() {
                Probe::Ready(())
            } else {
                Probe::Pending(format!(
                    "{total} of {TOTAL} bytes; finished {}",
                    finished.exists()
                ))
            }
        },
    );
}
