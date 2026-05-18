// process.rs --- T M4.4 process supervisor.

//! Process supervisor: spawn, monitor, signal, restart, reap child
//! processes. Pipe and PTY stdio. Per spec §3 (system overview) and
//! §5 (concurrency).
//!
//! # Topology
//!
//! The supervisor itself is **main-thread** state, like
//! [`crate::async_runtime::AsyncRuntime`]. Per-process I/O reader
//! threads stream stdout/stderr (or PTY master output) over a
//! crossbeam channel; [`ProcessSupervisor::tick`] drains the channel
//! into per-process event queues and polls each running child for
//! exit, applying the configured [`RestartPolicy`] when a process
//! terminates.
//!
//! # Lifecycle
//!
//! ```text
//!   Starting  --(spawn ok)-->  Running
//!   Running   --(SIGTERM/kill)-->  Exiting
//!   Running   --(child exited)-->  Terminated
//!   Exiting   --(child exited)-->  Terminated
//!   Terminated  --(restart policy + backoff)-->  Starting
//! ```
//!
//! Every transition emits a [`ProcessEvent`] visible through
//! [`ProcessSupervisor::take_events`]. The main thread (and the Lua
//! surface in [`crate::lua_bindings`]) is the sole consumer.
//!
//! # Cleanup
//!
//! [`ProcessSupervisor::shutdown`] sends SIGTERM to every running
//! child, waits up to a grace period, then SIGKILLs anything still
//! alive. The supervisor's `Drop` impl calls `shutdown` so that an
//! editor exit (panic or normal) cannot leave zombies. The reader
//! threads join when their pipe end closes (which the kernel does
//! once the child is reaped) so they don't need explicit teardown.
//!
//! # `unsafe_code` boundary
//!
//! pmacs's crate-level `unsafe_code = "forbid"` lint stands. Signal
//! sending uses [`nix`] (safe wrapper around `kill(2)`); PTY
//! support uses [`portable-pty`], which contains internal `unsafe`
//! but exposes a fully safe surface. PTY line-discipline setup
//! (raw/canonical mode per spec §sec:repl-supervisor) bridges to
//! `tcsetattr(3)` via a `/bin/sh` trampoline instead of a local
//! `unsafe` block — see [`build_pty_command`] for the full
//! rationale.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam::channel::{self, Receiver, Sender};
use nix::sys::signal::Signal;
use nix::unistd::Pid;

use crate::ansi::{AnsiEvent, AnsiParser};

// ---------------------------------------------------------------------------
// Identity and configuration
// ---------------------------------------------------------------------------

/// Stable identifier for a managed process. Allocated in monotonic
/// order from a process-wide counter. A restart re-uses the same
/// id (the *managed* process is the same; the OS pid changes per
/// generation).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ProcessId(u64);

impl ProcessId {
    /// Mint a fresh id.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw counter value. Useful for debug formatting and Lua
    /// boundary marshalling.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProcessId({})", self.0)
    }
}

/// Line-discipline configuration for a PTY-mode child.
///
/// Per spec §sec:repl-supervisor: REPLs run in raw mode by default
/// so the child controls echo (the kernel does not echo input back
/// to the master). Canonical mode is preserved as a fallback for
/// non-shell line-oriented filters that `read()` from stdin and
/// expect kernel line buffering.
///
/// The mode is applied to the master's termios after `openpty` and
/// before the child is spawned, so the child inherits the mode
/// from its very first read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalMode {
    /// `cfmakeraw`-equivalent: no kernel echo, no canonical
    /// processing, no signal generation from input characters.
    /// Shells, language REPLs, and anything that calls `tcsetattr`
    /// itself want this default.
    Raw,
    /// Kernel default: line-buffered input, kernel echo on, signal
    /// characters interpreted (`Ctrl-C` raises `SIGINT`, etc.).
    /// Useful for line-oriented filters.
    Canonical,
}

/// I/O mode for a managed process.
///
/// `Pipes` allocates separate stdout / stderr readers and a stdin
/// writer, all unbuffered byte streams. `Pty` allocates a single
/// pty pair: stdin is the master writer, output is the master
/// reader (stdout + stderr are merged at the kernel level), and the
/// child sees a controlling tty. PTY mode is what enables
/// terminal-aware children (REPLs that probe `isatty`, programs that
/// emit ANSI escape sequences when interactive, etc.) per the M4.4
/// acceptance criterion. The `mode` field selects the line
/// discipline per spec §sec:repl-supervisor.
#[derive(Clone, Copy, Debug)]
pub enum ProcessMode {
    /// Three plain pipes (stdin, stdout, stderr).
    Pipes,
    /// PTY pair sized to `(rows, cols)`. The child's stdin/stdout/
    /// stderr are all the pty slave; the supervisor holds the
    /// master.
    Pty {
        /// Rows in the pty's window size (`TIOCSWINSZ`).
        rows: u16,
        /// Cols in the pty's window size.
        cols: u16,
        /// Line discipline applied before the child is spawned.
        mode: TerminalMode,
    },
}

impl ProcessMode {
    /// Convenience: a 24x80 PTY in raw mode (the conventional REPL
    /// default per spec §sec:repl-supervisor).
    #[must_use]
    pub const fn default_pty() -> Self {
        Self::Pty {
            rows: 24,
            cols: 80,
            mode: TerminalMode::Raw,
        }
    }
}

/// What to do when a managed process terminates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    /// Never restart. The process stays in
    /// [`ProcessState::Terminated`] forever.
    Never,
    /// Restart only on a non-clean exit (signal or crash).
    OnCrash,
    /// Restart on any termination (clean or otherwise).
    Always,
}

/// Description of a managed process.
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    /// Human-readable label. Surfaced in events and the
    /// `pmacs.process.list` output. Distinct from the program name
    /// so multiple processes can run the same binary with
    /// distinguishable labels.
    pub label: String,
    /// Program to execute. Looked up via the system PATH unless an
    /// absolute path is supplied.
    pub command: String,
    /// Argument vector (does *not* include `argv[0]`; the supervisor
    /// supplies that).
    pub args: Vec<String>,
    /// Working directory. `None` inherits from the editor process.
    pub cwd: Option<PathBuf>,
    /// Environment variables to set / override for the child. Each
    /// entry replaces the inherited value; absent entries are
    /// inherited untouched.
    pub env: Vec<(String, String)>,
    /// I/O mode (pipes vs pty).
    pub mode: ProcessMode,
    /// What to do on termination.
    pub restart: RestartPolicy,
    /// Parse PTY output on a worker and emit structured ANSI events
    /// instead of raw stdout bytes. Opt-in so LSP and other byte-stream
    /// consumers keep their existing stdout/stderr contract.
    pub ansi_events: bool,
}

impl ProcessSpec {
    /// Construct a spec with the bare-minimum fields. Convenience
    /// for tests and one-off scripts.
    #[must_use]
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            mode: ProcessMode::Pipes,
            restart: RestartPolicy::Never,
            ansi_events: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle state
// ---------------------------------------------------------------------------

/// State of one managed process generation.
///
/// A *generation* is one spawn-to-terminate cycle. Restarts mint a
/// new generation under the same [`ProcessId`].
#[derive(Clone, Debug)]
pub enum ProcessState {
    /// The supervisor is between `spawn` and the kernel returning a
    /// pid (very brief). Reflects "we asked for a fork, haven't
    /// observed the child yet."
    Starting,
    /// Child is alive. `pid` is the OS process id; `started` is the
    /// instant the supervisor observed the spawn.
    Running {
        /// OS process id.
        pid: u32,
        /// When the supervisor observed the spawn.
        started: Instant,
    },
    /// SIGTERM (or equivalent) has been sent; the supervisor is
    /// waiting for the child to actually exit. Distinct from
    /// `Running` so a `kill`-then-respawn caller doesn't
    /// double-signal.
    Exiting {
        /// Last known OS pid (before signal).
        pid: u32,
        /// When the supervisor sent the terminating signal.
        signaled_at: Instant,
    },
    /// Final state for this generation. Restart policy may transition
    /// back to `Starting` later.
    Terminated(Termination),
}

/// Reason a generation ended.
#[derive(Clone, Debug)]
pub enum Termination {
    /// Clean exit with a status code.
    Exited {
        /// Exit code as reported by the OS.
        code: i32,
        /// When the spawn happened.
        started: Instant,
        /// When the exit was observed.
        ended: Instant,
    },
    /// Killed by signal. `signal` is the symbolic name (e.g.
    /// `"SIGTERM"`).
    Signaled {
        /// Symbolic signal name as reported by the OS.
        signal: String,
        /// When the spawn happened.
        started: Instant,
        /// When the signal was observed.
        ended: Instant,
    },
    /// The supervisor itself failed to spawn or interact with the
    /// child --- the child never reached `Running`, or the kernel
    /// returned an error during a poll. `error` carries the
    /// `Display`-formatted error.
    Crashed {
        /// Failure description.
        error: String,
        /// When the supervisor noticed the failure.
        ended: Instant,
    },
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// One event in a process's lifecycle.
#[derive(Clone, Debug)]
pub struct ProcessEvent {
    /// Process the event belongs to.
    pub id: ProcessId,
    /// What happened.
    pub kind: ProcessEventKind,
    /// When the event was generated. Monotonic; useful for ordering
    /// and replay.
    pub at: Instant,
}

/// Discriminator over what kind of thing happened.
#[derive(Clone, Debug)]
pub enum ProcessEventKind {
    /// `spawn` returned successfully and the kernel reported a pid.
    Started {
        /// OS process id of the new generation.
        pid: u32,
    },
    /// A chunk of bytes from the child's stdout.
    Stdout(Vec<u8>),
    /// A chunk of bytes from the child's stderr. PTY-mode processes
    /// never emit `Stderr` (the pty merges output streams); they
    /// emit only `Stdout`.
    Stderr(Vec<u8>),
    /// Structured ANSI events decoded from a PTY byte stream on the
    /// parser worker. Only emitted when [`ProcessSpec::ansi_events`]
    /// is true.
    Ansi(Vec<AnsiEvent>),
    /// Child exited cleanly.
    Exited {
        /// Exit code.
        code: i32,
    },
    /// Child died from a signal.
    Signaled {
        /// Symbolic signal name (`"SIGTERM"`, `"SIGKILL"`, etc.).
        signal: String,
    },
    /// Spawn or supervision failed.
    Crashed {
        /// Failure description.
        error: String,
    },
    /// The supervisor is about to spawn a fresh generation per the
    /// configured restart policy. `attempt` counts cumulative spawn
    /// attempts (1 = first spawn, 2 = first restart, ...).
    Restarting {
        /// Cumulative spawn attempt number.
        attempt: u32,
    },
}

// ---------------------------------------------------------------------------
// Streaming pipeline constants (T M6.2)
// ---------------------------------------------------------------------------

/// Size of one read from a child's stdout/stderr or PTY master.
/// 8 KiB is a kernel-pipe sweet spot: large enough that small
/// outputs land in one read, small enough that a saturating producer
/// emits chunks at a steady rate (rather than one giant read after
/// a long block).
pub const BYTE_CHUNK_SIZE: usize = 8 * 1024;

/// In-flight byte ceiling for a single generation's output, per spec
/// §sec:repl-streaming. The bounded channel between the reader thread
/// and the supervisor is sized so a saturating producer fills the
/// channel, then stalls in `send`, then stalls in `read`, then the
/// kernel pipe fills, then the child stalls in `write` --- which is
/// the right outcome. Default 1 MiB matches the spec's PTY-read →
/// parser ceiling.
pub const PTY_READ_CEILING_BYTES: usize = 1 << 20;

/// Capacity of the per-generation byte chunk channel. With 8 KiB
/// chunks and a 1 MiB ceiling, this is 128 slots.
const BYTE_CHUNK_CHANNEL_CAP: usize = PTY_READ_CEILING_BYTES / BYTE_CHUNK_SIZE;

/// In-flight structured-event ceiling between ANSI parser worker and
/// main-thread supervisor drain. The spec names this as 256 KiB; with
/// 8 KiB read chunks this is 32 parser batches in flight.
pub const ANSI_EVENT_CEILING_BYTES: usize = 256 * 1024;
const ANSI_EVENT_CHANNEL_CAP: usize = ANSI_EVENT_CEILING_BYTES / BYTE_CHUNK_SIZE;

/// How long a reader thread waits in a bounded `send` before polling
/// its cancel flag. 50 ms is long enough that healthy steady-state
/// flow doesn't burn cycles re-checking, short enough that a
/// shutting-down supervisor sees readers exit promptly.
const READER_SEND_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Bounded grace window used when a child has exited but its reader /
/// parser worker may still have already-read bytes in flight. This is
/// not process termination grace; it is only the final output flush
/// before the runtime handles are dropped.
const EXIT_OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Owner of all managed processes. One per editor.
pub struct ProcessSupervisor {
    processes: HashMap<ProcessId, ManagedProcess>,
    events_tx: Sender<ProcessEvent>,
    events_rx: Receiver<ProcessEvent>,
    /// Buffered events per process, populated by `tick` from
    /// `events_rx` and drained by [`Self::take_events`].
    pending: HashMap<ProcessId, Vec<ProcessEvent>>,
    /// SIGTERM-then-SIGKILL grace window for shutdown.
    grace_period: Duration,
    /// Restart back-off (constant for v0.1; M4.5+ may add
    /// exponential).
    restart_backoff: Duration,
    /// True once `shutdown()` has run; subsequent `spawn` calls
    /// fail.
    shut_down: bool,
}

struct ManagedProcess {
    spec: ProcessSpec,
    state: ProcessState,
    runtime: Option<RuntimeHandles>,
    attempt_count: u32,
    /// When the supervisor should attempt the next restart, or
    /// `None` if no restart is pending.
    next_restart_at: Option<Instant>,
}

/// Handles tied to one running generation. Dropped (and joined)
/// when the generation ends.
struct RuntimeHandles {
    child: ChildHandle,
    stdin: Option<Box<dyn Write + Send>>,
    pid: u32,
    /// Reader-thread join handles, drained by `Drop` of
    /// [`RuntimeHandles`] so a generation's worker threads don't
    /// outlive the supervisor.
    readers: Vec<JoinHandle<()>>,
    /// Bounded output channel drained by the supervisor. Raw processes
    /// expose bytes directly; ANSI-enabled PTY processes expose parser
    /// batches from the worker stage.
    output_rx: RuntimeOutputRx,
    /// Cancel flag observed by reader threads when their bounded
    /// `send` blocks. Set on generation end / supervisor drop so a
    /// reader stuck in `send` (consumer fell behind) wakes promptly
    /// instead of leaking until the kernel ends the producer.
    cancel: Arc<AtomicBool>,
}

impl Drop for RuntimeHandles {
    fn drop(&mut self) {
        // Wake any reader thread blocked in a bounded `send` ---
        // dropping the master closes the kernel pipe and unblocks
        // `read`, but does nothing for a reader stuck on a full
        // channel because the consumer fell behind. Cancel flag
        // unwedges that case before we join. T M6.2.
        self.cancel.store(true, Ordering::Relaxed);
        for h in std::mem::take(&mut self.readers) {
            let _ = h.join();
        }
    }
}

/// Reader-thread output: one chunk read from a stream of a given
/// kind (stdout vs stderr; PTY-mode generations only ever emit
/// `Stdout`). Lives on the per-generation bounded byte channel.
type ByteChunk = (ReaderKind, Vec<u8>);

type AnsiBatch = Vec<AnsiEvent>;

enum RuntimeOutputRx {
    Bytes(Receiver<ByteChunk>),
    Ansi(Receiver<AnsiBatch>),
}

/// Discriminated wrapper over a pipe-mode `std::process::Child` and
/// a pty-mode portable-pty pair. The variants share `try_wait` /
/// pid retrieval through a thin enum match.
enum ChildHandle {
    Pipes(std::process::Child),
    Pty {
        child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
        /// Master end held for as long as the generation lives;
        /// dropping the master closes all reader/writer handles
        /// derived from it. Held in `_master` even though we only
        /// access it via the cloned reader/writer to ensure the
        /// drop order is right.
        _master: Box<dyn portable_pty::MasterPty + Send>,
    },
}

impl ChildHandle {
    /// Non-blocking poll for child exit.
    fn try_wait(&mut self) -> Result<Option<TermStatus>, String> {
        match self {
            Self::Pipes(child) => match child.try_wait() {
                Ok(None) => Ok(None),
                Ok(Some(status)) => Ok(Some(TermStatus::from_std(status))),
                Err(e) => Err(format!("try_wait: {e}")),
            },
            Self::Pty { child, .. } => {
                let mut guard = child.lock().expect("pty child mutex poisoned");
                match guard.try_wait() {
                    Ok(None) => Ok(None),
                    Ok(Some(status)) => Ok(Some(TermStatus::from_pty(&status))),
                    Err(e) => Err(format!("try_wait: {e}")),
                }
            }
        }
    }
}

/// Termination status of one generation. Internal --- the supervisor
/// translates this into a [`Termination`] with timing.
enum TermStatus {
    Exited(i32),
    Signaled(String),
}

impl TermStatus {
    fn from_std(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                // Resolve to a symbolic name (`SIGTERM`, `SIGKILL`,
                // ...) when the signal number is one nix knows
                // about; fall back to a numeric placeholder
                // otherwise. The Display of `nix::sys::signal::Signal`
                // produces the SIGFOO form.
                let label = match nix::sys::signal::Signal::try_from(sig) {
                    Ok(s) => s.as_str().to_owned(),
                    Err(_) => format!("SIG{sig}"),
                };
                return Self::Signaled(label);
            }
        }
        Self::Exited(status.code().unwrap_or(-1))
    }

    fn from_pty(status: &portable_pty::ExitStatus) -> Self {
        if let Some(sig) = status.signal() {
            // portable-pty stringifies via libc::strsignal, which
            // returns descriptions ("Interrupt") rather than the
            // symbolic SIGFOO name. We canonicalize to match the
            // pipe-mode path (`from_unix`) so M6.5's exit-marker
            // contract surfaces "SIGINT" identically across modes.
            Self::Signaled(canonicalize_pty_signal_name(sig))
        } else {
            // portable-pty's exit code is `u32`; values above
            // i32::MAX are exotic and treated as -1.
            let code = i32::try_from(status.exit_code()).unwrap_or(-1);
            Self::Exited(code)
        }
    }
}

/// Map `libc::strsignal` description strings (as surfaced by
/// `portable-pty`) to symbolic SIGFOO names. Unknown descriptions pass
/// through unchanged — better to surface an unfamiliar string than to
/// fabricate a wrong name. Covers every signal in
/// [`super::lua_bindings::parse_signal`]'s accept-list plus the common
/// fault signals that surface during process crashes.
fn canonicalize_pty_signal_name(desc: &str) -> String {
    match desc {
        "Interrupt" => "SIGINT".to_owned(),
        "Terminated" => "SIGTERM".to_owned(),
        "Killed" => "SIGKILL".to_owned(),
        "Hangup" => "SIGHUP".to_owned(),
        "Quit" => "SIGQUIT".to_owned(),
        "User defined signal 1" => "SIGUSR1".to_owned(),
        "User defined signal 2" => "SIGUSR2".to_owned(),
        "Aborted" => "SIGABRT".to_owned(),
        "Segmentation fault" => "SIGSEGV".to_owned(),
        "Floating point exception" => "SIGFPE".to_owned(),
        "Illegal instruction" => "SIGILL".to_owned(),
        "Broken pipe" => "SIGPIPE".to_owned(),
        "Alarm clock" => "SIGALRM".to_owned(),
        "Bus error" => "SIGBUS".to_owned(),
        other => other.to_owned(),
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSupervisor {
    /// Construct an empty supervisor with sensible defaults
    /// (`grace_period` = 2s, `restart_backoff` = 250ms).
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = channel::unbounded();
        Self {
            processes: HashMap::new(),
            events_tx: tx,
            events_rx: rx,
            pending: HashMap::new(),
            grace_period: Duration::from_secs(2),
            restart_backoff: Duration::from_millis(250),
            shut_down: false,
        }
    }

    /// Override the SIGTERM-to-SIGKILL grace window. Test helper.
    pub fn set_grace_period(&mut self, d: Duration) {
        self.grace_period = d;
    }

    /// Override the restart back-off. Test helper.
    pub fn set_restart_backoff(&mut self, d: Duration) {
        self.restart_backoff = d;
    }

    /// Spawn a new managed process. Returns a stable id; consult
    /// [`Self::state`] / [`Self::take_events`] to follow its
    /// lifecycle.
    ///
    /// Errors only on synchronous spawn failure (the kernel rejects
    /// the exec, the binary is unreadable, etc.). A child that
    /// crashes *after* spawn shows up as a [`Termination::Crashed`]
    /// in the event stream, not as a return error.
    pub fn spawn(&mut self, spec: ProcessSpec) -> Result<ProcessId, String> {
        if self.shut_down {
            return Err("supervisor is shut down".to_owned());
        }
        let id = ProcessId::next();
        let mut managed = ManagedProcess {
            spec,
            state: ProcessState::Starting,
            runtime: None,
            attempt_count: 0,
            next_restart_at: None,
        };
        self.start_generation(id, &mut managed)?;
        self.processes.insert(id, managed);
        Ok(id)
    }

    /// Start a fresh generation for `managed`. Mutates `managed`
    /// in place; on failure the state is left as
    /// `Terminated(Crashed{...})` and an event is emitted.
    fn start_generation(&self, id: ProcessId, managed: &mut ManagedProcess) -> Result<(), String> {
        managed.attempt_count += 1;
        managed.next_restart_at = None;
        match build_runtime(&managed.spec, id) {
            Ok(runtime) => {
                let pid = runtime.pid;
                let now = Instant::now();
                managed.state = ProcessState::Running { pid, started: now };
                managed.runtime = Some(runtime);
                let _ = self.events_tx.send(ProcessEvent {
                    id,
                    kind: ProcessEventKind::Started { pid },
                    at: now,
                });
                Ok(())
            }
            Err(e) => {
                let now = Instant::now();
                managed.state = ProcessState::Terminated(Termination::Crashed {
                    error: e.clone(),
                    ended: now,
                });
                managed.runtime = None;
                let _ = self.events_tx.send(ProcessEvent {
                    id,
                    kind: ProcessEventKind::Crashed { error: e.clone() },
                    at: now,
                });
                Err(e)
            }
        }
    }

    /// Send `signal` to `id`. Errors if the id is unknown or the
    /// process is not currently running. The signal is applied to
    /// the OS pid via [`nix::sys::signal::kill`]; nothing about the
    /// supervisor's state changes synchronously --- the lifecycle
    /// transition happens when the supervisor next observes the
    /// child's exit through `tick`.
    pub fn signal(&mut self, id: ProcessId, signal: Signal) -> Result<(), String> {
        let proc = self
            .processes
            .get_mut(&id)
            .ok_or_else(|| format!("unknown process: {id}"))?;
        let (ProcessState::Running { pid, .. } | ProcessState::Exiting { pid, .. }) = proc.state
        else {
            return Err(format!("process {id} is not running"));
        };
        let nix_pid = Pid::from_raw(i32::try_from(pid).map_err(|e| e.to_string())?);
        nix::sys::signal::kill(nix_pid, Some(signal)).map_err(|e| format!("kill: {e}"))?;
        if matches!(signal, Signal::SIGTERM | Signal::SIGKILL | Signal::SIGHUP) {
            proc.state = ProcessState::Exiting {
                pid,
                signaled_at: Instant::now(),
            };
        }
        Ok(())
    }

    /// Convenience wrapper around `signal(id, SIGTERM)`. On a v0.1
    /// SIGTERM-tolerant child this is the polite shutdown path; the
    /// supervisor's `shutdown` enforces the SIGKILL fallback if the
    /// child doesn't exit within the grace window.
    pub fn terminate(&mut self, id: ProcessId) -> Result<(), String> {
        self.signal(id, Signal::SIGTERM)
    }

    /// Close `id`'s stdin pipe by dropping the writer. The child
    /// observes EOF on its next read, which is the canonical
    /// stdio-graceful-shutdown signal for protocols (notably MCP)
    /// that have no protocol-level shutdown message. Idempotent: a
    /// second call after the writer is gone is a no-op. Errors only
    /// if the process id is unknown.
    ///
    /// Note: this does NOT kill the process. Callers that want a
    /// guaranteed exit follow up with [`Self::terminate`] (SIGTERM)
    /// after a grace window, and the supervisor's
    /// [`Self::shutdown`] applies the SIGKILL fallback at editor
    /// drop time.
    pub fn close_stdin(&mut self, id: ProcessId) -> Result<(), String> {
        let proc = self
            .processes
            .get_mut(&id)
            .ok_or_else(|| format!("unknown process: {id}"))?;
        if let Some(runtime) = proc.runtime.as_mut() {
            // Dropping the writer closes the pipe at the kernel
            // level. `take()` is idempotent — second call sees None.
            let _ = runtime.stdin.take();
        }
        Ok(())
    }

    /// Write `bytes` to `id`'s stdin. Errors if the id is unknown,
    /// the process is not running, or stdin is closed (the child
    /// closed stdin on its end, or stdin was never piped in the
    /// first place). Synchronous write --- callers that worry about
    /// pipe-full blocking should chunk their writes.
    pub fn write_stdin(&mut self, id: ProcessId, bytes: &[u8]) -> Result<(), String> {
        let proc = self
            .processes
            .get_mut(&id)
            .ok_or_else(|| format!("unknown process: {id}"))?;
        let runtime = proc
            .runtime
            .as_mut()
            .ok_or_else(|| format!("process {id} has no live generation"))?;
        let stdin = runtime
            .stdin
            .as_mut()
            .ok_or_else(|| format!("process {id} stdin is not piped"))?;
        stdin
            .write_all(bytes)
            .map_err(|e| format!("write_stdin: {e}"))?;
        stdin.flush().map_err(|e| format!("flush_stdin: {e}"))?;
        Ok(())
    }

    /// Resize the PTY for `id`. Errors if the id is unknown, the
    /// process isn't running, or the process is in pipe mode.
    pub fn resize_pty(&mut self, id: ProcessId, rows: u16, cols: u16) -> Result<(), String> {
        let proc = self
            .processes
            .get_mut(&id)
            .ok_or_else(|| format!("unknown process: {id}"))?;
        let runtime = proc
            .runtime
            .as_mut()
            .ok_or_else(|| format!("process {id} has no live generation"))?;
        match &runtime.child {
            ChildHandle::Pty {
                _master: master, ..
            } => master
                .resize(portable_pty::PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("resize_pty: {e}")),
            ChildHandle::Pipes(_) => Err(format!("process {id} is not a PTY process")),
        }
    }

    /// Drain pending events into per-process buffers, poll each
    /// running child for exit, and apply restart policy. Call once
    /// per editor frame.
    pub fn tick(&mut self) {
        // Drain pending lifecycle events from the supervisor-wide
        // unbounded channel (Started/Exited/Signaled/Crashed/Restarting
        // — small, infrequent, not subject to byte-stream backpressure).
        while let Ok(ev) = self.events_rx.try_recv() {
            self.pending.entry(ev.id).or_default().push(ev);
        }
        // Drain per-generation byte channels and coalesce into one
        // event per (process, kind) per tick. T M6.2 / spec
        // §sec:repl-streaming + M3.5 coalescing model: many small
        // chunks land as O(ticks) events, not O(chunks) events.
        let ids: Vec<ProcessId> = self.processes.keys().copied().collect();
        for id in &ids {
            self.drain_byte_channel(*id);
        }
        // Poll each managed process; ids re-iterated to avoid
        // mutating the map while iterating.
        for id in ids {
            self.poll_one(id);
            self.maybe_restart(id);
        }
    }

    /// Drain the per-generation byte channel for `id` and emit at
    /// most one `Stdout` and one `Stderr` event into pending. Called
    /// from `tick()`. No-op if the process has no live runtime.
    fn drain_byte_channel(&mut self, id: ProcessId) {
        let drained = {
            let Some(proc) = self.processes.get(&id) else {
                return;
            };
            let Some(rt) = proc.runtime.as_ref() else {
                return;
            };
            match &rt.output_rx {
                RuntimeOutputRx::Bytes(byte_rx) => drain_raw_output(byte_rx),
                RuntimeOutputRx::Ansi(ansi_rx) => drain_ansi_output(ansi_rx),
            }
        };
        if drained.is_empty() {
            return;
        }
        let now = Instant::now();
        let queue = self.pending.entry(id).or_default();
        for kind in drained {
            queue.push(ProcessEvent { id, kind, at: now });
        }
    }

    /// Poll one process for exit. Transitions Running/Exiting →
    /// Terminated and emits the appropriate event.
    fn poll_one(&mut self, id: ProcessId) {
        let Some(proc) = self.processes.get_mut(&id) else {
            return;
        };
        let started = match proc.state {
            ProcessState::Running { started, .. } => started,
            ProcessState::Exiting { signaled_at, .. } => signaled_at,
            _ => return,
        };
        let Some(runtime) = proc.runtime.as_mut() else {
            return;
        };
        match runtime.child.try_wait() {
            Ok(None) => {}
            Ok(Some(TermStatus::Exited(code))) => {
                let now = Instant::now();
                let final_output = final_drain_runtime(runtime);
                proc.state = ProcessState::Terminated(Termination::Exited {
                    code,
                    started,
                    ended: now,
                });
                proc.runtime = None;
                append_process_events(&mut self.pending, id, final_output, now);
                self.pending.entry(id).or_default().push(ProcessEvent {
                    id,
                    kind: ProcessEventKind::Exited { code },
                    at: now,
                });
            }
            Ok(Some(TermStatus::Signaled(signal))) => {
                let now = Instant::now();
                let final_output = final_drain_runtime(runtime);
                proc.state = ProcessState::Terminated(Termination::Signaled {
                    signal: signal.clone(),
                    started,
                    ended: now,
                });
                proc.runtime = None;
                append_process_events(&mut self.pending, id, final_output, now);
                self.pending.entry(id).or_default().push(ProcessEvent {
                    id,
                    kind: ProcessEventKind::Signaled { signal },
                    at: now,
                });
            }
            Err(e) => {
                let now = Instant::now();
                let final_output = final_drain_runtime(runtime);
                proc.state = ProcessState::Terminated(Termination::Crashed {
                    error: e.clone(),
                    ended: now,
                });
                proc.runtime = None;
                append_process_events(&mut self.pending, id, final_output, now);
                self.pending.entry(id).or_default().push(ProcessEvent {
                    id,
                    kind: ProcessEventKind::Crashed { error: e },
                    at: now,
                });
            }
        }
    }

    /// Apply restart policy after `poll_one` may have transitioned
    /// the process to `Terminated`.
    fn maybe_restart(&mut self, id: ProcessId) {
        let now = Instant::now();
        let restart_now = {
            let Some(proc) = self.processes.get(&id) else {
                return;
            };
            let ProcessState::Terminated(termination) = &proc.state else {
                return;
            };
            let policy = proc.spec.restart;
            let should = match (policy, termination) {
                (RestartPolicy::Never, _) => false,
                (RestartPolicy::Always, _)
                | (
                    RestartPolicy::OnCrash,
                    Termination::Signaled { .. } | Termination::Crashed { .. },
                ) => true,
                (RestartPolicy::OnCrash, Termination::Exited { code, .. }) => *code != 0,
            };
            if !should {
                return;
            }
            // Schedule the restart after `restart_backoff` from the
            // termination time; we don't synchronously block.
            match proc.next_restart_at {
                Some(at) => at <= now,
                None => false,
            }
        };
        if restart_now {
            // Borrow mutably for the actual restart.
            let mut managed = self.processes.remove(&id).expect("checked existence above");
            let attempt = managed.attempt_count + 1;
            self.pending.entry(id).or_default().push(ProcessEvent {
                id,
                kind: ProcessEventKind::Restarting { attempt },
                at: now,
            });
            let _ = self.start_generation(id, &mut managed);
            self.processes.insert(id, managed);
        } else {
            // Schedule a restart attempt for `restart_backoff` from
            // now if not yet scheduled.
            if let Some(proc) = self.processes.get_mut(&id)
                && matches!(proc.state, ProcessState::Terminated(_))
                && !matches!(proc.spec.restart, RestartPolicy::Never)
                && proc.next_restart_at.is_none()
            {
                proc.next_restart_at = Some(now + self.restart_backoff);
            }
        }
    }

    /// Drain and return all events queued for `id` since the last
    /// call. Returns an empty vec for unknown ids and for known ids
    /// that haven't produced events yet.
    pub fn take_events(&mut self, id: ProcessId) -> Vec<ProcessEvent> {
        self.pending.remove(&id).unwrap_or_default()
    }

    /// Drain every queued event across every process. Returns events
    /// in the order they were enqueued. Useful for `*processes*`
    /// log-style buffers and tests.
    pub fn take_all_events(&mut self) -> Vec<ProcessEvent> {
        let mut all = Vec::new();
        for (_id, mut evs) in std::mem::take(&mut self.pending) {
            all.append(&mut evs);
        }
        all.sort_by_key(|e| e.at);
        all
    }

    /// Current state of `id`, or `None` if the id is unknown.
    #[must_use]
    pub fn state(&self, id: ProcessId) -> Option<&ProcessState> {
        self.processes.get(&id).map(|p| &p.state)
    }

    /// Spec for `id`, or `None` if the id is unknown.
    #[must_use]
    pub fn spec(&self, id: ProcessId) -> Option<&ProcessSpec> {
        self.processes.get(&id).map(|p| &p.spec)
    }

    /// Iterator over every managed process id, in arbitrary order.
    pub fn ids(&self) -> impl Iterator<Item = ProcessId> + '_ {
        self.processes.keys().copied()
    }

    /// Forget about `id`. The process must already be terminated;
    /// otherwise this returns an error and leaves the process
    /// alone. Use [`Self::terminate`] + tick + `forget` to
    /// permanently remove a running process.
    pub fn forget(&mut self, id: ProcessId) -> Result<(), String> {
        let proc = self
            .processes
            .get(&id)
            .ok_or_else(|| format!("unknown process: {id}"))?;
        if !matches!(proc.state, ProcessState::Terminated(_)) {
            return Err(format!("process {id} is not terminated"));
        }
        self.processes.remove(&id);
        self.pending.remove(&id);
        Ok(())
    }

    /// Send SIGTERM to every running process; wait up to the grace
    /// period for them to exit; SIGKILL anything still alive.
    /// Idempotent. Called automatically from `Drop`.
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        // SIGTERM phase.
        let ids: Vec<ProcessId> = self.processes.keys().copied().collect();
        for id in &ids {
            let _ = self.signal(*id, Signal::SIGTERM);
        }
        // Poll-loop with timeout. tick() is slightly heavier than
        // we need (it does restart accounting), but it's the
        // canonical exit-observation path.
        let deadline = Instant::now() + self.grace_period;
        while Instant::now() < deadline && self.any_running() {
            self.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        // SIGKILL anything left.
        for id in &ids {
            if let Some(proc) = self.processes.get(id)
                && matches!(
                    proc.state,
                    ProcessState::Running { .. } | ProcessState::Exiting { .. }
                )
            {
                let _ = self.signal(*id, Signal::SIGKILL);
            }
        }
        // Final reap loop. SIGKILL is delivered immediately by the
        // kernel; the child becomes a zombie until we reap. Bound
        // the wait so a pathological case can't hang the editor
        // exit forever.
        let final_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < final_deadline && self.any_running() {
            self.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn any_running(&self) -> bool {
        self.processes.values().any(|p| {
            matches!(
                p.state,
                ProcessState::Running { .. }
                    | ProcessState::Starting
                    | ProcessState::Exiting { .. }
            )
        })
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Spawn machinery
// ---------------------------------------------------------------------------

/// Build a fresh runtime (handles + reader threads) per
/// `spec`. Pulled out of [`ProcessSupervisor::start_generation`] so
/// the supervisor itself is small and the pipe-vs-pty branching
/// lives in one place.
///
/// Lifecycle events (Started/Exited/Signaled/Crashed/Restarting) are
/// emitted by the supervisor itself onto its unbounded
/// `events_tx`; reader threads emit only byte chunks onto the
/// per-generation bounded byte channel. T M6.2.
fn build_runtime(spec: &ProcessSpec, id: ProcessId) -> Result<RuntimeHandles, String> {
    if spec.ansi_events && matches!(spec.mode, ProcessMode::Pipes) {
        return Err("process spawn: ansi=true requires pty mode; pipe-mode consumers receive raw stdout/stderr bytes".to_owned());
    }
    match spec.mode {
        ProcessMode::Pipes => build_pipes_runtime(spec, id),
        ProcessMode::Pty { rows, cols, mode } => build_pty_runtime(spec, id, rows, cols, mode),
    }
}

fn build_pipes_runtime(spec: &ProcessSpec, _id: ProcessId) -> Result<RuntimeHandles, String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(ref cwd) = spec.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let pid = child.id();
    let stdin = child
        .stdin
        .take()
        .map(|s| Box::new(s) as Box<dyn Write + Send>);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (byte_tx, byte_rx) = channel::bounded::<ByteChunk>(BYTE_CHUNK_CHANNEL_CAP);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    if let Some(out) = stdout {
        readers.push(spawn_reader(
            byte_tx.clone(),
            Arc::clone(&cancel),
            out,
            ReaderKind::Stdout,
        ));
    }
    if let Some(err) = stderr {
        readers.push(spawn_reader(
            byte_tx,
            Arc::clone(&cancel),
            err,
            ReaderKind::Stderr,
        ));
    }
    Ok(RuntimeHandles {
        child: ChildHandle::Pipes(child),
        stdin,
        pid,
        readers,
        output_rx: RuntimeOutputRx::Bytes(byte_rx),
        cancel,
    })
}

fn build_pty_runtime(
    spec: &ProcessSpec,
    _id: ProcessId,
    rows: u16,
    cols: u16,
    mode: TerminalMode,
) -> Result<RuntimeHandles, String> {
    use portable_pty::PtySize;

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty: {e}"))?;

    // Line discipline per spec §sec:repl-supervisor is applied by
    // wrapping the command in a /bin/sh trampoline that calls
    // `stty` before `exec`-ing the target binary. Canonical mode is
    // the kernel default for a freshly-allocated PTY on Linux/macOS,
    // so it requires no trampoline.
    if matches!(mode, TerminalMode::Raw) && !std::path::Path::new("/bin/sh").is_file() {
        return Err("pty spawn: raw terminal mode requires /bin/sh \
             (the supervisor uses a /bin/sh trampoline to apply line \
             discipline per spec §sec:repl-supervisor); install a \
             sh-compatible shell at /bin/sh, or configure this \
             process with TerminalMode::Canonical"
            .to_owned());
    }
    let mut cmd = build_pty_command(spec, mode);
    if let Some(ref cwd) = spec.cwd {
        cmd.cwd(cwd);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("pty spawn: {e}"))?;
    let pid = child
        .process_id()
        .ok_or_else(|| "pty pid missing".to_owned())?;
    // Drop the slave: `child` keeps it alive on its end. Holding it
    // ourselves is unnecessary and prevents EOF detection on the
    // master once the child exits.
    drop(pair.slave);
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("pty writer: {e}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("pty reader: {e}"))?;
    let (byte_tx, byte_rx) = channel::bounded::<ByteChunk>(BYTE_CHUNK_CHANNEL_CAP);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut readers = vec![spawn_reader(
        byte_tx,
        Arc::clone(&cancel),
        reader,
        ReaderKind::Stdout,
    )];
    let output_rx = if spec.ansi_events {
        let (ansi_tx, ansi_rx) = channel::bounded::<AnsiBatch>(ANSI_EVENT_CHANNEL_CAP);
        readers.push(spawn_ansi_parser(byte_rx, ansi_tx, Arc::clone(&cancel)));
        RuntimeOutputRx::Ansi(ansi_rx)
    } else {
        RuntimeOutputRx::Bytes(byte_rx)
    };
    Ok(RuntimeHandles {
        child: ChildHandle::Pty {
            child: Arc::new(Mutex::new(into_send_sync_child(child))),
            _master: pair.master,
        },
        stdin: Some(writer),
        pid,
        readers,
        output_rx,
        cancel,
    })
}

/// portable-pty's `Child` is `Send` but not necessarily `Sync`.
/// Wrapping in a Mutex makes the supervisor's `try_wait` callable
/// from the main thread without `unsafe`. The `Sync` bound on the
/// supervisor's [`ChildHandle::Pty::child`] field is satisfied via
/// `Arc<Mutex<...>>`.
fn into_send_sync_child(
    child: Box<dyn portable_pty::Child + Send + Sync>,
) -> Box<dyn portable_pty::Child + Send + Sync> {
    child
}

/// Build the [`portable_pty::CommandBuilder`] for a PTY-mode child,
/// applying the requested line discipline.
///
/// # Why a `/bin/sh` trampoline (not a direct `tcsetattr`)
///
/// portable-pty 0.9 exposes no `set_termios` and no pre-exec hook.
/// `nix::sys::termios::tcsetattr` requires `AsFd`, and converting
/// `MasterPty::as_raw_fd` (a `RawFd`) to `AsFd` requires
/// `BorrowedFd::borrow_raw`, which is `unsafe`. pmacs's crate-level
/// `unsafe_code = "forbid"` rule is a project-identity property
/// (see `MEMORY.md` / `feedback_unsafe_code_posture.md`), not a
/// negotiable lint, so we trampoline through `/bin/sh` instead:
///
/// ```sh
/// /bin/sh -c 'stty raw -echo </dev/tty 2>/dev/null; exec "$@"' -- CMD ARGS...
/// ```
///
/// # Why this is shell-injection-safe
///
/// The argv-as-positional-parameters mechanism is the standard
/// pattern (the same one `xargs -0` relies on). When you invoke
/// `sh -c 'SCRIPT' -- ARG1 ARG2 ARG3`, the shell receives:
///
/// - `SCRIPT` as the literal source code to execute
/// - `--` as `$0` (the script name)
/// - `ARG1`, `ARG2`, `ARG3` as the positional parameters `$1`,
///   `$2`, `$3`
///
/// Critically, the positional parameters are **literal data from
/// the moment they enter `sh`'s argv**; the shell never re-parses
/// them. `"$@"` then expands to `"$1" "$2" "$3"` with each parameter
/// as a separate word, regardless of whether they contain spaces,
/// quotes, semicolons, or any other shell metacharacters. There is
/// no path through which user-controlled `spec.command` or
/// `spec.args` can become shell tokens; they remain argv all the
/// way through to `exec`.
///
/// # Why the redirections
///
/// `</dev/tty`: `stty` operates on its controlling terminal, which
/// in the trampoline's context is the PTY slave that's about to
/// become the child's stdin. `stty`'s default of "operate on stdin"
/// usually does the right thing, but the explicit redirection is
/// belt-and-braces for cases where the supervisor has fiddled with
/// stdin or the PTY is in some unusual state.
///
/// `2>/dev/null`: silences `stty` errors. If the slave isn't a tty
/// `stty` recognizes, we proceed to `exec` regardless and the child
/// runs in the kernel default (canonical) instead — graceful
/// degradation rather than a confusing failure mode.
///
/// # Canonical mode
///
/// `TerminalMode::Canonical` skips the trampoline entirely. A
/// freshly-allocated PTY's kernel default on Linux/macOS is
/// canonical + echo + isig, which is exactly the canonical-mode
/// contract from spec §sec:repl-supervisor. Adding a no-op `stty`
/// invocation would be churn.
fn build_pty_command(spec: &ProcessSpec, mode: TerminalMode) -> portable_pty::CommandBuilder {
    use portable_pty::CommandBuilder;
    match mode {
        TerminalMode::Raw => {
            let mut cmd = CommandBuilder::new("/bin/sh");
            cmd.arg("-c");
            cmd.arg("stty raw -echo </dev/tty 2>/dev/null; exec \"$@\"");
            cmd.arg("--");
            cmd.arg(&spec.command);
            for arg in &spec.args {
                cmd.arg(arg);
            }
            cmd
        }
        TerminalMode::Canonical => {
            let mut cmd = CommandBuilder::new(&spec.command);
            for arg in &spec.args {
                cmd.arg(arg);
            }
            cmd
        }
    }
}

#[derive(Clone, Copy)]
enum ReaderKind {
    Stdout,
    Stderr,
}

/// Spawn a reader thread that pulls [`BYTE_CHUNK_SIZE`] chunks off
/// `read` and pushes them onto the per-generation bounded byte
/// channel. T M6.2 / spec §sec:repl-streaming.
///
/// Backpressure: when `byte_tx` is full (consumer fell behind), the
/// reader's `send` blocks. The kernel pipe then fills, the child's
/// `write` syscall blocks, and the producer rate is rate-limited to
/// the consumer's drain rate — exactly the spec's stalling chain.
///
/// Cancellation: blocked sends are pre-empted by `cancel`. Without
/// this, a reader stuck in `send` because the consumer fell behind
/// permanently would leak until OS-level pipe teardown reaches it
/// (which only happens once the producer is reaped). The cancel
/// flag is what makes "cancellation propagates to source" prompt.
///
/// Exits on: EOF (`Ok(0)`), closed channel (consumer dropped),
/// cancel flag set, or read error.
fn spawn_reader<R: Read + Send + 'static>(
    byte_tx: Sender<ByteChunk>,
    cancel: Arc<AtomicBool>,
    mut read: R,
    kind: ReaderKind,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; BYTE_CHUNK_SIZE];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match read.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    let mut payload: ByteChunk = (kind, buf[..n].to_vec());
                    loop {
                        match byte_tx.send_timeout(payload, READER_SEND_POLL_INTERVAL) {
                            Ok(()) => break,
                            Err(crossbeam::channel::SendTimeoutError::Timeout(rejected)) => {
                                if cancel.load(Ordering::Relaxed) {
                                    return;
                                }
                                payload = rejected;
                            }
                            Err(crossbeam::channel::SendTimeoutError::Disconnected(_)) => {
                                return;
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    })
}

fn drain_raw_output(byte_rx: &Receiver<ByteChunk>) -> Vec<ProcessEventKind> {
    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();
    while let Ok((kind, mut bytes)) = byte_rx.try_recv() {
        match kind {
            ReaderKind::Stdout => stdout_buf.append(&mut bytes),
            ReaderKind::Stderr => stderr_buf.append(&mut bytes),
        }
    }
    let mut out = Vec::with_capacity(2);
    if !stdout_buf.is_empty() {
        out.push(ProcessEventKind::Stdout(stdout_buf));
    }
    if !stderr_buf.is_empty() {
        out.push(ProcessEventKind::Stderr(stderr_buf));
    }
    out
}

fn drain_ansi_output(ansi_rx: &Receiver<AnsiBatch>) -> Vec<ProcessEventKind> {
    let mut events: Vec<AnsiEvent> = Vec::new();
    while let Ok(mut batch) = ansi_rx.try_recv() {
        events.append(&mut batch);
    }
    if events.is_empty() {
        Vec::new()
    } else {
        vec![ProcessEventKind::Ansi(events)]
    }
}

fn drain_runtime_output(rt: &RuntimeHandles) -> Vec<ProcessEventKind> {
    match &rt.output_rx {
        RuntimeOutputRx::Bytes(byte_rx) => drain_raw_output(byte_rx),
        RuntimeOutputRx::Ansi(ansi_rx) => drain_ansi_output(ansi_rx),
    }
}

fn final_drain_runtime(rt: &RuntimeHandles) -> Vec<ProcessEventKind> {
    let deadline = Instant::now() + EXIT_OUTPUT_DRAIN_TIMEOUT;
    let mut out = Vec::new();
    loop {
        let drained = drain_runtime_output(rt);
        let drained_any = !drained.is_empty();
        out.extend(drained);
        if rt.readers.iter().all(std::thread::JoinHandle::is_finished) && !drained_any {
            return out;
        }
        if Instant::now() >= deadline {
            return out;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn append_process_events(
    pending: &mut HashMap<ProcessId, Vec<ProcessEvent>>,
    id: ProcessId,
    kinds: Vec<ProcessEventKind>,
    at: Instant,
) {
    if kinds.is_empty() {
        return;
    }
    let queue = pending.entry(id).or_default();
    for kind in kinds {
        queue.push(ProcessEvent { id, kind, at });
    }
}

/// Spawn the ANSI parser worker for an ANSI-enabled PTY generation.
///
/// The reader thread remains responsible for the 1 MiB PTY-read ceiling.
/// This stage consumes those chunks, maintains parser state across chunk
/// boundaries, and forwards structured events through a second bounded
/// channel whose capacity represents the spec's 256 KiB parser→main
/// ceiling.
fn spawn_ansi_parser(
    byte_rx: Receiver<ByteChunk>,
    ansi_tx: Sender<AnsiBatch>,
    cancel: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut parser = AnsiParser::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let (kind, bytes) = match byte_rx.recv_timeout(READER_SEND_POLL_INTERVAL) {
                Ok(chunk) => chunk,
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => return,
            };
            if !matches!(kind, ReaderKind::Stdout) {
                continue;
            }
            let mut events = parser.feed(&bytes);
            if events.is_empty() {
                continue;
            }
            loop {
                match ansi_tx.send_timeout(events, READER_SEND_POLL_INTERVAL) {
                    Ok(()) => break,
                    Err(crossbeam::channel::SendTimeoutError::Timeout(rejected)) => {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        events = rejected;
                    }
                    Err(crossbeam::channel::SendTimeoutError::Disconnected(_)) => return,
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_until<F: Fn(&[ProcessEvent]) -> bool>(
        sup: &mut ProcessSupervisor,
        id: ProcessId,
        deadline: Duration,
        predicate: F,
    ) -> Vec<ProcessEvent> {
        let stop = Instant::now() + deadline;
        let mut all = Vec::new();
        while Instant::now() < stop {
            sup.tick();
            let mut evs = sup.take_events(id);
            all.append(&mut evs);
            if predicate(&all) {
                return all;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    fn has_exited(events: &[ProcessEvent]) -> bool {
        events.iter().any(|e| {
            matches!(
                e.kind,
                ProcessEventKind::Exited { .. } | ProcessEventKind::Signaled { .. }
            )
        })
    }

    #[test]
    fn spawn_pipes_lifecycle_started_then_exited() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("echo-test", "/bin/sh");
        spec.args = vec!["-c".into(), "echo hello && exit 0".into()];
        let id = sup.spawn(spec).expect("spawn");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Started { .. })),
            "must observe Started"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.kind, ProcessEventKind::Stdout(b) if b.starts_with(b"hello"))),
            "must observe stdout 'hello'"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Exited { code: 0 })),
            "must observe Exited{{code:0}}"
        );
    }

    #[test]
    fn signal_terminates_a_running_child() {
        let mut sup = ProcessSupervisor::new();
        // `sleep 30` is long enough that the test definitely needs
        // to terminate it deliberately.
        let mut spec = ProcessSpec::new("sleeper", "/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        let id = sup.spawn(spec).expect("spawn");
        // Wait for Started so we have a pid.
        let _ = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
        });
        sup.terminate(id).expect("terminate");
        let after = drain_until(&mut sup, id, Duration::from_secs(5), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Signaled { .. }))
        });
        assert!(
            after
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Signaled { .. })),
            "SIGTERM should produce a Signaled event"
        );
    }

    #[test]
    fn restart_on_crash_respawns_after_nonzero_exit() {
        let mut sup = ProcessSupervisor::new();
        sup.set_restart_backoff(Duration::from_millis(10));
        let mut spec = ProcessSpec::new("crasher", "/bin/sh");
        spec.args = vec!["-c".into(), "exit 7".into()];
        spec.restart = RestartPolicy::OnCrash;
        let id = sup.spawn(spec).expect("spawn");
        // Wait for at least one restart (Restarting + a second Started).
        let evs = drain_until(&mut sup, id, Duration::from_secs(5), |evs| {
            evs.iter()
                .filter(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
                .count()
                >= 2
        });
        let started_count = evs
            .iter()
            .filter(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
            .count();
        assert!(
            started_count >= 2,
            "OnCrash restart should respawn after non-zero exit; saw Started count {started_count}"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Restarting { .. })),
            "must emit Restarting"
        );
        // Stop the loop before test exit to keep things tidy.
        sup.processes.get_mut(&id).unwrap().spec.restart = RestartPolicy::Never;
    }

    #[test]
    fn restart_never_does_not_respawn_after_clean_exit() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("oneshot", "/bin/sh");
        spec.args = vec!["-c".into(), "exit 0".into()];
        let id = sup.spawn(spec).expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(2), has_exited);
        // Several more ticks; no restart should occur.
        for _ in 0..5 {
            sup.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        let starts = sup
            .take_events(id)
            .iter()
            .chain(sup.take_all_events().iter())
            .filter(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
            .count();
        assert_eq!(starts, 0, "no further Started events expected");
        let proc = sup.processes.get(&id).expect("still tracked");
        assert!(matches!(proc.state, ProcessState::Terminated(_)));
    }

    #[test]
    fn drop_supervisor_kills_running_children() {
        // Spawn a long-running child, drop the supervisor, and
        // verify the child is gone (try sending signal 0 via nix:
        // ESRCH means already reaped). Bounded wait because zombie
        // reaping is asynchronous on some platforms.
        let pid = {
            let mut sup = ProcessSupervisor::new();
            sup.set_grace_period(Duration::from_millis(200));
            let mut spec = ProcessSpec::new("victim", "/bin/sh");
            spec.args = vec!["-c".into(), "sleep 30".into()];
            let id = sup.spawn(spec).expect("spawn");
            // Drain until Started so we know the pid.
            let _ = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
                evs.iter()
                    .any(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
            });
            let ProcessState::Running { pid, .. } = sup.state(id).cloned().unwrap() else {
                panic!("expected Running");
            };
            pid
        }; // sup drops here -> shutdown -> SIGTERM/SIGKILL
        // Give the kernel a brief moment to deliver the signal; the
        // bounded loop tolerates jitter.
        let nix_pid = Pid::from_raw(i32::try_from(pid).unwrap());
        let dead_or_unknown = || {
            // signal 0 returns Ok if pid exists, ESRCH otherwise.
            // After Drop the child is reaped or dead; ESRCH is the
            // expected outcome.
            matches!(
                nix::sys::signal::kill(nix_pid, None),
                Err(nix::errno::Errno::ESRCH)
            )
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dead_or_unknown() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            dead_or_unknown(),
            "child pid {pid} should be reaped/gone after supervisor Drop"
        );
    }

    #[test]
    fn pty_mode_child_sees_a_tty() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("ttytest", "/bin/sh");
        spec.args = vec!["-c".into(), "tty".into()];
        spec.mode = ProcessMode::default_pty();
        let id = sup.spawn(spec).expect("spawn");
        let evs = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        // Concatenate all stdout chunks; tty(1) prints the path of
        // the controlling terminal, which on Linux/macOS starts with
        // /dev/pts/ or /dev/ttys.
        let mut out = Vec::new();
        for e in &evs {
            if let ProcessEventKind::Stdout(bytes) = &e.kind {
                out.extend_from_slice(bytes);
            }
        }
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("/dev/pts/") || s.contains("/dev/ttys"),
            "tty(1) should report a pty path in PTY mode; got {s:?}"
        );
    }

    fn collect_stdout(events: &[ProcessEvent]) -> String {
        let mut out = Vec::new();
        for e in events {
            if let ProcessEventKind::Stdout(bytes) = &e.kind {
                out.extend_from_slice(bytes);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// T M6.1: PTY child observes SIGWINCH when the supervisor
    /// resizes its window. The child traps WINCH and echoes the new
    /// dimensions; we resize and look for the marker on stdout.
    #[test]
    fn m6_1_pty_resize_delivers_sigwinch_to_child() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("winch-watch", "/bin/sh");
        // Trap WINCH, print READY for synchronization, then loop on
        // a short sleep so SIGWINCH can interrupt and fire the trap.
        spec.args = vec![
            "-c".into(),
            "trap 'echo RESIZED:$(stty size)' WINCH; \
             echo READY; \
             while :; do sleep 0.05; done"
                .into(),
        ];
        spec.mode = ProcessMode::default_pty();
        let id = sup.spawn(spec).expect("spawn");

        // Wait for READY so we know the trap is installed before we
        // signal.
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), |evs| {
            collect_stdout(evs).contains("READY")
        });

        sup.resize_pty(id, 40, 120).expect("resize");

        let evs = drain_until(&mut sup, id, Duration::from_secs(5), |evs| {
            collect_stdout(evs).contains("RESIZED:")
        });
        let stdout = collect_stdout(&evs);

        // `stty size` prints "rows cols" on Linux, with possible
        // leading/trailing whitespace differences across platforms.
        assert!(
            stdout.contains("RESIZED:40 120") || stdout.contains("RESIZED: 40 120"),
            "child should observe SIGWINCH and report new size 40x120; \
             collected stdout was: {stdout:?}"
        );

        // Stop the loop so the test exits cleanly.
        let _ = sup.terminate(id);
    }

    /// T M6.1: a PTY-mode child that exits cleanly produces the same
    /// `Started` -> `Exited` lifecycle as a pipe-mode child, with
    /// reader threads joining on EOF and the supervisor reaching
    /// `Terminated` state.
    #[test]
    fn m6_1_pty_mode_lifecycle_started_then_exited() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("pty-exit", "/bin/sh");
        spec.args = vec!["-c".into(), "echo done && exit 0".into()];
        spec.mode = ProcessMode::default_pty();
        let id = sup.spawn(spec).expect("spawn");

        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Started { .. })),
            "must observe Started for PTY-mode child"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.kind, ProcessEventKind::Stdout(b) if b.windows(4).any(|w| w == b"done"))),
            "must observe stdout 'done'"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Exited { code: 0 })),
            "PTY-mode child should exit cleanly with code 0"
        );
        let proc = sup.processes.get(&id).expect("still tracked");
        assert!(matches!(proc.state, ProcessState::Terminated(_)));
    }

    /// T M6.1: `TerminalMode::Raw` (the default) produces a PTY where
    /// the kernel does not echo input. Verified by running `stty -a`
    /// inside the PTY and looking for `-echo` and `-icanon` in its
    /// output.
    #[test]
    fn m6_1_pty_raw_mode_disables_kernel_echo() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("raw-stty", "/bin/sh");
        spec.args = vec!["-c".into(), "stty -a".into()];
        spec.mode = ProcessMode::default_pty(); // Raw by default.
        let id = sup.spawn(spec).expect("spawn");
        let evs = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let stdout = collect_stdout(&evs);
        assert!(
            stdout.contains("-echo"),
            "raw mode should disable echo; stty -a output was: {stdout:?}"
        );
        assert!(
            stdout.contains("-icanon"),
            "raw mode should disable canonical input; stty -a output was: {stdout:?}"
        );
    }

    /// T M6.1: `TerminalMode::Canonical` keeps the kernel default,
    /// where echo and canonical input are enabled. Mirrors the raw
    /// test in reverse.
    #[test]
    fn m6_1_pty_canonical_mode_keeps_kernel_echo() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("canon-stty", "/bin/sh");
        spec.args = vec!["-c".into(), "stty -a".into()];
        spec.mode = ProcessMode::Pty {
            rows: 24,
            cols: 80,
            mode: TerminalMode::Canonical,
        };
        let id = sup.spawn(spec).expect("spawn");
        let evs = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let stdout = collect_stdout(&evs);
        // Disambiguate `echo` from `-echo` (raw) and from longer flag
        // names like `iexten`. Word-boundary check via padded match.
        assert!(
            (stdout.contains(" echo ")
                || stdout.contains(" echo\n")
                || stdout.contains("\necho ")
                || stdout.starts_with("echo "))
                && !stdout.contains("-echo "),
            "canonical mode should leave echo enabled (no `-echo` flag); \
             stty -a output was: {stdout:?}"
        );
    }

    /// Test helper: number of byte chunks currently buffered in the
    /// per-generation bounded channel for `id`. Used by M6.2 tests to
    /// observe backpressure saturation.
    fn byte_channel_len(sup: &ProcessSupervisor, id: ProcessId) -> usize {
        sup.processes
            .get(&id)
            .and_then(|p| p.runtime.as_ref())
            .map_or(0, |rt| match &rt.output_rx {
                RuntimeOutputRx::Bytes(rx) => rx.len(),
                RuntimeOutputRx::Ansi(_) => 0,
            })
    }

    /// T M6.2 acceptance bullet 1: the per-generation byte channel
    /// caps in-flight bytes at the spec's 1 MiB ceiling, so a
    /// saturating producer stalls in `write` rather than ballooning
    /// supervisor memory. Asserts (a) the bounded channel never
    /// exceeds its slot cap, (b) the producer is still alive after a
    /// pause-drain window (was actually backpressured, not just very
    /// slow), and (c) every byte is delivered once draining resumes.
    #[test]
    fn m6_2_pty_streaming_respects_byte_ceiling() {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        // 10 MiB target: comfortably more than (1 MiB channel + 64
        // KiB kernel pipe), so the producer must stall in write
        // rather than fitting the entire payload in the un-drained
        // buffers.
        const TOTAL: usize = 10 * 1024 * 1024;
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("byte-flood", "/bin/sh");
        spec.args = vec!["-c".into(), format!("head -c {TOTAL} /dev/zero")];
        let id = sup.spawn(spec).expect("spawn");

        // Read pid synchronously --- spawn → start_generation
        // already set the state to Running. We deliberately do NOT
        // call drain_until(Started) here: that path ticks the
        // supervisor in a loop, which drains the byte channel and
        // unsticks the reader, defeating the saturation observation
        // we are about to make.
        let pid = match sup.state(id).expect("tracked") {
            ProcessState::Running { pid, .. } => *pid,
            s => panic!("expected Running immediately after spawn; got {s:?}"),
        };

        // Pause draining: no `tick()` calls. The reader thread
        // saturates the bounded channel, the kernel pipe fills, the
        // producer stalls in `write`.
        std::thread::sleep(Duration::from_millis(200));

        // (a) bounded channel never exceeds the slot cap.
        let in_flight = byte_channel_len(&sup, id);
        assert!(
            in_flight <= BYTE_CHUNK_CHANNEL_CAP,
            "byte channel must be bounded by {BYTE_CHUNK_CHANNEL_CAP} \
             slots; observed {in_flight}"
        );
        assert!(
            in_flight > 0,
            "after 200 ms producing {TOTAL} bytes without draining, \
             the channel should have data; got {in_flight}"
        );

        // (b) the producer is still alive --- having NOT delivered
        // 10 MiB through a ~1 MiB ceiling means it is stalled in
        // write. We use an OS-level liveness check (kill(0)) rather
        // than the supervisor's cached state, since the cached
        // state is only updated by `tick()` and ticking would drain
        // the channel.
        let live = kill(
            Pid::from_raw(i32::try_from(pid).expect("pid fits i32")),
            None,
        )
        .is_ok();
        assert!(
            live,
            "producer (pid {pid}) should still be alive (stalled in write); \
             without backpressure, head would have written 10 MiB and exited"
        );

        // (c) drain to completion; verify exact byte count is
        // preserved across the backpressure-release boundary.
        let mut total = 0usize;
        let deadline = Instant::now() + Duration::from_secs(15);
        while total < TOTAL && Instant::now() < deadline {
            sup.tick();
            for ev in sup.take_events(id) {
                if let ProcessEventKind::Stdout(b) = ev.kind {
                    total += b.len();
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            total, TOTAL,
            "expected {TOTAL} bytes after backpressure releases; got {total}"
        );
    }

    /// T M6.2 / M3.5 coalescing: many in-flight chunks present at
    /// the same tick produce one coalesced `Stdout` event, not one
    /// event per chunk. The supervisor concatenates per-process
    /// bytes into a single event per tick (per kind).
    #[test]
    fn m6_2_pty_streaming_coalesces_per_tick() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("chunky-stream", "/bin/sh");
        // 1 MiB of zeros from /dev/zero. The reader thread reads in
        // [`BYTE_CHUNK_SIZE`] (8 KiB) chunks --- ~128 reads --- all
        // queued onto the bounded channel within microseconds of
        // each other. Without coalescing, those would surface as
        // ~128 separate `Stdout` events; with coalescing, they
        // merge into a small handful (one per tick that drains
        // them). The trailing `END` marker is purely a
        // synchronization tag.
        spec.args = vec!["-c".into(), "head -c 1048576 /dev/zero; echo END".into()];
        let id = sup.spawn(spec).expect("spawn");

        let evs = drain_until(&mut sup, id, Duration::from_secs(10), |evs| {
            evs.iter().any(
                |e| matches!(&e.kind, ProcessEventKind::Stdout(b) if b.windows(3).any(|w| w == b"END")),
            )
        });
        let stdout_event_count = evs
            .iter()
            .filter(|e| matches!(e.kind, ProcessEventKind::Stdout(_)))
            .count();
        let total_bytes: usize = evs
            .iter()
            .filter_map(|e| match &e.kind {
                ProcessEventKind::Stdout(b) => Some(b.len()),
                _ => None,
            })
            .sum();

        // No-loss assertion: every byte arrives (1 MiB + "END\n").
        assert!(
            total_bytes >= 1_048_580,
            "expected ≥ 1 048 580 stdout bytes (1 MiB + END\\n); got {total_bytes}"
        );
        // Coalescing assertion: ~128 underlying reads collapse to a
        // small handful of events. A loose ceiling of 16 absorbs
        // scheduler jitter and the eventual END-marker tick; the
        // typical observed count is 1--3. An un-coalesced path
        // would emit ≥ 128 events.
        assert!(
            stdout_event_count <= 16,
            "too many stdout events ({stdout_event_count}); coalescing \
             not engaged --- expected O(ticks), not O(reads)"
        );
    }

    #[test]
    fn m6_2_ansi_enabled_pty_emits_structured_events() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("ansi-stream", "/bin/sh");
        spec.args = vec!["-c".into(), "printf '\\033[31mhi\\033[0m\\n'".into()];
        spec.mode = ProcessMode::Pty {
            rows: 24,
            cols: 80,
            mode: TerminalMode::Canonical,
        };
        spec.ansi_events = true;
        let id = sup.spawn(spec).expect("spawn ansi pty");
        let evs = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        assert!(
            evs.iter()
                .all(|e| !matches!(e.kind, ProcessEventKind::Stdout(_))),
            "ansi-enabled PTY should not surface raw stdout events: {evs:?}"
        );
        let mut saw_red = false;
        let mut saw_text = false;
        for ev in evs {
            if let ProcessEventKind::Ansi(events) = ev.kind {
                for event in events {
                    match event {
                        AnsiEvent::SetStyle(style)
                            if style.fg == crate::cell::Color::Indexed(1) =>
                        {
                            saw_red = true;
                        }
                        AnsiEvent::Text(text) if text.contains("hi") => {
                            saw_text = true;
                        }
                        _ => {}
                    }
                }
            }
        }
        assert!(saw_red, "expected structured red SetStyle event");
        assert!(saw_text, "expected structured text event");
    }

    #[test]
    fn m6_2_ansi_parser_worker_exits_when_reader_channel_closes() {
        let (byte_tx, byte_rx) = channel::bounded::<ByteChunk>(1);
        let (ansi_tx, _ansi_rx) = channel::bounded::<AnsiBatch>(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = spawn_ansi_parser(byte_rx, ansi_tx, Arc::clone(&cancel));
        drop(byte_tx);

        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline && !handle.is_finished() {
            std::thread::sleep(Duration::from_millis(5));
        }
        if !handle.is_finished() {
            cancel.store(true, Ordering::Relaxed);
        }
        assert!(
            handle.is_finished(),
            "ANSI parser worker should exit once the byte reader channel closes"
        );
        handle.join().expect("parser worker join");
    }

    /// T M6.2 acceptance bullet 2: stream cancellation propagates to
    /// the source. A long-lived producer with the consumer paused
    /// (so the reader is blocked in `send`) still terminates
    /// promptly when the supervisor is shut down --- the
    /// `RuntimeHandles::Drop` cancel-flag wake-out unblocks the
    /// reader before the join, and `shutdown` reaps the producer
    /// within the grace window.
    #[test]
    fn m6_2_pty_streaming_cancellation_propagates_to_child() {
        use std::sync::mpsc;

        // Run the supervisor in a worker thread so the test can
        // bound how long the cancellation path takes via a oneshot.
        let (done_tx, done_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut sup = ProcessSupervisor::new();
            sup.set_grace_period(Duration::from_millis(300));
            let mut spec = ProcessSpec::new("forever-flood", "/bin/sh");
            // Continuous writer; SIGTERM kills it (no signal handler).
            spec.args = vec!["-c".into(), "while :; do printf 'X'; done".into()];
            let id = sup.spawn(spec).expect("spawn");

            // Wait for the bounded channel to saturate. We are NOT
            // ticking, so once the channel is full the reader
            // thread is blocked in `send_timeout`.
            let saturate_deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < saturate_deadline
                && byte_channel_len(&sup, id) < BYTE_CHUNK_CHANNEL_CAP
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                byte_channel_len(&sup, id),
                BYTE_CHUNK_CHANNEL_CAP,
                "channel should saturate when consumer doesn't drain"
            );

            // Drop the supervisor. Its `Drop` runs `shutdown`:
            // SIGTERM, tick (which drains and unblocks the reader),
            // possibly SIGKILL, then `RuntimeHandles::Drop` which
            // sets the cancel flag and joins the reader. Any of
            // these mechanisms suffices to unblock the reader; the
            // assertion is that shutdown completes within a bound.
            drop(sup);
            let _ = done_tx.send(());
        });

        done_rx.recv_timeout(Duration::from_secs(5)).expect(
            "supervisor drop should complete within 5s --- if hung, \
                 cancellation is not propagating to a reader blocked in \
                 send (per-generation cancel flag is required)",
        );
        handle.join().expect("test thread should exit cleanly");
    }
}
