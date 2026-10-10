// tests/common/bounded.rs --- a bound on a whole row, held outside it.

//! Run a row's body on a thread of its own under a bound that the
//! test's thread holds, for a row that has hung rather than failed.
//!
//! **Containment, not repair.** Nothing here says why a row blocks. It
//! changes what a block costs: past its limit the row fails, naming the
//! last step its body entered and the processes beneath the test
//! binary, and the binary goes on to its other rows and exits, so the
//! binaries queued behind it run. Written for #313 in
//! `tests/lean4_server_acceptance.rs`, whose stall's cause is still
//! unknown; shared since #327, #344 and #349, three rows that did not
//! return on a macOS leg and held it to its 45-minute limit, every
//! binary after them unrun.
//!
//! **Why the deadline is outside the body.** Every wait in these suites
//! checks its deadline between ticks, so a call that never returns ---
//! a tick, an eval, a drop --- is past any deadline the body itself can
//! check. Here the deadline is a `recv_timeout` on the test's thread,
//! which a body blocked anywhere cannot hold up. The body's teardown
//! runs inside the bound, as named steps, because a drop can be the
//! call that hangs. The stuck thread cannot be unblocked, so it is
//! left, not joined; the binary's exit ends it.
//!
//! **What it covers.** The rows that call it, and no other. Any other
//! row that blocks still holds its leg until the leg's own limit. A
//! bound on every row is #351, a separate piece of work.
//!
//! **The stall seam.** `PMACS_BOUNDED_STALL` holds step names separated
//! by `;`. A body entering a step named there parks for good at its
//! entry, which is a call that never returns, placed: the witness for
//! the bound. A name is matched exactly, against every bounded row in
//! the binary. Nothing sets it in the gate or in CI.
//!
//! Included per suite with `#[path = "common/bounded.rs"] mod bounded;`.

#![allow(dead_code)] // not every including suite uses every helper

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// The step a stalled body is parked in, by name, when the environment
/// asks for one. See the module docs.
pub const STALL_ENV: &str = "PMACS_BOUNDED_STALL";

/// The last step a [`bounded`] body entered, and when.
pub struct Steps(Arc<Mutex<(&'static str, Instant)>>);

impl Steps {
    /// Record `step` as the one the body is in. Under the stall seam a
    /// step named in [`STALL_ENV`] never returns from here.
    pub fn enter(&self, step: &'static str) {
        *self.0.lock().unwrap() = (step, Instant::now());
        if std::env::var(STALL_ENV).is_ok_and(|names| names.split(';').any(|name| name == step)) {
            loop {
                std::thread::park();
            }
        }
    }
}

/// Run `body` under a bound of `limit` on the whole row, its teardown
/// included.
///
/// The body runs on a thread named after the test's own, and the
/// test's thread waits for it. Past `limit` the row fails, naming the
/// last step the body entered and the processes beneath this binary;
/// the body's thread is left where it stopped. A body that panics
/// fails the row with its own message.
pub fn bounded(limit: Duration, body: impl FnOnce(&Steps) + Send + 'static) {
    let steps = Steps(Arc::new(Mutex::new(("starting", Instant::now()))));
    let last = Arc::clone(&steps.0);
    let (tx, rx) = mpsc::channel();
    let name = std::thread::current()
        .name()
        .unwrap_or("bounded row")
        .to_owned();
    let handle = std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            body(&steps);
            let _ = tx.send(());
        })
        .expect("spawn the row's body");
    match rx.recv_timeout(limit) {
        Ok(()) => handle.join().expect("the body returned"),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(payload) = handle.join() {
                std::panic::resume_unwind(payload);
            }
            unreachable!("the body ended without returning or panicking");
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let (step, since) = *last.lock().unwrap();
            panic!(
                "the row did not finish within {limit:?}: its last step, {step:?}, \
                 was entered {:?} ago and has not returned (a panic above, if any, \
                 is the body's own, and the stall is in its unwinding); processes \
                 beneath this test binary:\n{}",
                since.elapsed(),
                processes_beneath(std::process::id())
            );
        }
    }
}

/// `ps` rows for every process descended from `root`, `ps` itself
/// left out, for a row's report when it has stopped.
pub fn processes_beneath(root: u32) -> String {
    let child = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,stat=,etime=,command="])
        .stdout(std::process::Stdio::piped())
        .spawn();
    let Ok(child) = child else {
        return "(ps did not start)".to_owned();
    };
    let ps = child.id();
    let Ok(out) = child.wait_with_output() else {
        return "(ps did not finish)".to_owned();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<(u32, u32, &str)> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            Some((pid, ppid, line.trim()))
        })
        .collect();
    let mut parents = vec![root];
    let mut found = Vec::new();
    let mut i = 0;
    while i < parents.len() {
        for &(pid, ppid, line) in &rows {
            if ppid == parents[i] && pid != ps {
                parents.push(pid);
                found.push(line);
            }
        }
        i += 1;
    }
    if found.is_empty() {
        "(none)".to_owned()
    } else {
        found.join("\n")
    }
}
