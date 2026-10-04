// parse_isolation.rs --- E7i: the editor's side of a parse unit, a
// worker process per buffer.

//! Parse containment (`syntax.isolation`), the process boundary the owner
//! ruled at E7i.
//!
//! A parse unit (`pmacs-parse-unit`) holds one buffer's trees and runs its
//! parse, its capture walks and every read of its tree. The editor keeps
//! the buffer's text, the edit log and the spans that came back; it never
//! holds a tree. Each buffer gets its own unit, a worker process, which
//! answers reads of the tree it shows while it parses a newer text, so no
//! read waits on a parse. Time is bounded by a watchdog here that kills the
//! worker at the deadline. Memory is bounded, per worker, by `RLIMIT_AS`,
//! which the worker applies to itself before serving (Linux), or where that
//! is refused (macOS) by the worker's own watch of its peak; and in total
//! by a cgroup v2 `memory.max` the editor creates beside its own cgroup
//! where the session's cgroup subtree is delegated, or else by a watchdog
//! here over the sizes the workers report. The limit and the cgroup are
//! preventive and the kernel's; the watch and the watchdog are reactive
//! and overshoot by what a parse grows between two looks ([`Enforcer`];
//! `docs/divergences.md` records the gap). A kill or an abort ends the
//! process.
//!
//! The boundary stops the operation, not one call inside it: the parse's
//! whole memory and the whole request's time are what the limits measure.
//! A unit that dies is discarded and the next request starts a fresh one
//! with the whole text. One that dies on its own (a signal, or an exit the
//! editor did not ask for) crashed, most likely in a grammar's C running
//! this buffer's bytes: the buffer's next worker waits out a back-off that
//! doubles from [`CRASH_BACKOFF`], and after [`MAX_CRASHES`] crashes in a
//! row with no parse installed between them the buffer is not parsed again
//! until it is killed and opened again, since every restart runs the same
//! bytes (E7i review 1, Medium 1). The settle path tells the user once.
//!
//! The buffer's previous parse survives the discard in the editor, as its
//! text and its spans ([`IsolatedHandle`]); a read those spans do not
//! answer is answered by re-parsing that text into a fresh unit, which
//! rebuilds the previous tree there. That re-parse runs on the thread that
//! asked, which for the grid, a fold command or Lua's node API is the main
//! thread: the first such read after a death holds the editor for a cold
//! parse of the previous text (E7i review 1, Low 4: 186 ms for 120 KB of
//! Rust inside `paint_frame` in a debug build; the E7i build measured a
//! release worker's cold parse at 154 ms for `src/editor.rs` and 409 ms
//! for the comparison's markdown note). It is bounded by the deadline and
//! then [`AFTER_PARSE_HARD`], not made while another parse of the buffer
//! runs, and not made at all once the buffer's crashes have stopped its
//! parsing; a frame painted before it shows what the editor already holds.
//!
//! Highlighting, folds and Lua's node API read the tree through the unit,
//! batched (the consumer scoping at E7i). `PMACS_E7I_TRACE=<path>` appends
//! one JSON line per unit event, a measurement hook and not a setting.

use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use pmacs_parse_unit::{Failure, ParseCall, Request, Response, SpanSet, TextUpdate, WireEdit};

use crate::buffer::BufferId;
use crate::syntax::{
    HighlightSpan, IsolatedLayerSpans, IsolatedSpans, IsolatedTree, NodeFacts, ParseError,
    ParseRequest, ParseTreeBundle,
};

/// Which boundary a parse runs behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// In the editor's own process, as before E7i.
    Native,
    /// A worker process running the unit.
    Process,
}

impl Isolation {
    /// The mode `syntax.isolation` names: `"none"` or `"process"`.
    #[must_use]
    pub fn from_config(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::Native),
            "process" => Some(Self::Process),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Native => "none",
            Self::Process => "process",
        }
    }
}

/// The limits a dispatch carries, from `pmacs.config`.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The boundary.
    pub mode: Isolation,
    /// What one unit may grow by beyond what it holds when it starts.
    pub unit_memory: u64,
    /// What every unit together may hold; 0 leaves the total unbounded.
    pub total_memory: u64,
    /// `syntax.parse-deadline-ms`: enforced inside the unit through the
    /// progress callback, and by the boundary [`HARD_GRACE`] after it.
    pub deadline: Option<Duration>,
    /// `syntax.parse-worker-recycle-mb`: a worker whose peak passed this
    /// many bytes in a parse is replaced at the buffer's next parse (E7i.4);
    /// 0 keeps workers for the buffer's life.
    pub recycle: u64,
}

/// How long after the deadline the boundary stops a unit whose parse has
/// not returned. A parse the progress callback reaches returns at the
/// deadline; this is for work it does not reach (#296's accept, #301's
/// condensation), and a little slack so a clean cancel lands first.
pub const HARD_GRACE: Duration = Duration::from_millis(100);

/// How long a unit may take after its parse returned: installing the tree
/// (on a fresh unit, compiling each layer's queries) and walking the spans
/// it answers with. The deadline bounds the parse, as in-process it did;
/// this bounds the rest, which in-process ran on the main thread unbounded.
pub const AFTER_PARSE_HARD: Duration = Duration::from_secs(10);

/// One isolated parse, as the async runtime hands it to a worker thread.
pub struct IsolatedJob {
    /// The buffer, which names its unit.
    pub buffer: BufferId,
    /// The request the editor would have parsed in-process.
    pub request: ParseRequest,
    /// Each pending edit's inserted bytes, in the order of `request.edits`.
    pub inserted: Vec<Vec<u8>>,
    /// Byte ranges of the new text whose spans the parse should return.
    pub interest: Vec<(u32, u32)>,
    /// The boundary and its limits.
    pub limits: Limits,
}

/// What stopped a unit's memory: the limit that applied and how it is
/// enforced, which the platform decides (E7i's condition 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enforcer {
    /// `RLIMIT_AS` on the worker: the allocation past it fails, before the
    /// memory exists (Linux).
    Rlimit,
    /// The worker's own watch of its peak, every millisecond while it
    /// parses: after the memory exists, by up to a millisecond's growth
    /// (macOS, where `RLIMIT_AS` is refused).
    Watch,
    /// A cgroup v2 `memory.max` over every worker: the kernel's OOM killer
    /// (a Linux session whose cgroup subtree is delegated).
    Cgroup,
    /// The editor's watchdog over the workers' reported memory, every
    /// [`TOTAL_WATCH_EVERY`]: after the memory exists (macOS, and Linux
    /// without delegation).
    Watchdog,
}

impl Enforcer {
    fn name(self) -> &'static str {
        match self {
            Self::Rlimit => "rlimit",
            Self::Watch => "watch",
            Self::Cgroup => "cgroup",
            Self::Watchdog => "watchdog",
        }
    }
}

/// Why a unit was discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Death {
    /// Its parse ran past the deadline plus [`HARD_GRACE`] without
    /// returning, and it was stopped.
    Time {
        /// The deadline that applied.
        deadline: Duration,
        /// How long the request had run when it was stopped.
        after: Duration,
    },
    /// Its parse returned, and installing the tree and walking the spans it
    /// answers with ran past [`AFTER_PARSE_HARD`]; it was stopped. Not the
    /// deadline, which bounds the parse alone (E7i review 1, Low 1).
    Stalled {
        /// How long the request had run when it was stopped.
        after: Duration,
    },
    /// A read of its tree ran past its bound (`READ_HARD`, or `Hello`'s at
    /// start) and it was stopped.
    Read {
        /// The bound.
        limit: Duration,
        /// How long the read had run.
        after: Duration,
    },
    /// It grew past its own allowance.
    Memory {
        /// The allowance, bytes.
        limit: u64,
        /// What enforced it.
        by: Enforcer,
        /// For the memory watch, how far past the allowance the worker's
        /// peak was when the watch stopped it, bytes: its overshoot.
        overshoot: Option<u64>,
    },
    /// The units together reached the editor-wide total.
    Total {
        /// The total, bytes.
        limit: u64,
        /// What enforced it.
        by: Enforcer,
    },
    /// It died on its own: by a signal, or an exit the editor did not ask
    /// for. Most likely the grammar's C crashed running this text.
    Crashed {
        /// The signal it died by, if it did.
        signal: Option<i32>,
        /// Its exit status, if it exited.
        code: Option<i32>,
        /// Its last line on stderr.
        last: String,
    },
    /// The editor ended it for another reason (its buffer was killed, the
    /// editor stopped, it answered out of turn).
    Ended(String),
    /// It could not be started.
    Unavailable(String),
}

impl Death {
    /// The text the settle path receives: a deadline-shaped message for a
    /// time stop, so the user is told once as for a cancelled parse.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Time { deadline, after } => ParseError::DeadlineExceeded {
                deadline: *deadline,
                after: *after,
            }
            .to_string(),
            Self::Stalled { after } => format!(
                "{STALLED_MESSAGE}: installing and answering it took past {} s, and its unit was stopped after {} ms",
                AFTER_PARSE_HARD.as_secs(),
                after.as_millis()
            ),
            Self::Read { limit, after } => format!(
                "a read of the tree ran past {} ms, and its unit was stopped after {} ms",
                limit.as_millis(),
                after.as_millis()
            ),
            Self::Memory { limit, .. } => {
                format!(
                    "{PARSE_LIMIT_MESSAGE}: its parse unit grew past {} MiB",
                    limit >> 20
                )
            }
            Self::Total { limit, .. } => format!(
                "{PARSE_LIMIT_MESSAGE}: the parse units together reached {} MiB",
                limit >> 20
            ),
            Self::Crashed { .. } => format!("{CRASHED_MESSAGE}: {}", self.how()),
            Self::Ended(why) => format!("parse unit ended: {why}"),
            Self::Unavailable(why) => format!("{UNAVAILABLE_MESSAGE}: {why}"),
        }
    }

    /// How a crashed unit died: `signal 11 (SIGSEGV)` or `exit 101`, with
    /// its last words when it left any.
    fn how(&self) -> String {
        let Self::Crashed { signal, code, last } = self else {
            return String::new();
        };
        let mut how = match (signal, code) {
            (Some(sig), _) => match nix::sys::signal::Signal::try_from(*sig) {
                Ok(name) => format!("signal {sig} ({name})"),
                Err(_) => format!("signal {sig}"),
            },
            (None, Some(code)) => format!("exit {code}"),
            (None, None) => "ended".to_owned(),
        };
        let last = last.trim();
        if !last.is_empty() {
            let cut: String = last.chars().take(160).collect();
            how.push_str(", saying: ");
            how.push_str(&cut);
        }
        how
    }

    /// How a crashed unit died, shortest: the signal's name (`SIGSEGV`),
    /// or `exit 101`; for the first words of the notice, which a narrow
    /// echo line keeps (E7i fix round 2).
    fn signal_name(&self) -> String {
        let Self::Crashed { signal, code, .. } = self else {
            return String::new();
        };
        match (signal, code) {
            (Some(sig), _) => nix::sys::signal::Signal::try_from(*sig)
                .map_or_else(|_| format!("signal {sig}"), |name| name.to_string()),
            (None, Some(code)) => format!("exit {code}"),
            (None, None) => "ended".to_owned(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Time { .. } => "time",
            Self::Stalled { .. } => "stalled",
            Self::Read { .. } => "read",
            Self::Memory { .. } => "memory",
            Self::Total { .. } => "total",
            Self::Crashed { .. } => "crashed",
            Self::Ended(_) => "ended",
            Self::Unavailable(_) => "unavailable",
        }
    }

    /// `kind: message`, with the enforcer of a memory stop, for the report.
    fn describe(&self) -> String {
        match self {
            Self::Memory {
                by,
                overshoot: Some(over),
                ..
            } => format!(
                "{}: {} (by {}, {over} bytes over)",
                self.kind(),
                self.message(),
                by.name()
            ),
            Self::Memory { by, .. } | Self::Total { by, .. } => {
                format!("{}: {} (by {})", self.kind(), self.message(), by.name())
            }
            _ => format!("{}: {}", self.kind(), self.message()),
        }
    }
}

/// How the message of a unit stopped after its parse returned begins.
pub const STALLED_MESSAGE: &str = "parse unit stalled after its parse returned";

/// Whether a parse job's failure text is a unit stopped after its parse
/// returned.
#[must_use]
pub fn is_stalled_message(message: &str) -> bool {
    message.starts_with(STALLED_MESSAGE)
}

/// How a memory stop's message begins, for the settle path.
pub const PARSE_LIMIT_MESSAGE: &str = "parse stopped at its memory limit";

/// How the message of a parse that found no worker begins: none could be
/// started (no `pmacs-parse-unit` where the editor looks, or one from
/// another build), so the buffer is not highlighted, which the settle path
/// says once.
pub const UNAVAILABLE_MESSAGE: &str = "parse unit unavailable";

/// How the message of a parse whose worker crashed begins.
pub const CRASHED_MESSAGE: &str = "parse unit crashed";

/// How the message of the crash that ends a buffer's parsing begins: its
/// [`MAX_CRASHES`]th in a row.
pub const CRASH_STOPPED_MESSAGE: &str = "parse unit crashed; parsing stopped";

/// How the message of a parse refused while its buffer backs off after a
/// crash, or after the crash that stopped its parsing, begins.
pub const HELD_MESSAGE: &str = "parse held after its unit crashed";

/// The first back-off after a crash; each crash in a row doubles it.
pub const CRASH_BACKOFF: Duration = Duration::from_secs(1);

/// Crashes in a row, with no parse installed between them, after which a
/// buffer is not parsed again until it is killed and opened again.
pub const MAX_CRASHES: u32 = 3;

/// The settle path's status for a failure text about a crash: `"crashed"`,
/// `"crash-stopped"` (the crash that ended the buffer's parsing) or
/// `"held"` (a parse refused for an earlier crash); `None` for any other.
#[must_use]
pub fn crash_status(message: &str) -> Option<&'static str> {
    if message.starts_with(CRASH_STOPPED_MESSAGE) {
        Some("crash-stopped")
    } else if message.starts_with(CRASHED_MESSAGE) {
        Some("crashed")
    } else if message.starts_with(HELD_MESSAGE) {
        Some("held")
    } else {
        None
    }
}

/// Whether a parse job's failure text is a worker that could not start.
#[must_use]
pub fn is_unavailable_message(message: &str) -> bool {
    message.starts_with(UNAVAILABLE_MESSAGE)
}

/// Whether a parse job's failure text is a memory stop.
#[must_use]
pub fn is_limit_message(message: &str) -> bool {
    message.starts_with(PARSE_LIMIT_MESSAGE)
}

/// A buffer's unit and what the editor knows about it.
struct Slot {
    mode: Isolation,
    unit: Option<Arc<ProcessUnit>>,
    /// The unit's text mirror equals the text the editor last sent.
    synced: bool,
    deaths: u64,
    last_death: Option<Death>,
    /// Times a discarded unit's previous parse was rebuilt in a fresh one.
    reestablished: u64,
    /// Bytes of text whose spans a renderer asked the unit for after its
    /// parse, beyond what came back with the parse.
    fetched: u64,
    /// The unit's peak passed `syntax.parse-worker-recycle-mb` in its last
    /// parse: the next parse runs in a fresh worker (E7i.4).
    recycle_next: bool,
    /// The worker the last parse replaced, kept for one parse so the tree
    /// the editor shows until the new one settles is still read from it.
    retiring: Option<Arc<ProcessUnit>>,
    /// Workers replaced after a large parse.
    recycled: u64,
    /// The buffer was killed: a tree a script still holds answers nothing,
    /// rather than starting a worker for a buffer that is gone.
    forgotten: bool,
    /// Its workers' crashes since a parse of it last installed.
    crashes: u32,
    /// No worker starts for it before then: the back-off after a crash.
    backoff_until: Option<Instant>,
    /// The length of that back-off, for the report.
    backoff: Duration,
    /// How its last worker to crash died, for a parse held after it.
    crash_how: String,
    /// The same, shortest: `SIGSEGV` or `exit 101` (E7i fix round 2).
    crash_signal: String,
}

impl Slot {
    /// Why no worker may start for this buffer now, if none may: it is
    /// backing off after a crash, or it crashed [`MAX_CRASHES`] times.
    fn held(&self) -> Option<String> {
        if self.stopped() {
            return Some(format!(
                "{HELD_MESSAGE}: {}, {} crashes in a row; it is not parsed again until it is killed and opened again",
                self.crash_how, self.crashes
            ));
        }
        let until = self.backoff_until?;
        let left = until.saturating_duration_since(Instant::now());
        (!left.is_zero()).then(|| {
            format!(
                "{HELD_MESSAGE}: {}; backing off for {} ms more",
                self.crash_how,
                left.as_millis()
            )
        })
    }

    /// Its parsing stopped: [`MAX_CRASHES`] crashes in a row.
    fn stopped(&self) -> bool {
        self.crashes >= MAX_CRASHES
    }

    /// Record `death` as this buffer's last, and a crash in its streak;
    /// the message the settle path receives for it, which says what
    /// follows a crash.
    fn note(&mut self, death: &Death) -> String {
        self.deaths += 1;
        self.last_death = Some(death.clone());
        if !matches!(death, Death::Crashed { .. }) {
            return death.message();
        }
        self.crashes += 1;
        self.crash_how = death.how();
        self.crash_signal = death.signal_name();
        if self.stopped() {
            self.backoff_until = None;
            self.backoff = Duration::ZERO;
            return format!(
                "{CRASH_STOPPED_MESSAGE}: {}, {} crashes in a row; the buffer is not parsed again until it is killed and opened again",
                death.how(),
                self.crashes
            );
        }
        let backoff = CRASH_BACKOFF * 2u32.pow(self.crashes - 1);
        self.backoff_until = Some(Instant::now() + backoff);
        self.backoff = backoff;
        format!(
            "{}; the buffer is parsed again in {} s at the earliest, and not after {MAX_CRASHES} crashes in a row",
            death.message(),
            backoff.as_secs()
        )
    }
}

/// A buffer's slot, and the lock a parse holds for its whole round trip so
/// two parses of one buffer reach its unit in the order their edits were
/// taken. Reads take only the slot, briefly, and never wait on a parse.
struct SlotCell {
    slot: Mutex<Slot>,
    parsing: Mutex<()>,
}

impl SlotCell {
    fn slot(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().expect("isolation slot poisoned")
    }

    /// Forget `unit` after it died, if it is still this slot's, and record
    /// its death; the message the settle path receives for it.
    fn bury(&self, unit: &ProcessUnit, death: &Death) -> String {
        let mut slot = self.slot();
        if slot.unit.as_ref().is_some_and(|u| u.serial == unit.serial) {
            slot.unit = None;
            slot.synced = false;
            slot.note(death)
        } else {
            death.message()
        }
    }
}

/// Process-wide state: the units, the cgroup, the total watchdog, the
/// trace.
struct Host {
    slots: Mutex<HashMap<BufferId, Arc<SlotCell>>>,
    cgroup: OnceLock<CgroupAttempt>,
    /// The total's watchdog, started when no cgroup holds the workers.
    watchdog: OnceLock<Arc<TotalWatchdog>>,
    /// `syntax.parse-unit-path`: the worker binary, when not beside the
    /// editor's own.
    unit_path: Mutex<Option<PathBuf>>,
    next_serial: AtomicU64,
    trace: Option<Mutex<std::fs::File>>,
    started: Instant,
}

static HOST_CELL: OnceLock<Host> = OnceLock::new();

fn host() -> &'static Host {
    HOST_CELL.get_or_init(|| Host {
        slots: Mutex::new(HashMap::new()),
        cgroup: OnceLock::new(),
        watchdog: OnceLock::new(),
        unit_path: Mutex::new(None),
        next_serial: AtomicU64::new(1),
        trace: std::env::var_os("PMACS_E7I_TRACE").and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(Mutex::new)
        }),
        started: Instant::now(),
    })
}

/// A measurement hook, not a setting: `PMACS_PARSE_UNIT_MEMORY=watch`
/// holds each worker with its memory watch even where `RLIMIT_AS` is
/// granted, and `PMACS_PARSE_UNIT_CGROUP=off` holds the total with the
/// watchdog even where a cgroup could be made, so a Linux machine exercises
/// what macOS and an undelegated Linux get (E7i's condition 3).
fn hook(name: &str, value: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == value)
}

impl Host {
    fn cell(&self, buffer: BufferId, mode: Isolation) -> Arc<SlotCell> {
        self.slots
            .lock()
            .expect("isolation slots poisoned")
            .entry(buffer)
            .or_insert_with(|| {
                Arc::new(SlotCell {
                    slot: Mutex::new(Slot {
                        mode,
                        unit: None,
                        synced: false,
                        deaths: 0,
                        last_death: None,
                        reestablished: 0,
                        fetched: 0,
                        recycle_next: false,
                        retiring: None,
                        recycled: 0,
                        forgotten: false,
                        crashes: 0,
                        backoff_until: None,
                        backoff: Duration::ZERO,
                        crash_how: String::new(),
                        crash_signal: String::new(),
                    }),
                    parsing: Mutex::new(()),
                })
            })
            .clone()
    }

    fn trace(&self, fields: &[(&str, String)]) {
        let Some(file) = self.trace.as_ref() else {
            return;
        };
        let mut line = format!(
            "{{\"t_ms\":{:.3}",
            self.started.elapsed().as_secs_f64() * 1000.0
        );
        for (key, value) in fields {
            use std::fmt::Write as _;
            let _ = write!(line, ",\"{key}\":{value}");
        }
        line.push_str("}\n");
        if let Ok(mut f) = file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// Start a worker under `limits`, in the workers' cgroup when there is
    /// one and watched by the total's watchdog when there is not.
    fn spawn(&self, limits: &Limits) -> Result<Arc<ProcessUnit>, Death> {
        if limits.mode != Isolation::Process {
            return Err(Death::Unavailable("native mode has no unit".into()));
        }
        let cgroup = self
            .cgroup
            .get_or_init(|| {
                if hook("PMACS_PARSE_UNIT_CGROUP", "off") {
                    CgroupAttempt {
                        cgroup: None,
                        report: "off: PMACS_PARSE_UNIT_CGROUP=off".into(),
                    }
                } else {
                    Cgroup::create(limits.total_memory)
                }
            })
            .cgroup
            .as_ref();
        // Every worker reports its size while it parses when there is a
        // total, so the watchdog can hold it for any worker the cgroup does
        // not: all of them where none was made, or one the kernel refused
        // to move into it.
        let serial = self.next_serial.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let unit = ProcessUnit::start(serial, limits, cgroup, limits.total_memory > 0)?;
        if limits.total_memory > 0 && !unit.held_by_cgroup {
            self.watchdog
                .get_or_init(|| TotalWatchdog::start(limits.total_memory));
        }
        self.trace(&[
            ("event", json_str("spawn")),
            ("mode", json_str(limits.mode.name())),
            ("unit", json_str(&unit.id())),
            ("spawn_us", started.elapsed().as_micros().to_string()),
        ]);
        Ok(Arc::new(unit))
    }

    /// Every live unit, retiring ones included, for the watchdog and the
    /// steady-state figure.
    fn units(&self) -> Vec<Arc<ProcessUnit>> {
        let cells: Vec<Arc<SlotCell>> = self
            .slots
            .lock()
            .expect("isolation slots poisoned")
            .values()
            .cloned()
            .collect();
        cells
            .iter()
            .flat_map(|c| {
                let slot = c.slot();
                [slot.unit.clone(), slot.retiring.clone()]
            })
            .flatten()
            .collect()
    }
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Where to find the worker binary: `syntax.parse-unit-path`, empty for
/// beside the running editor (`pmacs-parse-unit`, as the release ships
/// it). Set by every dispatch; read when a worker starts.
pub fn set_unit_path(path: Option<&str>) {
    let path = path.filter(|p| !p.is_empty()).map(PathBuf::from);
    *host().unit_path.lock().expect("unit path poisoned") = path;
}

/// Run one isolated parse on the calling (worker) thread and build the
/// bundle the settle path installs. `Err` carries the message the
/// in-process path would carry, or a [`Death`]'s.
pub fn run(job: &IsolatedJob) -> Result<ParseTreeBundle, String> {
    let host = host();
    let cell = host.cell(job.buffer, job.limits.mode);
    let _in_order = cell.parsing.lock().expect("parse order poisoned");
    let hard = job.limits.deadline.map(|d| d + HARD_GRACE);
    for attempt in 1..=2 {
        let (unit, full) = unit_for(host, &cell, job, attempt == 1)?;
        let (call, payload) = parse_call(job, full);
        let started = Instant::now();
        let answer = unit.call(&Request::Parse(call), &payload, hard);
        let elapsed = started.elapsed();
        match answer {
            Ok(Response::Parsed(parsed)) => {
                {
                    let mut slot = cell.slot();
                    slot.synced = true;
                    slot.recycle_next =
                        job.limits.recycle > 0 && parsed.peak_bytes > job.limits.recycle;
                    // A parse installed: the buffer's crash streak ends.
                    slot.crashes = 0;
                    slot.backoff_until = None;
                    slot.backoff = Duration::ZERO;
                }
                host.trace(&[
                    ("event", json_str("parsed")),
                    ("mode", json_str(job.limits.mode.name())),
                    ("unit", json_str(&unit.id())),
                    ("buffer", job.buffer.raw().to_string()),
                    ("full", full.to_string()),
                    ("bytes", job.request.source.len().to_string()),
                    ("layers", parsed.layers.len().to_string()),
                    ("root_us", parsed.root_parse_us.to_string()),
                    ("unit_us", parsed.total_us.to_string()),
                    ("call_us", elapsed.as_micros().to_string()),
                    ("peak", parsed.peak_bytes.to_string()),
                ]);
                return Ok(bundle_from(job, parsed, &cell, &unit));
            }
            Ok(Response::Failed(Failure::Desync { have, want })) if attempt == 1 => {
                host.trace(&[
                    ("event", json_str("desync")),
                    ("buffer", job.buffer.raw().to_string()),
                    ("have", have.to_string()),
                    ("want", want.to_string()),
                ]);
                cell.slot().synced = false;
            }
            Ok(Response::Failed(failure)) => {
                // The unit applied the text and is alive; its tree stays the
                // previous one and it parses cold next, as in-process.
                cell.slot().synced = !matches!(failure, Failure::Desync { .. });
                host.trace(&[
                    ("event", json_str("failed")),
                    ("unit", json_str(&unit.id())),
                    ("buffer", job.buffer.raw().to_string()),
                    ("failure", json_str(&format!("{failure:?}"))),
                    ("call_us", elapsed.as_micros().to_string()),
                ]);
                return Err(failure_message(&failure));
            }
            Ok(other) => {
                let death = Death::Ended(format!("answered out of turn: {other:?}"));
                unit.kill_for(death.clone());
                return Err(cell.bury(&unit, &death));
            }
            Err(death) => {
                // The worker is gone and its memory with it.
                let message = cell.bury(&unit, &death);
                host.trace(&[
                    ("event", json_str("death")),
                    ("unit", json_str(&unit.id())),
                    ("buffer", job.buffer.raw().to_string()),
                    ("kind", json_str(death.kind())),
                    ("message", json_str(&death.describe())),
                    ("call_us", elapsed.as_micros().to_string()),
                    ("resident", unit.resident().to_string()),
                ]);
                return Err(message);
            }
        }
    }
    Err(failure_message(&Failure::Desync { have: 0, want: 0 }))
}

/// The worker this parse goes to, and whether it needs the whole text:
/// the buffer's, a fresh one when it has none, or, on a parse's `first`
/// attempt after one that passed the recycle threshold, a fresh one while
/// the old one retires (E7i.4).
fn unit_for(
    host: &Host,
    cell: &SlotCell,
    job: &IsolatedJob,
    first: bool,
) -> Result<(Arc<ProcessUnit>, bool), String> {
    let mut slot = cell.slot();
    if first {
        // The worker the previous parse replaced has served the tree
        // the editor showed until then; this parse replaces this
        // slot's worker instead if its last parse passed the
        // threshold (E7i.4).
        slot.retiring = if slot.recycle_next {
            slot.recycle_next = false;
            slot.synced = false;
            slot.recycled += 1;
            host.trace(&[
                ("event", json_str("recycle")),
                ("buffer", job.buffer.raw().to_string()),
            ]);
            slot.unit.take()
        } else {
            None
        };
    }
    if slot.mode != job.limits.mode {
        slot.unit = None;
        slot.mode = job.limits.mode;
        slot.synced = false;
    }
    if slot.unit.is_none() {
        // After a crash the buffer's next worker waits out its back-off,
        // and after the last one none starts: every start runs the bytes
        // that crashed the last.
        if let Some(held) = slot.held() {
            return Err(held);
        }
        slot.unit = Some(host.spawn(&job.limits).map_err(|death| death.message())?);
        slot.synced = false;
    }
    Ok((slot.unit.clone().expect("started above"), !slot.synced))
}

fn failure_message(failure: &Failure) -> String {
    match failure {
        Failure::Deadline {
            deadline_ms,
            after_ms,
        } => ParseError::DeadlineExceeded {
            deadline: Duration::from_millis(*deadline_ms),
            after: Duration::from_millis(*after_ms),
        }
        .to_string(),
        Failure::NoTree => ParseError::NoTree.to_string(),
        Failure::Language(m) => ParseError::Language(m.clone()).to_string(),
        other => format!("parse unit: {other:?}"),
    }
}

fn parse_call(job: &IsolatedJob, full: bool) -> (ParseCall, Vec<u8>) {
    let req = &job.request;
    let (text, edits, payload) = if full {
        (TextUpdate::Full, Vec::new(), req.source.to_vec())
    } else {
        (
            TextUpdate::Edits,
            req.edits.iter().map(WireEdit::from_input_edit).collect(),
            job.inserted.concat(),
        )
    };
    let call = ParseCall {
        language: req.language_name.clone(),
        text,
        edits,
        expect_len: req.source.len() as u32,
        aliases: req
            .injection_aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        deadline_ms: req.deadline.map(|d| d.as_millis() as u64),
        interest: job.interest.clone(),
    };
    (call, payload)
}

fn spans_from(set: SpanSet) -> IsolatedSpans {
    IsolatedSpans {
        covered: set.covered,
        layers: set
            .layers
            .into_iter()
            .map(|l| IsolatedLayerSpans {
                layer: l.layer as usize,
                capture_names: Arc::from(Vec::<String>::new()),
                spans: l
                    .spans
                    .into_iter()
                    .map(|(start_byte, end_byte, capture_index)| HighlightSpan {
                        start_byte,
                        end_byte,
                        capture_index,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// [`spans_from`] with each layer's capture names attached, which needs
/// the layer languages the parse reported.
fn spans_with_names(set: SpanSet, languages: &[String]) -> IsolatedSpans {
    let names: HashMap<String, Arc<[String]>> = set
        .capture_names
        .iter()
        .map(|(language, names)| (language.clone(), Arc::from(names.clone())))
        .collect();
    let mut spans = spans_from(set);
    for layer in &mut spans.layers {
        if let Some(names) = languages.get(layer.layer).and_then(|l| names.get(l)) {
            layer.capture_names = names.clone();
        }
    }
    spans
}

fn bundle_from(
    job: &IsolatedJob,
    parsed: pmacs_parse_unit::Parsed,
    cell: &Arc<SlotCell>,
    unit: &ProcessUnit,
) -> ParseTreeBundle {
    let languages: Vec<String> = parsed.layers.iter().map(|l| l.language.clone()).collect();
    let spans = spans_with_names(parsed.spans, &languages);
    ParseTreeBundle {
        layers: Vec::new(),
        source: job.request.source.clone(),
        language_name: job.request.language_name.clone(),
        parse_duration: Duration::from_micros(parsed.root_parse_us),
        injection_capped: parsed.injection_capped,
        layers_cut_by_deadline: parsed.layers_cut_by_deadline,
        isolated: Some(Arc::new(IsolatedHandle {
            cell: cell.clone(),
            buffer: job.buffer,
            unit: AtomicU64::new(unit.serial),
            generation: AtomicU64::new(parsed.generation),
            source: job.request.source.clone(),
            language: job.request.language_name.clone(),
            aliases: job
                .request
                .injection_aliases
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            limits: job.limits,
            languages: Mutex::new(languages),
            pieces: Mutex::new(vec![Arc::new(spans)]),
        })),
    }
}

/// The editor's hold on a parse whose tree lives in a unit: its text, the
/// spans fetched so far, and which unit holds the tree under which
/// generation.
pub struct IsolatedHandle {
    cell: Arc<SlotCell>,
    buffer: BufferId,
    /// The serial of the unit holding the tree.
    unit: AtomicU64,
    /// The tree's generation in that unit.
    generation: AtomicU64,
    source: Arc<[u8]>,
    language: String,
    aliases: Vec<(String, String)>,
    limits: Limits,
    languages: Mutex<Vec<String>>,
    pieces: Mutex<Vec<Arc<IsolatedSpans>>>,
}

impl std::fmt::Debug for IsolatedHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IsolatedHandle")
            .field("buffer", &self.buffer)
            .field("unit", &self.unit.load(Ordering::Relaxed))
            .field("generation", &self.generation.load(Ordering::Relaxed))
            .field("bytes", &self.source.len())
            .finish_non_exhaustive()
    }
}

/// How many fetched span sets a handle keeps (the parse's own and the
/// most recent fetches).
const MAX_PIECES: usize = 8;

/// How long a read may take before its unit is discarded.
const READ_HARD: Duration = Duration::from_secs(2);

impl IsolatedTree for IsolatedHandle {
    fn spans_for(&self, range: std::ops::Range<usize>) -> Option<Arc<IsolatedSpans>> {
        let range = range.start.min(self.source.len())..range.end.min(self.source.len());
        if let Some(piece) = self
            .pieces
            .lock()
            .expect("span pieces poisoned")
            .iter()
            .find(|p| p.covers(&range))
        {
            return Some(piece.clone());
        }
        self.cell.slot().fetched += (range.end - range.start) as u64;
        let want = vec![(range.start as u32, range.end as u32)];
        let Response::Spans(fetched) = self.ask(|generation| Request::Spans {
            generation,
            ranges: want.clone(),
        })?
        else {
            return None;
        };
        let languages = self.languages.lock().expect("languages poisoned").clone();
        let piece = Arc::new(spans_with_names(fetched, &languages));
        let mut pieces = self.pieces.lock().expect("span pieces poisoned");
        if pieces.len() >= MAX_PIECES {
            pieces.remove(1);
        }
        pieces.push(piece.clone());
        Some(piece)
    }

    fn layer_languages(&self) -> Vec<String> {
        self.languages.lock().expect("languages poisoned").clone()
    }

    fn fold_candidates(&self, pos: u64) -> Option<Vec<(u64, u64)>> {
        match self.ask(|generation| Request::Folds {
            generation,
            at: Some(pos),
        })? {
            Response::Folds(ranges) => Some(ranges),
            _ => None,
        }
    }

    fn top_level_folds(&self) -> Option<Vec<(u64, u64)>> {
        match self.ask(|generation| Request::Folds {
            generation,
            at: None,
        })? {
            Response::Folds(ranges) => Some(ranges),
            _ => None,
        }
    }

    fn describe(&self, path: &[u32], children: bool) -> Option<Vec<NodeFacts>> {
        let request = |generation| Request::Describe {
            generation,
            path: path.to_vec(),
            children,
        };
        match self.ask(request)? {
            Response::Nodes(nodes) => Some(nodes.into_iter().map(NodeFacts::from).collect()),
            _ => None,
        }
    }

    fn sexp(&self, path: &[u32]) -> Option<String> {
        match self.ask(|generation| Request::Sexp {
            generation,
            path: path.to_vec(),
        })? {
            Response::Text(text) => text,
            _ => None,
        }
    }
}

impl IsolatedHandle {
    /// Ask this tree's unit the read `request` builds for the tree's
    /// generation: the unit holding it, answering while it parses a newer
    /// text, or, when that unit was discarded, a fresh one this tree's text
    /// is re-parsed into. `None` when a newer unit or tree has replaced
    /// this one, or the unit cannot answer.
    fn ask(&self, request: impl Fn(u64) -> Request) -> Option<Response> {
        let unit = {
            let slot = self.cell.slot();
            if slot.forgotten {
                return None;
            }
            let serial = self.unit.load(Ordering::Relaxed);
            let holding = [slot.unit.as_ref(), slot.retiring.as_ref()]
                .into_iter()
                .flatten()
                .find(|u| u.serial == serial);
            match (holding, slot.unit.as_ref()) {
                (Some(unit), _) => Some(unit.clone()),
                (None, Some(_)) => return None,
                (None, None) => None,
            }
        };
        let unit = match unit {
            Some(unit) => unit,
            None => self.reestablish()?,
        };
        let request = request(self.generation.load(Ordering::Relaxed));
        match unit.call(&request, &[], Some(READ_HARD)) {
            Ok(Response::Failed(_)) => None,
            Ok(response) => Some(response),
            Err(death) => {
                let _ = self.cell.bury(&unit, &death);
                None
            }
        }
    }

    /// The unit that held this tree was discarded: start a fresh one and
    /// parse this tree's text into it, which rebuilds the tree there. Not
    /// while a parse of the buffer runs, which will install a newer tree.
    /// It runs on the calling thread, the main one for every reader but the
    /// settle path's, so the read that triggers it waits a cold parse (the
    /// module doc says how long).
    fn reestablish(&self) -> Option<Arc<ProcessUnit>> {
        let _in_order = self.cell.parsing.try_lock().ok()?;
        {
            let slot = self.cell.slot();
            // Not while a worker holds the buffer, nor once its crashes
            // stopped its parsing. The back-off does not hold this re-parse:
            // the back-off keeps a crash's bytes from running again, and this
            // is the previous text, which parsed; if it crashes too, that
            // counts toward the stop like any crash.
            if slot.unit.is_some() || slot.stopped() {
                return None;
            }
        }
        let host = host();
        let started = Instant::now();
        let unit = host.spawn(&self.limits).ok()?;
        let call = ParseCall {
            language: self.language.clone(),
            text: TextUpdate::Full,
            edits: Vec::new(),
            expect_len: self.source.len() as u32,
            aliases: self.aliases.clone(),
            deadline_ms: self.limits.deadline.map(|d| d.as_millis() as u64),
            interest: Vec::new(),
        };
        let hard = self.limits.deadline.map(|d| d + HARD_GRACE);
        match unit.call(&Request::Parse(call), &self.source, hard) {
            Ok(Response::Parsed(parsed)) => {
                {
                    let mut slot = self.cell.slot();
                    slot.unit = Some(unit.clone());
                    // The unit's mirror is this older text, not the buffer's.
                    slot.synced = false;
                    slot.reestablished += 1;
                }
                self.unit.store(unit.serial, Ordering::Relaxed);
                self.generation.store(parsed.generation, Ordering::Relaxed);
                *self.languages.lock().expect("languages poisoned") =
                    parsed.layers.iter().map(|l| l.language.clone()).collect();
                host.trace(&[
                    ("event", json_str("reestablished")),
                    ("buffer", self.buffer.raw().to_string()),
                    ("bytes", self.source.len().to_string()),
                    ("us", started.elapsed().as_micros().to_string()),
                ]);
                Some(unit)
            }
            Err(death) => {
                // Recorded as any death is, so a text that crashes its
                // worker backs off here too.
                let _ = self.cell.slot().note(&death);
                None
            }
            Ok(_) => None,
        }
    }
}

/// A buffer was killed: end its worker, and any it is retiring, and forget
/// its slot, so a long-lived editor does not keep a worker per buffer it
/// ever opened. A parse of it still in flight fails as for a dead worker.
pub fn forget(buffer: BufferId) {
    let Some(host) = HOST_CELL.get() else {
        return;
    };
    let cell = host
        .slots
        .lock()
        .expect("isolation slots poisoned")
        .remove(&buffer);
    let Some(cell) = cell else {
        return;
    };
    let mut slot = cell.slot();
    slot.forgotten = true;
    for unit in [slot.unit.take(), slot.retiring.take()]
        .into_iter()
        .flatten()
    {
        unit.kill_for(Death::Ended("its buffer was killed".into()));
    }
}

/// End every unit (each worker killed and reaped) and remove the workers'
/// cgroup. The daemon calls this as it stops.
pub fn shutdown() {
    let Some(host) = HOST_CELL.get() else {
        return;
    };
    let cells: Vec<Arc<SlotCell>> = host
        .slots
        .lock()
        .expect("isolation slots poisoned")
        .drain()
        .map(|(_, c)| c)
        .collect();
    for cell in cells {
        let mut slot = cell.slot();
        for unit in [slot.unit.take(), slot.retiring.take()]
            .into_iter()
            .flatten()
        {
            unit.kill_for(Death::Ended("the editor stopped".into()));
        }
    }
    if let Some(Some(cgroup)) = host.cgroup.get().map(|a| a.cgroup.as_ref()) {
        let _ = std::fs::remove_dir(&cgroup.dir);
    }
}

/// What the editor found about holding its workers to a total: how the
/// cgroup attempt went (created, or the step that refused and why) and,
/// when created, how many workers the kernel moved into it; otherwise
/// whether the watchdog holds the total. `None` until a worker has
/// started, since the attempt is made then.
#[must_use]
pub fn isolation_report() -> Option<String> {
    let host = host();
    let attempt = host.cgroup.get()?;
    let mut report = format!("cgroup {}", attempt.report);
    if let Some(cgroup) = attempt.cgroup.as_ref() {
        report.push_str("; ");
        report.push_str(&cgroup.adoption());
    }
    if let Some(watchdog) = host.watchdog.get() {
        report.push_str("; the watchdog holds the total (");
        report.push_str(&watchdog.summary());
        report.push(')');
    }
    Some(report)
}

/// Trace an in-process parse (`syntax.isolation` none) beside the units'
/// own events, when `PMACS_E7I_TRACE` is set: `unit_us` is the parse's
/// whole time on the worker thread, as a unit's is inside the unit.
pub fn trace_native_parse(bytes: usize, layers: usize, root: Duration, total: Duration) {
    let host = host();
    if host.trace.is_none() {
        return;
    }
    host.trace(&[
        ("event", json_str("parsed")),
        ("mode", json_str("none")),
        ("bytes", bytes.to_string()),
        ("layers", layers.to_string()),
        ("root_us", root.as_micros().to_string()),
        ("unit_us", total.as_micros().to_string()),
    ]);
}

/// What the editor can say about a buffer's unit (for Lua and the trace).
#[derive(Clone, Debug, Default)]
pub struct UnitReport {
    /// `"none"` or `"process"`.
    pub mode: String,
    /// The worker's pid; empty when none runs.
    pub unit: String,
    /// What the unit holds now, bytes, when it runs.
    pub memory: Option<u64>,
    /// Units this buffer has had discarded.
    pub deaths: u64,
    /// Why the last one was, `kind: message`, with a memory stop's
    /// enforcer.
    pub last_death: Option<String>,
    /// A parse of the buffer is in flight.
    pub busy: bool,
    /// Times a discarded unit's previous parse was rebuilt in a fresh one.
    pub reestablished: u64,
    /// Bytes of text whose spans renderers asked the unit for beyond what
    /// came back with each parse.
    pub fetched: u64,
    /// Workers replaced after a large parse (E7i.4).
    pub recycled: u64,
    /// Its workers' crashes since a parse of it last installed.
    pub crashes: u32,
    /// Why no worker may start for it now, if none may.
    pub held: Option<String>,
    /// Its parsing stopped: [`MAX_CRASHES`] crashes in a row.
    pub stopped: bool,
    /// The back-off its last crash set, ms; 0 when none holds it.
    pub backoff_ms: u64,
    /// What is left of that back-off, ms.
    pub backoff_left_ms: u64,
    /// How its last worker to crash died: `signal 11 (SIGSEGV)`, with its
    /// last words when it left any.
    pub crash_how: String,
    /// The same, shortest: `SIGSEGV` or `exit 101`.
    pub crash_signal: String,
}

/// The report for `buffer`'s unit, if it has had one. Never waits on a
/// parse: the slot is held only briefly.
#[must_use]
pub fn report(buffer: BufferId) -> Option<UnitReport> {
    let cell = host()
        .slots
        .lock()
        .expect("isolation slots poisoned")
        .get(&buffer)?
        .clone();
    let busy = cell.parsing.try_lock().is_err();
    let slot = cell.slot();
    Some(UnitReport {
        busy,
        reestablished: slot.reestablished,
        fetched: slot.fetched,
        recycled: slot.recycled,
        mode: slot.mode.name().to_owned(),
        unit: slot.unit.as_ref().map(|u| u.id()).unwrap_or_default(),
        memory: slot.unit.as_ref().and_then(|u| u.memory_bytes()),
        deaths: slot.deaths,
        last_death: slot.last_death.as_ref().map(Death::describe),
        crashes: slot.crashes,
        held: slot.held(),
        stopped: slot.stopped(),
        backoff_ms: slot.backoff.as_millis() as u64,
        backoff_left_ms: slot.backoff_until.map_or(0, |until| {
            until.saturating_duration_since(Instant::now()).as_millis() as u64
        }),
        crash_how: slot.crash_how.clone(),
        crash_signal: slot.crash_signal.clone(),
    })
}

/// The editor-wide memory the units hold now, their PSS summed. For the
/// comparison's steady-state figure.
#[must_use]
pub fn total_unit_memory() -> u64 {
    host().units().iter().filter_map(|u| u.memory_bytes()).sum()
}

// ---------------------------------------------------------------------------
// The worker process
// ---------------------------------------------------------------------------

/// One frame read from a worker, or why none came.
type FrameRead = io::Result<Option<(Response, Vec<u8>)>>;

/// The requests a worker has not yet answered, by id; closed once its
/// output ends, so a request sent after that fails at once.
#[derive(Default)]
struct Waiting {
    by_id: HashMap<u64, mpsc::Sender<FrameRead>>,
    closed: bool,
}

/// A worker process running the unit. Shared: a parse on a pool thread and
/// reads on the main thread send requests at once, each answered by id.
struct ProcessUnit {
    serial: u64,
    /// The kernel moved it into the workers' cgroup.
    held_by_cgroup: bool,
    pid: u32,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    waiting: Arc<Mutex<Waiting>>,
    next_id: AtomicU64,
    /// The resident size the worker last reported, bytes (0 before any).
    resident: Arc<AtomicU64>,
    /// A parse is in flight.
    parsing: AtomicBool,
    stderr: Arc<Mutex<String>>,
    limits: Limits,
    cgroup: Option<Cgroup>,
    oom_before: u64,
    /// Why the editor killed it, when it did.
    killed: Mutex<Option<Death>>,
}

impl ProcessUnit {
    fn start(
        serial: u64,
        limits: &Limits,
        cgroup: Option<&Cgroup>,
        report_memory: bool,
    ) -> Result<Self, Death> {
        let configured = host().unit_path.lock().expect("unit path poisoned").clone();
        let bin = match configured {
            Some(path) if path.exists() => path,
            Some(path) => {
                return Err(Death::Unavailable(format!(
                    "syntax.parse-unit-path {} does not exist",
                    path.display()
                )));
            }
            None => sibling("pmacs-parse-unit").ok_or_else(|| {
                Death::Unavailable(format!(
                    "no pmacs-parse-unit beside pmacs{}",
                    std::env::current_exe()
                        .ok()
                        .and_then(|exe| exe.parent().map(|d| format!(" in {}", d.display())))
                        .unwrap_or_default()
                ))
            })?,
        };
        let mut command = Command::new(&bin);
        command
            .envs(pmacs_parse_unit::WORKER_ENV.iter().copied())
            .arg("--memory-limit-mb")
            .arg((limits.unit_memory >> 20).max(1).to_string());
        if hook("PMACS_PARSE_UNIT_MEMORY", "watch") {
            command.args(["--memory-enforcement", "watch"]);
        }
        if report_memory {
            command.arg("--report-memory");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Death::Unavailable(format!("{}: {e}", bin.display())))?;
        let held_by_cgroup = cgroup.is_some_and(|cg| cg.adopt(child.id()));
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let waiting = Arc::new(Mutex::new(Waiting::default()));
        let resident = Arc::new(AtomicU64::new(0));
        {
            let waiting = waiting.clone();
            let resident = resident.clone();
            thread::Builder::new()
                .name("pmacs-unit-read".into())
                .spawn(move || route_answers(stdout, &waiting, &resident))
                .map_err(|e| Death::Unavailable(e.to_string()))?;
        }
        let stderr = Arc::new(Mutex::new(String::new()));
        {
            let sink = stderr.clone();
            thread::Builder::new()
                .name("pmacs-unit-err".into())
                .spawn(move || keep_last_words(stderr_pipe, &sink))
                .map_err(|e| Death::Unavailable(e.to_string()))?;
        }
        let cgroup = cgroup.cloned();
        let oom_before = cgroup.as_ref().map_or(0, Cgroup::oom_kills);
        let unit = Self {
            serial,
            held_by_cgroup,
            pid: child.id(),
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            waiting,
            next_id: AtomicU64::new(1),
            resident,
            parsing: AtomicBool::new(false),
            stderr,
            limits: *limits,
            cgroup,
            oom_before,
            killed: Mutex::new(None),
        };
        // A worker from another build speaks another protocol: refuse it
        // rather than misread its frames.
        match unit.call(&Request::Hello, &[], Some(READ_HARD)) {
            Ok(Response::Hello { protocol }) if protocol == pmacs_parse_unit::PROTOCOL => Ok(unit),
            Ok(other) => Err(Death::Unavailable(format!(
                "{} speaks another protocol ({other:?}); pmacs needs {}; install the two from one build",
                bin.display(),
                pmacs_parse_unit::PROTOCOL
            ))),
            Err(death) => Err(Death::Unavailable(format!(
                "{} did not answer: {}",
                bin.display(),
                death.message()
            ))),
        }
    }

    /// One request and its answer, the worker killed if it has not answered
    /// within `hard`. Other requests may be in flight at once.
    fn call(
        &self,
        request: &Request,
        payload: &[u8],
        hard: Option<Duration>,
    ) -> Result<Response, Death> {
        let started = Instant::now();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let mut waiting = self.waiting.lock().expect("waiting poisoned");
            if waiting.closed {
                drop(waiting);
                return Err(self.death());
            }
            waiting.by_id.insert(id, tx);
        }
        let parse = matches!(request, Request::Parse(_));
        if parse {
            self.parsing.store(true, Ordering::Relaxed);
            if let Some(watchdog) = host().watchdog.get() {
                watchdog.wake();
            }
        }
        let written = {
            let mut stdin = self.stdin.lock().expect("stdin poisoned");
            pmacs_parse_unit::write_frame(&mut *stdin, &(id, request), payload)
        };
        let answer = if written.is_err() {
            Err(self.death())
        } else {
            // The bound is the deadline until the parse returns, and
            // `AFTER_PARSE_HARD` from then on (`Response::ParseReturned`);
            // a timeout says which it was, and a read's says it was a read.
            let mut until = hard.map(|limit| started + limit);
            let mut returned = false;
            loop {
                let frame = match until {
                    Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                    None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                };
                match frame {
                    Ok(Ok(Some((Response::ParseReturned(_), _)))) => {
                        returned = true;
                        until = until.map(|_| Instant::now() + AFTER_PARSE_HARD);
                    }
                    Ok(Ok(Some((response, _)))) => break Ok(response),
                    Err(RecvTimeoutError::Timeout) => {
                        let after = started.elapsed();
                        let death = if !parse {
                            Death::Read {
                                limit: hard.unwrap_or_default(),
                                after,
                            }
                        } else if returned {
                            Death::Stalled { after }
                        } else {
                            Death::Time {
                                deadline: hard.unwrap_or_default().saturating_sub(HARD_GRACE),
                                after,
                            }
                        };
                        self.kill_for(death.clone());
                        break Err(death);
                    }
                    _ => break Err(self.death()),
                }
            }
        };
        if parse {
            self.parsing.store(false, Ordering::Relaxed);
        }
        self.waiting
            .lock()
            .expect("waiting poisoned")
            .by_id
            .remove(&id);
        answer
    }

    /// Kill the worker for `why`, which every waiting request then reports.
    fn kill_for(&self, why: Death) {
        self.killed
            .lock()
            .expect("kill reason poisoned")
            .get_or_insert(why);
        let mut child = self.child.lock().expect("child poisoned");
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Why the worker ended: the editor's own reason when it killed it,
    /// else read from its exit and its last words. The memory watch exits
    /// with [`pmacs_parse_unit::MEMORY_WATCH_EXIT`]; the cgroup's OOM killer
    /// sends `SIGKILL`; a refused allocation under `RLIMIT_AS` makes
    /// tree-sitter's or Rust's allocator abort (`SIGABRT`) after saying so.
    fn death(&self) -> Death {
        if let Some(why) = self.killed.lock().expect("kill reason poisoned").clone() {
            return why;
        }
        let status = self.child.lock().expect("child poisoned").wait();
        let code = status
            .as_ref()
            .ok()
            .and_then(std::process::ExitStatus::code);
        let signal = status
            .as_ref()
            .ok()
            .and_then(std::os::unix::process::ExitStatusExt::signal);
        let stderr = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        if code == Some(pmacs_parse_unit::MEMORY_WATCH_EXIT) {
            return Death::Memory {
                limit: self.limits.unit_memory,
                by: Enforcer::Watch,
                overshoot: watch_overshoot(&stderr),
            };
        }
        let oom_now = self.cgroup.as_ref().map_or(0, Cgroup::oom_kills);
        if signal == Some(9) && oom_now > self.oom_before {
            return Death::Total {
                limit: self.limits.total_memory,
                by: Enforcer::Cgroup,
            };
        }
        if stderr.contains("failed to allocate")
            || stderr.contains("failed to reallocate")
            || stderr.contains("memory allocation of")
        {
            return Death::Memory {
                limit: self.limits.unit_memory,
                by: Enforcer::Rlimit,
                overshoot: None,
            };
        }
        // Neither the editor's kill nor a limit: the worker died on its
        // own.
        Death::Crashed {
            signal,
            code,
            last: stderr.lines().last().unwrap_or("").to_owned(),
        }
    }

    /// What the worker holds now: its PSS where `/proc` has it, else the
    /// resident size it last reported.
    fn memory_bytes(&self) -> Option<u64> {
        pss_bytes(self.pid).or_else(|| Some(self.resident()).filter(|&r| r > 0))
    }

    fn resident(&self) -> u64 {
        self.resident.load(Ordering::Relaxed)
    }

    fn id(&self) -> String {
        format!("pid {}", self.pid)
    }
}

impl Drop for ProcessUnit {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Route each of a worker's answers to the request that waits for it, and
/// keep its resident reports; at the end of its output, fail every request
/// still waiting.
fn route_answers(
    stdout: std::process::ChildStdout,
    waiting: &Mutex<Waiting>,
    resident: &AtomicU64,
) {
    let mut r = BufReader::new(stdout);
    loop {
        match pmacs_parse_unit::read_frame::<_, (u64, Response)>(&mut r) {
            Ok(Some(((0, Response::Resident(bytes)), _))) => {
                resident.store(bytes, Ordering::Relaxed);
            }
            Ok(Some(((0, Response::ParseReturned(id)), _))) => {
                // Not the answer: the request keeps waiting, under a new bound.
                let waiting = waiting.lock().expect("waiting poisoned");
                if let Some(tx) = waiting.by_id.get(&id) {
                    let _ = tx.send(Ok(Some((Response::ParseReturned(id), Vec::new()))));
                }
            }
            Ok(Some(((id, response), payload))) => {
                let tx = waiting.lock().expect("waiting poisoned").by_id.remove(&id);
                if let Some(tx) = tx {
                    let _ = tx.send(Ok(Some((response, payload))));
                }
            }
            _ => {
                let mut waiting = waiting.lock().expect("waiting poisoned");
                waiting.closed = true;
                waiting.by_id.clear();
                return;
            }
        }
    }
}

/// How far past its limit a worker's peak was when its memory watch
/// stopped it, from its last words (`peak resident P bytes passed the limit
/// L`).
fn watch_overshoot(stderr: &str) -> Option<u64> {
    let line = stderr.lines().rev().find(|l| l.contains("memory watch:"))?;
    let number = |after: &str| -> Option<u64> {
        line.split(after)
            .nth(1)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    Some(number("peak resident ")?.saturating_sub(number("the limit ")?))
}

/// Keep the last of a worker's stderr, which says why it ended.
fn keep_last_words(pipe: std::process::ChildStderr, sink: &Mutex<String>) {
    let mut r = BufReader::new(pipe);
    let mut buf = [0u8; 4096];
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        if let Ok(mut s) = sink.lock() {
            s.push_str(&String::from_utf8_lossy(&buf[..n]));
            if s.len() > 16_384 {
                let cut = s.len() - 8_192;
                s.drain(..cut);
            }
        }
    }
}

/// A process's PSS from `/proc/<pid>/smaps_rollup`, bytes.
#[must_use]
pub fn pss_bytes(pid: u32) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    text.lines()
        .find(|l| l.starts_with("Pss:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

/// A binary beside the running one, as `pmacs --gpu` finds `pmacs-gpu`;
/// from a test binary in cargo's `deps/`, the one beside `deps/`, where
/// `cargo build` puts the workspace's binaries, so an editor a test runs
/// in-process finds its workers as the daemon does.
fn sibling(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let here = dir.join(name);
    if here.exists() {
        return Some(here);
    }
    if dir.file_name()? == "deps" {
        let up = dir.parent()?.join(name);
        if up.exists() {
            return Some(up);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The total, where no cgroup holds the workers
// ---------------------------------------------------------------------------

/// How often the total's watchdog sums the workers' memory while any parses.
pub const TOTAL_WATCH_EVERY: Duration = Duration::from_millis(10);

/// The editor's watchdog over the workers' total where the kernel cannot
/// hold it (macOS, a Linux session without a delegated cgroup subtree):
/// while any worker parses, every [`TOTAL_WATCH_EVERY`] it sums the
/// resident sizes the workers report and, past the total, kills the
/// largest. Reactive: the total is passed before it acts, by what the
/// workers grow in a report's interval plus a watch's.
struct TotalWatchdog {
    total: u64,
    wake: Mutex<bool>,
    woken: std::sync::Condvar,
    kills: AtomicU64,
    /// The largest overshoot at a kill, bytes past the total.
    worst: AtomicU64,
}

impl TotalWatchdog {
    fn start(total: u64) -> Arc<Self> {
        let watchdog = Arc::new(Self {
            total,
            wake: Mutex::new(false),
            woken: std::sync::Condvar::new(),
            kills: AtomicU64::new(0),
            worst: AtomicU64::new(0),
        });
        let running = watchdog.clone();
        let _ = thread::Builder::new()
            .name("pmacs-total-watch".into())
            .spawn(move || running.watch());
        watchdog
    }

    fn wake(&self) {
        *self.wake.lock().expect("watchdog poisoned") = true;
        self.woken.notify_all();
    }

    fn summary(&self) -> String {
        format!(
            "total {} MiB, {} kills, worst overshoot {} bytes",
            self.total >> 20,
            self.kills.load(Ordering::Relaxed),
            self.worst.load(Ordering::Relaxed)
        )
    }

    fn watch(&self) {
        loop {
            {
                let mut woken = self.wake.lock().expect("watchdog poisoned");
                while !*woken {
                    woken = self.woken.wait(woken).expect("watchdog poisoned");
                }
                *woken = false;
            }
            loop {
                let units = host().units();
                if !units.iter().any(|u| u.parsing.load(Ordering::Relaxed)) {
                    break;
                }
                let sum: u64 = units.iter().map(|u| u.resident()).sum();
                if sum > self.total
                    && let Some(largest) = units.iter().max_by_key(|u| u.resident())
                {
                    self.kills.fetch_add(1, Ordering::Relaxed);
                    self.worst.fetch_max(sum - self.total, Ordering::Relaxed);
                    host().trace(&[
                        ("event", json_str("total-kill")),
                        ("unit", json_str(&largest.id())),
                        ("sum", sum.to_string()),
                        ("total", self.total.to_string()),
                        ("resident", largest.resident().to_string()),
                    ]);
                    largest.kill_for(Death::Total {
                        limit: self.total,
                        by: Enforcer::Watchdog,
                    });
                }
                thread::sleep(TOTAL_WATCH_EVERY);
            }
        }
    }
}

/// A cgroup v2 directory holding every worker, its `memory.max` the
/// editor-wide total: the kernel enforces the aggregate.
#[derive(Clone, Debug)]
struct Cgroup {
    dir: PathBuf,
    /// Workers the kernel moved into it.
    adopted: Arc<AtomicU64>,
    /// Workers it refused to move, which then run outside the total, and
    /// the first refusal's error.
    refused: Arc<AtomicU64>,
    first_refusal: Arc<Mutex<Option<String>>>,
}

/// What the editor found when it tried to create the workers' cgroup: the
/// cgroup, or `None` and the step that refused, said in `report`.
struct CgroupAttempt {
    cgroup: Option<Cgroup>,
    report: String,
}

impl Cgroup {
    /// Create the workers' cgroup beside this process's own (a cgroup
    /// holding processes cannot also hold children with controllers), when
    /// the subtree is delegated to this user. Otherwise no cgroup, and the
    /// report names the step that refused and why.
    fn create(total: u64) -> CgroupAttempt {
        let refused = |report: String| CgroupAttempt {
            cgroup: None,
            report,
        };
        if total == 0 {
            return refused("no total: syntax.parse-memory-total-mb is 0".into());
        }
        let own = match std::fs::read_to_string("/proc/self/cgroup") {
            Ok(own) => own,
            Err(e) => return refused(format!("no cgroup: /proc/self/cgroup: {e}")),
        };
        let Some(rel) = own.lines().find_map(|l| l.strip_prefix("0::")) else {
            return refused(format!(
                "no cgroup v2 entry in /proc/self/cgroup: {:?}",
                own.trim()
            ));
        };
        let mine = PathBuf::from("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
        let Some(parent) = mine.parent() else {
            return refused(format!("own cgroup {rel} has no parent"));
        };
        let controllers = std::fs::read_to_string(parent.join("cgroup.subtree_control"))
            .map_or_else(|e| format!("unreadable: {e}"), |s| s.trim().to_owned());
        // A daemon that crashed left its (empty) cgroup; sweep those.
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(pid) = name
                    .to_str()
                    .and_then(|n| n.strip_prefix("pmacs-parse-"))
                    .and_then(|p| p.parse::<u32>().ok())
                else {
                    continue;
                };
                if !PathBuf::from(format!("/proc/{pid}")).exists() {
                    let _ = std::fs::remove_dir(entry.path());
                }
            }
        }
        let dir = parent.join(format!("pmacs-parse-{}", std::process::id()));
        let attempt = if let Err(e) = std::fs::create_dir(&dir) {
            refused(format!(
                "refused at mkdir {}: {e}; own cgroup {rel}, parent's subtree_control {controllers:?}",
                dir.display()
            ))
        } else if let Err(e) = std::fs::write(dir.join("memory.max"), total.to_string()) {
            let _ = std::fs::remove_dir(&dir);
            refused(format!(
                "refused at memory.max in {}: {e}; parent's subtree_control {controllers:?}",
                dir.display()
            ))
        } else {
            let swap = std::fs::write(dir.join("memory.swap.max"), "0")
                .map_or_else(|e| format!("swap.max unset: {e}"), |()| "swap.max 0".into());
            CgroupAttempt {
                report: format!(
                    "created {} with memory.max {total} and {swap}; parent's subtree_control {controllers:?}",
                    dir.display()
                ),
                cgroup: Some(Self {
                    dir,
                    adopted: Arc::new(AtomicU64::new(0)),
                    refused: Arc::new(AtomicU64::new(0)),
                    first_refusal: Arc::new(Mutex::new(None)),
                }),
            }
        };
        host().trace(&[
            ("event", json_str("cgroup")),
            ("ok", attempt.cgroup.is_some().to_string()),
            ("total", total.to_string()),
            ("report", json_str(&attempt.report)),
        ]);
        attempt
    }

    /// Move `pid` into the cgroup; whether the kernel did. A refusal
    /// leaves that worker outside the cgroup's total, for the watchdog to
    /// hold; it is counted and its first error kept for the report.
    fn adopt(&self, pid: u32) -> bool {
        match std::fs::write(self.dir.join("cgroup.procs"), pid.to_string()) {
            Ok(()) => {
                self.adopted.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(e) => {
                self.refused.fetch_add(1, Ordering::Relaxed);
                let mut first = self.first_refusal.lock().expect("refusal poisoned");
                if first.is_none() {
                    *first = Some(format!("pid {pid}: {e}"));
                }
                false
            }
        }
    }

    /// The adoption counts, for the report.
    fn adoption(&self) -> String {
        let refused = self.refused.load(Ordering::Relaxed);
        let mut line = format!(
            "adopted {}, refused {refused}",
            self.adopted.load(Ordering::Relaxed)
        );
        if let Some(first) = self
            .first_refusal
            .lock()
            .expect("refusal poisoned")
            .as_ref()
        {
            use std::fmt::Write as _;
            let _ = write!(line, " (first: {first})");
        }
        line
    }

    fn oom_kills(&self) -> u64 {
        std::fs::read_to_string(self.dir.join("memory.events"))
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("oom_kill "))
                    .and_then(|n| n.trim().parse().ok())
            })
            .unwrap_or(0)
    }
}
