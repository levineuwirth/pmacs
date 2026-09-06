// tests/common/reap.rs --- a child that cannot outlive its test.

//! Every daemon a test spawns goes through [`Reaped::spawn`], which puts
//! the child in its own process group and, on drop, signals the whole
//! group: `SIGTERM`, a bounded wait, then `SIGKILL`.
//!
//! Why a group and not a pid. A `pmacs --daemon` spawns language
//! servers, terminals and the processes they run; `Child::kill` reaches
//! the daemon alone and leaves the rest parented to init, still holding
//! sockets under a temp directory the test has already deleted. A group
//! signal reaches every descendant that did not deliberately leave the
//! group, and the one production spawn that does leave it (the GPU
//! frontend's managed daemon, `process_group(0)` in
//! `pmacs-gpu/src/attach.rs`) reports its pid so the probe fixture can
//! signal that daemon by pid. The gate's post-step then fails the run if
//! any `pmacs --daemon` bound under its `TMPDIR` is still alive, so a
//! spawner that bypasses this module is caught rather than tolerated.
//!
//! Why `Drop`. A test that panics between spawn and its own cleanup
//! never reaches that cleanup; a `Drop` runs during the unwind. Measured
//! before this module: two daemons survived each workspace sweep on the
//! development machine, and ten had accumulated over one day of sweeps.
//!
//! Included per suite with `#[path = "common/reap.rs"] mod reap;` or
//! through `mod common;`. `#![forbid(unsafe_code)]` holds: `process_group`
//! is a safe `CommandExt` method, and the signals go through `nix`.

#![allow(dead_code)] // not every including suite uses every helper

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

/// How long a group gets to exit after `SIGTERM` before `SIGKILL`.
pub const TERM_GRACE: Duration = Duration::from_secs(2);

/// A spawned child in its own process group, reaped on drop.
pub struct Reaped {
    child: Child,
    reaped: bool,
}

impl Reaped {
    /// Spawn `command` in a new process group whose id is the child's
    /// pid. Every other aspect of `command` is the caller's.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        command.process_group(0);
        let child = command.spawn()?;
        Ok(Self {
            child,
            reaped: false,
        })
    }

    /// The child's pid, which is also its process group id.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The process group this child leads.
    pub fn pgid(&self) -> Pid {
        Pid::from_raw(self.child.id().cast_signed())
    }

    /// The underlying child, for stdin handles and `try_wait`.
    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    /// `Child::try_wait`.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Whether the child has not yet been observed to exit.
    pub fn is_alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }

    /// Block until the child exits and return its status. The group is
    /// still signalled on drop, for anything the child left behind.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    /// Send `signal` to the whole group.
    pub fn signal_group(&self, signal: Signal) {
        let _ = killpg(self.pgid(), signal);
    }

    /// `SIGKILL` the whole group and the child: the abrupt end a test
    /// asks for when it is done with a daemon it has already observed.
    pub fn kill(&mut self) -> io::Result<()> {
        self.signal_group(Signal::SIGKILL);
        self.child.kill()
    }

    /// `SIGTERM` the group, wait up to [`TERM_GRACE`] for the child,
    /// then `SIGKILL` the group and reap the child. Idempotent.
    pub fn reap(&mut self) {
        if self.reaped {
            return;
        }
        self.reaped = true;
        self.signal_group(Signal::SIGTERM);
        let deadline = Instant::now() + TERM_GRACE;
        while self.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.signal_group(Signal::SIGKILL);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        self.reap();
    }
}

/// Signal a process by pid: `SIGTERM`, a bounded wait for it to be
/// gone, then `SIGKILL`. For daemons a fixture learns about only by pid,
/// because their spawner put them in a group of their own.
pub fn reap_pid(pid: u32) {
    let target = Pid::from_raw(pid.cast_signed());
    let _ = nix::sys::signal::kill(target, Signal::SIGTERM);
    let deadline = Instant::now() + TERM_GRACE;
    while Instant::now() < deadline {
        // Signal 0 probes existence without delivering anything.
        if nix::sys::signal::kill(target, None).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = nix::sys::signal::kill(target, Signal::SIGKILL);
}
