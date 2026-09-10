// tests/common/ready.rs --- the one readiness wait.

//! A predicate plus a deadline, reporting elapsed time and the last
//! observed state when the deadline passes.
//!
//! Every wait in the suites E0.12 audited goes through [`wait`] or
//! [`tick_until`]: the lean4 and resource-reconciliation suites, m4's
//! fixed-iteration drains, and m4's D3 file-watch block. In those, a
//! fixed `thread::sleep` is a window before a negative assertion, named
//! with its length, and never a readiness wait. The claim is bounded on
//! purpose: the unconditional timed drains in the other suites were not
//! audited, `m9_2_acceptance.rs:547` is a declared one, and sweeping
//! the rest is a planning item and not a row of this phase.
//!
//! The reason is the two ways a hand-rolled wait has failed here:
//!
//! - a fixed number of ticks with a fixed sleep (`for _ in 0..8 { tick();
//!   sleep(2ms) }`) is a bet on the machine's speed, and it loses under
//!   load in the only way that matters, silently: the assertion after it
//!   fails with the *result* of the race (`rows.len() == 0`) and nothing
//!   says how long the test waited or what it saw last;
//! - a poll loop whose predicate is weaker than the assertion it guards
//!   (`contains("probe")` before asserting `"probe":true`) is a race on
//!   every platform that happens to lose it.
//!
//! So the probe here returns [`Probe::Ready`] with the value the caller
//! asserts on, or [`Probe::Pending`] with a description of what it saw,
//! and the failure message carries the deadline, the elapsed time, the
//! number of polls and that last description. A reader of a red log then
//! knows whether the wait was long or short, and what the world looked
//! like when it gave up.
//!
//! Included per suite with `#[path = "common/ready.rs"] mod ready;`, or
//! through `mod common;` for suites that already take the daemon and PTY
//! fixtures.

#![allow(dead_code)] // not every including suite uses every helper

use std::fmt;
use std::time::{Duration, Instant};

/// The interval between probes. Short enough that a wait ends within a
/// frame of the condition holding; long enough not to spin.
pub const POLL: Duration = Duration::from_millis(20);

/// What a probe saw: either the value the caller wanted, or a
/// description of the state that was not yet it.
pub enum Probe<T> {
    Ready(T),
    Pending(String),
}

/// A deadline that passed, with what was observed.
#[derive(Debug)]
pub struct Timeout {
    pub what: String,
    pub deadline: Duration,
    pub elapsed: Duration,
    pub polls: u32,
    pub last: String,
}

impl fmt::Display for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} did not become ready within {:?} (waited {:?}, {} polls); last observed: {}",
            self.what, self.deadline, self.elapsed, self.polls, self.last
        )
    }
}

impl std::error::Error for Timeout {}

/// Poll `probe` every [`POLL`] until it is ready or `deadline` passes.
///
/// The first probe runs immediately, so a condition that already holds
/// costs no sleep. The deadline is checked *after* a probe, so a
/// condition that becomes true at the last moment is still returned
/// rather than reported as a timeout.
pub fn wait<T>(
    what: &str,
    deadline: Duration,
    probe: impl FnMut() -> Probe<T>,
) -> Result<T, Timeout> {
    wait_with(what, deadline, POLL, || {}, probe)
}

/// [`wait`] with a caller-chosen poll interval and a step to run before
/// every probe (the tick of an in-process editor, for instance).
pub fn wait_with<T>(
    what: &str,
    deadline: Duration,
    poll: Duration,
    mut step: impl FnMut(),
    mut probe: impl FnMut() -> Probe<T>,
) -> Result<T, Timeout> {
    let start = Instant::now();
    let mut polls = 0u32;
    loop {
        step();
        polls += 1;
        let last = match probe() {
            Probe::Ready(value) => return Ok(value),
            Probe::Pending(state) => state,
        };
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            return Err(Timeout {
                what: what.to_owned(),
                deadline,
                elapsed,
                polls,
                last,
            });
        }
        std::thread::sleep(poll.min(deadline.saturating_sub(elapsed)));
    }
}

/// [`wait`] that panics with the [`Timeout`] report. For the common case
/// where a test has nothing to do with a timeout but fail.
#[track_caller]
pub fn expect<T>(what: &str, deadline: Duration, probe: impl FnMut() -> Probe<T>) -> T {
    match wait(what, deadline, probe) {
        Ok(value) => value,
        Err(timeout) => panic!("{timeout}"),
    }
}

/// [`expect`] for a plain boolean condition. `describe` renders the
/// state for the failure message; it runs only when the wait fails.
#[track_caller]
pub fn expect_true(
    what: &str,
    deadline: Duration,
    mut condition: impl FnMut() -> bool,
    describe: impl Fn() -> String,
) {
    let result = wait(what, deadline, || {
        if condition() {
            Probe::Ready(())
        } else {
            Probe::Pending(String::new())
        }
    });
    if let Err(mut timeout) = result {
        timeout.last = describe();
        panic!("{timeout}");
    }
}

/// Drive an in-process editor's frame order (processes, LSP, async)
/// until `probe` is ready, polling every `TICK_POLL`.
///
/// This is what the fixed-iteration `settle()` loops in the LSP suites
/// became: the same three ticks, but stopping when the condition holds
/// and reporting what was seen when it does not.
#[track_caller]
pub fn tick_until<T>(
    state: &mut pmacs::editor::EditorState,
    what: &str,
    deadline: Duration,
    mut probe: impl FnMut(&mut pmacs::editor::EditorState) -> Probe<T>,
) -> T {
    let start = Instant::now();
    let mut polls = 0u32;
    loop {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        polls += 1;
        let last = match probe(state) {
            Probe::Ready(value) => return value,
            Probe::Pending(seen) => seen,
        };
        let elapsed = start.elapsed();
        assert!(
            elapsed < deadline,
            "{}",
            Timeout {
                what: what.to_owned(),
                deadline,
                elapsed,
                polls,
                last,
            }
        );
        std::thread::sleep(TICK_POLL.min(deadline.saturating_sub(elapsed)));
    }
}

/// How long one readiness probe waits for the daemon's `Hello` after
/// its connect succeeded. The daemon binds its socket before it can
/// serve (`run_daemon` binds, then constructs the editor), so a connect
/// alone says nothing about readiness; on the hosted macOS runners the
/// gap is p50 597 ms and up to 1.1 s (run 34351035133). A probe that
/// connects and gets no `Hello` within this window reports `Pending`
/// and the wait tries again, so the deadline bounds the whole boot.
pub const HELLO_READ: Duration = Duration::from_millis(500);

/// Wait until a `pmacs --daemon` on `socket` **serves** --- accepts a
/// connection and answers it with a `Hello` --- or report why not.
///
/// Probes by connecting, never by `exists()`: a stale socket file
/// satisfies `exists` with nobody listening. Readiness is the `Hello`
/// and not the connect: before E5.0 the probe declared readiness on a
/// successful connect and read the `Hello` under [`HELLO_READ`] only
/// to keep the daemon's first send from logging, discarding the
/// result --- so a test whose own `Hello` read then ran under a
/// 200--250 ms timeout was bounded by the remainder of the daemon's
/// boot, which is the `read Hello` family (fifteen CI occurrences,
/// every one on a hosted macOS leg where bind-to-serving is p50 597
/// ms). Now the probe that first sees a `Hello` is the one that
/// returns, and every later connection finds a daemon that has already
/// served one.
///
/// A daemon that exits before serving is reported at once, with its
/// exit status, rather than after the whole deadline: the probe
/// observes the child, and an exit is terminal.
pub fn wait_for_daemon(
    socket: &std::path::Path,
    child: &mut std::process::Child,
    deadline: Duration,
) -> Result<(), Timeout> {
    let what = format!("a daemon serving on {}", socket.display());
    let start = Instant::now();
    let mut polls = 0u32;
    let outcome = wait(&what, deadline, || {
        polls += 1;
        match std::os::unix::net::UnixStream::connect(socket) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(HELLO_READ)).ok();
                match pmacs::transport::read_message::<pmacs::protocol::Hello>(&mut stream) {
                    Ok(_) => Probe::Ready(Ok(())),
                    Err(error) => Probe::Pending(format!(
                        "connected, but no Hello within {HELLO_READ:?}: {error}"
                    )),
                }
            }
            Err(connect) => match child.try_wait() {
                Ok(Some(status)) => Probe::Ready(Err(format!(
                    "the daemon exited with {status} before serving"
                ))),
                _ => Probe::Pending(format!("connect: {connect}")),
            },
        }
    });
    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(last)) => Err(Timeout {
            what,
            deadline,
            elapsed: start.elapsed(),
            polls,
            last,
        }),
        Err(timeout) => Err(timeout),
    }
}

/// The tick interval for [`tick_until`]: an in-process editor's frame
/// is cheap and its events arrive on pipes, so polling faster than the
/// socket waits costs little and shortens every LSP test.
pub const TICK_POLL: Duration = Duration::from_millis(2);

/// The default deadline for an in-process readiness wait. Generous on
/// purpose: a deadline asserts that something eventually happens, and
/// ten seconds on a loaded machine is still an order of magnitude past
/// what a fake server needs.
pub const DEADLINE: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_condition_that_already_holds_is_probed_once() {
        // The subject is "no sleep", and the probe count says it
        // exactly: the first probe runs before any sleep, so a second
        // one can only mean the wait slept first. Elapsed time measured
        // it indirectly, and a wall-clock assertion in a helper that
        // compiles into 33 test targets is the class of assertion D12
        // took out of the default run.
        let mut probes = 0u32;
        let value = expect("immediate", Duration::from_secs(5), || {
            probes += 1;
            Probe::Ready(7)
        });
        assert_eq!(value, 7);
        assert_eq!(
            probes, 1,
            "a condition true at the first probe must return on it"
        );
    }

    #[test]
    fn a_condition_that_becomes_true_is_returned_not_timed_out() {
        let mut n = 0;
        let value = wait("third poll", Duration::from_secs(5), || {
            n += 1;
            if n >= 3 {
                Probe::Ready(n)
            } else {
                Probe::Pending(format!("n={n}"))
            }
        })
        .expect("becomes ready");
        assert_eq!(value, 3);
    }

    #[test]
    fn a_timeout_reports_elapsed_polls_and_the_last_state() {
        let err = wait("never", Duration::from_millis(60), || {
            Probe::<()>::Pending("still nothing".to_owned())
        })
        .expect_err("must time out");
        assert_eq!(err.what, "never");
        assert!(err.elapsed >= Duration::from_millis(60), "{err}");
        // `>= 1` and not `>= 2`, deliberately. The count is asserted for
        // its *reporting* and not for its *rate*: a `wait` that timed out
        // without ever probing is the only regression this line can be
        // about, and `>= 1` still falsifies it, because `polls` is
        // incremented before every probe. `>= 2` asserted instead that a
        // 60 ms wall-clock window contains a second loop iteration, which
        // is 60 ms against a 20 ms `POLL` with no margin for the loop's
        // own overhead --- and nothing in the code guarantees it. It has
        // failed twice on that margin (#266): 279.8 ms locally under a
        // full sweep, and 67.0 ms with `1 polls` in CI, where the
        // deadline had passed at the first check, before the first sleep,
        // in an iteration that runs a no-op step and one allocation. D12
        // puts wall-clock numbers under `--perf`; this module's own
        // budget-free assertions belong in the default sweep, so the
        // assertion is the one that changed.
        assert!(err.polls >= 1, "{err}");
        assert_eq!(err.last, "still nothing");
        let text = err.to_string();
        assert!(
            text.contains("never did not become ready within 60ms"),
            "{text}"
        );
        assert!(text.contains("last observed: still nothing"), "{text}");
    }

    /// A daemon that dies before serving is reported on the probe that
    /// sees its exit, not after the deadline. The child is already
    /// reaped, so the first probe sees the exit; `polls == 1` is a count
    /// and not a wall-clock claim.
    #[test]
    fn a_daemon_that_exits_before_serving_is_reported_at_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("never.sock");
        let mut child = std::process::Command::new("false")
            .spawn()
            .expect("spawn false");
        let _ = child.wait();
        let err = wait_for_daemon(&socket, &mut child, Duration::from_secs(10))
            .expect_err("no daemon serves");
        assert_eq!(err.polls, 1, "{err}");
        assert!(err.last.contains("exited with"), "{err}");
        assert!(err.last.contains("before serving"), "{err}");
    }

    /// Readiness is a served `Hello`, not a successful connect: a
    /// listener that accepts and closes without a `Hello` is not ready,
    /// and the wait returns on the first connection that is answered.
    #[test]
    fn readiness_is_a_served_hello_not_a_connect() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("late.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = accepted.clone();
        let server = std::thread::spawn(move || {
            // Two connections are accepted and dropped unanswered (a
            // bound socket whose daemon is still booting); the third is
            // answered with a Hello.
            for mut stream in listener.incoming().flatten() {
                let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if n >= 3 {
                    let hello = pmacs::protocol::Hello {
                        protocol_version: 1,
                        assigned_frontend_id: pmacs::protocol::FrontendId(2),
                        instance_identity: pmacs::protocol::InstanceIdentity {
                            pmacs_version: "test".to_owned(),
                            build_hash: None,
                            instance_name: None,
                            uptime_secs: 0,
                            working_directory: "/".to_owned(),
                        },
                        instance_capabilities: pmacs::protocol::InstanceCapabilities::default(),
                    };
                    pmacs::transport::write_message(&mut stream, &hello).expect("write Hello");
                    let _ = stream.flush();
                    break;
                }
                drop(stream);
            }
        });
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let result = wait_for_daemon(&socket, &mut child, Duration::from_secs(10));
        let _ = child.kill();
        let _ = child.wait();
        // A wait that returned early (on a connect, say) leaves the
        // server blocked in `accept`; unblock it with connections of our
        // own so the join below cannot hang, and read the count before
        // they land.
        let probed = accepted.load(std::sync::atomic::Ordering::SeqCst);
        for _ in 0..3 {
            let _ = std::os::unix::net::UnixStream::connect(&socket);
        }
        server.join().expect("server thread");
        assert!(result.is_ok(), "{result:?}");
        assert!(
            probed >= 3,
            "the wait must keep probing past the unanswered connections; it returned after {probed}"
        );
    }
}
