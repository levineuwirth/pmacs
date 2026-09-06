// tests/common/ready.rs --- the one readiness wait.

//! A predicate plus a deadline, reporting elapsed time and the last
//! observed state when the deadline passes.
//!
//! Every wait in the integration suites goes through [`wait`] or
//! [`tick_until`]; a fixed `thread::sleep` is never a readiness wait.
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

/// Wait until a `pmacs --daemon` is listening on `socket`, or report
/// why not: the child's exit status if it died first, else the last
/// connect error. Probes by connecting, never by `exists()`: a stale
/// socket file satisfies `exists` with nobody listening. The `Hello` is
/// read so the daemon's first send succeeds and it logs no warning.
pub fn wait_for_daemon(
    socket: &std::path::Path,
    child: &mut std::process::Child,
    deadline: Duration,
) -> Result<(), Timeout> {
    wait(
        &format!("a daemon listening on {}", socket.display()),
        deadline,
        || match std::os::unix::net::UnixStream::connect(socket) {
            Ok(mut stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .ok();
                let _ = pmacs::transport::read_message::<pmacs::protocol::Hello>(&mut stream);
                Probe::Ready(())
            }
            Err(connect) => match child.try_wait() {
                Ok(Some(status)) => {
                    Probe::Pending(format!("the daemon exited with {status} before listening"))
                }
                _ => Probe::Pending(format!("connect: {connect}")),
            },
        },
    )
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
    fn a_condition_that_already_holds_returns_without_sleeping() {
        let start = Instant::now();
        let value = expect("immediate", Duration::from_secs(5), || Probe::Ready(7));
        assert_eq!(value, 7);
        assert!(start.elapsed() < Duration::from_secs(1));
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
        assert!(err.polls >= 2, "{err}");
        assert_eq!(err.last, "still nothing");
        let text = err.to_string();
        assert!(
            text.contains("never did not become ready within 60ms"),
            "{text}"
        );
        assert!(text.contains("last observed: still nothing"), "{text}");
    }
}
