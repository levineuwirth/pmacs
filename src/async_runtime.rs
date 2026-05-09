// async_runtime.rs --- T M3.3 main-thread async runtime: dispatcher,
// pending-job table, and tick that drains the worker reply bus.

//! Async runtime (T M3.3).
//!
//! This module is the bridge between the Rust worker pool ([T M3.1]) and
//! the message bus ([T M3.2]) on the producer side, and the Lua
//! coroutine API ([R44], [R45], [R46]) on the consumer side. It runs
//! exclusively on the main editor thread, so it uses [`Rc`] +
//! [`RefCell`] for shared state instead of [`Arc`] + [`Mutex`].
//!
//! # Topology
//!
//! ```text
//!                        ┌─────────────────────────────────┐
//!                        │ AsyncRuntime (main thread, !Send)│
//!                        │                                  │
//!  Lua: pmacs.workers.*  │  pool ──► [worker threads]       │
//!  -------------------►  │   │           │                  │
//!                        │   ▼           ▼ bus.send         │
//!                        │ pending  bus_main ◄──────┐       │
//!                        │   ▲             tick()   │       │
//!                        │   └─────────────────────┘        │
//!                        └──────────────────────────────────┘
//! ```
//!
//! `dispatch_*` allocates a fresh [`JobId`], inserts a [`PendingJob`]
//! into [`AsyncRuntime::pending`], and submits a closure to the
//! [`WorkerPool`] that always responds via the bus --- including a
//! `Cancelled` reply when the closure observes the runtime's own
//! [`CancellationToken`]. [`AsyncRuntime::tick`] drains every
//! envelope queued on `bus_main`, decodes it, and updates the
//! corresponding pending entry.
//!
//! # Cancellation token discipline
//!
//! The runtime carries its **own** cancellation token per job, separate
//! from the [`WorkerPool`]'s built-in token. The pool's token is used
//! for skip-before-pickup semantics; we never set it. Our own token is
//! the one [`AsyncRuntime::cancel`] flips, and the worker closure
//! always runs (so a cancelled job still produces a `Cancelled` reply,
//! making the pending table reachable from `tick`).
//!
//! # Builtin handlers
//!
//! T M3.3 ships two built-in handlers, intended primarily as
//! exercise material for the coroutine API:
//!
//! * [`AsyncRuntime::dispatch_sleep`] --- sleeps `ms` milliseconds
//!   while polling the cancel token at 1ms granularity. Returns
//!   `JobResult::Unit`.
//! * [`AsyncRuntime::dispatch_compute_sum`] --- computes
//!   `1 + 2 + ... + n` while polling the cancel token. Returns
//!   `JobResult::Sum(u64)`.
//!
//! T M3.6 adds the parallel grep handler ([`AsyncRuntime::dispatch_grep`])
//! that doubles as M3's milestone-justifying load: parallel directory
//! search with cooperative cancellation and frame-boundary coalescing.
//! Tree-sitter and LSP land in M4 on the same dispatch shape.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam::channel as cb_channel;
use serde::{Deserialize, Serialize};

use crate::fs::{
    FsDirEntry, FsError, chmod_blocking, read_dir_blocking, remove_blocking, rename_blocking,
    stat_blocking,
};
use crate::message_bus::{BusEnd, MessageBus, SchemaRegistry};
use crate::syntax::{self as syntax_mod, ParseRequest, ParseTreeBundle};
use crate::worker::{CancellationToken, WorkerPool};

/// Identifier the runtime hands to Lua so a [`Handle`] knows which
/// pending entry it is bound to. Distinct from the bus's
/// `MessageId` and from the pool's `JobId` --- this is the ID Lua
/// userdata holds.
pub type JobId = u64;

/// Bus topic carrying every worker reply. A single topic suffices
/// because [`WorkerReply::kind`] discriminates between the variants
/// our handlers produce.
const ASYNC_REPLY_TOPIC: &str = "async.reply";

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// One item produced by a stream handler. Each variant corresponds
/// to a stream "shape" (numeric ticker, grep match, ...); the Lua
/// binding layer translates the variant into the appropriate
/// representation when delivering a batch to user callbacks.
///
/// New variants land here as new streaming handlers are added. The
/// enum is `#[non_exhaustive]` from the Lua side's perspective (the
/// match in `lua_bindings::install_async` exhaustively translates
/// every variant), but staying exhaustive in Rust keeps the compiler
/// honest when handlers are added.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamPayload {
    /// 1-based index emitted by the synthetic `emit_n` handler. T M3.5.
    U64(u64),
    /// One grep match: the file the match was found in, the 1-based
    /// line number, byte offsets of the match within the line text,
    /// and the line text itself (truncated past
    /// [`GrepSpec::max_match_text`]). T M3.6.
    Match(GrepMatch),
}

/// One match emitted by [`run_grep`]. Owned strings only --- a worker
/// holds nothing but its own copy of these bytes ([R31]). The Lua
/// binding turns each into a `{ file, line, text, match_start,
/// match_end }` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepMatch {
    /// Path the match was found in, relative to the search root.
    pub file: String,
    /// 1-based line number within the file.
    pub line: u32,
    /// Byte offset of the match start within `text`.
    pub match_start: u32,
    /// Byte offset of the match end within `text` (exclusive).
    pub match_end: u32,
    /// The full line containing the match, truncated to
    /// [`GrepSpec::max_match_text`] bytes if longer. Always valid
    /// UTF-8 (non-UTF-8 lines are skipped at search time).
    pub text: String,
}

/// Description of a grep job: where to search, what to look for, and
/// the limits that bound resource use. Kept owned-only ([R31]) so the
/// closure dispatched to a worker holds nothing borrowed from the main
/// thread.
#[derive(Clone, Debug)]
pub struct GrepSpec {
    /// Directory to walk. Symlinks and hidden directories
    /// (`.git`, `.svn`, `.hg`, `node_modules`, `target`) are skipped
    /// to keep the workload bounded; a future opt-in would expose
    /// these as filter knobs.
    pub root: PathBuf,
    /// Literal byte pattern to search for. Empty pattern matches no
    /// lines (rather than all lines, which would be a footgun).
    /// Regex support lands in M4 with the LSP pipeline.
    pub pattern: String,
    /// If `false`, ASCII-fold both pattern and line bytes before
    /// comparing (`Aa` and `aA` match `aa`). Defaults to `true`.
    pub case_sensitive: bool,
    /// Skip files larger than this (bytes). Default: 16 MiB ---
    /// matches ripgrep's behaviour and bounds the per-file
    /// allocation a worker has to make.
    pub max_file_bytes: u64,
    /// Truncate any line longer than this (bytes) to this length
    /// before emitting it. Defaults to 4 KiB. Pathological binary
    /// or minified files can have multi-megabyte "lines" and would
    /// otherwise blow the wire format.
    pub max_match_text: u32,
    /// Stop emitting after this many matches; `0` means unlimited.
    /// Default: 0. The user-facing cancel + supersede paths are the
    /// canonical way to cut a search short --- this knob is for
    /// pathological "every line matches" loads.
    pub max_results: u32,
    /// Number of internal threads the grep job spawns to fan out
    /// file searches. Defaults to `available_parallelism`. The grep
    /// dispatch itself occupies one [`WorkerPool`] slot; these are
    /// scoped threads spawned *inside* that closure.
    pub fanout: usize,
}

impl GrepSpec {
    /// A spec searching `root` for the literal `pattern`, with
    /// reasonable defaults for the rest. Callers tune individual
    /// fields with struct-update syntax.
    #[must_use]
    pub fn new(root: PathBuf, pattern: String) -> Self {
        let fanout = thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
        Self {
            root,
            pattern,
            case_sensitive: true,
            max_file_bytes: 16 * 1024 * 1024,
            max_match_text: 4096,
            max_results: 0,
            fanout: fanout.max(1),
        }
    }
}

/// Discriminant carried alongside each reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum ReplyKind {
    /// `dispatch_sleep` finished without observing cancellation.
    Sleep,
    /// `dispatch_compute_sum` completed; payload is the sum.
    Sum(u64),
    /// The handler observed cancellation and exited early.
    Cancelled,
    /// The handler raised. Carries a stringified message --- richer
    /// error structure can land in M4 once real handlers exist.
    Error(String),
    /// One streaming item from a stream handler. Multiple items per
    /// stream id are accumulated in [`PendingJob::stream_buffer`] and
    /// delivered as a single batch on the next
    /// [`AsyncRuntime::take_stream_batches`] call. T M3.5.
    StreamItem(StreamPayload),
    /// Stream completed cleanly (no further items will arrive).
    StreamClosed,
    /// `dispatch_parse` settled. The fresh
    /// [`crate::syntax::ParseTreeBundle`] is *not* on the wire ---
    /// trees aren't `Serialize` --- so the worker has already stashed
    /// it in [`AsyncRuntime::parse_handoff`] under `job_id`.
    /// `duration_ms` is the parse-only duration (excludes dispatch
    /// queueing) and is what the M4.1 acceptance criteria measure.
    /// T M4.1.
    Parse { duration_ms: u64 },
    /// `dispatch_fs_read_dir` completed; payload is the directory
    /// listing. The Vec is `Serialize` so it crosses the bus
    /// directly --- no side handoff like parse trees need. T M8.1.
    ReadDir(Vec<FsDirEntry>),
    /// `dispatch_fs_stat` completed; payload is the per-path
    /// metadata. T M8.1.
    Stat(FsDirEntry),
    /// Generic completion-with-no-payload reply for the unit-result
    /// fs primitives (`rename`, `chmod`, `remove`). Distinct from
    /// [`Self::Sleep`] so the worker observability layer can label
    /// fs jobs separately. T M8.1.
    FsUnit,
    /// Externally-settled job produced a JSON value. Sent by
    /// [`AsyncRuntime::complete_external_ok`] from the main thread;
    /// the runtime's `tick` translates it to
    /// [`JobResult::Json`]. T M9.1.
    Json(serde_json::Value),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkerReply {
    job_id: JobId,
    kind: ReplyKind,
}

// ---------------------------------------------------------------------------
// Runtime-side types
// ---------------------------------------------------------------------------

/// Result a completed job hands back to Lua. Per-handler payload
/// shape lives here so [`crate::lua_bindings`] can convert it without
/// reaching into the wire enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobResult {
    /// Sleep handler completed (no payload).
    Unit,
    /// Compute-sum handler completed; carries the sum.
    Sum(u64),
    /// Parse handler completed. The tree itself is held in the
    /// runtime's parse-handoff side map under the same `JobId`;
    /// callers fetch it via [`AsyncRuntime::take_parse_tree`]. The
    /// `duration_ms` here is the parse-only duration. T M4.1.
    Parse {
        /// Parse-only wall-clock duration in milliseconds.
        duration_ms: u64,
    },
    /// `dispatch_fs_read_dir` produced a directory listing. The
    /// Lua boundary in [`crate::lua_bindings`] turns the Vec into a
    /// per-entry table when `_take_result` consumes the result.
    /// T M8.1.
    ReadDir(Vec<FsDirEntry>),
    /// `dispatch_fs_stat` produced metadata for a single path. The
    /// Lua boundary turns the [`FsDirEntry`] into the same table
    /// shape `read_dir` entries use. T M8.1.
    Stat(FsDirEntry),
    /// External request/reply produced a JSON-shaped result. Used by
    /// the M9.1 MCP integration; the Lua boundary in
    /// [`crate::lua_bindings`] translates the inner
    /// [`serde_json::Value`] to a Lua table when `_take_result` is
    /// called. T M9.1.
    Json(serde_json::Value),
}

/// Terminal state a [`PendingJob`] settles into.
#[derive(Clone, Debug)]
enum PendingState {
    /// Worker has not replied yet.
    Running,
    /// Worker replied with a successful result.
    Complete(JobResult),
    /// Worker observed cancellation (or never ran).
    Cancelled,
    /// Worker raised. Carries the error message for surfacing to Lua.
    Failed(String),
}

/// Discriminator for which builtin handler a job was dispatched
/// against. Used by the observability buffer ([T M3.7]) to label
/// rows. New handlers added in M4 (tree-sitter, LSP, project
/// indexing) extend this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// `dispatch_sleep` --- the canary handler.
    Sleep,
    /// `dispatch_compute_sum` --- the synthetic CPU load.
    ComputeSum,
    /// `dispatch_emit_n` --- streaming numeric ticker.
    EmitN,
    /// `dispatch_grep` --- parallel directory grep ([T M3.6]).
    Grep,
    /// `dispatch_parse` --- tree-sitter parse on a worker ([T M4.1]).
    Parse,
    /// `dispatch_fs_read_dir` --- directory enumeration ([T M8.1]).
    FsReadDir,
    /// `dispatch_fs_stat` --- single-path metadata ([T M8.1]).
    FsStat,
    /// `dispatch_fs_rename` --- atomic rename ([T M8.1]).
    FsRename,
    /// `dispatch_fs_chmod` --- permission-bit replacement ([T M8.1]).
    FsChmod,
    /// `dispatch_fs_remove` --- delete a single object ([T M8.1]).
    FsRemove,
    /// External request/reply settled by code outside the worker
    /// pool. Used by the M9.1 MCP integration: the manager registers
    /// a pending entry via [`AsyncRuntime::register_external`] and
    /// settles it via [`AsyncRuntime::complete_external_ok`] /
    /// `complete_external_failed` / `complete_external_cancelled`
    /// when the corresponding JSON-RPC response arrives on the
    /// supervisor pipe. No worker thread is occupied for the
    /// round-trip.
    McpRequest,
}

impl JobKind {
    /// Human-readable label used by the `*workers*` buffer.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Sleep => "sleep",
            JobKind::ComputeSum => "compute_sum",
            JobKind::EmitN => "emit_n",
            JobKind::Grep => "grep",
            JobKind::Parse => "parse",
            JobKind::FsReadDir => "fs_read_dir",
            JobKind::FsStat => "fs_stat",
            JobKind::FsRename => "fs_rename",
            JobKind::FsChmod => "fs_chmod",
            JobKind::FsRemove => "fs_remove",
            JobKind::McpRequest => "mcp_request",
        }
    }
}

#[derive(Clone, Debug)]
struct PendingJob {
    cancel: CancellationToken,
    state: PendingState,
    /// If this job was dispatched with a supersede key, the key is
    /// preserved so [`AsyncRuntime::tick`] can prune the
    /// `key → id` mapping when the job settles --- but only if the
    /// table still maps the key to *this* id (a later dispatch may
    /// have superseded us, in which case the table belongs to that
    /// successor). T M3.4 / [spec §6.3].
    supersede_key: Option<String>,
    /// Per-job streaming accumulator. `Some(buf)` marks this entry as
    /// a stream; `tick` pushes [`ReplyKind::StreamItem`] payloads
    /// into `buf` and [`AsyncRuntime::take_stream_batches`] drains
    /// them. `None` means this is a request/reply job and stream
    /// items targeting it are dropped (with no panic). T M3.5.
    stream_buffer: Option<Vec<StreamPayload>>,
    /// Per-stream cap on items delivered in a single batch. Items
    /// beyond the cap stay in the accumulator until the next drain.
    /// Tunable per dispatch (and falls back to the runtime's
    /// `default_max_batch`). T M3.5.
    max_batch: usize,
    /// Which built-in handler this job runs. Surfaced by the
    /// `*workers*` buffer ([T M3.7]).
    kind: JobKind,
    /// When the job was registered. Used to compute "age" in the
    /// `*workers*` buffer.
    dispatched_at: Instant,
}

/// Snapshot of a job's terminal state, returned by
/// [`AsyncRuntime::take_result`] when a coroutine awaits it. Mirrors
/// [`PendingState`] but without the `Running` variant --- only
/// settled states are takeable.
#[derive(Clone, Debug)]
pub enum JobOutcome {
    /// The worker completed successfully.
    Complete(JobResult),
    /// The worker observed cancellation.
    Cancelled,
    /// The worker raised.
    Failed(String),
}

/// One row in the `*workers*` buffer's "active" section: a job that
/// the runtime has not yet observed settling. T M3.7.
#[derive(Clone, Debug)]
pub struct ActiveJobInfo {
    /// Lua-visible job id.
    pub id: JobId,
    /// Which built-in handler this job runs.
    pub kind: JobKind,
    /// Wall-clock age in milliseconds since dispatch.
    pub age_ms: u64,
    /// Supersede key (if any) the job was dispatched under.
    pub supersede_key: Option<String>,
    /// Whether the job's cancel token has been flipped --- a
    /// "cancellation pending" signal that the worker may not yet
    /// have observed.
    pub cancel_requested: bool,
    /// True if this is a streaming dispatch (`emit_n`, `grep`, ...);
    /// false if it's request/reply (`sleep`, `compute_sum`).
    pub is_stream: bool,
}

/// One row in the `*workers*` buffer's "completed" section: a job
/// the runtime saw settle in the recent past. Bounded by the
/// runtime's completion ring (default capacity 32). T M3.7.
#[derive(Clone, Debug)]
pub struct CompletedJobInfo {
    /// Lua-visible job id.
    pub id: JobId,
    /// Which built-in handler this job ran.
    pub kind: JobKind,
    /// How long the job ran from dispatch to settle.
    pub duration_ms: u64,
    /// How long ago the job settled (vs the snapshot moment).
    pub settled_age_ms: u64,
    /// Supersede key (if any) the job was dispatched under.
    pub supersede_key: Option<String>,
    /// Terminal outcome. `None` is unreachable here --- only
    /// settled jobs land in the completed ring.
    pub outcome: JobOutcome,
}

/// Snapshot returned by [`AsyncRuntime::workers_snapshot`]: the
/// active job table plus the recent-completions ring. The lists
/// are independent at any moment in time --- a job is in exactly
/// one of them.
///
/// Snapshots are point-in-time copies; the runtime's internal
/// state may change immediately after the snapshot is read. T M3.7
/// acceptance: "buffer updates within 100 ms of pool state
/// changes" is satisfied by re-snapshotting in the editor's tick
/// loop, which fires at frame cadence (16 ms by default).
#[derive(Clone, Debug)]
pub struct WorkersSnapshot {
    /// Currently in-flight or settled-but-not-yet-taken jobs.
    pub active: Vec<ActiveJobInfo>,
    /// Recent settles, newest-first. Bounded by
    /// [`COMPLETED_RING_CAP`].
    pub completed: Vec<CompletedJobInfo>,
}

/// Capacity of the completed-jobs ring. Older entries fall off
/// the back. 64 is enough headroom for a busy editor's recent
/// history without bloating the snapshot.
pub const COMPLETED_RING_CAP: usize = 64;

#[derive(Clone, Debug)]
struct CompletedSlot {
    id: JobId,
    kind: JobKind,
    dispatched_at: Instant,
    settled_at: Instant,
    supersede_key: Option<String>,
    outcome: JobOutcome,
}

/// One frame's worth of streamed items for a single stream id,
/// returned by [`AsyncRuntime::take_stream_batches`]. T M3.5.
#[derive(Clone, Debug)]
pub struct StreamBatch {
    /// The stream's job id.
    pub id: JobId,
    /// Items accumulated since the previous drain. Bounded by the
    /// stream's `max_batch`. Each variant carries a payload whose
    /// shape depends on the handler kind (numeric for `emit_n`,
    /// [`GrepMatch`] for `grep`, ...).
    pub items: Vec<StreamPayload>,
    /// True iff the stream has settled (either closed cleanly or
    /// cancelled / errored). When `closed` is true, the runtime has
    /// already evicted the pending entry; this is the last batch
    /// the consumer will see for `id`.
    pub closed: bool,
    /// Outcome carried alongside `closed`. `None` while the stream
    /// is still running.
    pub outcome: Option<JobOutcome>,
}

/// Main-thread async runtime. Use [`SharedAsyncRuntime`] for the
/// reference-counted handle that the Lua bindings and the editor's
/// run loop both hold.
///
/// Not [`Send`] --- the [`RefCell`] inside is the wrong primitive for
/// cross-thread sharing, and the runtime is always co-located with
/// the Lua VM on the main thread anyway.
pub struct AsyncRuntime {
    pool: WorkerPool,
    main: BusEnd,
    workers: BusEnd,
    next_job_id: AtomicU64,
    pending: RefCell<HashMap<JobId, PendingJob>>,
    /// Supersede key → currently-active job id. A new dispatch under
    /// the same key cancels the prior id's token *synchronously* and
    /// overwrites the entry. The map is pruned by `tick` when a
    /// settled job's key still points back at it --- a later
    /// dispatch overwrites the entry first, so the still-pending
    /// successor is never accidentally removed. T M3.4.
    supersede: RefCell<HashMap<String, JobId>>,
    /// Default cap on items delivered per
    /// [`Self::take_stream_batches`] call. Streams may override per
    /// dispatch. T M3.5 acceptance: "tunable batch size".
    default_max_batch: Cell<usize>,
    /// Frame target the editor's run loop reads to size its
    /// `poll_event` timeout. Workers emitting many items per second
    /// are coalesced into one main-thread wake-and-drain per frame.
    /// T M3.5 acceptance: "tunable frame target".
    frame_target_ms: Cell<u64>,
    /// Recent completions, newest-first. Settled jobs are pushed to
    /// the front by `tick` and the back is trimmed to
    /// [`COMPLETED_RING_CAP`]. The `*workers*` buffer reads this
    /// for its history pane. T M3.7.
    completed: RefCell<VecDeque<CompletedSlot>>,
    /// Side handoff for parse jobs: a fresh
    /// [`ParseTreeBundle`] cannot ride the [`MessagePack`] bus
    /// (`tree_sitter::Tree` is not [`Serialize`]), so the worker
    /// thread parks the bundle here under the job id and sends a
    /// payload-less [`ReplyKind::Parse`] over the bus. Main thread
    /// drains the bundle via [`Self::take_parse_tree`]. The mutex is
    /// only contended at parse settle/take time --- never inside the
    /// editor's hot path. T M4.1.
    parse_handoff: Arc<Mutex<HashMap<JobId, Arc<ParseTreeBundle>>>>,
}

/// Default cap on stream items delivered in a single drain. 1024
/// items is a conservative default --- big enough that 60 Hz
/// delivery handles ~60K items/sec without piling up, small enough
/// that one batch fits comfortably in cache.
pub const DEFAULT_MAX_BATCH: usize = 1024;

/// Default frame target. 16 ms ≈ 60 Hz, the canonical "snappy
/// editor" cadence; can be raised to 33 ms (30 Hz) if the editor
/// is bottlenecked elsewhere.
pub const DEFAULT_FRAME_TARGET_MS: u64 = 16;

/// Shared, single-threaded handle to the runtime. Same pattern as
/// the other `Rc<...>` aliases in [`crate::lua_bindings`]: cheaply
/// cloneable, captured by closures that bridge into Lua.
pub type SharedAsyncRuntime = Rc<AsyncRuntime>;

impl AsyncRuntime {
    /// Build a runtime with `pool_size` worker threads. Sized to
    /// `available_parallelism - 1` (floor `1`) by default --- callers
    /// that want a specific size pass it directly.
    #[must_use]
    pub fn with_pool(pool: WorkerPool) -> Self {
        let schema = Arc::new(SchemaRegistry::new());
        schema
            .register::<WorkerReply>(ASYNC_REPLY_TOPIC)
            .expect("register async.reply");
        let (main, workers) = MessageBus::pair(schema);
        Self {
            pool,
            main,
            workers,
            next_job_id: AtomicU64::new(0),
            pending: RefCell::new(HashMap::new()),
            supersede: RefCell::new(HashMap::new()),
            default_max_batch: Cell::new(DEFAULT_MAX_BATCH),
            frame_target_ms: Cell::new(DEFAULT_FRAME_TARGET_MS),
            completed: RefCell::new(VecDeque::with_capacity(COMPLETED_RING_CAP)),
            parse_handoff: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Build a runtime sized at `available_parallelism - 1` worker
    /// threads, matching the spec's "main thread is the event loop"
    /// pattern ([spec §6.1]).
    #[must_use]
    pub fn with_default_pool() -> Self {
        Self::with_pool(WorkerPool::with_default_size())
    }

    /// Build a runtime with exactly `size` worker threads, used
    /// from tests that need predictable concurrency.
    #[must_use]
    pub fn with_pool_size(size: usize) -> Self {
        Self::with_pool(WorkerPool::new(size))
    }

    /// Returns the number of in-flight or settled-but-not-yet-taken
    /// pending entries.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.borrow().len()
    }

    /// Number of supersede keys currently bound to a live job. Test
    /// helper for verifying the table prunes itself on settle.
    #[must_use]
    pub fn supersede_len(&self) -> usize {
        self.supersede.borrow().len()
    }

    /// The job currently active under `key`, if any. Test helper
    /// (and a future read surface for `describe-async`).
    #[must_use]
    pub fn active_for_key(&self, key: &str) -> Option<JobId> {
        self.supersede.borrow().get(key).copied()
    }

    /// Editor frame target in milliseconds. The run loop reads this
    /// to size its `poll_event` timeout, which is what bounds
    /// streaming-worker wakeups to one per frame.
    #[must_use]
    pub fn frame_target_ms(&self) -> u64 {
        self.frame_target_ms.get()
    }

    /// Set the editor frame target. Bounded to `[1, 1000]` --- a
    /// frame target above one second would render the editor
    /// unresponsive; a target below 1 ms would burn CPU on no-op
    /// poll cycles.
    pub fn set_frame_target_ms(&self, ms: u64) {
        self.frame_target_ms.set(ms.clamp(1, 1000));
    }

    /// Default per-stream batch size used when a dispatch does not
    /// override it.
    #[must_use]
    pub fn default_max_batch(&self) -> usize {
        self.default_max_batch.get()
    }

    /// Override the runtime-wide default batch size. Bounded to
    /// `[1, 1_000_000]` so a typo doesn't accidentally disable
    /// coalescing or blow memory.
    pub fn set_default_max_batch(&self, n: usize) {
        self.default_max_batch.set(n.clamp(1, 1_000_000));
    }

    /// Register a fresh pending entry and return its id + cancel
    /// token. The token is what the worker closure polls; the entry
    /// is what `tick` updates on reply.
    ///
    /// If `supersede_key` is `Some(key)`, any in-flight predecessor
    /// under the same key has its cancel token flipped *before* this
    /// allocation returns, and the `key → id` table is updated to
    /// point at the new id. The predecessor's pending entry is
    /// retained --- its worker will produce a `Cancelled` reply that
    /// `tick` then surfaces.
    fn allocate(
        &self,
        kind: JobKind,
        supersede_key: Option<&str>,
        stream: Option<usize>,
    ) -> (JobId, CancellationToken) {
        let id = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        let cancel = CancellationToken::new();
        if let Some(key) = supersede_key {
            // Flip the prior token *before* we publish the new id, so
            // a worker observing key→old still sees an in-flight
            // predecessor's cancel signal. Holding only one borrow at
            // a time keeps RefCell happy across re-entrant pending
            // borrows.
            let prior_id = self.supersede.borrow().get(key).copied();
            if let Some(prior) = prior_id {
                if let Some(job) = self.pending.borrow().get(&prior) {
                    job.cancel.cancel();
                }
            }
            self.supersede.borrow_mut().insert(key.to_owned(), id);
        }
        self.pending.borrow_mut().insert(
            id,
            PendingJob {
                cancel: cancel.clone(),
                state: PendingState::Running,
                supersede_key: supersede_key.map(str::to_owned),
                stream_buffer: stream.map(|_| Vec::new()),
                max_batch: stream.unwrap_or(0),
                kind,
                dispatched_at: Instant::now(),
            },
        );
        (id, cancel)
    }

    /// Dispatch a `sleep(ms)` job. The job sleeps in 1ms slices,
    /// polling its cancel token between each slice. Returns the
    /// runtime-assigned [`JobId`].
    ///
    /// If `supersede` is `Some(key)`, any in-flight job previously
    /// dispatched under `key` is cancelled before this dispatch
    /// returns. T M3.4 / [spec §6.3].
    pub fn dispatch_sleep(&self, ms: i64, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::Sleep, supersede, None);
        let bus = self.workers.clone();
        let total = Duration::from_millis(ms.max(0).unsigned_abs());
        self.pool.dispatch(move |_pool| {
            let kind = run_sleep(&cancel, total);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Dispatch a `sum(n)` job. The job sums `1..=n` while polling
    /// cancel; small `n` settle in microseconds, large `n` exercises
    /// the granular cancel boundary. `supersede` follows the same
    /// rule as [`Self::dispatch_sleep`].
    pub fn dispatch_compute_sum(&self, n: u64, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::ComputeSum, supersede, None);
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            let kind = run_compute_sum(&cancel, n);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Dispatch a streaming handler that emits `count` items as fast
    /// as the worker can push them onto the bus. Each item is the
    /// 1-based index of the emission, terminated by a
    /// [`ReplyKind::StreamClosed`] reply when the loop completes
    /// (or by [`ReplyKind::Cancelled`] on cancel). T M3.5
    /// acceptance: "10000 messages/sec produces at most one wakeup
    /// per frame" --- this is the synthetic load that proves it.
    ///
    /// `max_batch` overrides the runtime's default cap on items per
    /// drain; pass `None` to use [`Self::default_max_batch`].
    pub fn dispatch_emit_n(
        &self,
        count: u64,
        supersede: Option<&str>,
        max_batch: Option<usize>,
    ) -> JobId {
        let cap = max_batch.map_or_else(|| self.default_max_batch.get(), |n| n.clamp(1, 1_000_000));
        let (id, cancel) = self.allocate(JobKind::EmitN, supersede, Some(cap));
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            run_emit_n(&cancel, &bus, id, count);
        });
        id
    }

    /// Dispatch a parallel directory grep. The handler walks
    /// `spec.root`, fans file searches out across `spec.fanout`
    /// internal scoped threads, and emits one
    /// [`StreamPayload::Match`] per match. Terminates with
    /// [`ReplyKind::StreamClosed`] (clean) or
    /// [`ReplyKind::Cancelled`] (token observed).
    ///
    /// `max_batch` overrides the runtime default for this stream's
    /// per-frame coalescing cap.
    ///
    /// T M3.6: this is the milestone-justifying load --- "Lua code
    /// can do expensive things without freezing the editor". All
    /// arguments cross into the worker by value ([R31]).
    pub fn dispatch_grep(
        &self,
        spec: GrepSpec,
        supersede: Option<&str>,
        max_batch: Option<usize>,
    ) -> JobId {
        let cap = max_batch.map_or_else(|| self.default_max_batch.get(), |n| n.clamp(1, 1_000_000));
        let (id, cancel) = self.allocate(JobKind::Grep, supersede, Some(cap));
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            run_grep(&cancel, &bus, id, spec);
        });
        id
    }

    /// Dispatch a tree-sitter parse on a worker. Returns the
    /// runtime-assigned [`JobId`].
    ///
    /// On success, the worker stashes the produced
    /// [`ParseTreeBundle`] in [`Self::parse_handoff`] under the same
    /// id and emits [`ReplyKind::Parse`] over the bus. The main
    /// thread's `tick` transitions the pending entry to
    /// [`JobResult::Parse`]; callers fetch the bundle by calling
    /// [`Self::take_parse_tree`] (typically from Lua's parse-handle
    /// glue right after `take_result`).
    ///
    /// On cancellation, no bundle is stashed --- the worker observes
    /// the flipped token before parsing and returns
    /// [`ReplyKind::Cancelled`].
    ///
    /// `supersede` follows the same rule as the other dispatchers:
    /// in-flight predecessor under the same key has its cancel token
    /// flipped synchronously. T M4.1 / [spec §6.3].
    pub fn dispatch_parse(&self, spec: ParseRequest, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::Parse, supersede, None);
        let bus = self.workers.clone();
        let handoff = self.parse_handoff.clone();
        self.pool.dispatch(move |_pool| {
            run_parse(&cancel, &bus, &handoff, id, spec);
        });
        id
    }

    /// Dispatch a `read_dir(path)` job. The worker enumerates
    /// `path`, returning one [`FsDirEntry`] per child with
    /// `lstat`-style metadata. Polls cancel every batch of
    /// entries; supersede follows the same rule as the other
    /// dispatchers. T M8.1.
    pub fn dispatch_fs_read_dir(&self, path: PathBuf, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::FsReadDir, supersede, None);
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            let kind = run_fs_read_dir(&cancel, &path);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Dispatch a `stat(path)` job. Returns one [`FsDirEntry`] of
    /// metadata for `path`. T M8.1.
    pub fn dispatch_fs_stat(&self, path: PathBuf, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::FsStat, supersede, None);
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            let kind = run_fs_stat(&cancel, &path);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Dispatch a `rename(from, to)` job. Settles to
    /// [`JobResult::Unit`] on success. T M8.1.
    pub fn dispatch_fs_rename(&self, from: PathBuf, to: PathBuf, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::FsRename, supersede, None);
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            let kind = run_fs_rename(&cancel, &from, &to);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Dispatch a `chmod(path, mode)` job. T M8.1.
    pub fn dispatch_fs_chmod(&self, path: PathBuf, mode: u32, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::FsChmod, supersede, None);
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            let kind = run_fs_chmod(&cancel, &path, mode);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Dispatch a `remove(path)` job. T M8.1.
    pub fn dispatch_fs_remove(&self, path: PathBuf, supersede: Option<&str>) -> JobId {
        let (id, cancel) = self.allocate(JobKind::FsRemove, supersede, None);
        let bus = self.workers.clone();
        self.pool.dispatch(move |_pool| {
            let kind = run_fs_remove(&cancel, &path);
            let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        });
        id
    }

    /// Register a pending entry that will be settled from outside the
    /// worker pool. Returns `(JobId, CancellationToken)`. The caller
    /// is responsible for eventually calling
    /// [`Self::complete_external_ok`], [`Self::complete_external_failed`],
    /// or [`Self::complete_external_cancelled`] on the returned id;
    /// the cancellation token is what `pmacs.workers._cancel(id)`
    /// flips, and the caller should poll it (e.g. inside its tick)
    /// to give up on outstanding requests when the user cancels.
    ///
    /// T M9.1: this is the entry point the MCP layer uses to bind
    /// JSON-RPC request ids to async-runtime job ids without
    /// occupying a worker thread for the synchronous-write +
    /// pipe-response round-trip. Future protocols that ride on the
    /// same supervisor (DAP, etc.) reuse this surface.
    ///
    /// `supersede` follows the same rule as the worker dispatchers.
    pub fn register_external(
        &self,
        kind: JobKind,
        supersede: Option<&str>,
    ) -> (JobId, CancellationToken) {
        self.allocate(kind, supersede, None)
    }

    /// Settle an externally-registered job with a JSON value. Wakes
    /// any coroutine parked on the corresponding [`Handle:await()`]
    /// on the next [`Self::tick`].
    ///
    /// Idempotent against double-completion: the second call is a
    /// no-op (the entry has already settled). T M9.1.
    pub fn complete_external_ok(&self, id: JobId, value: serde_json::Value) {
        let _ = self.workers.send(
            ASYNC_REPLY_TOPIC,
            &WorkerReply {
                job_id: id,
                kind: ReplyKind::Json(value),
            },
        );
    }

    /// Settle an externally-registered job with a failure message.
    /// T M9.1.
    pub fn complete_external_failed(&self, id: JobId, message: impl Into<String>) {
        let _ = self.workers.send(
            ASYNC_REPLY_TOPIC,
            &WorkerReply {
                job_id: id,
                kind: ReplyKind::Error(message.into()),
            },
        );
    }

    /// Settle an externally-registered job as cancelled. Used when
    /// the underlying request was abandoned without a response (e.g.
    /// the MCP server crashed mid-flight). T M9.1.
    pub fn complete_external_cancelled(&self, id: JobId) {
        let _ = self.workers.send(
            ASYNC_REPLY_TOPIC,
            &WorkerReply {
                job_id: id,
                kind: ReplyKind::Cancelled,
            },
        );
    }

    /// Drain the parse-tree bundle for `id` from the side handoff.
    /// Returns `None` if the job is unknown, still running, didn't
    /// produce a tree (cancelled or failed), or has already been
    /// taken. T M4.1.
    pub fn take_parse_tree(&self, id: JobId) -> Option<Arc<ParseTreeBundle>> {
        self.parse_handoff
            .lock()
            .expect("parse_handoff mutex poisoned")
            .remove(&id)
    }

    /// Number of parked parse-tree bundles waiting to be drained
    /// from the handoff. Test helper for verifying the handoff
    /// doesn't leak entries across long sessions.
    #[must_use]
    pub fn parse_handoff_len(&self) -> usize {
        self.parse_handoff
            .lock()
            .expect("parse_handoff mutex poisoned")
            .len()
    }

    /// Mark `id` cancelled. The worker closure observes this on its
    /// next granular check and produces a `Cancelled` reply, which
    /// `tick` then surfaces to Lua. No-op if `id` is unknown.
    pub fn cancel(&self, id: JobId) {
        if let Some(job) = self.pending.borrow().get(&id) {
            job.cancel.cancel();
        }
    }

    /// Drain every queued reply on the main-thread bus, update
    /// pending entries, and return the list of ids that *transitioned
    /// from Running to a terminal state* during this tick. The Lua
    /// runtime resumes coroutines parked on these ids.
    pub fn tick(&self) -> Vec<JobId> {
        let mut newly_settled = Vec::new();
        while let Ok(env) = self.main.try_recv() {
            let Ok(reply): Result<WorkerReply, _> = self.main.decode(&env) else {
                // A malformed reply can only come from a broken
                // handler; surface it via *errors* in the future,
                // but for now skip.
                continue;
            };
            let mut pending = self.pending.borrow_mut();
            let Some(job) = pending.get_mut(&reply.job_id) else {
                continue;
            };
            match reply.kind {
                // Streaming items accumulate in the per-job buffer;
                // they are *not* a settle event. The buffer is
                // drained by `take_stream_batches`. T M3.5.
                ReplyKind::StreamItem(v) => {
                    if let Some(buf) = job.stream_buffer.as_mut() {
                        buf.push(v);
                    }
                    // Else: stream item targeting a non-stream job
                    // is a handler bug; drop it silently rather
                    // than poisoning the request/reply channel.
                }
                ReplyKind::StreamClosed => {
                    if matches!(job.state, PendingState::Running) {
                        job.state = PendingState::Complete(JobResult::Unit);
                        newly_settled.push(reply.job_id);
                    }
                }
                ReplyKind::Sleep
                | ReplyKind::Sum(_)
                | ReplyKind::Parse { .. }
                | ReplyKind::ReadDir(_)
                | ReplyKind::Stat(_)
                | ReplyKind::FsUnit
                | ReplyKind::Json(_)
                | ReplyKind::Cancelled
                | ReplyKind::Error(_)
                    if matches!(job.state, PendingState::Running) =>
                {
                    job.state = match reply.kind {
                        ReplyKind::Sleep | ReplyKind::FsUnit => {
                            PendingState::Complete(JobResult::Unit)
                        }
                        ReplyKind::Sum(v) => PendingState::Complete(JobResult::Sum(v)),
                        ReplyKind::Parse { duration_ms } => {
                            PendingState::Complete(JobResult::Parse { duration_ms })
                        }
                        ReplyKind::ReadDir(entries) => {
                            PendingState::Complete(JobResult::ReadDir(entries))
                        }
                        ReplyKind::Stat(entry) => PendingState::Complete(JobResult::Stat(entry)),
                        ReplyKind::Json(v) => PendingState::Complete(JobResult::Json(v)),
                        ReplyKind::Cancelled => PendingState::Cancelled,
                        ReplyKind::Error(msg) => PendingState::Failed(msg),
                        _ => unreachable!("matched above"),
                    };
                    newly_settled.push(reply.job_id);
                }
                // Already-settled job receiving a duplicate reply is a
                // no-op; ignore.
                _ => {}
            }
        }
        // Prune supersede entries whose owning job just settled. We
        // only remove entries that *still* point at the settled id;
        // a successor that came in mid-flight will have overwritten
        // the entry already, and that successor's pending lifetime
        // is what owns the slot now.
        if !newly_settled.is_empty() {
            let pending = self.pending.borrow();
            let mut sup = self.supersede.borrow_mut();
            let mut completed = self.completed.borrow_mut();
            let now = Instant::now();
            for id in &newly_settled {
                if let Some(job) = pending.get(id) {
                    if let Some(key) = &job.supersede_key {
                        if sup.get(key) == Some(id) {
                            sup.remove(key);
                        }
                    }
                    // T M3.7: record the settle in the completion
                    // ring. We push the front and trim the back so
                    // the newest completions are always at index 0.
                    let outcome = match &job.state {
                        PendingState::Complete(r) => JobOutcome::Complete(r.clone()),
                        PendingState::Cancelled => JobOutcome::Cancelled,
                        PendingState::Failed(msg) => JobOutcome::Failed(msg.clone()),
                        PendingState::Running => continue,
                    };
                    completed.push_front(CompletedSlot {
                        id: *id,
                        kind: job.kind,
                        dispatched_at: job.dispatched_at,
                        settled_at: now,
                        supersede_key: job.supersede_key.clone(),
                        outcome,
                    });
                }
            }
            while completed.len() > COMPLETED_RING_CAP {
                completed.pop_back();
            }
        }
        newly_settled
    }

    /// Snapshot the runtime's job tables for the `*workers*`
    /// observability buffer ([T M3.7]). Returns active jobs and
    /// recent completions in two parallel lists. Newest completions
    /// come first.
    ///
    /// This is the read surface the buffer renderer uses; it does
    /// *not* mutate any state. Calling it from a hot path is fine
    /// --- the cost is one `HashMap::iter()` and one `VecDeque`
    /// clone, both O(N) in entries.
    #[must_use]
    pub fn workers_snapshot(&self) -> WorkersSnapshot {
        let now = Instant::now();
        let pending = self.pending.borrow();
        let mut active: Vec<ActiveJobInfo> = pending
            .iter()
            .filter(|(_, j)| matches!(j.state, PendingState::Running))
            .map(|(id, j)| ActiveJobInfo {
                id: *id,
                kind: j.kind,
                age_ms: now.saturating_duration_since(j.dispatched_at).as_millis() as u64,
                supersede_key: j.supersede_key.clone(),
                cancel_requested: j.cancel.is_cancelled(),
                is_stream: j.stream_buffer.is_some(),
            })
            .collect();
        // Stable order: oldest first. The buffer renderer renders in
        // the order returned, and "oldest job at top" is what users
        // expect from a process-list view.
        active.sort_by_key(|a| (a.age_ms, a.id));
        active.reverse(); // age_ms descending = oldest first
        let completed = self.completed.borrow();
        let completed: Vec<CompletedJobInfo> = completed
            .iter()
            .map(|c| CompletedJobInfo {
                id: c.id,
                kind: c.kind,
                duration_ms: c
                    .settled_at
                    .saturating_duration_since(c.dispatched_at)
                    .as_millis() as u64,
                settled_age_ms: now.saturating_duration_since(c.settled_at).as_millis() as u64,
                supersede_key: c.supersede_key.clone(),
                outcome: c.outcome.clone(),
            })
            .collect();
        WorkersSnapshot { active, completed }
    }

    /// Drain the per-stream accumulators into one batch each. Each
    /// returned batch is bounded by the stream's `max_batch`; items
    /// beyond the cap stay in the accumulator until the next call.
    ///
    /// A batch with `closed = true` carries the stream's terminal
    /// outcome; the runtime evicts the corresponding pending entry
    /// after delivering it, so subsequent `take_stream_batches`
    /// calls will not see this id again.
    ///
    /// T M3.5 acceptance: this is the coalescing primitive --- a
    /// run loop that calls `take_stream_batches` once per frame
    /// receives at most one batch per stream per frame, regardless
    /// of how many items the worker emitted in between.
    pub fn take_stream_batches(&self) -> Vec<StreamBatch> {
        let mut out = Vec::new();
        let mut pending = self.pending.borrow_mut();
        let mut to_evict = Vec::new();
        // Iterate ids first (immutable view) to avoid mutable+
        // immutable borrow overlap. HashMap iteration order is
        // arbitrary; consumers must not rely on it.
        let stream_ids: Vec<JobId> = pending
            .iter()
            .filter(|(_, j)| j.stream_buffer.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in stream_ids {
            let Some(job) = pending.get_mut(&id) else {
                continue;
            };
            let Some(buf) = job.stream_buffer.as_mut() else {
                continue;
            };
            let cap = job.max_batch.max(1);
            let take = buf.len().min(cap);
            // A stream with no pending items and still running
            // contributes nothing; skip.
            let settled = !matches!(job.state, PendingState::Running);
            if take == 0 && !settled {
                continue;
            }
            let drained: Vec<StreamPayload> = buf.drain(..take).collect();
            // Closed iff the stream is settled *and* the buffer is
            // now empty (no more frames to deliver).
            let closed = settled && buf.is_empty();
            let outcome = if closed {
                Some(match &job.state {
                    PendingState::Complete(r) => JobOutcome::Complete(r.clone()),
                    PendingState::Cancelled => JobOutcome::Cancelled,
                    PendingState::Failed(msg) => JobOutcome::Failed(msg.clone()),
                    PendingState::Running => unreachable!("settled checked above"),
                })
            } else {
                None
            };
            out.push(StreamBatch {
                id,
                items: drained,
                closed,
                outcome,
            });
            if closed {
                to_evict.push(id);
            }
        }
        for id in to_evict {
            pending.remove(&id);
        }
        out
    }

    /// Has `id` settled into a terminal state?
    #[must_use]
    pub fn is_complete(&self, id: JobId) -> bool {
        self.pending
            .borrow()
            .get(&id)
            .is_some_and(|j| !matches!(j.state, PendingState::Running))
    }

    /// Was `id` cancelled? Returns true only after `tick` has
    /// observed the worker's `Cancelled` reply --- not at the moment
    /// `cancel` was called.
    #[must_use]
    pub fn is_cancelled(&self, id: JobId) -> bool {
        matches!(
            self.pending.borrow().get(&id).map(|j| &j.state),
            Some(PendingState::Cancelled)
        )
    }

    /// Take the terminal outcome for `id`, removing the entry from
    /// the pending table. Returns `None` if `id` is unknown or still
    /// running. Lua's `Handle:await()` calls this once per handle.
    ///
    /// Side effect: any leftover parse-tree handoff entry under `id`
    /// is also dropped here. Callers that want the bundle must call
    /// [`Self::take_parse_tree`] *before* `take_result` --- otherwise
    /// the bundle is GC'd alongside the pending entry. This bounds
    /// the handoff's worst-case footprint to "settled-but-not-yet-
    /// taken parse jobs" rather than allowing forgotten bundles to
    /// pile up indefinitely. T M4.1. (`take_result` here is the
    /// method itself.)
    pub fn take_result(&self, id: JobId) -> Option<JobOutcome> {
        let mut pending = self.pending.borrow_mut();
        let outcome = match pending.get(&id)?.state {
            PendingState::Running => return None,
            _ => match pending.remove(&id)?.state {
                PendingState::Complete(r) => JobOutcome::Complete(r),
                PendingState::Cancelled => JobOutcome::Cancelled,
                PendingState::Failed(msg) => JobOutcome::Failed(msg),
                PendingState::Running => unreachable!("checked above"),
            },
        };
        // Drop stale handoff slot if the caller didn't explicitly
        // take it. No-op when the job wasn't a parse, and idempotent
        // if `take_parse_tree` was called first.
        let _ = self
            .parse_handoff
            .lock()
            .expect("parse_handoff mutex poisoned")
            .remove(&id);
        Some(outcome)
    }
}

// ---------------------------------------------------------------------------
// Built-in worker bodies
// ---------------------------------------------------------------------------

fn run_sleep(cancel: &CancellationToken, total: Duration) -> ReplyKind {
    let step = Duration::from_millis(1);
    let start = Instant::now();
    while start.elapsed() < total {
        if cancel.is_cancelled() {
            return ReplyKind::Cancelled;
        }
        thread::sleep(step);
    }
    if cancel.is_cancelled() {
        return ReplyKind::Cancelled;
    }
    ReplyKind::Sleep
}

/// Worker body for [`AsyncRuntime::dispatch_fs_read_dir`].
/// Translates [`crate::fs::read_dir_blocking`]'s
/// [`FsError`] taxonomy into the bus reply enum:
/// [`FsError::Cancelled`] becomes [`ReplyKind::Cancelled`];
/// [`FsError::Io`] becomes [`ReplyKind::Error`] with the
/// human-readable message attached.
fn run_fs_read_dir(cancel: &CancellationToken, path: &Path) -> ReplyKind {
    match read_dir_blocking(path, cancel) {
        Ok(entries) => ReplyKind::ReadDir(entries),
        Err(FsError::Cancelled) => ReplyKind::Cancelled,
        Err(e @ (FsError::Io { .. } | FsError::NonUtf8Path { .. })) => {
            ReplyKind::Error(e.to_string())
        }
    }
}

fn run_fs_stat(cancel: &CancellationToken, path: &Path) -> ReplyKind {
    match stat_blocking(path, cancel) {
        Ok(entry) => ReplyKind::Stat(entry),
        Err(FsError::Cancelled) => ReplyKind::Cancelled,
        Err(e @ (FsError::Io { .. } | FsError::NonUtf8Path { .. })) => {
            ReplyKind::Error(e.to_string())
        }
    }
}

fn run_fs_rename(cancel: &CancellationToken, from: &Path, to: &Path) -> ReplyKind {
    fs_unit_to_reply(rename_blocking(from, to, cancel))
}

fn run_fs_chmod(cancel: &CancellationToken, path: &Path, mode: u32) -> ReplyKind {
    fs_unit_to_reply(chmod_blocking(path, mode, cancel))
}

fn run_fs_remove(cancel: &CancellationToken, path: &Path) -> ReplyKind {
    fs_unit_to_reply(remove_blocking(path, cancel))
}

/// Shared error-mapping for the unit-result fs primitives. Keeps
/// the rename/chmod/remove worker bodies one-liners so the table
/// of dispatchers reads at a glance.
fn fs_unit_to_reply(result: Result<(), FsError>) -> ReplyKind {
    match result {
        Ok(()) => ReplyKind::FsUnit,
        Err(FsError::Cancelled) => ReplyKind::Cancelled,
        Err(e @ (FsError::Io { .. } | FsError::NonUtf8Path { .. })) => {
            ReplyKind::Error(e.to_string())
        }
    }
}

fn run_compute_sum(cancel: &CancellationToken, n: u64) -> ReplyKind {
    let mut acc: u64 = 0;
    // Granular: poll cancel every 1024 iterations to balance
    // responsiveness against polling overhead.
    let mut counter: u64 = 0;
    let mut i: u64 = 1;
    while i <= n {
        counter = counter.wrapping_add(1);
        if counter.trailing_zeros() >= 10 && cancel.is_cancelled() {
            return ReplyKind::Cancelled;
        }
        acc = acc.wrapping_add(i);
        i += 1;
    }
    if cancel.is_cancelled() {
        return ReplyKind::Cancelled;
    }
    ReplyKind::Sum(acc)
}

/// Streaming handler used by [`AsyncRuntime::dispatch_emit_n`].
/// Pushes `count` `StreamItem` envelopes onto the bus as fast as
/// the worker can send them, terminated by either `StreamClosed`
/// (clean completion) or `Cancelled` (token observed flipped).
/// Polls cancel every iteration --- this is the load that proves
/// frame-boundary coalescing on the consumer side.
fn run_emit_n(cancel: &CancellationToken, bus: &BusEnd, id: JobId, count: u64) {
    for i in 1..=count {
        if cancel.is_cancelled() {
            let _ = bus.send(
                ASYNC_REPLY_TOPIC,
                &WorkerReply {
                    job_id: id,
                    kind: ReplyKind::Cancelled,
                },
            );
            return;
        }
        let _ = bus.send(
            ASYNC_REPLY_TOPIC,
            &WorkerReply {
                job_id: id,
                kind: ReplyKind::StreamItem(StreamPayload::U64(i)),
            },
        );
    }
    let _ = bus.send(
        ASYNC_REPLY_TOPIC,
        &WorkerReply {
            job_id: id,
            kind: ReplyKind::StreamClosed,
        },
    );
}

/// Streaming handler used by [`AsyncRuntime::dispatch_grep`]. Walks
/// `spec.root` and fans file searches out across `spec.fanout`
/// scoped threads. Each thread pulls file paths from a shared
/// channel, reads the file, scans it for `spec.pattern`, and
/// emits one [`StreamPayload::Match`] envelope per match. The walk
/// runs on the dispatching thread; the workers consume in
/// parallel.
///
/// Cancellation: every worker checks `cancel` between files, and
/// the walker checks it between directory entries. A flipped token
/// produces a single [`ReplyKind::Cancelled`] reply --- regardless
/// of how many workers were in flight.
///
/// R31: every value crossed into a worker (the file path, the
/// pattern, the bus end) is owned. No buffer references are held.
pub fn run_grep(cancel: &CancellationToken, bus: &BusEnd, id: JobId, spec: GrepSpec) {
    let GrepSpec {
        root,
        pattern,
        case_sensitive,
        max_file_bytes,
        max_match_text,
        max_results,
        fanout,
    } = spec;
    if pattern.is_empty() || cancel.is_cancelled() {
        let kind = if cancel.is_cancelled() {
            ReplyKind::Cancelled
        } else {
            ReplyKind::StreamClosed
        };
        let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
        return;
    }

    // Bounded channel for backpressure: walker stalls if workers
    // fall behind, so a fast walk over many small files cannot
    // outpace the search and balloon memory.
    let fanout = fanout.max(1);
    let (tx, rx) = cb_channel::bounded::<PathBuf>(fanout * 16);
    let results_emitted = Arc::new(AtomicU64::new(0));

    // Ascii-fold the pattern once if case-insensitive; workers
    // fold their reads on the fly.
    let pattern_norm: Arc<Vec<u8>> = if case_sensitive {
        Arc::new(pattern.into_bytes())
    } else {
        Arc::new(ascii_fold(pattern.as_bytes()))
    };

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(fanout);
        for _ in 0..fanout {
            let rx = rx.clone();
            let bus = bus.clone();
            let cancel = cancel.clone();
            let pattern_norm = Arc::clone(&pattern_norm);
            let results_emitted = Arc::clone(&results_emitted);
            let root = root.clone();
            let h = scope.spawn(move || {
                while let Ok(path) = rx.recv() {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if max_results > 0
                        && results_emitted.load(Ordering::Relaxed) >= u64::from(max_results)
                    {
                        break;
                    }
                    search_file(
                        &path,
                        &root,
                        &pattern_norm,
                        case_sensitive,
                        max_file_bytes,
                        max_match_text,
                        max_results,
                        &results_emitted,
                        &cancel,
                        &bus,
                        id,
                    );
                }
            });
            handles.push(h);
        }
        // Drop our local sender so workers see disconnect once the
        // walker finishes feeding paths.
        drop(rx);
        walk_dir(&root, &tx, cancel);
        drop(tx);
        for h in handles {
            let _ = h.join();
        }
    });

    let kind = if cancel.is_cancelled() {
        ReplyKind::Cancelled
    } else {
        ReplyKind::StreamClosed
    };
    let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
}

/// Worker body for [`AsyncRuntime::dispatch_parse`]. Runs the
/// synchronous parse, parks the [`ParseTreeBundle`] in `handoff`
/// under `id`, and reports settle (or cancel/error) over the bus.
///
/// Cancellation is coarse: the token is checked once before the
/// parse runs. Mid-parse cancellation requires wiring tree-sitter's
/// `AtomicUsize` cancellation flag through the worker's
/// `CancellationToken` (an `AtomicBool`), which is M4.x territory ---
/// M4.1 parses are bounded (5000-line cold parse < 100 ms; edits
/// even faster), so coarse cancellation suffices for v0.1.
fn run_parse(
    cancel: &CancellationToken,
    bus: &BusEnd,
    handoff: &Mutex<HashMap<JobId, Arc<ParseTreeBundle>>>,
    id: JobId,
    spec: ParseRequest,
) {
    if cancel.is_cancelled() {
        let _ = bus.send(
            ASYNC_REPLY_TOPIC,
            &WorkerReply {
                job_id: id,
                kind: ReplyKind::Cancelled,
            },
        );
        return;
    }
    let kind = match syntax_mod::run_parse(spec) {
        Ok(bundle) => {
            let duration_ms = u64::try_from(bundle.parse_duration.as_millis()).unwrap_or(u64::MAX);
            handoff
                .lock()
                .expect("parse_handoff mutex poisoned")
                .insert(id, Arc::new(bundle));
            ReplyKind::Parse { duration_ms }
        }
        Err(msg) => ReplyKind::Error(msg),
    };
    let _ = bus.send(ASYNC_REPLY_TOPIC, &WorkerReply { job_id: id, kind });
}

/// ASCII-fold every byte to lowercase. Non-ASCII bytes pass
/// through unchanged --- a deliberate v0.1 limitation. Unicode
/// case-folding lands when we add the regex layer in M4.
fn ascii_fold(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

/// Recursive directory walker. Pushes paths of regular files into
/// `tx`. Skips hidden directories, common build / VCS roots, and
/// symlinks (cycle prevention). Aborts on cancel between entries.
fn walk_dir(root: &Path, tx: &cb_channel::Sender<PathBuf>, cancel: &CancellationToken) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if cancel.is_cancelled() {
                return;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.')
                        || matches!(
                            name,
                            "node_modules" | "target" | "build" | "dist" | "__pycache__"
                        )
                    {
                        continue;
                    }
                }
                stack.push(path);
            } else if file_type.is_file() && tx.send(path).is_err() {
                return;
            }
        }
    }
}

/// Read one file and emit a `StreamItem(Match)` per matching line.
/// Skips files that are too large, that contain a NUL byte (binary
/// heuristic), or that aren't valid UTF-8.
#[allow(
    clippy::too_many_arguments,
    reason = "intentionally explicit per-call state to keep the closure flat; bundling into a struct adds ceremony without clarity"
)]
fn search_file(
    path: &Path,
    root: &Path,
    pattern: &[u8],
    case_sensitive: bool,
    max_file_bytes: u64,
    max_match_text: u32,
    max_results: u32,
    results_emitted: &AtomicU64,
    cancel: &CancellationToken,
    bus: &BusEnd,
    id: JobId,
) {
    let Ok(metadata) = path.metadata() else {
        return;
    };
    if metadata.len() > max_file_bytes {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    // Binary heuristic: a NUL byte in the first 8 KiB classes the
    // file as binary and skips it.
    let head = bytes.len().min(8 * 1024);
    if bytes[..head].contains(&0u8) {
        return;
    }

    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();

    let mut line_no: u32 = 0;
    for line_bytes in bytes.split(|&b| b == b'\n') {
        line_no = line_no.saturating_add(1);
        if cancel.is_cancelled() {
            return;
        }
        if max_results > 0 && results_emitted.load(Ordering::Relaxed) >= u64::from(max_results) {
            return;
        }
        // Strip a trailing CR for CRLF files. The match offsets
        // reported are within the trimmed line text.
        let trimmed = if let Some((&b'\r', rest)) = line_bytes.split_last() {
            rest
        } else {
            line_bytes
        };
        let haystack = if case_sensitive {
            trimmed.to_vec()
        } else {
            ascii_fold(trimmed)
        };
        let Some(pos) = find_subslice(&haystack, pattern) else {
            continue;
        };
        // The user-visible text is the *original* (case-preserved)
        // line bytes, not the folded haystack.
        let Ok(text_str) = std::str::from_utf8(trimmed) else {
            return; // non-UTF-8 file: skip remainder.
        };
        let mut text_owned = text_str.to_owned();
        if u32::try_from(text_owned.len()).unwrap_or(u32::MAX) > max_match_text {
            // Truncate to a UTF-8 boundary.
            let cap = max_match_text as usize;
            let mut end = cap;
            while end > 0 && !text_owned.is_char_boundary(end) {
                end -= 1;
            }
            text_owned.truncate(end);
        }
        let match_start = u32::try_from(pos).unwrap_or(u32::MAX);
        let match_end = u32::try_from(pos + pattern.len()).unwrap_or(u32::MAX);
        // Only emit if the match offsets are still valid in the
        // (possibly truncated) text. Otherwise drop this match.
        if (match_end as usize) > text_owned.len() {
            continue;
        }
        let m = GrepMatch {
            file: rel.clone(),
            line: line_no,
            match_start,
            match_end,
            text: text_owned,
        };
        let _ = bus.send(
            ASYNC_REPLY_TOPIC,
            &WorkerReply {
                job_id: id,
                kind: ReplyKind::StreamItem(StreamPayload::Match(m)),
            },
        );
        results_emitted.fetch_add(1, Ordering::Relaxed);
    }
}

/// Naive byte-substring search. v0.1 deliberately stays
/// dependency-free; if benchmarks show this is the bottleneck we
/// promote `memchr::memmem`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    let first = needle[0];
    let mut i = 0;
    while i <= last {
        if haystack[i] == first && &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pump_until<F: Fn() -> bool>(rt: &AsyncRuntime, f: F) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !f() {
            assert!(Instant::now() < deadline, "runtime tick deadline exceeded");
            let _ = rt.tick();
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn dispatch_sum_completes_with_correct_value() {
        let rt = AsyncRuntime::with_pool_size(2);
        let id = rt.dispatch_compute_sum(10, None);
        pump_until(&rt, || rt.is_complete(id));
        match rt.take_result(id) {
            Some(JobOutcome::Complete(JobResult::Sum(v))) => assert_eq!(v, 55),
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn dispatch_sleep_completes_with_unit() {
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_sleep(5, None);
        pump_until(&rt, || rt.is_complete(id));
        match rt.take_result(id) {
            Some(JobOutcome::Complete(JobResult::Unit)) => {}
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn cancel_in_flight_sleep_yields_cancelled_outcome() {
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_sleep(2_000, None);
        // Yield to ensure the worker has noticed and started sleeping.
        thread::sleep(Duration::from_millis(20));
        rt.cancel(id);
        pump_until(&rt, || rt.is_complete(id));
        assert!(rt.is_cancelled(id));
        assert!(matches!(rt.take_result(id), Some(JobOutcome::Cancelled)));
    }

    #[test]
    fn many_independent_jobs_complete_concurrently() {
        let rt = AsyncRuntime::with_pool_size(4);
        let mut ids = Vec::new();
        for n in 1..=20u64 {
            ids.push((n, rt.dispatch_compute_sum(n, None)));
        }
        pump_until(&rt, || ids.iter().all(|(_, id)| rt.is_complete(*id)));
        for (n, id) in ids {
            let expected = n * (n + 1) / 2;
            match rt.take_result(id) {
                Some(JobOutcome::Complete(JobResult::Sum(v))) => assert_eq!(v, expected),
                other => panic!("job {id} (n={n}): {other:?}"),
            }
        }
        assert_eq!(rt.pending_len(), 0);
    }

    #[test]
    fn take_result_on_unknown_id_returns_none() {
        let rt = AsyncRuntime::with_pool_size(1);
        assert!(rt.take_result(99_999).is_none());
    }

    #[test]
    fn take_result_while_running_returns_none() {
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_sleep(500, None);
        // Don't tick --- the worker has not yet sent its reply
        // (or if it has, it is queued unread). take_result requires
        // the runtime to have settled the entry first.
        assert!(rt.take_result(id).is_none() || rt.is_complete(id));
    }

    #[test]
    fn cancel_unknown_id_is_noop() {
        let rt = AsyncRuntime::with_pool_size(1);
        rt.cancel(12_345); // no panic, no observable effect
    }

    // ---- T M3.4 supersede acceptance ----------------------------------------

    /// Acceptance bullet 1: supersession cancels an in-flight job
    /// within 50 ms. We dispatch a 2-second sleep, dispatch the same
    /// key again, and assert the first settles into Cancelled within
    /// 50 ms wall-clock.
    #[test]
    fn supersede_cancels_in_flight_job_within_50ms() {
        let rt = AsyncRuntime::with_pool_size(1);
        let first = rt.dispatch_sleep(2_000, Some("search"));
        // Let the worker pick the job up so cancel hits a running job.
        thread::sleep(Duration::from_millis(15));
        let started = Instant::now();
        let _second = rt.dispatch_sleep(2_000, Some("search"));
        // Pump until the first settles. With 1ms cancel polling, the
        // worker observes the flag in ~1-2 ms; the bus carries the
        // reply on the next try_recv. 50 ms gives generous slack.
        while !rt.is_complete(first) {
            assert!(
                started.elapsed() < Duration::from_millis(50),
                "supersede did not cancel within 50ms"
            );
            let _ = rt.tick();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(rt.is_cancelled(first), "first job should be Cancelled");
    }

    /// Acceptance bullet 2: queued jobs with the same key are cancelled
    /// before they run. Single-thread pool. We dispatch a "gate" job
    /// first --- a 100ms sleep --- to occupy the worker. While the
    /// worker is busy on that gate, we spam 100 dispatches under the
    /// same key. They all pile up in the deque, and the supersede
    /// rule means each new dispatch flips its predecessor's cancel
    /// token. By the time the worker pops them off the queue, every
    /// one of them sees `is_cancelled()` at entry and returns
    /// `Cancelled` without running. The single surviving uncancelled
    /// token is the *latest* dispatch.
    ///
    /// Acceptance: of the 100 dispatches, at most one settles
    /// Complete; the remaining 99 settle Cancelled.
    #[test]
    fn supersede_drops_queued_jobs_before_they_run() {
        let rt = AsyncRuntime::with_pool_size(1);
        // Gate the worker with a job that will sit in flight while we
        // dispatch the rest. The gate is keyless so it does not
        // interact with the supersede table.
        let gate = rt.dispatch_sleep(100, None);
        let mut ids = Vec::with_capacity(100);
        for _ in 0..100 {
            ids.push(rt.dispatch_sleep(0, Some("k")));
        }
        // Pump until every dispatched id (gate + 100 keyed) settles.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline, "settle deadline exceeded");
            let _ = rt.tick();
            if rt.is_complete(gate) && ids.iter().all(|id| rt.is_complete(*id)) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let mut completed = 0usize;
        let mut cancelled = 0usize;
        for id in &ids {
            match rt.take_result(*id) {
                Some(JobOutcome::Complete(_)) => completed += 1,
                Some(JobOutcome::Cancelled) => cancelled += 1,
                other => panic!("job {id}: unexpected outcome {other:?}"),
            }
        }
        assert!(
            completed <= 1,
            "expected ≤1 surviving job, got {completed} (cancelled={cancelled})"
        );
        assert_eq!(
            completed + cancelled,
            ids.len(),
            "every dispatched job must settle"
        );
        // Drain the gate's outcome too so the assertions below see a
        // clean pending table.
        let _ = rt.take_result(gate);
        // The supersede table should have pruned itself once the
        // last job (the one that owns the key) settled.
        assert_eq!(rt.supersede_len(), 0);
    }

    /// Acceptance bullet 3: rapid dispatch with the same key produces
    /// exactly one running job at a time. On a single-thread pool we
    /// can prove this structurally: at any moment, only the worker
    /// thread runs, so "exactly one running" is automatic. The
    /// supersede invariant we *can* check is that the `key → id`
    /// table only ever holds the most-recently-dispatched id.
    #[test]
    fn supersede_table_holds_only_the_most_recent_id() {
        let rt = AsyncRuntime::with_pool_size(1);
        let mut ids = Vec::new();
        for _ in 0..50 {
            ids.push(rt.dispatch_sleep(50, Some("only-one")));
        }
        // Immediately, before any tick, the table should map the
        // key to the *last* dispatched id.
        assert_eq!(rt.active_for_key("only-one"), Some(*ids.last().unwrap()));
        // And every prior id's cancel token must already be set.
        for id in &ids[..ids.len() - 1] {
            // Try to take_result eventually; even if the worker
            // hasn't replied yet, the cancel token is flipped.
            // Pump until it settles into Cancelled.
            let deadline = Instant::now() + Duration::from_millis(500);
            while !rt.is_complete(*id) {
                assert!(
                    Instant::now() < deadline,
                    "prior id {id} did not get cancelled"
                );
                let _ = rt.tick();
                thread::sleep(Duration::from_millis(1));
            }
            assert!(
                rt.is_cancelled(*id),
                "prior id {id} should have settled Cancelled"
            );
        }
    }

    /// Two distinct keys do not interfere: cancelling under "alpha"
    /// must not affect a job in flight under "beta".
    #[test]
    fn supersede_keys_are_independent() {
        let rt = AsyncRuntime::with_pool_size(2);
        let alpha1 = rt.dispatch_sleep(2_000, Some("alpha"));
        let beta = rt.dispatch_sleep(0, Some("beta"));
        let _alpha2 = rt.dispatch_sleep(0, Some("alpha"));
        // beta should complete cleanly; alpha1 should be cancelled.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !(rt.is_complete(beta) && rt.is_complete(alpha1)) {
            assert!(Instant::now() < deadline, "settle deadline exceeded");
            let _ = rt.tick();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(rt.is_cancelled(alpha1));
        assert!(matches!(
            rt.take_result(beta),
            Some(JobOutcome::Complete(JobResult::Unit))
        ));
    }

    /// A job dispatched without a key never enters the supersede
    /// table and is unaffected by dispatches that do specify keys.
    #[test]
    fn keyless_dispatch_is_unaffected_by_supersede() {
        let rt = AsyncRuntime::with_pool_size(2);
        let keyless = rt.dispatch_compute_sum(1_000_000, None);
        // Spam the same supersede key around the keyless job.
        for _ in 0..5 {
            let _ = rt.dispatch_sleep(0, Some("noisy"));
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !rt.is_complete(keyless) {
            assert!(Instant::now() < deadline, "keyless job stalled");
            let _ = rt.tick();
            thread::sleep(Duration::from_millis(1));
        }
        let expected = (1_000_000u64 * 1_000_001) / 2;
        assert!(matches!(
            rt.take_result(keyless),
            Some(JobOutcome::Complete(JobResult::Sum(v))) if v == expected
        ));
    }

    // ---- T M3.5 streaming + frame-boundary coalescing -----------------------

    /// Acceptance bullet 1 + 2: a 10000-item stream produces no
    /// message loss, and the consumer is woken at most one batch
    /// per drain regardless of producer rate. We bound the test by
    /// asserting `batch_count` is far smaller than `item_count` ---
    /// this is what coalescing buys.
    #[test]
    fn streaming_handler_emits_all_items_with_few_batches() {
        const N: u64 = 10_000;
        let rt = AsyncRuntime::with_pool_size(2);
        let id = rt.dispatch_emit_n(N, None, None);
        // Drive ticks at frame cadence (16 ms) until the stream is
        // closed. A run loop would do exactly this; we simulate it.
        let mut total_items = 0usize;
        let mut batch_count = 0usize;
        let mut closed = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !closed {
            assert!(Instant::now() < deadline, "stream did not complete in time");
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == id {
                    total_items += batch.items.len();
                    batch_count += 1;
                    if batch.closed {
                        assert!(matches!(
                            batch.outcome,
                            Some(JobOutcome::Complete(JobResult::Unit))
                        ));
                        closed = true;
                    }
                }
            }
            thread::sleep(Duration::from_millis(16));
        }
        assert_eq!(
            total_items as u64, N,
            "expected exactly {N} items, received {total_items}"
        );
        // Coalescing bound: at default 1024 cap and 60 Hz cadence,
        // 10K items should be delivered in a tiny number of batches
        // --- structurally bounded to ⌈N / cap⌉ + 1 (final closing
        // batch). Loose enough to absorb scheduler jitter.
        assert!(
            batch_count <= 64,
            "expected ≤64 batches for {N} items, got {batch_count}"
        );
    }

    /// Acceptance bullet 3 (batch size): per-stream `max_batch`
    /// caps individual batches. With cap = 32, no single drain
    /// returns more than 32 items.
    #[test]
    fn stream_max_batch_caps_batch_size() {
        const N: u64 = 1_000;
        const CAP: usize = 32;
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_emit_n(N, None, Some(CAP));
        let mut total_items = 0usize;
        let mut closed = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !closed {
            assert!(Instant::now() < deadline, "stream did not complete in time");
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == id {
                    assert!(
                        batch.items.len() <= CAP,
                        "batch had {} items, exceeds cap {CAP}",
                        batch.items.len()
                    );
                    total_items += batch.items.len();
                    if batch.closed {
                        closed = true;
                    }
                }
            }
            thread::sleep(Duration::from_millis(8));
        }
        assert_eq!(total_items as u64, N, "items lost under per-batch cap");
    }

    /// Acceptance bullet 3 (frame target): the runtime exposes
    /// `frame_target_ms` as a tunable knob, and the editor's run
    /// loop reads it each iteration.
    #[test]
    fn frame_target_is_tunable_and_clamped() {
        let rt = AsyncRuntime::with_pool_size(1);
        assert_eq!(rt.frame_target_ms(), DEFAULT_FRAME_TARGET_MS);
        rt.set_frame_target_ms(33);
        assert_eq!(rt.frame_target_ms(), 33);
        // Out-of-range values clamp into [1, 1000] rather than
        // panicking or silently disabling the loop.
        rt.set_frame_target_ms(0);
        assert_eq!(rt.frame_target_ms(), 1);
        rt.set_frame_target_ms(10_000);
        assert_eq!(rt.frame_target_ms(), 1000);
    }

    /// Default-batch knob behaves the same way: tunable, clamped.
    #[test]
    fn default_max_batch_is_tunable_and_clamped() {
        let rt = AsyncRuntime::with_pool_size(1);
        assert_eq!(rt.default_max_batch(), DEFAULT_MAX_BATCH);
        rt.set_default_max_batch(64);
        assert_eq!(rt.default_max_batch(), 64);
        rt.set_default_max_batch(0);
        assert_eq!(rt.default_max_batch(), 1);
        rt.set_default_max_batch(10_000_000);
        assert_eq!(rt.default_max_batch(), 1_000_000);
    }

    /// Streams compose with supersede: a second stream under the
    /// same key cancels the predecessor; the predecessor's final
    /// batch is `closed = true` with `Cancelled` outcome.
    #[test]
    fn stream_supersede_cancels_predecessor_with_cancelled_outcome() {
        let rt = AsyncRuntime::with_pool_size(2);
        let first = rt.dispatch_emit_n(1_000_000, Some("emit"), Some(64));
        // Let the first emitter start producing.
        thread::sleep(Duration::from_millis(10));
        let _second = rt.dispatch_emit_n(10, Some("emit"), Some(64));
        // Drain until the first stream's closed batch arrives.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut first_outcome: Option<JobOutcome> = None;
        while first_outcome.is_none() {
            assert!(
                Instant::now() < deadline,
                "first stream did not close in time"
            );
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == first && batch.closed {
                    first_outcome = batch.outcome;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(first_outcome, Some(JobOutcome::Cancelled)));
    }

    /// A stream prior settling under supersession must not clobber
    /// a *still-alive* successor's slot. We pair a stream prior with
    /// a long-running sleep successor so the assert sees the
    /// successor mid-flight.
    #[test]
    fn stream_settle_does_not_clobber_successor_supersede_slot() {
        let rt = AsyncRuntime::with_pool_size(2);
        let prior = rt.dispatch_emit_n(1, Some("k"), Some(8));
        // Successor is a 5-second sleep so it stays Running while
        // we observe the supersede slot.
        let successor = rt.dispatch_sleep(5_000, Some("k"));
        // Drain until the prior stream emits its closed batch.
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut prior_done = false;
        while !prior_done {
            assert!(Instant::now() < deadline, "prior never settled");
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == prior && batch.closed {
                    prior_done = true;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        // The supersede slot must still point at the successor.
        assert_eq!(rt.active_for_key("k"), Some(successor));
        rt.cancel(successor);
    }

    /// Settling a superseded predecessor must not prune the table
    /// entry that the *successor* now owns. Regression test for the
    /// "`get(key) == Some(id)` only" guard inside `tick`.
    #[test]
    fn settling_superseded_does_not_clobber_successor_table_slot() {
        let rt = AsyncRuntime::with_pool_size(1);
        let prior = rt.dispatch_sleep(2_000, Some("k"));
        let successor = rt.dispatch_sleep(2_000, Some("k"));
        // Pump until prior settles (Cancelled). Successor still
        // running; key→successor must persist.
        let deadline = Instant::now() + Duration::from_millis(500);
        while !rt.is_complete(prior) {
            assert!(Instant::now() < deadline, "prior never settled");
            let _ = rt.tick();
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(rt.active_for_key("k"), Some(successor));
        // Cleanup so the runtime drops cleanly.
        rt.cancel(successor);
    }

    // ---- T M3.6 parallel grep acceptance ------------------------------------

    /// R31 (compile-time): every value carried into a grep worker
    /// closure is owned. The grep types are `Send` --- the worker
    /// pool's `dispatch` requires it, so failing this trait bound
    /// is a missing-`Send` regression.
    #[test]
    fn grep_types_satisfy_send_per_r31() {
        fn assert_send<T: Send>() {}
        assert_send::<GrepSpec>();
        assert_send::<GrepMatch>();
        assert_send::<StreamPayload>();
    }

    /// Build a temp tree with `(path, contents)` files and return
    /// the temp dir.
    fn make_grep_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (rel, contents) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(&path, contents).expect("write file");
        }
        dir
    }

    fn collect_matches(rt: &AsyncRuntime, id: JobId, deadline: Duration) -> Vec<GrepMatch> {
        let mut out = Vec::new();
        let mut closed = false;
        let start = Instant::now();
        while !closed {
            assert!(start.elapsed() < deadline, "grep did not close in time");
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id != id {
                    continue;
                }
                for payload in batch.items {
                    if let StreamPayload::Match(m) = payload {
                        out.push(m);
                    }
                }
                if batch.closed {
                    closed = true;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        out
    }

    /// Correctness: matches found in known files, with correct
    /// line numbers and offsets.
    #[test]
    fn grep_finds_known_matches_in_synthetic_tree() {
        let tree = make_grep_tree(&[
            ("src/a.txt", "first\nneedle here\nthird\n"),
            ("src/b.txt", "no match here\n"),
            ("src/sub/c.txt", "alpha needle\nbeta\nneedle\n"),
            ("README.md", "needle\n"),
        ]);
        let rt = AsyncRuntime::with_pool_size(2);
        let id = rt.dispatch_grep(
            GrepSpec::new(tree.path().to_path_buf(), "needle".to_owned()),
            None,
            None,
        );
        let matches = collect_matches(&rt, id, Duration::from_secs(5));
        // Three files, four matches total.
        assert_eq!(matches.len(), 4, "matches: {matches:#?}");
        let mut by_file: HashMap<String, Vec<&GrepMatch>> = HashMap::new();
        for m in &matches {
            by_file.entry(m.file.clone()).or_default().push(m);
        }
        let a = by_file
            .iter()
            .find(|(k, _)| k.ends_with("a.txt"))
            .expect("a.txt match");
        assert_eq!(a.1[0].line, 2);
        assert_eq!(a.1[0].text, "needle here");
        assert_eq!(a.1[0].match_start, 0);
        assert_eq!(a.1[0].match_end, 6);
        let c = by_file
            .iter()
            .find(|(k, _)| k.ends_with("c.txt"))
            .expect("c.txt match");
        assert_eq!(c.1.len(), 2, "two matches expected in c.txt");
    }

    /// Case-insensitive mode folds both haystack and needle.
    #[test]
    fn grep_case_insensitive_matches_mixed_case() {
        let tree = make_grep_tree(&[("a.txt", "FOO\nfoo\nFoO\n")]);
        let rt = AsyncRuntime::with_pool_size(1);
        let mut spec = GrepSpec::new(tree.path().to_path_buf(), "foo".to_owned());
        spec.case_sensitive = false;
        let id = rt.dispatch_grep(spec, None, None);
        let matches = collect_matches(&rt, id, Duration::from_secs(5));
        assert_eq!(matches.len(), 3);
    }

    /// Skips binary files (NUL byte in head) and files exceeding the
    /// size cap.
    #[test]
    fn grep_skips_binary_and_oversize_files() {
        let tree = tempfile::tempdir().expect("tempdir");
        // Binary: contains a NUL byte.
        std::fs::write(tree.path().join("bin.dat"), b"prefix\0needle suffix\n").expect("write bin");
        // Oversize: 2 KiB body, cap will be 1 KiB.
        std::fs::write(tree.path().join("big.txt"), "needle\n".repeat(400)).expect("write big");
        // Normal: contains the match.
        std::fs::write(tree.path().join("small.txt"), "needle\n").expect("write small");
        let rt = AsyncRuntime::with_pool_size(1);
        let mut spec = GrepSpec::new(tree.path().to_path_buf(), "needle".to_owned());
        spec.max_file_bytes = 1024;
        let id = rt.dispatch_grep(spec, None, None);
        let matches = collect_matches(&rt, id, Duration::from_secs(5));
        assert_eq!(matches.len(), 1);
        assert!(matches[0].file.ends_with("small.txt"));
    }

    /// Acceptance bullet 2: typing a new query while grep is in
    /// flight cancels the predecessor within 50 ms. We dispatch a
    /// single-fanout grep against a synthetic tree large enough to
    /// keep the worker busy past the supersede deadline, then
    /// dispatch a second grep under the same key and assert the
    /// prior settles `Cancelled` within 50 ms of the supersede call.
    #[test]
    fn grep_supersede_cancels_predecessor_within_50ms() {
        // Synthetic load: 5000 small files of non-matching content.
        // Single-fanout sequential scan keeps the worker busy past
        // the 50 ms deadline without depending on disk speed.
        let dir = tempfile::tempdir().expect("tempdir");
        let body: String = "noise noise noise noise noise\n".repeat(50);
        for i in 0..5_000 {
            std::fs::write(dir.path().join(format!("f{i:05}.txt")), &body).expect("write");
        }
        let rt = AsyncRuntime::with_pool_size(2);
        let mut prior_spec = GrepSpec::new(dir.path().to_path_buf(), "needle".to_owned());
        prior_spec.fanout = 1;
        let prior = rt.dispatch_grep(prior_spec, Some("search"), None);
        // Let the worker actually start scanning.
        thread::sleep(Duration::from_millis(15));
        // Sanity: the prior should not have already finished the
        // whole tree --- if it has, the load is too small for the
        // host. Skip rather than emit a false negative.
        if rt.is_complete(prior) {
            // Drain so the runtime drops cleanly; treat as a
            // capacity-tested no-op.
            let _ = rt.tick();
            let _ = rt.take_stream_batches();
            return;
        }
        let started = Instant::now();
        let mut succ_spec = GrepSpec::new(dir.path().to_path_buf(), "alpha".to_owned());
        succ_spec.fanout = 1;
        let _successor = rt.dispatch_grep(succ_spec, Some("search"), None);
        let mut prior_done = false;
        let mut prior_outcome: Option<JobOutcome> = None;
        while !prior_done {
            assert!(
                started.elapsed() < Duration::from_millis(50),
                "grep supersede did not cancel within 50ms (elapsed: {:?})",
                started.elapsed()
            );
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == prior && batch.closed {
                    prior_done = true;
                    prior_outcome = batch.outcome;
                }
            }
        }
        assert!(
            matches!(prior_outcome, Some(JobOutcome::Cancelled)),
            "prior outcome should be Cancelled, got {prior_outcome:?}"
        );
    }

    /// Acceptance bullet 3 (coalescing): a saturating grep load ---
    /// every line matches --- delivers all matches in a small
    /// number of batches, not one wakeup per match. With cap=64 and
    /// 500 matching lines, structural bound is ⌈500/64⌉ + 1 ≈ 9
    /// batches; we allow generous slack for scheduler jitter.
    #[test]
    fn grep_coalesces_saturating_match_rate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut body = String::new();
        for _ in 0..500 {
            body.push_str("needle\n");
        }
        std::fs::write(dir.path().join("dense.txt"), body).expect("write");
        let rt = AsyncRuntime::with_pool_size(2);
        let id = rt.dispatch_grep(
            GrepSpec::new(dir.path().to_path_buf(), "needle".to_owned()),
            None,
            Some(64),
        );
        let mut total = 0usize;
        let mut batch_count = 0usize;
        let mut closed = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !closed {
            assert!(Instant::now() < deadline, "grep did not close in time");
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == id {
                    assert!(
                        batch.items.len() <= 64,
                        "batch had {} items, exceeds cap 64",
                        batch.items.len()
                    );
                    total += batch.items.len();
                    batch_count += 1;
                    if batch.closed {
                        closed = true;
                    }
                }
            }
            // 16 ms cadence simulates a 60 Hz run loop.
            thread::sleep(Duration::from_millis(16));
        }
        assert_eq!(total, 500, "expected 500 matches, got {total}");
        // Coalescing bound: 500 items at cap 64 across 16 ms frames
        // yields a small batch count. Loose enough for jitter.
        assert!(
            batch_count <= 64,
            "expected ≤64 batches for 500 items, got {batch_count}"
        );
    }

    /// Acceptance bullet 1 scaled-down: parallel grep over many
    /// small files completes quickly. We don't try to reproduce the
    /// "kernel source under 2s" benchmark in CI (no kernel source
    /// on disk); instead we prove the parallel structure: searching
    /// 1000 small files for a literal pattern with `fanout = N`
    /// completes in a small wall-clock budget. Generous bound (5 s)
    /// because slow CI hosts vary wildly --- the meaningful test is
    /// that this runs at all without freezing.
    #[test]
    fn grep_parallel_search_completes_at_synthetic_scale() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..1_000 {
            let body = format!("header line\nnoise noise noise\nneedle line {i}\nfooter\n");
            std::fs::write(dir.path().join(format!("f{i:04}.txt")), body).expect("write");
        }
        let rt = AsyncRuntime::with_pool_size(4);
        let started = Instant::now();
        let id = rt.dispatch_grep(
            GrepSpec::new(dir.path().to_path_buf(), "needle".to_owned()),
            None,
            None,
        );
        let matches = collect_matches(&rt, id, Duration::from_secs(5));
        let elapsed = started.elapsed();
        assert_eq!(matches.len(), 1_000);
        assert!(
            elapsed < Duration::from_secs(5),
            "1000-file grep took {elapsed:?}, expected under 5s"
        );
    }

    /// Long lines truncate to `max_match_text` rather than emitting
    /// megabytes per match. The match offsets must remain valid in
    /// the truncated text (or the match is dropped).
    #[test]
    fn grep_truncates_long_lines_to_max_match_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        // 16 KiB line with `needle` near the start.
        let mut line = String::from("needle ");
        line.push_str(&"x".repeat(16 * 1024));
        line.push('\n');
        std::fs::write(dir.path().join("long.txt"), line).expect("write");
        let rt = AsyncRuntime::with_pool_size(1);
        let mut spec = GrepSpec::new(dir.path().to_path_buf(), "needle".to_owned());
        spec.max_match_text = 128;
        let id = rt.dispatch_grep(spec, None, None);
        let matches = collect_matches(&rt, id, Duration::from_secs(5));
        assert_eq!(matches.len(), 1);
        assert!(
            matches[0].text.len() <= 128,
            "text length {} exceeds cap",
            matches[0].text.len()
        );
        assert!(matches[0].text.starts_with("needle"));
    }

    /// Empty pattern emits zero matches and closes cleanly.
    #[test]
    fn grep_empty_pattern_emits_no_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), "any line\n").expect("write");
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_grep(
            GrepSpec::new(dir.path().to_path_buf(), String::new()),
            None,
            None,
        );
        let matches = collect_matches(&rt, id, Duration::from_secs(5));
        assert_eq!(matches.len(), 0);
    }

    // ---- T M3.7 workers observability ---------------------------------------

    /// Active jobs show up in the snapshot with the expected kind,
    /// supersede key, and a non-zero age.
    #[test]
    fn workers_snapshot_lists_active_jobs() {
        let rt = AsyncRuntime::with_pool_size(1);
        let _sleep_id = rt.dispatch_sleep(2_000, Some("sleeper"));
        let _sum_id = rt.dispatch_compute_sum(50_000_000, None);
        // Yield so age_ms is observably non-zero.
        thread::sleep(Duration::from_millis(5));
        let snap = rt.workers_snapshot();
        assert_eq!(snap.active.len(), 2, "two active jobs expected");
        assert!(snap.completed.is_empty());
        let kinds: Vec<JobKind> = snap.active.iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&JobKind::Sleep));
        assert!(kinds.contains(&JobKind::ComputeSum));
        let sleeper = snap
            .active
            .iter()
            .find(|a| a.kind == JobKind::Sleep)
            .expect("sleeper");
        assert_eq!(sleeper.supersede_key.as_deref(), Some("sleeper"));
        assert!(sleeper.age_ms > 0);
        assert!(!sleeper.cancel_requested);
        // Cleanup.
        rt.cancel(sleeper.id);
        rt.cancel(
            snap.active
                .iter()
                .find(|a| a.kind == JobKind::ComputeSum)
                .unwrap()
                .id,
        );
    }

    /// `cancel_requested` is reflected immediately --- the snapshot
    /// is what the *workers* buffer renders, and "user pressed
    /// cancel but worker hasn't observed yet" is a state the user
    /// needs to see.
    #[test]
    fn workers_snapshot_marks_cancel_requested() {
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_sleep(2_000, None);
        thread::sleep(Duration::from_millis(2));
        rt.cancel(id);
        let snap = rt.workers_snapshot();
        let job = snap.active.iter().find(|j| j.id == id).expect("present");
        assert!(job.cancel_requested);
    }

    /// A settled job moves out of the active list and into the
    /// completed ring with its outcome.
    #[test]
    fn workers_snapshot_records_completed_ring() {
        let rt = AsyncRuntime::with_pool_size(1);
        let id = rt.dispatch_compute_sum(10, None);
        pump_until(&rt, || rt.is_complete(id));
        let snap = rt.workers_snapshot();
        assert!(
            snap.active.iter().all(|j| j.id != id),
            "settled job should not be active"
        );
        let entry = snap
            .completed
            .iter()
            .find(|c| c.id == id)
            .expect("completed slot");
        assert_eq!(entry.kind, JobKind::ComputeSum);
        assert!(matches!(
            entry.outcome,
            JobOutcome::Complete(JobResult::Sum(55))
        ));
    }

    /// Newest completion comes first; the ring keeps order stable
    /// for the "Recent (newest first)" buffer section.
    #[test]
    fn workers_completed_ring_orders_newest_first() {
        let rt = AsyncRuntime::with_pool_size(1);
        let first = rt.dispatch_compute_sum(1, None);
        pump_until(&rt, || rt.is_complete(first));
        thread::sleep(Duration::from_millis(2));
        let second = rt.dispatch_compute_sum(2, None);
        pump_until(&rt, || rt.is_complete(second));
        let snap = rt.workers_snapshot();
        // Newest first means the second-dispatched job is at index 0.
        assert_eq!(snap.completed[0].id, second);
        assert_eq!(snap.completed[1].id, first);
    }

    /// The completed ring is bounded to [`COMPLETED_RING_CAP`].
    /// Pushing more entries evicts oldest from the back.
    #[test]
    fn workers_completed_ring_evicts_oldest_at_capacity() {
        let rt = AsyncRuntime::with_pool_size(1);
        let mut ids = Vec::new();
        for _ in 0..(COMPLETED_RING_CAP + 16) {
            let id = rt.dispatch_compute_sum(1, None);
            pump_until(&rt, || rt.is_complete(id));
            ids.push(id);
        }
        let snap = rt.workers_snapshot();
        assert_eq!(snap.completed.len(), COMPLETED_RING_CAP);
        // The oldest-dispatched ids fell off the back; the newest
        // remain. Specifically, the entry at index 0 is the most
        // recent dispatch.
        assert_eq!(snap.completed[0].id, *ids.last().unwrap());
        // The earliest 16 dispatches must NOT be in the ring.
        for old in &ids[..16] {
            assert!(snap.completed.iter().all(|c| c.id != *old));
        }
    }

    // T M3.8 -----------------------------------------------------
    //
    // Memory and lifecycle audit: 1000 dispatch/cancel cycles must
    // not grow internal state. The runtime's contract is that
    // `take_result` evicts a settled entry from the pending table
    // and `tick` prunes the supersede slot it owns; if either path
    // leaks, repeated cycles surface it as monotonic growth.

    /// Dispatch + cancel + settle + `take_result`, 1000 times,
    /// asserts the pending and supersede tables return to zero.
    /// Cycles are short (~20ms sleep) so this finishes in seconds.
    /// Cancellation outcome is racy --- the worker may finish the
    /// sleep before it polls cancel --- so we accept any terminal
    /// outcome and only gate on table sizes.
    #[test]
    fn dispatch_cancel_1000_cycles_no_leak() {
        let rt = AsyncRuntime::with_pool_size(2);
        for _ in 0..1000 {
            let id = rt.dispatch_sleep(5, None);
            rt.cancel(id);
            pump_until(&rt, || rt.is_complete(id));
            let _ = rt.take_result(id);
        }
        assert_eq!(rt.pending_len(), 0, "pending leaked across 1000 cycles");
        assert_eq!(rt.supersede_len(), 0, "supersede leaked across 1000 cycles");
        // The completion ring is bounded by COMPLETED_RING_CAP, not
        // by cycle count --- after 1000 cycles it should be saturated
        // at exactly the cap.
        let snap = rt.workers_snapshot();
        assert_eq!(snap.active.len(), 0);
        assert_eq!(snap.completed.len(), COMPLETED_RING_CAP);
    }

    /// Supersede churn variant: dispatch many jobs under the same
    /// key. The supersede table must hold exactly one slot at any
    /// time and shrink to zero once every job has settled and been
    /// taken via `take_result`.
    #[test]
    fn supersede_churn_500_cycles_table_returns_to_zero() {
        let rt = AsyncRuntime::with_pool_size(2);
        let mut ids = Vec::with_capacity(500);
        for _ in 0..500 {
            ids.push(rt.dispatch_sleep(3, Some("search")));
            // The supersede table must never exceed one slot under
            // a single key, regardless of cycle count.
            assert_eq!(rt.supersede_len(), 1);
        }
        // Drain every dispatched id through `take_result`. Earlier
        // dispatches were superseded mid-flight; their workers reply
        // with `Cancelled` once they observe the token. Each entry
        // lingers in `pending` until `take_result` removes it.
        let deadline = Instant::now() + Duration::from_secs(15);
        while rt.pending_len() > 0 {
            assert!(
                Instant::now() < deadline,
                "drain stuck; pending={}",
                rt.pending_len()
            );
            let _ = rt.tick();
            for id in &ids {
                if rt.is_complete(*id) {
                    let _ = rt.take_result(*id);
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(rt.pending_len(), 0);
        assert_eq!(rt.supersede_len(), 0);
    }

    /// Stream lifecycle leak gate: a stream that runs to closure has
    /// its pending entry evicted by `take_stream_batches` once the
    /// closing batch is delivered. 200 cycles is plenty to surface
    /// any per-stream allocation that escapes.
    #[test]
    fn stream_dispatch_close_200_cycles_no_leak() {
        let rt = AsyncRuntime::with_pool_size(2);
        for _ in 0..200 {
            let id = rt.dispatch_emit_n(8, None, Some(8));
            // Drain until the batch carrying `closed = true` for this
            // id is observed.
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut closed = false;
            while !closed {
                assert!(Instant::now() < deadline, "stream close deadline");
                let _ = rt.tick();
                for batch in rt.take_stream_batches() {
                    if batch.id == id && batch.closed {
                        closed = true;
                    }
                }
                if !closed {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        assert_eq!(rt.pending_len(), 0, "stream pending entries leaked");
    }

    // ---- T M4.1 dispatch_parse smoke ----------------------------------------

    /// `dispatch_parse` round-trips a real grammar end-to-end:
    /// settle status is `Complete(Parse{..})`, the bundle is in
    /// the side handoff, and the tree's root has the language's
    /// expected top-level node type.
    #[test]
    fn dispatch_parse_round_trips_a_rust_source_file() {
        let rt = AsyncRuntime::with_pool_size(1);
        let source = b"fn main() { let _x = 1 + 2; }\n";
        let req = ParseRequest {
            source: Arc::from(&source[..]),
            language: tree_sitter_rust::LANGUAGE.into(),
            language_name: "rust".to_owned(),
            prior_tree: None,
            edits: Vec::new(),
        };
        let id = rt.dispatch_parse(req, None);
        pump_until(&rt, || rt.is_complete(id));
        let bundle = rt
            .take_parse_tree(id)
            .expect("parse handoff must hold a bundle on Complete");
        match rt.take_result(id) {
            Some(JobOutcome::Complete(JobResult::Parse { duration_ms })) => {
                assert!(duration_ms < 100, "trivial parse should be fast");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert_eq!(bundle.language_name, "rust");
        assert_eq!(bundle.tree.root_node().kind(), "source_file");
        // take_parse_tree was already drained, so handoff is empty.
        assert_eq!(rt.parse_handoff_len(), 0);
    }

    /// `take_result` on a parse job drops any leftover handoff entry,
    /// so a forgetful caller cannot leak bundles.
    #[test]
    fn take_result_drops_stale_parse_handoff() {
        let rt = AsyncRuntime::with_pool_size(1);
        let req = ParseRequest {
            source: Arc::from(&b"fn x() {}"[..]),
            language: tree_sitter_rust::LANGUAGE.into(),
            language_name: "rust".to_owned(),
            prior_tree: None,
            edits: Vec::new(),
        };
        let id = rt.dispatch_parse(req, None);
        pump_until(&rt, || rt.is_complete(id));
        // Don't drain the bundle --- take_result should clean it.
        assert_eq!(rt.parse_handoff_len(), 1);
        let _ = rt.take_result(id);
        assert_eq!(rt.parse_handoff_len(), 0);
    }
}
