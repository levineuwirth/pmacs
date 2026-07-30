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

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam::channel::{self, Receiver, Sender};
use nix::sys::signal::Signal;
use nix::unistd::Pid;

use crate::ansi::{AnsiEvent, AnsiParser, AnsiParserProfile};

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

/// Stdin disposition for a pipe-mode child.
///
/// Compile-mode (Q#CM3) runs noninteractive commands that may probe
/// or read stdin (`cat`, tools that block on a tty check); `Null`
/// gives them immediate EOF from `/dev/null` with no writer thread
/// and no close-after-spawn race. PTY children have no separable
/// stdin, so `Null` is rejected at spawn under PTY mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdinMode {
    /// Piped writer thread (the default; see [`StdinWriter`]).
    Piped,
    /// `/dev/null`: immediate EOF; `write_stdin` errors with the
    /// stdin-not-piped message.
    Null,
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
    /// Compatibility profile for structured ANSI parsing. Ignored unless
    /// `ansi_events` is true; ordinary process/Lua callers remain line-oriented.
    pub ansi_profile: AnsiParserProfile,
    /// Stdin disposition (pipe-mode only; rejected under PTY).
    pub stdin: StdinMode,
    /// Compile-mode group lifecycle (Q#CM3; pipe-mode only, rejected
    /// under PTY — PTY children already lead their own session).
    /// When set: the child is spawned as the leader of a fresh
    /// process group (`process_group(0)`), fatal signals are
    /// group-directed (negative pid, mirroring the PTY branch of
    /// [`signal_target`]), the group receives SIGTERM and enters the
    /// liveness-probed reap ledger on the leader's terminal event,
    /// and the generation's readers are poll-based and cancellable
    /// so teardown is bounded even when an escaped descendant holds
    /// the output pipe.
    pub group: bool,
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
            ansi_profile: AnsiParserProfile::LineOriented,
            stdin: StdinMode::Piped,
            group: false,
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

/// TERM→KILL escalation window for `group = true` process groups
/// (Q#CM3). Armed into the reap ledger when the group receives
/// SIGTERM — on explicit kill/supersede and on the leader's terminal
/// event —
/// and enforced both by the per-tick ledger probe and from inside
/// the group-aware final drain loop. Deliberately short: this is
/// child-tree cleanup, not polite application shutdown (the polite
/// TERM already went out when the window starts).
pub const GROUP_TERM_GRACE: Duration = Duration::from_millis(500);

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
    /// fail and `maybe_restart` is inert (a `restart = always`
    /// process must not respawn mid-teardown).
    shut_down: bool,
    /// Liveness-probed TERM→KILL reap ledger for `group = true`
    /// process groups (Q#CM3). Keyed by pgid; independent of the
    /// managed-process records so it survives `forget` and leader
    /// exit. Armed insert-if-absent (earliest deadline wins — a
    /// repeated TERM must not push the SIGKILL bound out). Probed
    /// every tick with `kill(-pgid, 0)`: ESRCH drops the entry;
    /// alive past the deadline SIGKILLs the group. `shutdown()`
    /// force-kills outstanding entries and probes them to ESRCH
    /// inside its bounded reap loop.
    reap_ledger: HashMap<i32, GroupReap>,
    /// TERM→KILL window used when arming the ledger. Constant
    /// [`GROUP_TERM_GRACE`] in production; overridable in tests.
    group_term_grace: Duration,
    /// Q#PD4 test seam: forces the next `kill(2)` attempt in
    /// [`Self::signal`] to fail with this errno, consumed once.
    /// Always `None` in production — there is no way to set it outside
    /// `cfg(test)`. It replaces the *kill result only*, so the leader
    /// observation still runs against the real child handle; a stubbed
    /// observation would bypass the code path under test.
    forced_kill_errno: Option<nix::errno::Errno>,
    /// Test seam for the PTY foreground-group lookup (see
    /// `force_next_pty_lookup`). Always `None` outside tests.
    forced_pty_lookup: Option<Result<i32, PtyLookupFailure>>,
    /// Directed test seam for the reap ledger's **own** `kill(2)` calls
    /// (see [`ReapKillFaults`]). Always empty in production: the only
    /// ways to add an outcome are `#[cfg(test)]`.
    ///
    /// Shared behind an `Arc` because `final_drain_runtime` is a free
    /// function taking `&RuntimeHandles` — there is no `&mut self` to
    /// reach — so the plan travels into it through [`GroupDrainCtx`].
    /// It is owned by the supervisor rather than living in a global, so
    /// fixture teardown can assert its plan was consumed even when unit
    /// tests run in parallel.
    reap_kill_faults: Arc<Mutex<ReapKillFaults>>,
}

/// One armed group in the reap ledger.
struct GroupReap {
    /// When to SIGKILL the group if it still probes alive.
    deadline: Instant,
    /// SIGKILL already sent — keep probing to ESRCH but don't
    /// re-kill every tick.
    killed: bool,
}

/// Which of the reap ledger's `kill(2)` calls a planned test outcome
/// belongs to.
///
/// **The seam is directed, and that is the whole point.** `shutdown()`
/// signals every managed process through [`ProcessSupervisor::signal`]
/// *before* it reaches its ledger force-kill, so a single undirected
/// "next kill fails" slot would be eaten by the wrong call and the test
/// would report a pass while proving nothing.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum ReapKillSite {
    /// `tick_reap_ledger`'s liveness probe, `kill(-pgid, None)`.
    LedgerProbe,
    /// `tick_reap_ledger`'s deadline escalation, `kill(-pgid, SIGKILL)`.
    LedgerEscalation,
    /// `shutdown()`'s pre-loop force-kill, `kill(-pgid, SIGKILL)`.
    ShutdownForceKill,
}

/// Planned failures for the reap ledger's own `kill(2)` calls. Empty in
/// production; populated only by `#[cfg(test)]` helpers.
///
/// Two shapes, because the sites need different lifetimes:
///
/// * The three persistent-ledger paths take a **FIFO of one-shot**
///   outcomes each, since one tick makes one call per site. A queue
///   rather than a slot because the shutdown-coupling pin needs a failed
///   force-kill *and* a failed subsequent probe pending at once.
/// * The in-drain probe takes **one outcome that repeats for one whole
///   drain**. A one-shot cannot work there: `final_drain_runtime` probes
///   again every millisecond and `quiesced` needs a false answer to
///   persist across a full [`READER_SEND_POLL_INTERVAL`], so a one-shot
///   error is long gone before reader cancellation can be reached.
#[derive(Default)]
struct ReapKillFaults {
    /// Per-site queue; each entry is consumed by one call at that site.
    queued: HashMap<ReapKillSite, VecDeque<nix::errno::Errno>>,
    /// The errno every probe inside the one claiming `GroupDrainCtx`
    /// reports, for that drain's whole lifetime.
    in_drain: Option<nix::errno::Errno>,
    /// Set once a drain has claimed `in_drain`, so a second drain in the
    /// same test probes for real instead of inheriting the fault.
    in_drain_claimed: bool,
    /// Set the first time the claiming drain actually reported the
    /// fault. Teardown asserts it: an armed-but-never-reported plan
    /// proves the fixture never reached the site it aimed at.
    #[cfg(test)]
    in_drain_used: bool,
}

impl ReapKillFaults {
    /// Consume one planned failure for `site`, if any.
    fn take(&mut self, site: ReapKillSite) -> Option<nix::errno::Errno> {
        self.queued.get_mut(&site)?.pop_front()
    }

    /// Claim the armed in-drain fault for one `GroupDrainCtx`. Returns
    /// `false` in production, where `in_drain` is always `None`.
    fn claim_in_drain(&mut self) -> bool {
        if self.in_drain.is_none() || self.in_drain_claimed {
            return false;
        }
        self.in_drain_claimed = true;
        true
    }

    /// The errno the claiming drain's probe should report.
    fn in_drain_report(&mut self) -> Option<nix::errno::Errno> {
        let errno = self.in_drain?;
        #[cfg(test)]
        {
            self.in_drain_used = true;
        }
        Some(errno)
    }
}

/// The reap ledger's own `kill(2)`, with the directed seam of
/// [`ReapKillSite`] applied first.
///
/// On Q#PD4's terms: the injection replaces the **result only**. The
/// branch it feeds, the ledger bookkeeping, and every other syscall run
/// as production code.
fn reap_kill(
    faults: &Mutex<ReapKillFaults>,
    site: ReapKillSite,
    pgid: i32,
    signal: Option<Signal>,
) -> nix::Result<()> {
    if let Some(errno) = faults
        .lock()
        .expect("reap fault plan is never held across a panic")
        .take(site)
    {
        return Err(errno);
    }
    nix::sys::signal::kill(Pid::from_raw(-pgid), signal)
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
    stdin: Option<StdinWriter>,
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
    /// Live reader-thread count for this generation, maintained by
    /// [`spawn_group_reader`] via a drop guard. Unit tests hold a
    /// clone across teardown as the deterministic proof that the
    /// joined threads ended and their owned read FDs dropped
    /// (join-return alone cannot distinguish "never started", and a
    /// process-global thread/FD count is racy under the parallel
    /// test runner). Always present — one Arc and two atomics per
    /// reader lifetime — because cfg-gating the field would spread
    /// cfg attributes through every construction site; only the
    /// probe accessor is test-gated, hence the not(test) allow.
    #[cfg_attr(not(test), allow(dead_code))]
    active_readers: Arc<AtomicUsize>,
}

/// Byte budget for stdin data queued but not yet written, per
/// generation. A child this far behind on reading its own stdin is
/// effectively not consuming it; erroring beats unbounded queue
/// growth, and callers already treat `write_stdin` errors as
/// process failure. Generous so it never triggers for a merely-busy
/// child (LSP full-document didChange on a large file is ~MB-scale).
const STDIN_QUEUE_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Queued stdin writer: a dedicated thread owns the child's stdin
/// handle and drains a channel of byte chunks. This decouples
/// callers — the editor main thread, notably the LSP manager's
/// full-document `didChange` notifications — from pipe
/// backpressure: a child that stops reading (kernel pipe buffers
/// are ~64 KiB) stalls this queue, not the editor frame loop.
///
/// Closing: dropping the sender (`close_stdin` / generation end)
/// lets the thread drain whatever is queued, then drop the handle —
/// the child sees EOF *after* the queued bytes, preserving the
/// flush-then-EOF shutdown contract MCP relies on. The thread is
/// detached rather than joined: joining at drop could block forever
/// on a wedged pipe, and generation teardown (SIGTERM/SIGKILL)
/// breaks the pipe and ends the thread shortly after anyway.
struct StdinWriter {
    tx: Sender<Vec<u8>>,
    /// Bytes accepted by [`Self::write`] but not yet written by the
    /// thread. Backpressure signal for the queue budget.
    queued_bytes: Arc<AtomicUsize>,
    /// First write error observed by the writer thread. Writes are
    /// asynchronous, so the failure surfaces on the *next* `write`
    /// call instead of the one that hit it.
    error: Arc<Mutex<Option<String>>>,
}

impl StdinWriter {
    fn spawn(mut sink: Box<dyn Write + Send>) -> Self {
        let (tx, rx) = channel::unbounded::<Vec<u8>>();
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let error = Arc::new(Mutex::new(None));
        let thread_queued = Arc::clone(&queued_bytes);
        let thread_error = Arc::clone(&error);
        std::thread::Builder::new()
            .name("pmacs stdin writer".into())
            .spawn(move || {
                while let Ok(bytes) = rx.recv() {
                    let result = sink.write_all(&bytes).and_then(|()| sink.flush());
                    thread_queued.fetch_sub(bytes.len(), Ordering::Relaxed);
                    if let Err(e) = result {
                        *thread_error
                            .lock()
                            .expect("stdin writer error mutex poisoned") = Some(e.to_string());
                        return;
                    }
                }
                // Channel closed: all queued chunks written. `sink`
                // drops here, closing the pipe — the child sees EOF.
            })
            .expect("spawn stdin writer thread");
        Self {
            tx,
            queued_bytes,
            error,
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if let Some(e) = self
            .error
            .lock()
            .expect("stdin writer error mutex poisoned")
            .as_ref()
        {
            return Err(format!("write_stdin: {e}"));
        }
        let queued = self.queued_bytes.load(Ordering::Relaxed);
        if queued.saturating_add(bytes.len()) > STDIN_QUEUE_MAX_BYTES {
            return Err(format!(
                "write_stdin: child is not draining stdin ({queued} bytes already queued)"
            ));
        }
        self.queued_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
        self.tx.send(bytes.to_vec()).map_err(|_| {
            // Thread exited after a write error; report the stored
            // cause when we have it.
            self.queued_bytes.fetch_sub(bytes.len(), Ordering::Relaxed);
            let stored = self
                .error
                .lock()
                .expect("stdin writer error mutex poisoned")
                .clone();
            stored.map_or_else(
                || "write_stdin: writer thread stopped".to_owned(),
                |e| format!("write_stdin: {e}"),
            )
        })
    }
}

impl Drop for RuntimeHandles {
    fn drop(&mut self) {
        // Wake any reader thread blocked in a bounded `send` ---
        // dropping the master closes the kernel pipe and unblocks
        // `read`, but does nothing for a reader stuck on a full
        // channel because the consumer fell behind. Cancel flag
        // unwedges that case before we join. T M6.2.
        self.cancel.store(true, Ordering::Relaxed);
        // Close the child's stdin BEFORE joining. `cancel` covers a
        // reader stuck in `send`; it does NOT cover one stuck in
        // `read`, which is only consulted between reads. What actually
        // unblocks that reader is the child exiting and closing its
        // output pipe --- and a stdio child exits on stdin EOF.
        //
        // The premise in the comment above ("dropping the master
        // closes the kernel pipe") holds for a PTY master but NOT for
        // pipe mode, where `read` unblocks only once *every* write end
        // closes. An escaped descendant holding one (a shim-launched
        // language server that orphans its real process) keeps the
        // reader blocked indefinitely.
        //
        // The sink lives in the `stdin` FIELD, and a type's `Drop::drop`
        // body runs before *all* of its fields regardless of their
        // declaration order --- so reordering the struct cannot fix
        // this. Joining first deadlocks against the very EOF that would
        // have ended the join. `take()` is idempotent, matching
        // `close_stdin`.
        let _ = self.stdin.take();
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

/// Which branch of [`signal_target`] chose the target (Q#PD1).
///
/// Recorded on failure because the branches differ in what a failing
/// `kill` can possibly mean. Two of the four aim at the spawned child
/// itself — [`Self::LeaderPid`] for a pipe child with no group, and
/// [`Self::PtyForegroundFallback`] for a PTY whose terminal named no
/// group. The other two aim at a *group*:
/// [`Self::SpawnGroup`] at one computed from the spawn-time `pgid ==
/// pid` assumption, and [`Self::ForegroundGroup`] at one read from the
/// terminal, which can belong to something the supervisor never spawned.
///
/// The pid-versus-group split is the classification that matters here,
/// and it does **not** line up with the PTY-versus-pipe split — which is
/// exactly why the fallback needed its own variant instead of reusing
/// `LeaderPid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetSource {
    /// The tty's current foreground process group, read at signal
    /// time. Diverges from the leader exactly when job control has
    /// moved the terminal.
    ForegroundGroup,
    /// A `group = true` pipe child leading its own process group.
    SpawnGroup,
    /// The child's own pid, for a pipe child that leads no group.
    LeaderPid,
    /// A **PTY** child whose foreground-group lookup did not yield a
    /// group, so the target fell back to the leader pid.
    ///
    /// Distinct from [`Self::LeaderPid`] on purpose. Before this
    /// variant existed both rendered "leader-pid", so a PTY whose
    /// terminal query failed was indistinguishable in the report from
    /// an ordinary pipe child that never had a terminal — two very
    /// different situations reading as one.
    PtyForegroundFallback(PtyLookupFailure),
}

/// Why a PTY's foreground-group lookup produced no group.
///
/// Each arm is a different fact and none is forged into another: a
/// missing fd is not an errno, and a failure to *duplicate* the master
/// is not a failure to *query* the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtyLookupFailure {
    /// The master reported no file descriptor to query.
    NoMasterFd,
    /// Duplicating the master fd failed, so the terminal was never
    /// queried at all.
    Duplicate(nix::errno::Errno),
    /// `tcgetpgrp` itself failed on a successfully duplicated fd.
    Query(nix::errno::Errno),
    /// The terminal answered, but with a non-positive group id, which
    /// names no group.
    NonPositive(i32),
}

impl PtyLookupFailure {
    fn render(self) -> String {
        match self {
            Self::NoMasterFd => "no-master-fd".to_owned(),
            Self::Duplicate(e) => format!("duplicate-master-fd: {e}"),
            Self::Query(e) => format!("tcgetpgrp: {e}"),
            Self::NonPositive(v) => format!("tcgetpgrp-non-positive: {v}"),
        }
    }
}

impl TargetSource {
    fn render(self) -> String {
        match self {
            Self::ForegroundGroup => "tcgetpgrp".to_owned(),
            Self::SpawnGroup => "group".to_owned(),
            Self::LeaderPid => "leader-pid".to_owned(),
            Self::PtyForegroundFallback(why) => {
                format!("pty-leader-fallback({})", why.render())
            }
        }
    }

    /// Whether the target is a process group rather than one process.
    fn is_group(self) -> bool {
        matches!(self, Self::ForegroundGroup | Self::SpawnGroup)
    }
}

/// A lifetime-tied view of a `MasterPty`'s file descriptor.
///
/// `MasterPty` exposes only `Option<RawFd>`, and every std route from a
/// raw fd to something implementing `AsFd` — `BorrowedFd::borrow_raw`,
/// `OwnedFd::from_raw_fd`, `File::from_raw_fd` — is `unsafe`, which this
/// crate forbids. `filedescriptor::OwnedHandle::dup` accepts any
/// `AsRawFd` through a safe blanket impl and hands back an owned handle
/// that *is* `AsFd`, so implementing this one safe trait is the whole
/// bridge.
///
/// The borrow is what makes it sound: the view cannot outlive the master
/// it read the descriptor from, so the fd cannot have been closed
/// underneath it.
struct MasterFdView<'a> {
    fd: std::os::fd::RawFd,
    _master: &'a (dyn portable_pty::MasterPty + Send),
}

impl std::os::fd::AsRawFd for MasterFdView<'_> {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.fd
    }
}

/// Recover the OS errno from a `filedescriptor` error.
///
/// Its error type is an enum of thiserror variants, each carrying a
/// `std::io::Error` as a `#[source]` rather than exposing
/// `raw_os_error` itself. Walking the source chain and downcasting keeps
/// every variant working, including ones added later, instead of
/// matching the one arm that exists today.
///
/// Returns `UnknownErrno` when the chain carries no OS error, rather
/// than inventing a plausible one — a forged errno in a diagnostic is
/// worse than an honest absence.
fn os_errno_of(err: &filedescriptor::Error) -> nix::errno::Errno {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(io) = e.downcast_ref::<std::io::Error>()
            && let Some(code) = io.raw_os_error()
        {
            return nix::errno::Errno::from_raw(code);
        }
        current = e.source();
    }
    nix::errno::Errno::UnknownErrno
}

/// Read the terminal's foreground process group, keeping the errno.
///
/// `portable_pty::MasterPty::process_group_leader` collapses every
/// failure into `None`, so pmacs could not tell "this tty has no
/// foreground group" from "the query failed and here is why". This does
/// the query itself and returns the reason on every non-success path.
fn pty_foreground_group(
    master: &(dyn portable_pty::MasterPty + Send),
) -> Result<i32, PtyLookupFailure> {
    let Some(fd) = master.as_raw_fd() else {
        return Err(PtyLookupFailure::NoMasterFd);
    };
    let view = MasterFdView {
        fd,
        _master: master,
    };
    let owned = filedescriptor::OwnedHandle::dup(&view)
        .map_err(|e| PtyLookupFailure::Duplicate(os_errno_of(&e)))?;
    match nix::unistd::tcgetpgrp(&owned) {
        Ok(pgrp) => {
            let raw = pgrp.as_raw();
            if raw > 0 {
                Ok(raw)
            } else {
                Err(PtyLookupFailure::NonPositive(raw))
            }
        }
        Err(e) => Err(PtyLookupFailure::Query(e)),
    }
}

/// The entity a signal was actually aimed at, plus the branch that
/// chose it. Carried so a failure can report the target as a fact
/// separate from the leader's state (Q#PD1).
#[derive(Debug, Clone, Copy)]
struct SignalTarget {
    pid: Pid,
    source: TargetSource,
}

fn signal_target(
    proc: &ManagedProcess,
    pid: u32,
    forced_lookup: Option<Result<i32, PtyLookupFailure>>,
) -> Result<SignalTarget, String> {
    if let Some(runtime) = proc.runtime.as_ref()
        && let ChildHandle::Pty {
            _master: master, ..
        } = &runtime.child
    {
        // A PTY child is always group-directed when the terminal names a
        // foreground group. When it does not, the target falls back to
        // the leader — and *why* it fell back is carried into the source
        // so the report can say it. Previously every one of these paths
        // produced a bare `LeaderPid`, identical to a pipe child that
        // never had a terminal at all.
        let lookup = match forced_lookup {
            Some(outcome) => outcome,
            None => pty_foreground_group(master.as_ref()),
        };
        return match lookup {
            Ok(pgrp) => Ok(SignalTarget {
                pid: Pid::from_raw(-pgrp),
                source: TargetSource::ForegroundGroup,
            }),
            Err(why) => Ok(SignalTarget {
                pid: Pid::from_raw(i32::try_from(pid).map_err(|e| e.to_string())?),
                source: TargetSource::PtyForegroundFallback(why),
            }),
        };
    }
    // `group = true` pipe children lead a fresh process group
    // (`process_group(0)` at spawn ⇒ pgid == pid), so fatal signals
    // reach the whole `sh -c` tree — mirroring the PTY branch above
    // (Q#CM3).
    if proc.spec.group {
        let pgid = i32::try_from(pid).map_err(|e| e.to_string())?;
        return Ok(SignalTarget {
            pid: Pid::from_raw(-pgid),
            source: TargetSource::SpawnGroup,
        });
    }
    Ok(SignalTarget {
        pid: Pid::from_raw(i32::try_from(pid).map_err(|e| e.to_string())?),
        source: TargetSource::LeaderPid,
    })
}

/// The spawned leader's state at the moment a `kill` failed (Q#PD1).
///
/// Deliberately reported *beside* the target rather than folded into a
/// verdict: for a PTY the two are different entities whenever job
/// control has moved the terminal, and three successive designs for
/// this code were unsound precisely because they collapsed them.
enum LeaderObservation {
    Exited(TermStatus),
    Live,
    Unobservable(String),
    NoRuntime,
}

impl LeaderObservation {
    fn render(&self) -> String {
        match self {
            Self::Exited(TermStatus::Exited(code)) => format!("exited(code {code})"),
            Self::Exited(TermStatus::Signaled(sig)) => format!("exited(signal {sig})"),
            Self::Live => "live".to_owned(),
            Self::Unobservable(e) => format!("unobservable({e})"),
            Self::NoRuntime => "no-runtime".to_owned(),
        }
    }
}

/// Observe the spawned leader. Note this *reaps* an exited child and
/// caches its status; that is why Q#PD3 claims "no disposition change"
/// rather than "strictly additive", and why an event-count test pins
/// that `poll_one` still emits exactly one exit event afterwards.
fn observe_leader(proc: &mut ManagedProcess) -> LeaderObservation {
    let Some(runtime) = proc.runtime.as_mut() else {
        return LeaderObservation::NoRuntime;
    };
    match runtime.child.try_wait() {
        Ok(Some(status)) => LeaderObservation::Exited(status),
        Ok(None) => LeaderObservation::Live,
        Err(e) => LeaderObservation::Unobservable(e),
    }
}

/// The leader's process group as the kernel reports it, for a target
/// that was *computed* from the spawn-time assumption `pgid == pid`.
///
/// `expected_group` is that assumption restated — it is `-leader_pid`,
/// and on the `SpawnGroup` path the target is `-leader_pid` too, so the
/// two agreeing is arithmetic rather than evidence. This is the only
/// field in the report that can disagree with the input, which is what
/// makes it worth printing.
///
/// **It does not establish identity** (framing §1.5). It is read before
/// the `kill`, in the same read-then-act window, and a number cannot
/// distinguish the original group from a recycled one. No portable
/// mechanism can: `pidfd` closes pid reuse for a process, not a group,
/// and macOS has none at all. This records an observation; it settles
/// nothing.
fn measured_group_of(leader_pid: u32) -> String {
    let Ok(raw) = i32::try_from(leader_pid) else {
        return ", measured_group=unobservable(pid out of range)".to_owned();
    };
    match nix::unistd::getpgid(Some(Pid::from_raw(raw))) {
        Ok(pgid) => format!(", measured_group=-{}", pgid.as_raw()),
        Err(e) => format!(", measured_group=unobservable({e})"),
    }
}

/// Render a failing `kill` as the facts of Q#PD1. The disposition is
/// unchanged (Q#PD2) — this only replaces a message that said nothing
/// but the errno.
///
/// The signal is named because it could not be recovered otherwise: a
/// failed `SIGUSR1` and a failed `SIGTERM` were previously identical
/// text. Note this is a *reporting* gap only — every failed `kill`
/// returns before the fatal-signal branch, so failed signals are
/// disposition-identical whatever they are. The disposition difference
/// is real only for calls that succeed.
fn signal_failure_report(
    target: SignalTarget,
    leader_pid: u32,
    signal: Signal,
    errno: nix::errno::Errno,
    leader: &LeaderObservation,
    measured: Option<&str>,
) -> String {
    let expected = if target.source.is_group() {
        match i32::try_from(leader_pid) {
            Ok(p) => format!(", expected_group=-{p}"),
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };
    // Supplied by the caller, which samples it BEFORE the `kill`. Doing
    // it here would describe the group as it stands *after* the failure
    // and after `observe_leader`, which is post-hoc state presented as
    // evidence about the attempted target.
    let measured = measured.unwrap_or("");
    format!(
        "kill: {errno} (signal={signal:?}, target={} via {}, leader_pid={leader_pid}{expected}{measured}, leader={})",
        target.pid.as_raw(),
        target.source.render(),
        leader.render(),
    )
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
/// `portable-pty`) to symbolic SIGFOO names. Darwin appends the signal
/// number (for example, `"Terminated: 15"`), while glibc returns only
/// the description. Unknown descriptions pass through unchanged —
/// better to surface an unfamiliar string than to fabricate a wrong
/// name. Covers every signal in
/// [`super::lua_bindings::parse_signal`]'s accept-list plus the common
/// fault signals that surface during process crashes.
fn canonicalize_pty_signal_name(desc: &str) -> String {
    let base = desc
        .rsplit_once(": ")
        .filter(|(_, number)| number.parse::<u32>().is_ok())
        .map_or(desc, |(description, _)| description);
    match base {
        "Interrupt" => "SIGINT",
        "Terminated" => "SIGTERM",
        "Killed" => "SIGKILL",
        "Hangup" => "SIGHUP",
        "Quit" => "SIGQUIT",
        "User defined signal 1" => "SIGUSR1",
        "User defined signal 2" => "SIGUSR2",
        "Aborted" => "SIGABRT",
        "Segmentation fault" => "SIGSEGV",
        "Floating point exception" => "SIGFPE",
        "Illegal instruction" => "SIGILL",
        "Broken pipe" => "SIGPIPE",
        "Alarm clock" => "SIGALRM",
        "Bus error" => "SIGBUS",
        _ => desc,
    }
    .to_owned()
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
            reap_ledger: HashMap::new(),
            group_term_grace: GROUP_TERM_GRACE,
            forced_kill_errno: None,
            forced_pty_lookup: None,
            reap_kill_faults: Arc::new(Mutex::new(ReapKillFaults::default())),
        }
    }

    /// Plan a one-shot failure for the reap ledger's `kill(2)` at
    /// `site`. Queued per site (see [`ReapKillFaults`]), so a plan for
    /// one site can never be consumed by another's call.
    #[cfg(test)]
    fn plan_reap_kill_failure(&mut self, site: ReapKillSite, errno: nix::errno::Errno) {
        self.reap_kill_faults
            .lock()
            .expect("reap fault plan")
            .queued
            .entry(site)
            .or_default()
            .push_back(errno);
    }

    /// Plan the in-drain probe failure: the *next* group drain to start
    /// claims it and reports `errno` from every probe for that drain's
    /// whole lifetime. See [`ReapKillFaults`] for why this one is not a
    /// one-shot.
    #[cfg(test)]
    fn plan_in_drain_probe_failure(&mut self, errno: nix::errno::Errno) {
        self.reap_kill_faults
            .lock()
            .expect("reap fault plan")
            .in_drain = Some(errno);
    }

    /// Fixture teardown: every planned outcome must have been consumed
    /// by the production site it was aimed at. An unconsumed plan means
    /// the fixture never reached that site, which would otherwise leave
    /// a test asserting the *absence* of an effect it never provoked.
    #[cfg(test)]
    fn assert_reap_faults_consumed(&self) {
        let faults = self.reap_kill_faults.lock().expect("reap fault plan");
        for (site, queue) in &faults.queued {
            assert!(
                queue.is_empty(),
                "{} planned {site:?} failure(s) were never consumed — the fixture did not reach that production site",
                queue.len()
            );
        }
        if faults.in_drain.is_some() {
            assert!(
                faults.in_drain_used,
                "the in-drain probe fault was armed but never reported — the fixture did not reach final_drain_runtime's probe"
            );
        }
    }

    /// Q#PD4 test seam: make the next `kill(2)` attempt in
    /// [`Self::signal`] report `errno` instead of calling the kernel.
    /// Consumed by that one attempt. Everything downstream — target
    /// selection, the leader observation against the real child, and
    /// the error construction — runs unmodified.
    #[cfg(test)]
    fn force_next_kill_errno(&mut self, errno: nix::errno::Errno) {
        self.forced_kill_errno = Some(errno);
    }

    /// Test seam for the PTY foreground-group lookup, on the same terms
    /// as [`Self::force_next_kill_errno`] and for the same reason.
    ///
    /// Injects either outcome. The failure arms — no master fd, a failed
    /// duplicate, a failed `tcgetpgrp` — cannot be produced on demand
    /// from a healthy PTY: they need an exhausted descriptor table or a
    /// master that has stopped being a terminal.
    ///
    /// The **success** arm exists because a genuinely divergent
    /// foreground group is not portable. `bash -m` produces one on
    /// Linux and **does not on macOS**, where the terminal stays with
    /// the leader for the whole wait (observed in CI on both macOS
    /// legs). Injecting the group keeps the divergent case pinned
    /// everywhere; `job_control_really_diverges_the_foreground_group`
    /// corroborates it against a real shell where the platform allows.
    ///
    /// Either way the injection covers only the *lookup result*: the
    /// branch, the target choice, the leader observation against the
    /// real child, and the report construction all run as production
    /// code. Consumed by one call.
    #[cfg(test)]
    fn force_next_pty_lookup(&mut self, outcome: Result<i32, PtyLookupFailure>) {
        self.forced_pty_lookup = Some(outcome);
    }

    /// Override the SIGTERM-to-SIGKILL grace window. Test helper.
    pub fn set_grace_period(&mut self, d: Duration) {
        self.grace_period = d;
    }

    /// Override the group TERM→KILL escalation window. Test helper.
    pub fn set_group_term_grace(&mut self, d: Duration) {
        self.group_term_grace = d;
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
        self.spawn_inner(spec, true)
    }

    /// Spawn an unpublished terminal-owned process.
    ///
    /// Unlike the public Lua/process path, synchronous failure does not emit an
    /// event for an ID no caller can own. `TerminalManager` rolls back its
    /// temporary identity buffer and returns the error directly.
    pub(crate) fn spawn_terminal(&mut self, spec: ProcessSpec) -> Result<ProcessId, String> {
        self.spawn_inner(spec, false)
    }

    fn spawn_inner(
        &mut self,
        spec: ProcessSpec,
        publish_synchronous_failure: bool,
    ) -> Result<ProcessId, String> {
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
        self.start_generation(id, &mut managed, publish_synchronous_failure)?;
        self.processes.insert(id, managed);
        Ok(id)
    }

    /// Start a fresh generation for `managed`. Mutates `managed` in place; on
    /// failure its state is `Terminated(Crashed{...})`, and the event is emitted
    /// only when `publish_failure` is true.
    fn start_generation(
        &self,
        id: ProcessId,
        managed: &mut ManagedProcess,
        publish_failure: bool,
    ) -> Result<(), String> {
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
                if publish_failure {
                    let _ = self.events_tx.send(ProcessEvent {
                        id,
                        kind: ProcessEventKind::Crashed { error: e.clone() },
                        at: now,
                    });
                }
                Err(e)
            }
        }
    }

    /// Send `signal` to `id`. Errors if the id is unknown or the
    /// process is not currently running. Pipe-mode children are
    /// signaled by OS pid; PTY-mode children are signaled via the
    /// foreground process group when the kernel reports one, matching
    /// terminal C-c behavior for shells and REPLs. Nothing about the
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
        let forced_lookup = self.forced_pty_lookup.take();
        let target = signal_target(proc, pid, forced_lookup)?;
        // Sample the real group BEFORE signalling. Only the spawn-group
        // path computes its target from the `pgid == pid` assumption, so
        // it is the only one a measurement can contradict; a PTY target
        // came from the terminal and a leader-directed target is not a
        // group at all.
        let measured =
            matches!(target.source, TargetSource::SpawnGroup).then(|| measured_group_of(pid));
        // Q#PD4: the seam injects the KILL attempt's result only —
        // never the observation below — so target selection, the real
        // `ChildHandle::try_wait` against the real child, and the error
        // construction all run for real. Consumed once.
        let kill_result = match self.forced_kill_errno.take() {
            Some(errno) => Err(errno),
            None => nix::sys::signal::kill(target.pid, Some(signal)),
        };
        if let Err(errno) = kill_result {
            // Q#PD1/Q#PD2: the failure describes itself; the
            // disposition is unchanged — this still returns `Err`,
            // with no state transition and no ledger arming.
            let leader = observe_leader(proc);
            return Err(signal_failure_report(
                target,
                pid,
                signal,
                errno,
                &leader,
                measured.as_deref(),
            ));
        }
        if matches!(signal, Signal::SIGTERM | Signal::SIGKILL | Signal::SIGHUP) {
            proc.state = ProcessState::Exiting {
                pid,
                signaled_at: Instant::now(),
            };
            // Arm the group reap ledger on the first fatal signal
            // (Q#CM3). Insert-if-absent: a repeated `terminate` must
            // not push the SIGKILL bound out.
            if proc.spec.group
                && let Ok(pgid) = i32::try_from(pid)
            {
                let deadline = Instant::now() + self.group_term_grace;
                self.reap_ledger.entry(pgid).or_insert(GroupReap {
                    deadline,
                    killed: false,
                });
            }
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

    /// Close `id`'s stdin pipe by dropping the writer. The writer
    /// thread drains any queued bytes first, then drops the handle,
    /// so the child observes EOF *after* everything already written
    /// — the canonical stdio-graceful-shutdown signal for protocols
    /// (notably MCP) that have no protocol-level shutdown message.
    /// Idempotent: a second call after the writer is gone is a
    /// no-op. Errors only if the process id is unknown.
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
    /// the process is not running, stdin is closed (the child
    /// closed stdin on its end, or stdin was never piped in the
    /// first place), or the per-generation queue budget is
    /// exhausted. The write itself is queued to a dedicated writer
    /// thread, so this never blocks on pipe backpressure — a write
    /// *failure* (broken pipe) therefore surfaces on a subsequent
    /// call rather than the one that queued the bytes; callers that
    /// need liveness should watch the supervisor's exit events.
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
            .as_ref()
            .ok_or_else(|| format!("process {id} stdin is not piped"))?;
        stdin.write(bytes)
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
        // Probe the group reap ledger last so groups TERMed by this
        // tick's poll_one get their liveness checked from the very
        // next tick onward (Q#CM3).
        self.tick_reap_ledger();
    }

    /// Probe every armed group: ESRCH → group gone, drop the entry;
    /// alive past its deadline → SIGKILL the group (once), then keep
    /// probing to ESRCH. Independent of managed-process records by
    /// design — this is what catches a TERM-ignoring descendant that
    /// survived its leader's clean exit with its output redirected
    /// (round-3 finding 1: neither leader state nor reader state can
    /// see that survivor; only group liveness can).
    fn tick_reap_ledger(&mut self) {
        let now = Instant::now();
        // Cloned out before `retain` takes `&mut self.reap_ledger`.
        let faults = Arc::clone(&self.reap_kill_faults);
        self.reap_ledger.retain(|pgid, entry| {
            // ESRCH: no such group — done. Any other probe error is
            // also treated as "nothing left we can reach", so the
            // ledger cannot grow without bound.
            //
            // **That is a bounded-growth policy, not a claim that the
            // group is gone.** This comment previously justified it with
            // "EPERM cannot happen for our own children". That reasoning
            // does not hold: the probe targets a *group*, and owning the
            // spawned child says nothing about a group unless the child
            // is still a member of it — which nothing here measures. A
            // group-directed EPERM against a live leader has since been
            // observed in CI (macOS, PR #191, run 30553376486), via an
            // explicit signal rather than this probe.
            //
            // So this arm can silently cancel an escalation, and the
            // `SIGKILL` below can fail while the entry is marked killed.
            // Both are known and deliberately unchanged here: the
            // diagnostic lane that found them does not alter
            // disposition. Fixing it is its own lane.
            if reap_kill(&faults, ReapKillSite::LedgerProbe, *pgid, None).is_err() {
                return false;
            }
            if now >= entry.deadline && !entry.killed {
                let _ = reap_kill(
                    &faults,
                    ReapKillSite::LedgerEscalation,
                    *pgid,
                    Some(Signal::SIGKILL),
                );
                entry.killed = true;
            }
            true
        });
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
        let status = runtime.child.try_wait();
        if matches!(status, Ok(None)) {
            return;
        }
        // Terminal from here on. Group leader-exit reap (Q#CM3):
        // TERM the remaining group and arm the reap ledger BEFORE
        // the final drain — a leader that exits leaving `sleep 60 &`
        // holding the merged pipe would otherwise burn the full
        // drain timeout and then block the reader join. Arming is
        // insert-if-absent, so a deadline already armed by an
        // explicit kill is not extended.
        let group_ctx = if proc.spec.group {
            i32::try_from(runtime.pid).ok().map(|pgid| {
                let _ = nix::sys::signal::kill(Pid::from_raw(-pgid), Some(Signal::SIGTERM));
                let deadline = Instant::now() + self.group_term_grace;
                let entry = self.reap_ledger.entry(pgid).or_insert(GroupReap {
                    deadline,
                    killed: false,
                });
                GroupDrainCtx {
                    pgid,
                    deadline: entry.deadline,
                    faults: Arc::clone(&self.reap_kill_faults),
                    in_drain_fault_claimed: self
                        .reap_kill_faults
                        .lock()
                        .expect("reap fault plan is never held across a panic")
                        .claim_in_drain(),
                }
            })
        } else {
            None
        };
        let now = Instant::now();
        let final_output = final_drain_runtime(runtime, group_ctx.as_ref());
        let (termination, event) = match status {
            Ok(Some(TermStatus::Exited(code))) => (
                Termination::Exited {
                    code,
                    started,
                    ended: now,
                },
                ProcessEventKind::Exited { code },
            ),
            Ok(Some(TermStatus::Signaled(signal))) => (
                Termination::Signaled {
                    signal: signal.clone(),
                    started,
                    ended: now,
                },
                ProcessEventKind::Signaled { signal },
            ),
            Err(e) => (
                Termination::Crashed {
                    error: e.clone(),
                    ended: now,
                },
                ProcessEventKind::Crashed { error: e },
            ),
            // Guarded above; kept explicit so the match stays total.
            Ok(None) => return,
        };
        proc.state = ProcessState::Terminated(termination);
        proc.runtime = None;
        append_process_events(&mut self.pending, id, final_output, now);
        self.pending.entry(id).or_default().push(ProcessEvent {
            id,
            kind: event,
            at: now,
        });
    }

    /// Apply restart policy after `poll_one` may have transitioned
    /// the process to `Terminated`.
    fn maybe_restart(&mut self, id: ProcessId) {
        // Inert during and after shutdown: shutdown's own tick()
        // calls must not respawn a `restart = always` process
        // mid-teardown (round-4 finding 1).
        if self.shut_down {
            return;
        }
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
            let _ = self.start_generation(id, &mut managed, true);
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
        // Editor exit owes group survivors no grace: force-kill every
        // outstanding reap-ledger entry now, then probe it to ESRCH in
        // the bounded loop below. Without this, a pre-deadline ledger
        // (leader exited promptly, TERM-ignoring group member alive)
        // would be silently discarded at Drop and leak the member
        // (Q#CM3, round-4 finding 1).
        let faults = Arc::clone(&self.reap_kill_faults);
        for (pgid, entry) in &mut self.reap_ledger {
            let _ = reap_kill(
                &faults,
                ReapKillSite::ShutdownForceKill,
                *pgid,
                Some(Signal::SIGKILL),
            );
            entry.killed = true;
        }
        // Final reap loop. SIGKILL is delivered immediately by the
        // kernel; the child becomes a zombie until we reap. Bound
        // the wait so a pathological case can't hang the editor
        // exit forever. The tick also probes the reap ledger, so the
        // loop holds until force-killed groups observe ESRCH.
        let final_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < final_deadline
            && (self.any_running() || !self.reap_ledger.is_empty())
        {
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

    /// Clone of a live generation's active-reader counter (see
    /// [`RuntimeHandles::active_readers`]). Unit tests grab it while
    /// the generation runs and assert zero after teardown.
    #[cfg(test)]
    fn active_reader_probe(&self, id: ProcessId) -> Option<Arc<AtomicUsize>> {
        Some(Arc::clone(
            &self.processes.get(&id)?.runtime.as_ref()?.active_readers,
        ))
    }

    /// Number of armed reap-ledger entries. Test observability for
    /// the shutdown/drop-twin pins.
    #[cfg(test)]
    fn reap_ledger_len(&self) -> usize {
        self.reap_ledger.len()
    }

    /// Whether the ledger has recorded a SIGKILL as sent for `pgid`.
    /// `None` if no entry is armed. The whole point of the
    /// failed-escalation pin is that this reads `true` after a kill
    /// that never happened.
    #[cfg(test)]
    fn reap_ledger_killed(&self, pgid: i32) -> Option<bool> {
        self.reap_ledger.get(&pgid).map(|e| e.killed)
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
    if matches!(spec.mode, ProcessMode::Pty { .. }) {
        if matches!(spec.stdin, StdinMode::Null) {
            return Err(
                "process spawn: stdin=\"null\" requires pipe mode; a PTY has no separable stdin"
                    .to_owned(),
            );
        }
        if spec.group {
            return Err("process spawn: group=true requires pipe mode; PTY children already lead their own session and are signaled group-wide".to_owned());
        }
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
        .stdin(match spec.stdin {
            StdinMode::Piped => Stdio::piped(),
            // Immediate EOF, no writer thread, zero close-after-spawn
            // race (Q#CM3).
            StdinMode::Null => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if spec.group {
        // Fresh process group with the child as leader (pgid == pid).
        // Safe std API — no `unsafe`, no trampoline (stable 1.64).
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
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
        .map(|s| StdinWriter::spawn(Box::new(s) as Box<dyn Write + Send>));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (byte_tx, byte_rx) = channel::bounded::<ByteChunk>(BYTE_CHUNK_CHANNEL_CAP);
    let cancel = Arc::new(AtomicBool::new(false));
    let active_readers = Arc::new(AtomicUsize::new(0));
    let mut readers = Vec::new();
    if let Some(out) = stdout {
        readers.push(if spec.group {
            spawn_group_reader(
                byte_tx.clone(),
                Arc::clone(&cancel),
                out,
                ReaderKind::Stdout,
                Arc::clone(&active_readers),
            )
        } else {
            spawn_reader(
                byte_tx.clone(),
                Arc::clone(&cancel),
                out,
                ReaderKind::Stdout,
            )
        });
    }
    if let Some(err) = stderr {
        readers.push(if spec.group {
            spawn_group_reader(
                byte_tx,
                Arc::clone(&cancel),
                err,
                ReaderKind::Stderr,
                Arc::clone(&active_readers),
            )
        } else {
            spawn_reader(byte_tx, Arc::clone(&cancel), err, ReaderKind::Stderr)
        });
    }
    Ok(RuntimeHandles {
        child: ChildHandle::Pipes(child),
        stdin,
        pid,
        readers,
        output_rx: RuntimeOutputRx::Bytes(byte_rx),
        cancel,
        active_readers,
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
        readers.push(spawn_ansi_parser(
            byte_rx,
            ansi_tx,
            Arc::clone(&cancel),
            spec.ansi_profile,
        ));
        RuntimeOutputRx::Ansi(ansi_rx)
    } else {
        RuntimeOutputRx::Bytes(byte_rx)
    };
    Ok(RuntimeHandles {
        child: ChildHandle::Pty {
            child: Arc::new(Mutex::new(into_send_sync_child(child))),
            _master: pair.master,
        },
        stdin: Some(StdinWriter::spawn(writer)),
        pid,
        readers,
        output_rx,
        cancel,
        // PTY readers are the blocking kind; the counter is only
        // maintained by group readers and stays zero here.
        active_readers: Arc::new(AtomicUsize::new(0)),
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

/// RAII live-count for group reader threads: increments on
/// construction, decrements on every exit path (panic included), so
/// [`RuntimeHandles::active_readers`] reaching zero is a
/// deterministic "thread ended, its read FD dropped" signal.
struct ActiveReaderGuard(Arc<AtomicUsize>);

impl ActiveReaderGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for ActiveReaderGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Poll-based cancellable reader for `group = true` generations
/// (Q#CM3). Unlike [`spawn_reader`], the fd is set nonblocking and
/// every wait — for readability or for channel space — re-checks
/// `cancel` each [`READER_SEND_POLL_INTERVAL`], with an extra check
/// between poll and read/send, so `RuntimeHandles::Drop`'s retained
/// join completes within one interval regardless of who still holds
/// the pipe's write end (a setsid'd descendant, notably). Non-group
/// consumers (REPL, LSP) keep the blocking [`spawn_reader`] they
/// were tuned on — the M6.6 ingest gate; unifying is a named
/// deferral in the compile-mode framing.
fn spawn_group_reader<R>(
    byte_tx: Sender<ByteChunk>,
    cancel: Arc<AtomicBool>,
    read: R,
    kind: ReaderKind,
    active: Arc<AtomicUsize>,
) -> JoinHandle<()>
where
    R: Read + std::os::fd::AsFd + Send + 'static,
{
    std::thread::spawn(move || {
        let _guard = ActiveReaderGuard::new(active);
        let mut read = read;
        // nix 0.29's fcntl still takes a RawFd (poll takes BorrowedFd).
        let raw_fd = std::os::fd::AsRawFd::as_raw_fd(&read.as_fd());
        if nix::fcntl::fcntl(
            raw_fd,
            nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
        )
        .is_err()
        {
            // Cannot go nonblocking (does not happen for pipe fds in
            // practice): exit rather than risk an uncancellable
            // blocking read.
            return;
        }
        let poll_timeout = nix::poll::PollTimeout::try_from(READER_SEND_POLL_INTERVAL)
            .unwrap_or(nix::poll::PollTimeout::MAX);
        let mut buf = [0u8; BYTE_CHUNK_SIZE];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let ready = {
                let mut fds = [nix::poll::PollFd::new(
                    read.as_fd(),
                    nix::poll::PollFlags::POLLIN,
                )];
                nix::poll::poll(&mut fds, poll_timeout)
            };
            match ready {
                // Timeout or interrupt: loop around and re-check the
                // cancel flag.
                Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
                Ok(_) => {}
                Err(_) => return,
            }
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
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
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

/// Context for a group-aware final drain (Q#CM3). Carries the reap
/// ledger's deadline for this group: the drain enforces it from
/// inside its loop because no other tick runs while the drain
/// blocks the frame.
struct GroupDrainCtx {
    pgid: i32,
    deadline: Instant,
    /// The supervisor's fault plan (empty in production), carried here
    /// because this drain runs in a free function with no `&mut self`.
    faults: Arc<Mutex<ReapKillFaults>>,
    /// True when this drain claimed the armed in-drain probe fault at
    /// construction. Exactly one drain can claim it.
    in_drain_fault_claimed: bool,
}

/// The in-drain liveness probe, with the seam of §1.2a applied first.
///
/// The injected errno is *returned*, and the caller's `.is_ok()` then
/// discards it — which is the collapse under test, left exactly as it
/// is. This function changes what the probe reports, never what the
/// drain does with the report.
fn in_drain_probe(ctx: &GroupDrainCtx) -> nix::Result<()> {
    if ctx.in_drain_fault_claimed
        && let Some(errno) = ctx
            .faults
            .lock()
            .expect("reap fault plan is never held across a panic")
            .in_drain_report()
    {
        return Err(errno);
    }
    nix::sys::signal::kill(Pid::from_raw(-ctx.pgid), None)
}

fn final_drain_runtime(
    rt: &RuntimeHandles,
    group: Option<&GroupDrainCtx>,
) -> Vec<ProcessEventKind> {
    let deadline = Instant::now() + EXIT_OUTPUT_DRAIN_TIMEOUT;
    let mut out = Vec::new();
    // Group drains get tighter bounds than the plain byte-flush
    // timeout (Q#CM3, round-4 finding 2 / round-5 revision):
    //  - the ledger deadline is enforced in-loop — SIGKILL the group
    //    at the grace bound;
    //  - once the group probes ESRCH, readers get one quiescent
    //    READER_SEND_POLL_INTERVAL to flush already-read and
    //    kernel-buffered bytes; new data resets the window;
    //  - independently, no group drain may pass the absolute cancel
    //    deadline of ledger deadline + one poll interval — reaching
    //    it cancels the readers even when an escaped (setsid'd)
    //    writer still holds the pipe past its group's death. Honest
    //    trailing output gets a bounded flush; escaped output may be
    //    truncated. The retained join in RuntimeHandles::Drop then
    //    completes within one further poll interval because group
    //    readers are poll-based and observe the cancel flag.
    let mut group_killed = false;
    let mut last_data = Instant::now();
    loop {
        let drained = drain_runtime_output(rt);
        let drained_any = !drained.is_empty();
        out.extend(drained);
        if drained_any {
            last_data = Instant::now();
        }
        if rt.readers.iter().all(std::thread::JoinHandle::is_finished) && !drained_any {
            return out;
        }
        if let Some(ctx) = group {
            let now = Instant::now();
            let group_alive = in_drain_probe(ctx).is_ok();
            if group_alive && now >= ctx.deadline && !group_killed {
                let _ = nix::sys::signal::kill(Pid::from_raw(-ctx.pgid), Some(Signal::SIGKILL));
                group_killed = true;
            }
            let quiesced =
                !group_alive && now.duration_since(last_data) >= READER_SEND_POLL_INTERVAL;
            if quiesced || now >= ctx.deadline + READER_SEND_POLL_INTERVAL {
                rt.cancel.store(true, Ordering::Relaxed);
                out.extend(drain_runtime_output(rt));
                return out;
            }
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
    profile: AnsiParserProfile,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut parser = AnsiParser::with_profile(profile);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let (kind, bytes) = match byte_rx.recv_timeout(READER_SEND_POLL_INTERVAL) {
                Ok(chunk) => chunk,
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                    let events = parser.finish();
                    if !events.is_empty() {
                        let _ = send_ansi_batch(&ansi_tx, &cancel, events);
                    }
                    return;
                }
            };
            if !matches!(kind, ReaderKind::Stdout) {
                continue;
            }
            let events = parser.feed(&bytes);
            if !events.is_empty() && !send_ansi_batch(&ansi_tx, &cancel, events) {
                return;
            }
        }
    })
}

fn send_ansi_batch(
    ansi_tx: &Sender<AnsiBatch>,
    cancel: &AtomicBool,
    mut events: AnsiBatch,
) -> bool {
    loop {
        match ansi_tx.send_timeout(events, READER_SEND_POLL_INTERVAL) {
            Ok(()) => return true,
            Err(crossbeam::channel::SendTimeoutError::Timeout(rejected)) => {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                events = rejected;
            }
            Err(crossbeam::channel::SendTimeoutError::Disconnected(_)) => return false,
        }
    }
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
    fn pty_signal_names_are_canonical_across_libc_variants() {
        assert_eq!(canonicalize_pty_signal_name("Terminated"), "SIGTERM");
        assert_eq!(canonicalize_pty_signal_name("Terminated: 15"), "SIGTERM");
        assert_eq!(canonicalize_pty_signal_name("Killed: 9"), "SIGKILL");
        assert_eq!(
            canonicalize_pty_signal_name("Unknown signal: 99"),
            "Unknown signal: 99"
        );
    }

    #[test]
    fn terminal_transactional_spawn_failure_has_no_event_or_process_residue() {
        let mut supervisor = ProcessSupervisor::new();
        let spec = ProcessSpec::new(
            "unpublished-terminal",
            "/definitely/not/a/real/pmacs-terminal-program",
        );
        assert!(supervisor.spawn_terminal(spec).is_err());
        supervisor.tick();
        assert_eq!(supervisor.ids().count(), 0);
        assert!(supervisor.take_all_events().is_empty());
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

    /// Spawn a PTY child that leads its own session and stays alive
    /// until terminated, returning its id and OS pid.
    ///
    /// `/bin/sleep` directly rather than through a shell: a shell may
    /// place the command in a different foreground process group, and
    /// these tests assert the exact target the tty reports.
    /// The OS pid straight from the supervisor's own record, WITHOUT
    /// ticking.
    ///
    /// `drain_until` ticks, and a tick can observe a fast child's exit
    /// and transition the record out of `Running` — after which
    /// `signal` returns "is not running" and never reaches the
    /// diagnostic at all. Any test whose child exits promptly must read
    /// the pid this way. (Found by the parallel workspace sweep: the
    /// drain-based helper raced only under load.)
    fn record_pid(sup: &ProcessSupervisor, id: ProcessId) -> u32 {
        match sup.processes.get(&id).expect("record").state {
            ProcessState::Running { pid, .. } | ProcessState::Exiting { pid, .. } => pid,
            ProcessState::Starting => panic!("spawn has not reported a pid yet"),
            ProcessState::Terminated(_) => {
                panic!("the record already left Running; the pid is unavailable")
            }
        }
    }

    /// Drain until `Started` and return the OS pid it carries. Safe
    /// only for children that outlive the drain; see [`record_pid`].
    fn spawn_started_pid(sup: &mut ProcessSupervisor, id: ProcessId) -> u32 {
        let evs = drain_until(sup, id, Duration::from_secs(5), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
        });
        evs.iter()
            .find_map(|e| match e.kind {
                ProcessEventKind::Started { pid } => Some(pid),
                _ => None,
            })
            .expect("Started carries a pid")
    }

    /// Drive the production diagnostic until it observes the leader as
    /// exited, bounded by `timeout`.
    ///
    /// A fixed sleep is NOT proof of exit — on a loaded runner the child
    /// can still be live, which would turn these tests into false
    /// failures. This synchronises on the very observation under test.
    /// Each failing attempt leaves the record untouched, because the
    /// failure path returns before any bookkeeping (Q#PD2), so looping
    /// is side-effect free.
    fn terminate_until_leader_exited(
        sup: &mut ProcessSupervisor,
        id: ProcessId,
        timeout: Duration,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            sup.force_next_kill_errno(nix::errno::Errno::EPERM);
            let err = sup.terminate(id).expect_err("injected EPERM must fail");
            if err.contains("leader=exited(") {
                return err;
            }
            assert!(
                !err.contains("is not running"),
                "the record left Running before the diagnostic could run, so \
                 this test never exercised it: {err}"
            );
            assert!(
                Instant::now() < deadline,
                "leader never observed as exited within {timeout:?}: {err}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The job-control shell the divergence fixture drives. Named once so
    /// the availability guard and the spawn cannot drift apart.
    const BASH: &str = "/bin/bash";

    /// A plain PTY child, for tests that care about the PTY *branch*
    /// rather than about job control.
    fn spawn_live_pty(sup: &mut ProcessSupervisor, name: &str) -> (ProcessId, u32) {
        let mut spec = ProcessSpec::new(name, "/bin/sleep");
        spec.args = vec!["30".into()];
        spec.mode = ProcessMode::Pty {
            rows: 24,
            cols: 80,
            mode: TerminalMode::Canonical,
        };
        let id = sup.spawn(spec).expect("spawn");
        (id, spawn_started_pid(sup, id))
    }

    /// The tty's current foreground process group, read through the
    /// **production** lookup — not `portable_pty`'s
    /// `process_group_leader`, which this crate no longer uses on the
    /// signal path. Reading it any other way would let
    /// `pty_foreground_group` fall back on every call while every test
    /// that depends on it stayed green. `None` for a pipe
    /// generation, or when the terminal reports no foreground group.
    fn foreground_pgid(sup: &ProcessSupervisor, id: ProcessId) -> Option<i32> {
        let runtime = sup.processes.get(&id)?.runtime.as_ref()?;
        match &runtime.child {
            ChildHandle::Pty {
                _master: master, ..
            } => pty_foreground_group(master.as_ref()).ok(),
            ChildHandle::Pipes(_) => None,
        }
    }

    /// Fixture for Q#DC1: a PTY child whose terminal foreground group is
    /// genuinely **not** the spawned leader.
    ///
    /// `bash -m` enables job control, so it runs the script's command in
    /// a fresh process group and hands that group the terminal. The
    /// trailing `; :` matters — with a single simple command `bash -c`
    /// execs in place, which would leave the leader owning the terminal
    /// and silently restore the very agreement this fixture exists to
    /// break.
    ///
    /// **The wait is load-bearing, not defensive.** The handoff is not
    /// instantaneous: a probe of this exact fixture observed the
    /// foreground group as the leader first and only then as the job's
    /// group. Measuring immediately would pin the non-divergent case and
    /// the test would assert the opposite of its purpose.
    ///
    /// Returns `(id, leader_pid, foreground_pgid)` with the two pids
    /// known to differ and the foreground group known to hold a live
    /// member.
    fn spawn_pty_with_diverged_foreground_group(
        sup: &mut ProcessSupervisor,
        name: &str,
    ) -> (ProcessId, u32, i32) {
        let mut spec = ProcessSpec::new(name, BASH);
        spec.args = vec![
            "--noprofile".into(),
            "--norc".into(),
            "-m".into(),
            "-c".into(),
            "sleep 30; :".into(),
        ];
        spec.mode = ProcessMode::Pty {
            rows: 24,
            cols: 80,
            mode: TerminalMode::Canonical,
        };
        let id = sup.spawn(spec).expect("spawn");
        let leader = spawn_started_pid(sup, id);

        let leader_i32 = i32::try_from(leader).expect("pid fits i32");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut observed: Vec<i32> = Vec::new();
        let mut diverged = None;
        while Instant::now() < deadline {
            if let Some(fg) = foreground_pgid(sup, id) {
                if observed.last() != Some(&fg) {
                    observed.push(fg);
                }
                if fg > 0 && fg != leader_i32 {
                    diverged = Some(fg);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let fg = diverged.unwrap_or_else(|| {
            panic!(
                "job control never moved the terminal off the leader \
                 (leader={leader}, foreground groups observed: {observed:?})"
            )
        });

        // Positive control: a divergent number proves nothing if the
        // group is already dead. The signal target must be a group that
        // could actually receive a signal.
        nix::sys::signal::kill(Pid::from_raw(-fg), None).unwrap_or_else(|e| {
            panic!("foreground group {fg} has no live member ({e}); divergence is vacuous")
        });

        (id, leader, fg)
    }

    /// Q#DC1 / acceptance 1 — a group-directed failure names the target,
    /// the branch that chose it, the expected group, the errno, and the
    /// leader's own state, as facts **that are not the same fact
    /// repeated**.
    ///
    /// The pre-Stage-B version spawned `/bin/sleep` on a PTY and asserted
    /// the same pid three times, conceding in its own comment that the
    /// values "are asserted to agree only because nothing has moved the
    /// terminal". An implementation that ignored `tcgetpgrp` and
    /// substituted `leader_pid` passed it — so it pinned the substitution
    /// as acceptable.
    ///
    /// **The foreground group is injected, not produced by a shell.**
    /// Framing Bet 1 wagered that a real job-control fixture would be
    /// deterministic in CI; it is not. `bash -m` diverges reliably on
    /// Linux and never on macOS, where CI observed the terminal stay with
    /// the leader for a full 10s wait on both legs. The framing's stated
    /// fallback is this: pin the divergence at the `signal_target` level
    /// with an injected foreground group, and **say plainly that it is
    /// weaker** than a real one.
    ///
    /// What it still proves: the target is read from the *lookup* rather
    /// than substituted from the leader, because the two values differ
    /// here and the assertion names both. What it no longer proves on its
    /// own: that a real shell ever produces that divergence —
    /// `job_control_really_diverges_the_foreground_group` carries that,
    /// on the platforms where it is real.
    #[test]
    fn a_group_directed_kill_failure_reports_target_and_leader_separately() {
        let mut sup = ProcessSupervisor::new();
        let (id, pid) = spawn_live_pty(&mut sup, "diag-group");

        // A foreground group that is deliberately NOT the leader.
        let leader_i32 = i32::try_from(pid).expect("pid fits i32");
        let fg = leader_i32 + 1;
        assert_ne!(
            fg, leader_i32,
            "the injected group must differ from the leader or this test \
             cannot distinguish a substitution"
        );

        sup.force_next_pty_lookup(Ok(fg));
        sup.force_next_kill_errno(nix::errno::Errno::EPERM);
        let err = sup.terminate(id).expect_err("injected EPERM must fail");

        let expected = format!(
            "kill: {} (signal=SIGTERM, target=-{fg} via tcgetpgrp, leader_pid={pid}, expected_group=-{pid}, leader=live)",
            nix::errno::Errno::EPERM
        );
        assert_eq!(
            err, expected,
            "the report names the exact group the lookup returned, the exact \
             leader pid, and observes the leader as live"
        );

        // Stated separately so a regression that reintroduces the
        // substitution fails by name rather than inside a long string
        // comparison.
        assert!(
            err.contains(&format!("target=-{fg} via tcgetpgrp")),
            "the target must be the group the lookup returned: {err}"
        );
        assert!(
            !err.contains(&format!("target=-{pid} via tcgetpgrp")),
            "the target must NOT be the leader pid: {err}"
        );

        let _ = sup.signal(id, Signal::SIGKILL);
    }

    /// Corroboration for the injected divergence above: a **real** shell
    /// under job control does hand the terminal to a different process
    /// group, and the production lookup reads it.
    ///
    /// Linux-only by arming. macOS is not a skip-because-untested: CI
    /// observed `bash -m` there keep the terminal on the leader for the
    /// entire bounded wait, on both legs, so the precondition this test
    /// needs genuinely does not hold on that platform. Running it there
    /// would assert a false claim about macOS rather than find a bug.
    #[test]
    fn job_control_really_diverges_the_foreground_group() {
        if !std::path::Path::new(BASH).exists() {
            let armed = std::env::var_os("PMACS_REQUIRE_BASH").is_some_and(|v| !v.is_empty());
            assert!(
                !armed,
                "PMACS_REQUIRE_BASH is set but {BASH} does not exist: the \
                 job-control divergence fixture cannot run"
            );
            eprintln!(
                "{BASH} not present; skipping job_control_really_diverges_the_foreground_group"
            );
            return;
        }
        if !cfg!(target_os = "linux") {
            eprintln!(
                "job control does not hand over the terminal for a \
                 non-interactive `bash -m` on this platform; skipping"
            );
            return;
        }

        let mut sup = ProcessSupervisor::new();
        let (id, pid, fg) = spawn_pty_with_diverged_foreground_group(&mut sup, "diag-jobctl");

        let leader_i32 = i32::try_from(pid).expect("pid fits i32");
        assert_ne!(
            fg, leader_i32,
            "a real job-control shell must move the terminal off the leader"
        );

        // Force ONLY the kill failure. The lookup is left alone, so
        // `pty_foreground_group` runs for real against a real terminal
        // and the report below is built from what it returned.
        //
        // This is the assertion that makes the injected pin meaningful:
        // without it, `pty_foreground_group` could fall back on every
        // call and every other test here would still pass, because they
        // all supply the group themselves.
        sup.force_next_kill_errno(nix::errno::Errno::EPERM);
        let err = sup.terminate(id).expect_err("injected EPERM must fail");

        let expected = format!(
            "kill: {} (signal=SIGTERM, target=-{fg} via tcgetpgrp, leader_pid={pid}, expected_group=-{pid}, leader=live)",
            nix::errno::Errno::EPERM
        );
        assert_eq!(
            err, expected,
            "the production lookup must report the real foreground group"
        );
        assert!(
            !err.contains("pty-leader-fallback"),
            "a healthy terminal must not take the fallback branch: {err}"
        );

        let _ = sup.signal(id, Signal::SIGKILL);
    }

    /// Q#DC2 / acceptance 2 — a PTY whose foreground-group lookup fails
    /// is distinguishable from a pipe child that never had a terminal.
    ///
    /// Before this, both rendered "leader-pid". The PTY fallback was
    /// therefore invisible: a terminal query that failed, and a process
    /// with no terminal at all, produced the same word. Each arm now
    /// names its own stage, and `portable-pty`'s
    /// `process_group_leader` — which collapses every failure into
    /// `None` before pmacs can see it — is bypassed so the errno
    /// survives.
    #[test]
    fn a_pty_foreground_lookup_failure_names_its_stage() {
        let arms = [
            (PtyLookupFailure::NoMasterFd, "no-master-fd".to_owned()),
            (
                PtyLookupFailure::Duplicate(nix::errno::Errno::EMFILE),
                format!("duplicate-master-fd: {}", nix::errno::Errno::EMFILE),
            ),
            (
                PtyLookupFailure::Query(nix::errno::Errno::ENOTTY),
                format!("tcgetpgrp: {}", nix::errno::Errno::ENOTTY),
            ),
            (
                PtyLookupFailure::NonPositive(0),
                "tcgetpgrp-non-positive: 0".to_owned(),
            ),
        ];

        for (failure, rendered) in arms {
            let mut sup = ProcessSupervisor::new();
            let (id, pid) = spawn_live_pty(&mut sup, "diag-pty-fallback");

            sup.force_next_pty_lookup(Err(failure));
            sup.force_next_kill_errno(nix::errno::Errno::EPERM);
            let err = sup.terminate(id).expect_err("injected EPERM must fail");

            // The target falls back to the leader — positive, not a
            // negated group — and the source says why.
            let expected = format!(
                "kill: {} (signal=SIGTERM, target={pid} via pty-leader-fallback({rendered}), leader_pid={pid}, leader=live)",
                nix::errno::Errno::EPERM
            );
            assert_eq!(err, expected, "arm {failure:?} must name its own stage");

            // And it must NOT read like a pipe child.
            assert!(
                !err.contains("via leader-pid,"),
                "a PTY fallback must not render as a bare pipe leader target: {err}"
            );

            let _ = sup.signal(id, Signal::SIGKILL);
        }
    }

    /// The companion half of acceptance 2: a genuine pipe child still
    /// renders "leader-pid", so the two really are distinct strings
    /// rather than both having moved.
    ///
    /// Asserted here as well as in the leader-directed test because a
    /// rename of one side would otherwise pass every test — the pair is
    /// the point, not either string alone.
    #[test]
    fn a_pipe_child_still_renders_a_bare_leader_target() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("diag-pipe-leader", "/bin/sleep");
        spec.args = vec!["30".into()];
        let id = sup.spawn(spec).expect("spawn");
        let pid = spawn_started_pid(&mut sup, id);

        sup.force_next_kill_errno(nix::errno::Errno::EPERM);
        let err = sup.terminate(id).expect_err("injected EPERM must fail");

        assert!(
            err.contains(&format!("target={pid} via leader-pid,")),
            "a pipe child with no group renders the bare leader source: {err}"
        );
        assert!(
            !err.contains("pty-leader-fallback"),
            "a pipe child never took the PTY branch: {err}"
        );

        let _ = sup.signal(id, Signal::SIGKILL);
    }

    /// Q#DC3 / acceptance 3(a) — the report names the signal, so two
    /// failures that differ only in which signal was sent are no longer
    /// the same text.
    ///
    /// **They differ in text only.** Every failed `kill` returns before
    /// the fatal-signal branch, so both leave the state and the ledger
    /// exactly as they were. That is asserted here rather than assumed,
    /// because revision 2 of the framing claimed the opposite.
    #[test]
    fn a_failed_signal_names_which_signal_and_changes_nothing() {
        let mut reports = Vec::new();
        for signal in [Signal::SIGTERM, Signal::SIGUSR1] {
            let mut sup = ProcessSupervisor::new();
            let mut spec = ProcessSpec::new("diag-signal-name", "/bin/sh");
            spec.args = vec!["-c".into(), "sleep 30".into()];
            spec.group = true;
            let id = sup.spawn(spec).expect("spawn");
            let pid = spawn_started_pid(&mut sup, id);

            sup.force_next_kill_errno(nix::errno::Errno::EPERM);
            let err = sup
                .signal(id, signal)
                .expect_err("injected EPERM must fail");

            assert!(
                err.contains(&format!("signal={signal:?},")),
                "the report must name {signal:?}: {err}"
            );
            assert!(
                matches!(
                    sup.processes.get(&id).expect("record").state,
                    ProcessState::Running { .. }
                ),
                "a failed {signal:?} must not transition the record"
            );
            assert!(
                sup.reap_ledger.is_empty(),
                "a failed {signal:?} must not arm the ledger"
            );

            reports.push(err.replace(&format!("{pid}"), "<pid>"));
            let _ = nix::sys::signal::kill(
                Pid::from_raw(-i32::try_from(pid).unwrap()),
                Signal::SIGKILL,
            );
        }

        assert_ne!(
            reports[0], reports[1],
            "SIGTERM and SIGUSR1 failures must no longer be identical text"
        );
    }

    /// Q#DC3 / acceptance 3(b) — the disposition control. A *successful*
    /// non-fatal signal changes nothing, while a *successful* fatal one
    /// transitions the record and arms the ledger.
    ///
    /// This is the check that gives the previous test its meaning: it
    /// shows the fatal/non-fatal distinction is real, and therefore that
    /// "failed signals are disposition-identical" is a statement about
    /// the failure path rather than about signals generally.
    #[test]
    fn a_successful_signal_disposition_depends_on_whether_it_is_fatal() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("diag-disposition-live", "/bin/sh");
        // Ignore USR1 so the successful non-fatal signal cannot end the
        // child and confuse the state assertion with a real exit.
        spec.args = vec!["-c".into(), "trap '' USR1; sleep 30".into()];
        spec.group = true;
        let id = sup.spawn(spec).expect("spawn");
        let pid = spawn_started_pid(&mut sup, id);

        sup.signal(id, Signal::SIGUSR1).expect("USR1 delivers");
        assert!(
            matches!(
                sup.processes.get(&id).expect("record").state,
                ProcessState::Running { .. }
            ),
            "a successful non-fatal signal leaves the record Running"
        );
        assert!(
            sup.reap_ledger.is_empty(),
            "a successful non-fatal signal arms no ledger entry"
        );

        sup.terminate(id).expect("TERM delivers");
        assert!(
            matches!(
                sup.processes.get(&id).expect("record").state,
                ProcessState::Exiting { .. }
            ),
            "a successful fatal signal transitions the record to Exiting"
        );
        assert!(
            !sup.reap_ledger.is_empty(),
            "a successful fatal signal arms the group reap ledger"
        );

        let _ =
            nix::sys::signal::kill(Pid::from_raw(-i32::try_from(pid).unwrap()), Signal::SIGKILL);
    }

    /// Q#DC4 / acceptance 4 — the measured group is a real observation,
    /// not a restatement of the input.
    ///
    /// `expected_group` is `-leader_pid` by construction, so on the
    /// spawn-group path it can never disagree with the target. The
    /// measured field is the only one that can, and this proves it does:
    /// a child placed into an *anchor* group reports that group, not its
    /// own pid.
    ///
    /// Without this the field would be exactly the vacuous readout the
    /// framing was written to eliminate — an implementation returning
    /// `-pid` unconditionally would satisfy every other test.
    #[test]
    fn the_measured_group_reports_the_real_group_not_the_pid() {
        use std::os::unix::process::CommandExt as _;

        // An anchor process leading its own group.
        let mut anchor = std::process::Command::new("/bin/sleep");
        anchor.arg("30");
        anchor.process_group(0);
        let mut anchor = anchor.spawn().expect("spawn anchor");
        let anchor_pgid = i32::try_from(anchor.id()).expect("pid fits i32");

        // A second process placed INTO the anchor's group, so its pgid
        // is genuinely not its own pid.
        let mut joiner = std::process::Command::new("/bin/sleep");
        joiner.arg("30");
        joiner.process_group(anchor_pgid);
        let mut joiner = joiner.spawn().expect("spawn joiner");
        let joiner_pid = joiner.id();

        assert_ne!(
            i32::try_from(joiner_pid).unwrap(),
            anchor_pgid,
            "precondition: the joiner must not be the anchor itself"
        );

        let rendered = measured_group_of(joiner_pid);
        assert_eq!(
            rendered,
            format!(", measured_group=-{anchor_pgid}"),
            "the measurement must report the group the kernel actually has"
        );
        assert_ne!(
            rendered,
            format!(", measured_group=-{joiner_pid}"),
            "and must NOT restate the pid it was given"
        );

        let _ = joiner.kill();
        let _ = joiner.wait();
        let _ = anchor.kill();
        let _ = anchor.wait();
    }

    /// Q#PD1 acceptance 2 — a leader-directed failure records the
    /// fallback branch and a positive target, and omits the group field
    /// that would be meaningless for it. Exact message again.
    #[test]
    fn a_leader_directed_kill_failure_reports_the_fallback_branch() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("diag-leader", "/bin/sleep");
        spec.args = vec!["30".into()];
        let id = sup.spawn(spec).expect("spawn");
        let pid = spawn_started_pid(&mut sup, id);

        sup.force_next_kill_errno(nix::errno::Errno::ESRCH);
        let err = sup.terminate(id).expect_err("injected ESRCH must fail");

        let expected = format!(
            "kill: {} (signal=SIGTERM, target={pid} via leader-pid, leader_pid={pid}, leader=live)",
            nix::errno::Errno::ESRCH
        );
        assert_eq!(
            err, expected,
            "a non-group pipe child targets its own pid, and the group \
             field is omitted where it has no meaning"
        );

        let _ = sup.signal(id, Signal::SIGKILL);
    }

    /// Q#PD1 acceptance 3 — every leader state renders distinctly. The
    /// `Unobservable` and `NoRuntime` arms cannot be produced by a real
    /// child on demand, so they are pinned directly; `live` and `exited`
    /// are pinned through the real path by the tests around this one.
    #[test]
    fn every_leader_observation_renders_distinctly() {
        assert_eq!(
            LeaderObservation::Exited(TermStatus::Exited(0)).render(),
            "exited(code 0)"
        );
        assert_eq!(
            LeaderObservation::Exited(TermStatus::Signaled("SIGTERM".into())).render(),
            "exited(signal SIGTERM)"
        );
        assert_eq!(LeaderObservation::Live.render(), "live");
        assert_eq!(
            LeaderObservation::Unobservable("try_wait: boom".into()).render(),
            "unobservable(try_wait: boom)"
        );
        assert_eq!(LeaderObservation::NoRuntime.render(), "no-runtime");
    }

    /// Q#PD1 acceptance 3, exited arm through the REAL path — the leader
    /// has genuinely exited and the report carries its exact code, not
    /// merely "some exit".
    #[test]
    fn a_failure_after_the_child_exits_reports_the_leader_as_exited() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("diag-exited", "/bin/sh");
        spec.args = vec!["-c".into(), "exit 3".into()];
        let id = sup.spawn(spec).expect("spawn");
        // NOT `spawn_started_pid`: draining ticks, and this child exits
        // immediately.
        let pid = record_pid(&sup, id);

        let err = terminate_until_leader_exited(&mut sup, id, Duration::from_secs(10));

        let expected = format!(
            "kill: {} (signal=SIGTERM, target={pid} via leader-pid, leader_pid={pid}, leader=exited(code 3))",
            nix::errno::Errno::EPERM
        );
        assert_eq!(
            err, expected,
            "the exact exit code is observed from the real child, not \
             inferred from the errno"
        );
    }

    /// Q#PD2 acceptance 4 — **the disposition is unchanged.** An
    /// injected failure still fails, and neither the state transition
    /// nor the reap-ledger arming runs. This is the assertion that
    /// separates a diagnostic from the tolerance rules three review
    /// rounds rejected; flipping any arm to `Ok` fails it.
    #[test]
    fn an_injected_failure_changes_no_state_and_arms_no_ledger() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("diag-disposition", "/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        spec.group = true;
        let id = sup.spawn(spec).expect("spawn");
        let pid = spawn_started_pid(&mut sup, id);
        assert!(
            sup.reap_ledger.is_empty(),
            "precondition: nothing armed before the attempt"
        );

        sup.force_next_kill_errno(nix::errno::Errno::EPERM);
        let err = sup.terminate(id).expect_err("injected EPERM must fail");

        let expected = format!(
            "kill: {} (signal=SIGTERM, target=-{pid} via group, leader_pid={pid}, expected_group=-{pid}, measured_group=-{pid}, leader=live)",
            nix::errno::Errno::EPERM
        );
        assert_eq!(err, expected, "a group=true pipe child reports via group");

        assert!(
            matches!(
                sup.processes.get(&id).expect("record").state,
                ProcessState::Running { .. }
            ),
            "a failed kill must not transition the record to Exiting"
        );
        assert!(
            sup.reap_ledger.is_empty(),
            "a failed kill must not arm the reap ledger"
        );

        let _ = sup.signal(id, Signal::SIGKILL);
    }

    /// Q#PD3/Q#PD4 acceptance 5 — the diagnostic consults the REAL
    /// `ChildHandle::try_wait` on the REAL child, which reaps it and
    /// caches the status. `poll_one` must still emit exactly one exit
    /// event, carrying the exact code.
    ///
    /// A stubbed observation would bypass the double-`try_wait` path
    /// entirely and pin nothing, so the injection replaces the kill
    /// result only.
    #[test]
    fn observing_the_leader_does_not_consume_the_exit_event() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("diag-one-event", "/bin/sh");
        spec.args = vec!["-c".into(), "exit 7".into()];
        spec.mode = ProcessMode::Pty {
            rows: 24,
            cols: 80,
            mode: TerminalMode::Canonical,
        };
        let id = sup.spawn(spec).expect("spawn");
        // NOT `spawn_started_pid`: draining ticks, and a tick can reap
        // this immediately-exiting child before the diagnostic runs.
        let _ = record_pid(&sup, id);

        // Drives `observe_leader`, which try_waits the real PTY child
        // for the first time and reaps it.
        let err = terminate_until_leader_exited(&mut sup, id, Duration::from_secs(10));
        assert!(
            err.contains("leader=exited(code 7)"),
            "the real handle was consulted and carries the exact code: {err}"
        );

        // The supervisor's own try_wait must still see that status.
        let evs = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let terminal: Vec<i32> = evs
            .iter()
            .filter_map(|e| match e.kind {
                ProcessEventKind::Exited { code, .. } => Some(code),
                ProcessEventKind::Signaled { .. } => Some(-1),
                _ => None,
            })
            .collect();
        assert_eq!(
            terminal,
            vec![7],
            "exactly one terminal event survives the diagnostic's try_wait, \
             carrying the child's real exit code"
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
    fn write_stdin_queues_without_blocking_when_child_never_reads() {
        let mut sup = ProcessSupervisor::new();
        // The child never reads its stdin, so the kernel pipe buffer
        // (~64 KiB) fills almost immediately. The pre-writer-thread
        // implementation blocked the caller in `write_all` here —
        // which in the editor was the main thread, wedging the frame
        // loop whenever an LSP server fell behind on its stdin.
        let mut spec = ProcessSpec::new("stdin-ignorer", "/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        let id = sup.spawn(spec).expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
        });
        let payload = vec![b'x'; 1024 * 1024]; // 16x the pipe buffer
        let start = Instant::now();
        sup.write_stdin(id, &payload).expect("queued write");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "write_stdin must queue, not block on pipe backpressure (took {:?})",
            start.elapsed()
        );
        sup.terminate(id).expect("terminate");
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
    }

    #[test]
    fn close_stdin_flushes_queued_bytes_before_eof() {
        let mut sup = ProcessSupervisor::new();
        // `cat` echoes stdin and exits on EOF. Receiving the full
        // payload back followed by a clean exit proves the writer
        // thread drains its queue before dropping the pipe (the
        // flush-then-EOF contract `close_stdin` documents).
        let mut spec = ProcessSpec::new("cat-echo", "/bin/sh");
        spec.args = vec!["-c".into(), "cat".into()];
        let id = sup.spawn(spec).expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Started { .. }))
        });
        let payload = vec![b'y'; 256 * 1024];
        sup.write_stdin(id, &payload).expect("queued write");
        sup.close_stdin(id).expect("close stdin");
        let evs = drain_until(&mut sup, id, Duration::from_secs(10), |evs| {
            let echoed: usize = evs
                .iter()
                .filter_map(|e| match &e.kind {
                    ProcessEventKind::Stdout(b) => Some(b.len()),
                    _ => None,
                })
                .sum();
            echoed >= 256 * 1024
                && evs
                    .iter()
                    .any(|e| matches!(e.kind, ProcessEventKind::Exited { .. }))
        });
        let echoed: usize = evs
            .iter()
            .filter_map(|e| match &e.kind {
                ProcessEventKind::Stdout(b) => Some(b.len()),
                _ => None,
            })
            .sum();
        assert_eq!(
            echoed,
            payload.len(),
            "child must receive every queued byte before EOF"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Exited { code: 0 })),
            "EOF after drain must let the child exit cleanly"
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
        let handle = spawn_ansi_parser(
            byte_rx,
            ansi_tx,
            Arc::clone(&cancel),
            AnsiParserProfile::LineOriented,
        );
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

    /// The stdin sink lives in a *field* of [`RuntimeHandles`], so it
    /// cannot drop until `Drop::drop`'s body returns --- and a type's
    /// drop body runs before *all* of its fields, whatever their
    /// declaration order (so reordering the struct cannot fix this).
    /// Joining readers inside that body therefore deadlocks against any
    /// child that exits on stdin EOF while still holding the output
    /// pipe: no EOF, so no exit, so no pipe close, so a blocking
    /// `spawn_reader` never returns.
    ///
    /// This is the root cause of
    /// `m4_5_basedpyright_initializes_and_negotiates_encoding` hanging
    /// forever. Modelled with an orphaned grandchild, which is exactly
    /// what a shim-launched language server is: the basedpyright
    /// console script spawns bundled `node` and exits, leaving the real
    /// server at `PPid 1` holding the inherited pipes.
    ///
    /// `setsid --fork` is used rather than a shell background job, and
    /// that choice is LOAD-BEARING. POSIX XCU 2.9.3 assigns `/dev/null`
    /// to an asynchronous list's stdin when job control is off --- i.e.
    /// in every non-interactive `sh` --- so `sh -c 'cat & exit 0'` reads
    /// EOF immediately and exits *against the unfixed tree*, giving a
    /// test that passes either way and proves nothing. The obvious
    /// repair does not work either: the rule applies **before explicit
    /// redirections**, so by the time `<&0` runs, fd 0 already *is*
    /// `/dev/null` and the redirect faithfully duplicates it onto
    /// itself. `bash` happens to skip the default when a stdin redirect
    /// is present; `dash` --- Ubuntu's `/bin/sh`, and CI's --- does not,
    /// so `<&0` passed locally and failed in CI.
    ///
    /// `setsid --fork` sidesteps all of it: it forks, the parent exits,
    /// and the child inherits stdin/stdout/stderr untouched by any shell.
    /// No async list, no `/dev/null` rule, no implementation variance.
    ///
    /// Linux-gated deliberately rather than incidentally: the controls
    /// read `/proc`, and `setsid(1)` is util-linux (absent on macOS).
    ///
    /// On the failure path this leaks a wedged worker thread, and `cat`
    /// survives until the harness's fds close at process exit. Bounded
    /// and intentional --- a test that *hung* on regression would
    /// reproduce the very hazard it exists to catch.
    #[cfg(target_os = "linux")]
    #[test]
    fn teardown_closes_stdin_before_joining_readers() {
        use std::sync::mpsc;

        /// `sh` becomes a zombie when it exits, because this test
        /// deliberately never ticks (a tick runs `poll_one`, which is
        /// the teardown path under test). `kill(pid, None)` succeeds on
        /// a zombie, so liveness has to come from the process state
        /// rather than from signal 0.
        fn reaped_or_zombie(pid: u32) -> bool {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Err(_) => true,
                Ok(s) => s
                    .rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_whitespace().next())
                    .is_some_and(|state| state == "Z"),
            }
        }

        // setsid(1) is util-linux, not coreutils, and the standard
        // `cargo test --lib` gate must not hard-fail on a tool the
        // README does not require --- a minimal or BusyBox container
        // would fail without ever testing pmacs. So: skip when absent,
        // but FAIL when `PMACS_REQUIRE_SETSID` is set, which CI sets on
        // Linux. That is the arming pattern from the silent-skip lane,
        // and it is what keeps this from becoming a test that reports
        // `ok` having never run. Presence decides, so an empty value
        // counts as unset (a `${{ cond && '1' || '' }}` expression sets
        // the empty string, not nothing).
        let armed = std::env::var_os("PMACS_REQUIRE_SETSID").is_some_and(|v| !v.is_empty());
        if !binary_available("setsid") {
            assert!(
                !armed,
                "PMACS_REQUIRE_SETSID is set but setsid(1) is not on PATH: \
                 install util-linux, or unset the variable to allow the skip"
            );
            eprintln!(
                "setsid(1) not on PATH; skipping \
                 teardown_closes_stdin_before_joining_readers"
            );
            return;
        }

        let (done_tx, done_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut sup = ProcessSupervisor::new();
            sup.set_grace_period(Duration::from_millis(300));
            let mut spec = ProcessSpec::new("orphan-holds-pipe", "setsid");
            // `setsid --fork` forks and the parent exits, so the
            // *recorded* pid terminates promptly (letting `poll_one`
            // reach the teardown path) while `cat` survives holding the
            // inherited pipes. `cat` reads stdin and exits on EOF,
            // exactly as a stdio language server does.
            spec.args = vec!["--fork".into(), "cat".into()];
            // The default, restated because it is the whole point: with
            // `StdinMode::Null` there is no sink to drop and no EOF to
            // deliver.
            spec.stdin = StdinMode::Piped;
            let id = sup.spawn(spec).expect("spawn");

            let sh_pid = sup
                .processes
                .get(&id)
                .and_then(|p| p.runtime.as_ref())
                .map(|rt| rt.pid)
                .expect("runtime records the spawned pid");

            // CONTROL 1: the recorded child must actually exit. Until it
            // does, *it* holds the output pipe, and control 2 would pass
            // for the wrong reason. (`setsid` without `--fork` may exec
            // directly instead of forking, in which case there is no
            // grandchild and this is the control that notices.)
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !reaped_or_zombie(sh_pid) {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                reaped_or_zombie(sh_pid),
                "control 1 failed: the recorded child (`sh`) should exit \
                 promptly, leaving the grandchild orphaned. While `sh` is \
                 alive it holds the output pipe itself, so control 2 would \
                 pass without the grandchild modelling anything"
            );

            // CONTROL 2: both readers must still be blocked in `read`,
            // which is only true while something still holds the output
            // pipe's write ends. If the grandchild never inherited the
            // real stdin, it has already read EOF and exited, the write
            // ends are closed, the readers have finished --- and the
            // deadlock is not being modelled at all. This control is
            // what caught the shell form failing on dash after it
            // passed on bash.
            let readers = sup
                .processes
                .get(&id)
                .and_then(|p| p.runtime.as_ref())
                .map(|rt| {
                    (
                        rt.readers.len(),
                        rt.readers.iter().filter(|h| !h.is_finished()).count(),
                    )
                })
                .expect("runtime still present before teardown");
            assert_eq!(
                readers,
                (2, 2),
                "control 2 failed: both readers must still be blocked in \
                 `read`, i.e. an escaped grandchild still holds the output \
                 pipe. Finished readers mean `cat` read EOF and exited \
                 already, so it never inherited the real stdin --- check \
                 that `setsid --fork` still forks and passes fds 0/1/2 \
                 through untouched on this runner"
            );

            // The deadlock, if present, is here:
            // shutdown -> tick -> poll_one -> RuntimeHandles::drop -> join.
            drop(sup);
            let _ = done_tx.send(());
        });

        done_rx.recv_timeout(Duration::from_secs(10)).expect(
            "supervisor drop should complete within 10s --- if hung, \
             `RuntimeHandles::drop` is joining its readers before dropping \
             the `stdin` field, so the child never receives EOF, never \
             exits, and never closes the output pipe the readers are \
             blocked on",
        );
        handle.join().expect("test thread should exit cleanly");
    }

    // -----------------------------------------------------------------
    // Compile-mode group lifecycle (Q#CM3; framing acceptance 34)
    // -----------------------------------------------------------------

    fn sh_group_spec(label: &str, script: &str) -> ProcessSpec {
        let mut spec = ProcessSpec::new(label, "/bin/sh");
        spec.args = vec!["-c".into(), script.to_owned()];
        spec.stdin = StdinMode::Null;
        spec.group = true;
        spec
    }

    fn started_pid(events: &[ProcessEvent]) -> Option<u32> {
        events.iter().find_map(|e| match e.kind {
            ProcessEventKind::Started { pid } => Some(pid),
            _ => None,
        })
    }

    fn stdout_contains(events: &[ProcessEvent], needle: &[u8]) -> bool {
        let mut all = Vec::new();
        for e in events {
            if let ProcessEventKind::Stdout(b) = &e.kind {
                all.extend_from_slice(b);
            }
        }
        all.windows(needle.len()).any(|w| w == needle)
    }

    fn pid_alive(pid: i32) -> bool {
        nix::sys::signal::kill(Pid::from_raw(pid), None).is_ok()
    }

    /// Process group of `pid` via `ps` (portable across Linux and
    /// macOS CI — the previous /proc/<pid>/stat read has no macOS
    /// equivalent; `ps -o pgid=` avoids widening the nix feature set
    /// with `process` for `getpgid`).
    fn pgid_of(pid: u32) -> i32 {
        let out = std::process::Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .output()
            .expect("run ps");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .expect("pgid parses")
    }

    /// True when `name` resolves on PATH. Fixture-dependency gate:
    /// the setsid escape-hatch test needs util-linux's setsid(1),
    /// absent on macOS — skip per-test rather than fail (the
    /// `m6_5_repl_acceptance` selective-skip precedent).
    fn binary_available(name: &str) -> bool {
        std::process::Command::new("which")
            .arg(name)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Fixture: background a TERM-ignoring survivor and let the
    /// leader exit only after the survivor's trap is INSTALLED
    /// (readiness file). Without the gate, a slow scheduler (macOS
    /// CI, observed) can deliver the leader-exit group-TERM before
    /// the subshell's `trap` runs — killing the "survivor": flaky
    /// red for tests that need it alive, vacuous green for tests
    /// that assert its death. `redirect` sheds the survivor's
    /// stdout/stderr (the acceptance-8 shape); without it the
    /// survivor keeps fd1 (the acceptance-9 shape). Returns
    /// (script, pidfile).
    fn survivor_script(dir: &std::path::Path, redirect: bool) -> (String, std::path::PathBuf) {
        let pidfile = dir.join("pid");
        let ready = dir.join("ready");
        let redirect_part = if redirect {
            "exec >/dev/null 2>&1; "
        } else {
            ""
        };
        let script = format!(
            "( trap '' TERM; : > {ready}; {redirect_part}sleep 30 ) & echo $! > {pid}; \
             while [ ! -e {ready} ]; do sleep 0.01; done",
            ready = ready.display(),
            pid = pidfile.display(),
        );
        (script, pidfile)
    }

    /// Poll `path` until it holds a parseable pid. Fixture scripts
    /// write descendant pids there.
    fn wait_pidfile(path: &std::path::Path) -> i32 {
        let stop = Instant::now() + Duration::from_secs(5);
        while Instant::now() < stop {
            if let Ok(s) = std::fs::read_to_string(path)
                && let Ok(pid) = s.trim().parse::<i32>()
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("pidfile {} never appeared", path.display());
    }

    #[test]
    fn stdin_null_yields_immediate_eof() {
        let mut sup = ProcessSupervisor::new();
        // `cat` exits only at stdin EOF; under piped stdin this test
        // would hang until the drain deadline killed it. (Framing
        // acceptance 34 / round-1 finding 3.)
        let spec = sh_group_spec("eof-test", "cat; echo done");
        let id = sup.spawn(spec).expect("spawn");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Exited { code: 0 })),
            "cat must see EOF and exit 0; events: {events:?}"
        );
        assert!(
            stdout_contains(&events, b"done"),
            "post-cat echo must run; events: {events:?}"
        );
    }

    #[test]
    fn group_true_spawns_distinct_process_group() {
        let mut sup = ProcessSupervisor::new();
        let id = sup
            .spawn(sh_group_spec("group-test", "sleep 30"))
            .expect("spawn");
        let events = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
            started_pid(evs).is_some()
        });
        let pid = started_pid(&events).expect("Started event");
        assert_eq!(
            pgid_of(pid),
            i32::try_from(pid).unwrap(),
            "group child must lead its own process group (pgid == pid)"
        );
        // Control: a non-group child inherits the test process's
        // group instead of leading its own.
        let mut plain = ProcessSpec::new("plain", "/bin/sh");
        plain.args = vec!["-c".into(), "sleep 30".into()];
        let plain_id = sup.spawn(plain).expect("spawn plain");
        let plain_events = drain_until(&mut sup, plain_id, Duration::from_secs(2), |evs| {
            started_pid(evs).is_some()
        });
        let plain_pid = started_pid(&plain_events).expect("Started event");
        assert_ne!(
            pgid_of(plain_pid),
            i32::try_from(plain_pid).unwrap(),
            "non-group child must not lead its own group"
        );
        sup.terminate(id).ok();
        sup.terminate(plain_id).ok();
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let _ = drain_until(&mut sup, plain_id, Duration::from_secs(5), has_exited);
    }

    #[test]
    fn terminate_group_escalates_to_sigkill_on_term_trapping_child() {
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_millis(200));
        // Readiness echo: terminating before the trap is installed
        // would let plain SIGTERM win and vacuously pass.
        let id = sup
            .spawn(sh_group_spec(
                "trap-test",
                "trap '' TERM; echo ready; sleep 30",
            ))
            .expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
            stdout_contains(evs, b"ready")
        });
        let t0 = Instant::now();
        sup.terminate(id).expect("terminate");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let elapsed = t0.elapsed();
        assert!(
            events.iter().any(|e| matches!(
                &e.kind,
                ProcessEventKind::Signaled { signal } if signal == "SIGKILL"
            )),
            "TERM-trapping child must fall to the ledger's SIGKILL; events: {events:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1500),
            "escalation must land near the 200ms grace, not the 2s drain timeout; took {elapsed:?}"
        );
    }

    #[test]
    fn liveness_probe_reaps_term_ignoring_survivor_after_leader_exit() {
        // Unit twin of framing acceptance 8: the survivor ignores
        // TERM *and* sheds its stdout/stderr, so the leader's
        // terminal event arrives and the readers finish — only the
        // ledger's kill(-pgid, 0) probe can catch it.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_millis(200));
        let (script, pidfile) = survivor_script(dir.path(), true);
        let id = sup
            .spawn(sh_group_spec("survivor", &script))
            .expect("spawn");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Exited { code: 0 })),
            "leader must exit cleanly; events: {events:?}"
        );
        let survivor = wait_pidfile(&pidfile);
        // The ledger fires on subsequent ticks — keep ticking.
        let stop = Instant::now() + Duration::from_secs(3);
        while Instant::now() < stop && pid_alive(survivor) {
            sup.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !pid_alive(survivor),
            "TERM-ignoring redirected survivor must be SIGKILLed by the ledger probe"
        );
        // Ledger converges to empty once the group probes ESRCH.
        let stop = Instant::now() + Duration::from_secs(2);
        while Instant::now() < stop && sup.reap_ledger_len() > 0 {
            sup.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(sup.reap_ledger_len(), 0, "ledger must drain to empty");
    }

    #[test]
    fn repeated_terminate_does_not_extend_ledger_deadline() {
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_millis(500));
        let id = sup
            .spawn(sh_group_spec(
                "re-term",
                "trap '' TERM; echo ready; sleep 30",
            ))
            .expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
            stdout_contains(evs, b"ready")
        });
        let t0 = Instant::now();
        sup.terminate(id).expect("terminate");
        // Re-terminate at half the grace window: with plain
        // HashMap::insert arming, this would reset the 500ms clock
        // and push SIGKILL past 800ms.
        std::thread::sleep(Duration::from_millis(300));
        sup.tick();
        sup.terminate(id).expect("re-terminate");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let elapsed = t0.elapsed();
        assert!(
            events.iter().any(|e| matches!(
                &e.kind,
                ProcessEventKind::Signaled { signal } if signal == "SIGKILL"
            )),
            "must escalate; events: {events:?}"
        );
        assert!(
            elapsed < Duration::from_millis(750),
            "earliest deadline must win: SIGKILL by ~500ms, not 800ms; took {elapsed:?}"
        );
    }

    #[test]
    fn shutdown_force_kills_outstanding_ledger_groups() {
        // Drop-twin of framing acceptance 8 (round-4 finding 1): the
        // grace is long enough that the ledger cannot fire on its
        // own — only shutdown's force-kill can reap the survivor.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_secs(30));
        let (script, pidfile) = survivor_script(dir.path(), true);
        let id = sup
            .spawn(sh_group_spec("survivor", &script))
            .expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let survivor = wait_pidfile(&pidfile);
        assert!(pid_alive(survivor), "survivor alive pre-shutdown");
        assert!(sup.reap_ledger_len() > 0, "ledger armed pre-shutdown");
        sup.shutdown();
        assert!(
            !pid_alive(survivor),
            "shutdown must force-kill outstanding ledger groups"
        );
        assert_eq!(
            sup.reap_ledger_len(),
            0,
            "shutdown must probe forced kills to ESRCH"
        );
    }

    // -----------------------------------------------------------------
    // Reap-ledger silent failures (framing §4).
    //
    // Every pin below asserts the *consequence* of a discarded kill(2)
    // result — a live group the ledger stopped tracking, a survivor the
    // ledger records as killed, output cancelled before it arrived —
    // and never that a function was called. All five pin CURRENT
    // behaviour, including the behaviour that is wrong: this lane
    // changes no disposition (§7).
    //
    // Each ends with `assert_reap_faults_consumed`, which is not
    // ceremony: a planned failure that was never consumed means the
    // fixture never reached the production site, and an
    // absence-assertion under those conditions is vacuous.
    // -----------------------------------------------------------------

    /// Kill a leaked fixture descendant that a pin deliberately let
    /// survive. Called after the assertions, never before them.
    fn reap_fixture_survivor(pid: i32) {
        let _ = nix::sys::signal::kill(Pid::from_raw(pid), Some(Signal::SIGKILL));
    }

    #[test]
    fn an_unreachable_probe_drops_an_entry_whose_group_is_still_alive() {
        // §1.2 (a). `retain` returning false deletes the entry, so
        // escalation is cancelled — and the probe cannot tell ESRCH
        // ("the group is gone", correct) from any other errno ("we
        // could not ask", not correct).
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        // Long enough that the escalation cannot fire and confuse the
        // reading: the only thing that empties the ledger here is (a).
        sup.set_group_term_grace(Duration::from_secs(30));
        let (script, pidfile) = survivor_script(dir.path(), true);
        let id = sup
            .spawn(sh_group_spec("probe-eperm", &script))
            .expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let survivor = wait_pidfile(&pidfile);
        assert!(pid_alive(survivor), "survivor alive before the probe");
        assert_eq!(sup.reap_ledger_len(), 1, "ledger armed before the probe");

        sup.plan_reap_kill_failure(ReapKillSite::LedgerProbe, nix::errno::Errno::EPERM);
        sup.tick();

        assert_eq!(
            sup.reap_ledger_len(),
            0,
            "an EPERM probe drops the entry exactly as ESRCH would"
        );
        assert!(
            pid_alive(survivor),
            "and the group it stopped tracking is still alive — this is the leak"
        );
        sup.assert_reap_faults_consumed();
        reap_fixture_survivor(survivor);
    }

    #[test]
    fn a_failed_escalation_is_recorded_as_a_successful_one() {
        // §1.2 (b). `entry.killed = true` runs unconditionally, so a
        // SIGKILL that never landed satisfies `!entry.killed == false`
        // forever. The consequence is not bookkeeping: the survivor is
        // never killed again, by anything.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_millis(150));
        let (script, pidfile) = survivor_script(dir.path(), true);
        let id = sup
            .spawn(sh_group_spec("kill-eperm", &script))
            .expect("spawn");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let pgid = i32::try_from(started_pid(&events).expect("leader pid")).expect("pgid fits");
        let survivor = wait_pidfile(&pidfile);

        sup.plan_reap_kill_failure(ReapKillSite::LedgerEscalation, nix::errno::Errno::EPERM);
        // Past the grace, so the escalation arm is the one that runs.
        std::thread::sleep(Duration::from_millis(250));
        sup.tick();

        assert_eq!(
            sup.reap_ledger_killed(pgid),
            Some(true),
            "the entry records a SIGKILL that returned EPERM as sent"
        );
        // The retry that `killed = true` forecloses: keep ticking well
        // past the grace and the survivor is still there.
        let stop = Instant::now() + Duration::from_millis(600);
        while Instant::now() < stop {
            sup.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            pid_alive(survivor),
            "no tick ever retries the failed SIGKILL, so the survivor outlives the ledger's only escalation"
        );
        assert_eq!(sup.reap_ledger_len(), 1, "the entry is retained, and inert");
        sup.assert_reap_faults_consumed();
        reap_fixture_survivor(survivor);
    }

    #[test]
    fn a_failed_shutdown_force_kill_leaks_the_group_and_burns_the_bound() {
        // §1.2 (c). The path that exists specifically to stop a leak at
        // editor exit discards its own kill result — and because it
        // still sets `killed`, the final loop can never escalate again.
        // It therefore runs to its full 2s bound and exits with the
        // member alive.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        // The ledger must not be able to reap on its own; only
        // shutdown's force-kill could have.
        sup.set_group_term_grace(Duration::from_secs(30));
        let (script, pidfile) = survivor_script(dir.path(), true);
        let id = sup
            .spawn(sh_group_spec("shutdown-eperm", &script))
            .expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let survivor = wait_pidfile(&pidfile);
        assert!(pid_alive(survivor), "survivor alive pre-shutdown");

        sup.plan_reap_kill_failure(ReapKillSite::ShutdownForceKill, nix::errno::Errno::EPERM);
        let t0 = Instant::now();
        sup.shutdown();
        let elapsed = t0.elapsed();

        assert!(
            pid_alive(survivor),
            "the group survives editor exit — the leak this path was written to prevent"
        );
        assert_eq!(
            sup.reap_ledger_len(),
            1,
            "the entry never reaches ESRCH, so the loop holds it to the bound"
        );
        assert!(
            elapsed >= Duration::from_millis(1900),
            "the final loop runs to its 2s bound rather than converging; took {elapsed:?}"
        );
        sup.assert_reap_faults_consumed();
        reap_fixture_survivor(survivor);
    }

    #[test]
    fn a_probe_error_after_a_failed_force_kill_exits_the_shutdown_loop_early() {
        // Bet 3, and §1.3's coupling. The loop runs while
        // `any_running() || !reap_ledger.is_empty()`, so an early exit
        // needs BOTH: the fixture is a leader that has already exited
        // (any_running() false) leaving a group survivor, which is
        // exactly the case the ledger exists to serve.
        //
        // This is also the pin that justifies a multi-outcome seam:
        // it needs a failed force-kill AND a failed subsequent probe
        // pending at the same time.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_secs(30));
        let (script, pidfile) = survivor_script(dir.path(), true);
        let id = sup
            .spawn(sh_group_spec("coupling", &script))
            .expect("spawn");
        let _ = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let survivor = wait_pidfile(&pidfile);
        assert!(
            !sup.any_running(),
            "the leader has exited: the other arm of the disjunction is already false"
        );

        sup.plan_reap_kill_failure(ReapKillSite::ShutdownForceKill, nix::errno::Errno::EPERM);
        sup.plan_reap_kill_failure(ReapKillSite::LedgerProbe, nix::errno::Errno::EPERM);
        let t0 = Instant::now();
        sup.shutdown();
        let elapsed = t0.elapsed();

        assert_eq!(
            sup.reap_ledger_len(),
            0,
            "the errored probe empties the ledger"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "and the loop then exits early, concluding cleanup finished because the probe failed; took {elapsed:?}"
        );
        assert!(
            pid_alive(survivor),
            "while the survivor it concluded about is alive"
        );
        sup.assert_reap_faults_consumed();
        reap_fixture_survivor(survivor);
    }

    #[test]
    fn a_collapsed_in_drain_probe_cancels_readers_before_late_output() {
        // §1.2a, the fourth site. `is_ok()` collapses every errno into
        // "the group is dead", which makes `quiesced` true and cancels
        // the readers. Unlike the persistent ledger, no later tick can
        // revisit this: the decision is terminal for that drain, and
        // the failure mode is truncated output rather than a leak.
        //
        // The fault repeats for the whole drain because a one-shot
        // cannot reach `quiesced`: the loop probes again every 1ms and
        // the quiescent window is a full READER_SEND_POLL_INTERVAL.
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("pid");
        let mut sup = ProcessSupervisor::new();
        // Far enough out that the real path would wait, and the drain's
        // own 2s EXIT_OUTPUT_DRAIN_TIMEOUT would still collect the late
        // marker at ~500ms. That is what makes the absence meaningful.
        sup.set_group_term_grace(Duration::from_secs(3));
        // The descendant keeps fd1, so the readers stay open after the
        // leader exits. EARLY is written before the leader exits, so it
        // is in the pipe before the drain begins.
        //
        // **`trap '' TERM` is load-bearing, and its absence made the
        // first draft of this pin vacuous.** `poll_one` TERMs the whole
        // group on leader exit, so an untrapped descendant dies before
        // its 0.5s sleep ends — the late marker then never arrives on
        // *either* path, and "LATE-MARKER is absent" holds for a reason
        // that has nothing to do with the probe. The bite caught it:
        // with the seam reverted the pin still passed both content
        // assertions and failed only the consumed-plan check.
        //
        // The readiness gate is `survivor_script`'s, for its reason: a
        // slow scheduler can otherwise deliver the group TERM before
        // the subshell's `trap` runs.
        let ready = dir.path().join("ready");
        let script = format!(
            "echo EARLY; ( trap '' TERM; : > {ready}; sleep 0.5; echo LATE-MARKER; sleep 5 ) & \
             echo $! > {pid}; while [ ! -e {ready} ]; do sleep 0.01; done",
            ready = ready.display(),
            pid = pidfile.display(),
        );
        sup.plan_in_drain_probe_failure(nix::errno::Errno::EPERM);
        let id = sup
            .spawn(sh_group_spec("in-drain", &script))
            .expect("spawn");
        let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
        let survivor = wait_pidfile(&pidfile);

        // Positive control on the fixture itself. Without it, "LATE is
        // absent" would also hold if the pipe never carried anything.
        assert!(
            stdout_contains(&events, b"EARLY"),
            "the fixture's pipe must actually deliver output; events: {events:?}"
        );
        assert!(
            !stdout_contains(&events, b"LATE-MARKER"),
            "a probe that reports EPERM as 'dead' quiesces the drain and cancels the readers \
             before the live descendant's later output can arrive; events: {events:?}"
        );
        sup.assert_reap_faults_consumed();
        reap_fixture_survivor(survivor);
        sup.shutdown();
    }

    #[test]
    fn maybe_restart_inert_once_shut_down() {
        let mut sup = ProcessSupervisor::new();
        sup.set_restart_backoff(Duration::from_millis(30));
        let mut spec = ProcessSpec::new("restarter", "/bin/sh");
        spec.args = vec!["-c".into(), "echo x".into()];
        spec.restart = RestartPolicy::Always;
        let id = sup.spawn(spec).expect("spawn");
        // Prove the policy is live: observe at least one restart.
        let events = drain_until(&mut sup, id, Duration::from_secs(5), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Restarting { .. }))
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, ProcessEventKind::Restarting { .. })),
            "restart=always must restart pre-shutdown; events: {events:?}"
        );
        sup.shutdown();
        let _ = sup.take_events(id);
        // Give a reset restart-backoff window plenty of room, then
        // confirm no respawn happened during or after teardown.
        for _ in 0..8 {
            sup.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        let after = sup.take_events(id);
        assert!(
            !after.iter().any(|e| matches!(
                e.kind,
                ProcessEventKind::Restarting { .. } | ProcessEventKind::Started { .. }
            )),
            "restart accounting must be inert once shut down; events: {after:?}"
        );
    }

    #[test]
    fn leader_exit_reap_bounds_drain_with_pipe_holding_descendant() {
        // Unit twin of framing acceptance 9: the descendant ignores
        // TERM and KEEPS fd1, so the readers stay alive and the old
        // drain would block ~2s per EXIT_OUTPUT_DRAIN_TIMEOUT (and
        // then the join would hang). In-drain ledger enforcement
        // SIGKILLs at the grace bound instead. Readiness-gated so an
        // early leader-exit TERM can't reap the holder and let the
        // bound hold vacuously.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sup = ProcessSupervisor::new();
        sup.set_group_term_grace(Duration::from_millis(300));
        let (script, _pidfile) = survivor_script(dir.path(), false);
        let id = sup.spawn(sh_group_spec("holder", &script)).expect("spawn");
        let stop = Instant::now() + Duration::from_secs(5);
        let mut max_tick = Duration::ZERO;
        let mut events = Vec::new();
        while Instant::now() < stop && !has_exited(&events) {
            let t = Instant::now();
            sup.tick();
            max_tick = max_tick.max(t.elapsed());
            events.append(&mut sup.take_events(id));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            has_exited(&events),
            "leader exit must be observed; events: {events:?}"
        );
        assert!(
            max_tick < Duration::from_millis(1200),
            "the blocking tick must be bounded by ~grace + 2 poll intervals, \
             not the 2s drain timeout; max tick {max_tick:?}"
        );
    }

    #[test]
    fn setsid_escapee_is_not_reaped_and_teardown_reclaims_readers() {
        // The setsid'd descendant leaves the group (the deliberate
        // daemonization escape hatch) while inheriting fd1, so it
        // holds the pipe after its old group is ESRCH. The
        // quiescence/cancel cap must bound the drain, the retained
        // joins must complete, and the per-runtime active-reader
        // count must return to zero — across repeated cycles, so
        // nothing accumulates.
        if !binary_available("setsid") {
            // util-linux's setsid(1) is absent on macOS CI; the
            // escape hatch is a Linux-production behavior. Skip
            // rather than fail — the other group-lifecycle tests
            // still run everywhere.
            eprintln!("skipping: setsid(1) not on PATH");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let mut escapees = Vec::new();
        for round in 0..3 {
            let pidfile = dir.path().join(format!("pid{round}"));
            let mut sup = ProcessSupervisor::new();
            sup.set_group_term_grace(Duration::from_millis(300));
            // Do not let the group leader exit until the background child has
            // completed `setsid` and published its pid. Without this readiness
            // gate, teardown can TERM the old process group before `setsid`
            // runs; the child then dies before creating the pidfile (a race
            // exposed consistently by the Ubuntu 20260714 runner image).
            let script = format!(
                "setsid /bin/sh -c 'echo $$ > {pid}; exec sleep 30' & \
                 while [ ! -s {pid} ]; do sleep 0.01; done; echo started",
                pid = pidfile.display()
            );
            let id = sup.spawn(sh_group_spec("escapee", &script)).expect("spawn");
            let ready = drain_until(&mut sup, id, Duration::from_secs(2), |evs| {
                started_pid(evs).is_some()
            });
            assert!(started_pid(&ready).is_some(), "Started must arrive");
            let probe = sup.active_reader_probe(id).expect("live runtime probe");
            let t0 = Instant::now();
            let events = drain_until(&mut sup, id, Duration::from_secs(5), has_exited);
            let elapsed = t0.elapsed();
            assert!(
                has_exited(&events),
                "leader exit must be observed; events: {events:?}"
            );
            assert!(
                elapsed < Duration::from_millis(1500),
                "escaped-writer drain must be cancelled at the cap, \
                 not ride the 2s timeout; took {elapsed:?}"
            );
            assert_eq!(
                probe.load(Ordering::Relaxed),
                0,
                "reader threads must have ended and dropped their FDs"
            );
            let escapee = wait_pidfile(&pidfile);
            assert!(
                pid_alive(escapee),
                "setsid escapee must NOT be reaped (deliberate escape hatch)"
            );
            escapees.push(escapee);
        }
        // Fixture owns the escapees the supervisor deliberately
        // does not: kill them explicitly.
        for pid in escapees {
            let _ = nix::sys::signal::kill(Pid::from_raw(pid), Some(Signal::SIGKILL));
        }
    }

    #[test]
    fn group_and_null_stdin_rejected_under_pty() {
        let mut sup = ProcessSupervisor::new();
        let mut spec = ProcessSpec::new("pty-null", "/bin/sh");
        spec.mode = ProcessMode::default_pty();
        spec.stdin = StdinMode::Null;
        let err = sup
            .spawn(spec)
            .expect_err("stdin=null must be rejected under pty");
        assert!(
            err.contains("pipe mode"),
            "error points at pipe mode: {err}"
        );

        let mut spec = ProcessSpec::new("pty-group", "/bin/sh");
        spec.mode = ProcessMode::default_pty();
        spec.group = true;
        let err = sup
            .spawn(spec)
            .expect_err("group=true must be rejected under pty");
        assert!(
            err.contains("pipe mode"),
            "error points at pipe mode: {err}"
        );
    }
}
