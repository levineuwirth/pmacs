// parse_isolation.rs --- E7i: the editor's side of a parse unit, behind a
// wasm instance or a worker process.

//! Parse containment, the two candidates E7i compares (`syntax.isolation`).
//!
//! A parse unit (`pmacs-parse-unit`) holds one buffer's trees and runs its
//! parse and capture walks. The editor keeps the buffer's text, the edit
//! log and the spans that came back; it never holds a tree. Each buffer
//! gets its own unit, behind one of two boundaries:
//!
//! * **wasm** --- the unit built for `wasm32-wasip1` (the tree-sitter
//!   runtime, every grammar and the unit in one module), run under
//!   wasmtime on a thread of its own. Memory is bounded by a resource
//!   limiter that refuses linear-memory growth past the unit's allowance
//!   and past the editor-wide total; time by epoch interruption, which
//!   traps the instance wherever its code is, the runtime's included,
//!   because the runtime is wasm here. A trap ends the instance.
//! * **process** --- the same unit built natively, one worker process per
//!   buffer. Memory is bounded by `RLIMIT_AS`, which the worker applies to
//!   itself before serving, and the total by a cgroup v2 `memory.max` the
//!   editor creates beside its own cgroup when the session's cgroup
//!   subtree is delegated; time by a watchdog here that kills the worker
//!   at the deadline. A kill or an abort ends the process.
//!
//! Either way the boundary stops the operation, not one call inside it:
//! the parse's whole memory and the whole request's time are what the
//! limits measure. A unit that dies is discarded and the next request
//! starts a fresh one with the whole text. The buffer's previous parse
//! survives the discard in the editor, as its text and its spans
//! ([`IsolatedHandle`]); a range those spans do not cover is answered by
//! re-parsing that text into a fresh unit, which rebuilds the previous
//! tree there.
//!
//! This is the comparison's prototype: highlighting goes through the unit;
//! folds and Lua's node API see no tree under isolation (designed in the
//! comparison, not built). `PMACS_E7I_TRACE=<path>` appends one JSON line
//! per unit event, for the comparison's retained runs.

use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use pmacs_parse_unit::{Failure, ParseCall, Request, Response, SpanSet, TextUpdate, WireEdit};

use crate::buffer::BufferId;
use crate::syntax::{
    HighlightSpan, IsolatedLayerSpans, IsolatedSpans, IsolatedTree, ParseError, ParseRequest,
    ParseTreeBundle,
};

/// Which boundary a parse runs behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// In the editor's own process, as before E7i.
    Native,
    /// A wasm instance of the unit, under wasmtime.
    Wasm,
    /// A worker process running the unit.
    Process,
}

impl Isolation {
    /// The mode `syntax.isolation` names: `"none"`, `"wasm"`, `"process"`.
    #[must_use]
    pub fn from_config(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::Native),
            "wasm" => Some(Self::Wasm),
            "process" => Some(Self::Process),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Native => "none",
            Self::Wasm => "wasm",
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
    /// `syntax.isolation-wasm-cache`: keep wasmtime's compiled module on
    /// disk, so a later start loads it instead of compiling it. Read once,
    /// when the first wasm unit starts.
    pub wasm_cache: bool,
}

/// How long after the deadline the boundary stops a unit that has not
/// answered. A parse the progress callback reaches returns at the
/// deadline; this is for work it does not reach (#296's accept, #301's
/// condensation), and a little slack so a clean cancel lands first.
pub const HARD_GRACE: Duration = Duration::from_millis(100);

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

/// Why a unit was discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Death {
    /// It ran past the deadline plus [`HARD_GRACE`] and was stopped.
    Time {
        /// The deadline that applied.
        deadline: Duration,
        /// How long the request had run when it was stopped.
        after: Duration,
    },
    /// It grew past its own allowance.
    Memory {
        /// The allowance, bytes.
        limit: u64,
    },
    /// The units together reached the editor-wide total.
    Total {
        /// The total, bytes.
        limit: u64,
    },
    /// It ended for another reason (a crash, an exit, a broken pipe).
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
            Self::Memory { limit } => {
                format!(
                    "{PARSE_LIMIT_MESSAGE}: its parse unit grew past {} MiB",
                    limit >> 20
                )
            }
            Self::Total { limit } => format!(
                "{PARSE_LIMIT_MESSAGE}: the parse units together reached {} MiB",
                limit >> 20
            ),
            Self::Ended(why) => format!("parse unit ended: {why}"),
            Self::Unavailable(why) => format!("parse unit unavailable: {why}"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Time { .. } => "time",
            Self::Memory { .. } => "memory",
            Self::Total { .. } => "total",
            Self::Ended(_) => "ended",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

/// How a memory stop's message begins, for the settle path.
pub const PARSE_LIMIT_MESSAGE: &str = "parse stopped at its memory limit";

/// Whether a parse job's failure text is a memory stop.
#[must_use]
pub fn is_limit_message(message: &str) -> bool {
    message.starts_with(PARSE_LIMIT_MESSAGE)
}

/// A unit behind its boundary.
trait Transport: Send {
    /// One request and its answer, the boundary stopping the unit if it
    /// has not answered within `hard`.
    fn call(
        &mut self,
        request: &Request,
        payload: &[u8],
        hard: Option<Duration>,
    ) -> Result<Response, Death>;
    /// What the unit holds now: linear memory for wasm, PSS for a process.
    fn memory_bytes(&self) -> Option<u64>;
    /// A name for the trace: the worker's pid, or the instance's number.
    fn id(&self) -> String;
}

/// A buffer's unit and what the editor knows about its state.
struct Slot {
    mode: Isolation,
    transport: Option<Box<dyn Transport>>,
    /// Bumped by every installed parse, so a handle can tell whether the
    /// unit still holds its tree.
    generation: u64,
    /// The unit's text mirror equals the text the editor last sent.
    synced: bool,
    deaths: u64,
    last_death: Option<Death>,
    /// Times a discarded unit's previous parse was rebuilt in a fresh one.
    reestablished: u64,
}

/// Process-wide state: the units, the wasm engine, the cgroup, the trace.
struct Host {
    slots: Mutex<HashMap<BufferId, Arc<Mutex<Slot>>>>,
    wasm: OnceLock<Result<WasmShared, String>>,
    cgroup: OnceLock<Option<Cgroup>>,
    trace: Option<Mutex<std::fs::File>>,
    started: Instant,
    next_instance: AtomicU64,
}

static HOST_CELL: OnceLock<Host> = OnceLock::new();

fn host() -> &'static Host {
    HOST_CELL.get_or_init(|| Host {
        slots: Mutex::new(HashMap::new()),
        wasm: OnceLock::new(),
        cgroup: OnceLock::new(),
        trace: std::env::var_os("PMACS_E7I_TRACE").and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(Mutex::new)
        }),
        started: Instant::now(),
        next_instance: AtomicU64::new(1),
    })
}

impl Host {
    fn slot(&self, buffer: BufferId, mode: Isolation) -> Arc<Mutex<Slot>> {
        self.slots
            .lock()
            .expect("isolation slots poisoned")
            .entry(buffer)
            .or_insert_with(|| {
                Arc::new(Mutex::new(Slot {
                    mode,
                    transport: None,
                    generation: 0,
                    synced: false,
                    deaths: 0,
                    last_death: None,
                    reestablished: 0,
                }))
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

    fn spawn(&self, limits: &Limits) -> Result<Box<dyn Transport>, Death> {
        match limits.mode {
            Isolation::Wasm => {
                let shared = self
                    .wasm
                    .get_or_init(|| WasmShared::load(limits.wasm_cache))
                    .as_ref()
                    .map_err(|e| Death::Unavailable(e.clone()))?;
                let n = self.next_instance.fetch_add(1, Ordering::Relaxed);
                WasmUnit::start(shared, n, limits).map(|u| Box::new(u) as Box<dyn Transport>)
            }
            Isolation::Process => {
                let cgroup = self
                    .cgroup
                    .get_or_init(|| Cgroup::create(limits.total_memory))
                    .as_ref();
                ProcessUnit::start(limits, cgroup).map(|u| Box::new(u) as Box<dyn Transport>)
            }
            Isolation::Native => Err(Death::Unavailable("native mode has no unit".into())),
        }
    }
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Start `slot`'s unit if it has none.
fn ensure_unit(host: &Host, slot: &mut Slot, job: &IsolatedJob) -> Result<(), String> {
    if slot.transport.is_some() {
        return Ok(());
    }
    let started = Instant::now();
    let t = host.spawn(&job.limits).map_err(|death| death.message())?;
    host.trace(&[
        ("event", json_str("spawn")),
        ("mode", json_str(job.limits.mode.name())),
        ("unit", json_str(&t.id())),
        ("buffer", job.buffer.raw().to_string()),
        ("spawn_us", started.elapsed().as_micros().to_string()),
    ]);
    slot.transport = Some(t);
    slot.synced = false;
    Ok(())
}

/// Run one isolated parse on the calling (worker) thread and build the
/// bundle the settle path installs. `Err` carries the message the
/// in-process path would carry, or a [`Death`]'s.
pub fn run(job: &IsolatedJob) -> Result<ParseTreeBundle, String> {
    let host = host();
    let slot_arc = host.slot(job.buffer, job.limits.mode);
    let mut slot = slot_arc.lock().expect("isolation slot poisoned");
    if slot.mode != job.limits.mode {
        slot.transport = None;
        slot.mode = job.limits.mode;
        slot.synced = false;
    }
    let mut attempt = 0;
    loop {
        attempt += 1;
        ensure_unit(host, &mut slot, job)?;
        let full = !slot.synced;
        let (call, payload) = parse_call(job, full);
        let hard = job.limits.deadline.map(|d| d + HARD_GRACE);
        let started = Instant::now();
        let transport = slot.transport.as_mut().expect("spawned above");
        let unit_id = transport.id();
        let answer = transport.call(&Request::Parse(call), &payload, hard);
        let elapsed = started.elapsed();
        let memory = slot.transport.as_ref().and_then(|t| t.memory_bytes());
        match answer {
            Ok(Response::Parsed(parsed)) => {
                slot.synced = true;
                slot.generation += 1;
                host.trace(&[
                    ("event", json_str("parsed")),
                    ("mode", json_str(job.limits.mode.name())),
                    ("unit", json_str(&unit_id)),
                    ("buffer", job.buffer.raw().to_string()),
                    ("full", full.to_string()),
                    ("bytes", job.request.source.len().to_string()),
                    ("layers", parsed.layers.len().to_string()),
                    ("root_us", parsed.root_parse_us.to_string()),
                    ("unit_us", parsed.total_us.to_string()),
                    ("call_us", elapsed.as_micros().to_string()),
                    (
                        "unit_memory",
                        memory.map_or("null".into(), |m| m.to_string()),
                    ),
                ]);
                let generation = slot.generation;
                drop(slot);
                return Ok(bundle_from(job, parsed, &slot_arc, generation));
            }
            Ok(Response::Failed(Failure::Desync { have, want })) if attempt == 1 => {
                host.trace(&[
                    ("event", json_str("desync")),
                    ("buffer", job.buffer.raw().to_string()),
                    ("have", have.to_string()),
                    ("want", want.to_string()),
                ]);
                slot.synced = false;
            }
            Ok(Response::Failed(failure)) => {
                // The unit applied the text and is alive; its tree stays the
                // previous one and it parses cold next, as in-process.
                slot.synced = !matches!(failure, Failure::Desync { .. });
                host.trace(&[
                    ("event", json_str("failed")),
                    ("mode", json_str(job.limits.mode.name())),
                    ("unit", json_str(&unit_id)),
                    ("buffer", job.buffer.raw().to_string()),
                    ("failure", json_str(&format!("{failure:?}"))),
                    ("call_us", elapsed.as_micros().to_string()),
                ]);
                return Err(failure_message(&failure));
            }
            Ok(other) => {
                slot.transport = None;
                return Err(format!("parse unit answered out of turn: {other:?}"));
            }
            Err(death) => {
                // Dropping the transport ends the instance or reaps the
                // worker; its memory goes with it.
                slot.transport = None;
                slot.synced = false;
                slot.deaths += 1;
                slot.last_death = Some(death.clone());
                host.trace(&[
                    ("event", json_str("death")),
                    ("mode", json_str(job.limits.mode.name())),
                    ("unit", json_str(&unit_id)),
                    ("buffer", job.buffer.raw().to_string()),
                    ("kind", json_str(death.kind())),
                    ("message", json_str(&death.message())),
                    ("call_us", elapsed.as_micros().to_string()),
                    (
                        "unit_memory",
                        memory.map_or("null".into(), |m| m.to_string()),
                    ),
                ]);
                return Err(death.message());
            }
        }
    }
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
    slot: &Arc<Mutex<Slot>>,
    generation: u64,
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
            slot: slot.clone(),
            buffer: job.buffer,
            generation: AtomicU64::new(generation),
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
/// spans fetched so far, and the generation the unit gave the tree.
pub struct IsolatedHandle {
    slot: Arc<Mutex<Slot>>,
    buffer: BufferId,
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
            .field("generation", &self.generation.load(Ordering::Relaxed))
            .field("bytes", &self.source.len())
            .finish_non_exhaustive()
    }
}

/// How many fetched span sets a handle keeps (the parse's own and the
/// most recent fetches).
const MAX_PIECES: usize = 8;

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
        // Not covered: ask the unit, only if it is idle (a busy unit is
        // parsing a newer text, and the renderer must not wait on it).
        let mut slot = self.slot.try_lock().ok()?;
        let want = vec![(range.start as u32, range.end as u32)];
        let fetched = if slot.transport.is_some()
            && slot.generation == self.generation.load(Ordering::Relaxed)
        {
            let transport = slot.transport.as_mut()?;
            match transport.call(&Request::Spans { ranges: want }, &[], Some(SPANS_HARD)) {
                Ok(Response::Spans(set)) => Some(set),
                Ok(_) => None,
                Err(death) => {
                    slot.transport = None;
                    slot.synced = false;
                    slot.last_death = Some(death);
                    None
                }
            }
        } else if slot.transport.is_none() {
            self.reestablish(&mut slot, want)
        } else {
            // The unit holds a newer tree; this parse is about to be
            // replaced and keeps what it fetched.
            None
        }?;
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
}

/// How long a spans request may take before its unit is discarded.
const SPANS_HARD: Duration = Duration::from_secs(2);

impl IsolatedHandle {
    /// The unit that held this tree was discarded: start a fresh one and
    /// parse this tree's text into it, which rebuilds the tree there.
    fn reestablish(&self, slot: &mut Slot, interest: Vec<(u32, u32)>) -> Option<SpanSet> {
        let host = host();
        let started = Instant::now();
        let transport = host.spawn(&self.limits).ok()?;
        slot.transport = Some(transport);
        let call = ParseCall {
            language: self.language.clone(),
            text: TextUpdate::Full,
            edits: Vec::new(),
            expect_len: self.source.len() as u32,
            aliases: self.aliases.clone(),
            deadline_ms: self.limits.deadline.map(|d| d.as_millis() as u64),
            interest,
        };
        let hard = self.limits.deadline.map(|d| d + HARD_GRACE);
        let answer = slot
            .transport
            .as_mut()?
            .call(&Request::Parse(call), &self.source, hard);
        match answer {
            Ok(Response::Parsed(parsed)) => {
                slot.generation += 1;
                slot.reestablished += 1;
                // The unit's mirror is this older text, not the buffer's.
                slot.synced = false;
                self.generation.store(slot.generation, Ordering::Relaxed);
                *self.languages.lock().expect("languages poisoned") =
                    parsed.layers.iter().map(|l| l.language.clone()).collect();
                host.trace(&[
                    ("event", json_str("reestablished")),
                    ("buffer", self.buffer.raw().to_string()),
                    ("bytes", self.source.len().to_string()),
                    ("us", started.elapsed().as_micros().to_string()),
                ]);
                Some(parsed.spans)
            }
            Ok(_) => None,
            Err(death) => {
                slot.transport = None;
                slot.last_death = Some(death);
                None
            }
        }
    }
}

/// End every unit (instances return, workers are killed and reaped) and
/// remove the workers' cgroup. The daemon calls this as it stops.
pub fn shutdown() {
    let Some(host) = HOST_CELL.get() else {
        return;
    };
    let slots: Vec<Arc<Mutex<Slot>>> = host
        .slots
        .lock()
        .expect("isolation slots poisoned")
        .drain()
        .map(|(_, s)| s)
        .collect();
    for slot in slots {
        if let Ok(mut slot) = slot.lock() {
            slot.transport = None;
        }
    }
    if let Some(Some(cgroup)) = host.cgroup.get() {
        let _ = std::fs::remove_dir(&cgroup.dir);
    }
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
    /// `"none"`, `"wasm"` or `"process"`.
    pub mode: String,
    /// The worker's pid or the instance's number; empty when none runs.
    pub unit: String,
    /// What the unit holds now, bytes, when it runs.
    pub memory: Option<u64>,
    /// Units this buffer has had discarded.
    pub deaths: u64,
    /// Why the last one was, `kind: message`.
    pub last_death: Option<String>,
    /// The unit is answering a request now; the other fields are empty.
    pub busy: bool,
    /// Times a discarded unit's previous parse was rebuilt in a fresh one.
    pub reestablished: u64,
}

/// The report for `buffer`'s unit, if it has had one. Never waits: the
/// main thread must not block on a unit that is parsing (E7i), so a busy
/// unit reports only that it is busy.
#[must_use]
pub fn report(buffer: BufferId) -> Option<UnitReport> {
    let slot = host()
        .slots
        .lock()
        .expect("isolation slots poisoned")
        .get(&buffer)?
        .clone();
    let Ok(slot) = slot.try_lock() else {
        return Some(UnitReport {
            busy: true,
            ..UnitReport::default()
        });
    };
    Some(UnitReport {
        busy: false,
        reestablished: slot.reestablished,
        mode: slot.mode.name().to_owned(),
        unit: slot.transport.as_ref().map(|t| t.id()).unwrap_or_default(),
        memory: slot.transport.as_ref().and_then(|t| t.memory_bytes()),
        deaths: slot.deaths,
        last_death: slot
            .last_death
            .as_ref()
            .map(|d| format!("{}: {}", d.kind(), d.message())),
    })
}

/// The editor-wide memory the units hold now: wasm linear memory summed,
/// process PSS summed. For the comparison's steady-state figure.
#[must_use]
pub fn total_unit_memory() -> u64 {
    let slots: Vec<Arc<Mutex<Slot>>> = host()
        .slots
        .lock()
        .expect("isolation slots poisoned")
        .values()
        .cloned()
        .collect();
    // Busy units are skipped rather than waited for.
    slots
        .iter()
        .filter_map(|s| {
            s.try_lock()
                .ok()
                .and_then(|s| s.transport.as_ref()?.memory_bytes())
        })
        .sum()
}

// ---------------------------------------------------------------------------
// The process boundary
// ---------------------------------------------------------------------------

/// One frame read from a worker, or why none came.
type FrameRead = io::Result<Option<(Response, Vec<u8>)>>;

/// A worker process running the unit.
struct ProcessUnit {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<FrameRead>,
    stderr: Arc<Mutex<String>>,
    limits: Limits,
    cgroup: Option<Cgroup>,
    oom_before: u64,
}

impl ProcessUnit {
    fn start(limits: &Limits, cgroup: Option<&Cgroup>) -> Result<Self, Death> {
        let bin = sibling("pmacs-parse-unit")
            .ok_or_else(|| Death::Unavailable("no pmacs-parse-unit beside pmacs".into()))?;
        let mut child = Command::new(&bin)
            .arg("--memory-limit-mb")
            .arg((limits.unit_memory >> 20).max(1).to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Death::Unavailable(format!("{}: {e}", bin.display())))?;
        if let Some(cg) = cgroup {
            cg.adopt(child.id());
        }
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let (tx, responses) = mpsc::channel();
        thread::Builder::new()
            .name("pmacs-unit-read".into())
            .spawn(move || {
                let mut r = BufReader::new(stdout);
                loop {
                    let frame = pmacs_parse_unit::read_frame::<_, Response>(&mut r);
                    let end = !matches!(frame, Ok(Some(_)));
                    if tx.send(frame).is_err() || end {
                        break;
                    }
                }
            })
            .map_err(|e| Death::Unavailable(e.to_string()))?;
        let stderr = Arc::new(Mutex::new(String::new()));
        let sink = stderr.clone();
        thread::Builder::new()
            .name("pmacs-unit-err".into())
            .spawn(move || {
                let mut r = BufReader::new(stderr_pipe);
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
            })
            .map_err(|e| Death::Unavailable(e.to_string()))?;
        let cgroup = cgroup.cloned();
        let oom_before = cgroup.as_ref().map_or(0, Cgroup::oom_kills);
        Ok(Self {
            child,
            stdin,
            responses,
            stderr,
            limits: *limits,
            cgroup,
            oom_before,
        })
    }

    /// Why the worker ended, read from its exit signal and its last words:
    /// the cgroup's OOM killer sends `SIGKILL`, and a refused allocation
    /// under `RLIMIT_AS` makes tree-sitter's or Rust's allocator abort
    /// (`SIGABRT`) after saying so.
    fn death(&mut self) -> Death {
        let status = self.child.wait();
        let signal = status
            .as_ref()
            .ok()
            .and_then(std::os::unix::process::ExitStatusExt::signal);
        let stderr = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        let oom_now = self.cgroup.as_ref().map_or(0, Cgroup::oom_kills);
        if signal == Some(9) && oom_now > self.oom_before {
            return Death::Total {
                limit: self.limits.total_memory,
            };
        }
        if stderr.contains("failed to allocate")
            || stderr.contains("failed to reallocate")
            || stderr.contains("memory allocation of")
        {
            return Death::Memory {
                limit: self.limits.unit_memory,
            };
        }
        let last = stderr.lines().last().unwrap_or("").to_owned();
        Death::Ended(format!("{status:?} signal {signal:?} {last}"))
    }
}

impl Transport for ProcessUnit {
    fn call(
        &mut self,
        request: &Request,
        payload: &[u8],
        hard: Option<Duration>,
    ) -> Result<Response, Death> {
        let started = Instant::now();
        if pmacs_parse_unit::write_frame(&mut self.stdin, request, payload).is_err() {
            return Err(self.death());
        }
        let answer = match hard {
            Some(limit) => match self.responses.recv_timeout(limit) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Err(Death::Time {
                        deadline: limit.saturating_sub(HARD_GRACE),
                        after: started.elapsed(),
                    });
                }
                Err(RecvTimeoutError::Disconnected) => return Err(self.death()),
            },
            None => match self.responses.recv() {
                Ok(frame) => frame,
                Err(_) => return Err(self.death()),
            },
        };
        match answer {
            Ok(Some((response, _))) => Ok(response),
            _ => Err(self.death()),
        }
    }

    fn memory_bytes(&self) -> Option<u64> {
        pss_bytes(self.child.id())
    }

    fn id(&self) -> String {
        format!("pid {}", self.child.id())
    }
}

impl Drop for ProcessUnit {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// A binary beside the running one, as `pmacs --gpu` finds `pmacs-gpu`.
fn sibling(name: &str) -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(name);
    path.exists().then_some(path)
}

/// A cgroup v2 directory holding every worker, its `memory.max` the
/// editor-wide total: the kernel enforces the aggregate.
#[derive(Clone, Debug)]
struct Cgroup {
    dir: PathBuf,
}

impl Cgroup {
    /// Create the workers' cgroup beside this process's own (a cgroup
    /// holding processes cannot also hold children with controllers), when
    /// the subtree is delegated to this user; `None` otherwise, and the
    /// total is then unenforced, which the trace records.
    fn create(total: u64) -> Option<Self> {
        if total == 0 {
            return None;
        }
        let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        let rel = own.lines().find_map(|l| l.strip_prefix("0::"))?;
        let mine = PathBuf::from("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
        let parent = mine.parent()?;
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
        let made = std::fs::create_dir(&dir).is_ok()
            && std::fs::write(dir.join("memory.max"), total.to_string()).is_ok();
        let _ = std::fs::write(dir.join("memory.swap.max"), "0");
        let cgroup = Self { dir };
        host().trace(&[
            ("event", json_str("cgroup")),
            ("dir", json_str(&cgroup.dir.display().to_string())),
            ("ok", made.to_string()),
            ("total", total.to_string()),
        ]);
        made.then_some(cgroup)
    }

    fn adopt(&self, pid: u32) {
        let _ = std::fs::write(self.dir.join("cgroup.procs"), pid.to_string());
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

// ---------------------------------------------------------------------------
// The wasm boundary
// ---------------------------------------------------------------------------

/// The engine and the compiled module, shared by every instance, and the
/// thread that ticks the engine's epoch.
struct WasmShared {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    total: Arc<AtomicUsize>,
}

/// The epoch tick: a deadline of `d` is `d / EPOCH_TICK` ticks.
const EPOCH_TICK: Duration = Duration::from_millis(1);

impl WasmShared {
    fn load(cache: bool) -> Result<Self, String> {
        let path = sibling("pmacs-parse-unit.wasm")
            .ok_or_else(|| "no pmacs-parse-unit.wasm beside pmacs".to_owned())?;
        let mut config = wasmtime::Config::new();
        config.epoch_interruption(true);
        if cache {
            // wasmtime's own cache, keyed by the module and the engine's
            // settings, under the user's cache directory.
            let dir = std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                .ok_or_else(|| "no cache directory".to_owned())?
                .join("pmacs")
                .join("wasmtime");
            let mut cache_config = wasmtime::CacheConfig::new();
            cache_config.with_directory(dir);
            let cache = wasmtime::Cache::new(cache_config).map_err(|e| e.to_string())?;
            config.cache(Some(cache));
        }
        let engine = wasmtime::Engine::new(&config).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let module = wasmtime::Module::from_file(&engine, &path).map_err(|e| e.to_string())?;
        host().trace(&[
            ("event", json_str("wasm-compiled")),
            ("cache", cache.to_string()),
            ("path", json_str(&path.display().to_string())),
            ("compile_us", started.elapsed().as_micros().to_string()),
        ]);
        let ticker = engine.clone();
        thread::Builder::new()
            .name("pmacs-wasm-epoch".into())
            .spawn(move || {
                loop {
                    thread::sleep(EPOCH_TICK);
                    ticker.increment_epoch();
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            engine,
            module,
            total: Arc::new(AtomicUsize::new(0)),
        })
    }
}

/// What a running instance's host functions own.
struct UnitState {
    requests: Receiver<(Vec<u8>, u64)>,
    responses: Sender<Vec<u8>>,
    input: Vec<u8>,
    input_at: usize,
    output: Vec<u8>,
    limiter: Limiter,
    stderr: Vec<u8>,
    rng: u64,
}

/// Refuses linear-memory growth past the unit's allowance and past the
/// editor-wide total; the refusal makes the guest's allocator fail, and
/// the guest aborts, which traps and ends the instance.
struct Limiter {
    initial: Option<usize>,
    current: usize,
    unit: usize,
    total_max: usize,
    total: Arc<AtomicUsize>,
    shown: Arc<AtomicUsize>,
    hit: Arc<Mutex<Option<Death>>>,
}

impl wasmtime::ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let initial = *self
            .initial
            .get_or_insert(if current == 0 { desired } else { current });
        if desired.saturating_sub(initial) > self.unit {
            *self.hit.lock().expect("limit flag poisoned") = Some(Death::Memory {
                limit: self.unit as u64,
            });
            return Ok(false);
        }
        let grow = desired - current;
        let before = self.total.fetch_add(grow, Ordering::SeqCst);
        if self.total_max > 0 && before + grow > self.total_max {
            self.total.fetch_sub(grow, Ordering::SeqCst);
            *self.hit.lock().expect("limit flag poisoned") = Some(Death::Total {
                limit: self.total_max as u64,
            });
            return Ok(false);
        }
        self.current = desired;
        self.shown.store(desired, Ordering::Relaxed);
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }
}

impl Drop for Limiter {
    fn drop(&mut self) {
        self.total.fetch_sub(self.current, Ordering::SeqCst);
    }
}

/// A wasm instance of the unit on its own thread.
struct WasmUnit {
    requests: Option<Sender<(Vec<u8>, u64)>>,
    responses: Receiver<Vec<u8>>,
    thread: Option<thread::JoinHandle<Result<(), String>>>,
    memory: Arc<AtomicUsize>,
    hit: Arc<Mutex<Option<Death>>>,
    number: u64,
}

impl WasmUnit {
    fn start(shared: &WasmShared, number: u64, limits: &Limits) -> Result<Self, Death> {
        let (req_tx, req_rx) = mpsc::channel::<(Vec<u8>, u64)>();
        let (resp_tx, resp_rx) = mpsc::channel::<Vec<u8>>();
        let memory = Arc::new(AtomicUsize::new(0));
        let hit = Arc::new(Mutex::new(None));
        let engine = shared.engine.clone();
        let module = shared.module.clone();
        let limiter = Limiter {
            initial: None,
            current: 0,
            unit: usize::try_from(limits.unit_memory).unwrap_or(usize::MAX),
            total_max: usize::try_from(limits.total_memory).unwrap_or(usize::MAX),
            total: shared.total.clone(),
            shown: memory.clone(),
            hit: hit.clone(),
        };
        let thread = thread::Builder::new()
            .name(format!("pmacs-wasm-unit-{number}"))
            .spawn(move || run_instance(&engine, &module, req_rx, resp_tx, limiter))
            .map_err(|e| Death::Unavailable(e.to_string()))?;
        Ok(Self {
            requests: Some(req_tx),
            responses: resp_rx,
            thread: Some(thread),
            memory,
            hit,
            number,
        })
    }

    /// Why the instance ended: the limiter's refusal, else the trap.
    fn death(&mut self, deadline: Option<Duration>, after: Duration) -> Death {
        self.requests = None;
        let ended = self.thread.take().map_or_else(
            || Err("no thread".to_owned()),
            |t| t.join().unwrap_or_else(|_| Err("panicked".into())),
        );
        if let Some(death) = self.hit.lock().expect("limit flag poisoned").take() {
            return death;
        }
        match ended {
            Err(why) if why.contains("interrupt") => Death::Time {
                deadline: deadline.unwrap_or_default(),
                after,
            },
            Err(why) => Death::Ended(why),
            Ok(()) => Death::Ended("instance exited".into()),
        }
    }
}

fn run_instance(
    engine: &wasmtime::Engine,
    module: &wasmtime::Module,
    requests: Receiver<(Vec<u8>, u64)>,
    responses: Sender<Vec<u8>>,
    limiter: Limiter,
) -> Result<(), String> {
    let state = UnitState {
        requests,
        responses,
        input: Vec::new(),
        input_at: 0,
        output: Vec::new(),
        limiter,
        stderr: Vec::new(),
        rng: 0x9e37_79b9_7f4a_7c15,
    };
    let mut store = wasmtime::Store::new(engine, state);
    store.limiter(|s| &mut s.limiter);
    store.epoch_deadline_trap();
    store.set_epoch_deadline(u64::MAX / 2);
    let mut linker = wasmtime::Linker::new(engine);
    wasi_shim(&mut linker).map_err(|e| e.to_string())?;
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|e| format!("{e:#}"))?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|e| e.to_string())?;
    match start.call(&mut store, ()) {
        Ok(()) => Ok(()),
        Err(error) => {
            if let Some(code) = error.downcast_ref::<ProcExit>() {
                return if code.0 == 0 {
                    Ok(())
                } else {
                    Err(format!("exit {}", code.0))
                };
            }
            match error.downcast_ref::<wasmtime::Trap>() {
                Some(wasmtime::Trap::Interrupt) => Err("interrupt".into()),
                Some(trap) => {
                    let said = String::from_utf8_lossy(&store.data().stderr).into_owned();
                    Err(format!(
                        "trap {trap}: {}",
                        said.lines().last().unwrap_or("")
                    ))
                }
                None => Err(format!("{error:#}")),
            }
        }
    }
}

/// `proc_exit`'s code, carried out of the instance as an error.
#[derive(Debug)]
struct ProcExit(i32);

impl std::fmt::Display for ProcExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "proc_exit({})", self.0)
    }
}

impl std::error::Error for ProcExit {}

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_BADF: i32 = 8;
const ERRNO_INVAL: i32 = 28;
const ERRNO_SPIPE: i32 = 70;

type ShimCaller<'a> = wasmtime::Caller<'a, UnitState>;

fn guest_memory(caller: &mut ShimCaller<'_>) -> Option<wasmtime::Memory> {
    caller
        .get_export("memory")
        .and_then(wasmtime::Extern::into_memory)
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn write_bytes(data: &mut [u8], at: usize, bytes: &[u8]) -> bool {
    match data.get_mut(at..at + bytes.len()) {
        Some(slot) => {
            slot.copy_from_slice(bytes);
            true
        }
        None => false,
    }
}

/// The nine `wasi_snapshot_preview1` imports the unit's module makes, and
/// the clock and random source Rust's standard library asks for. Stdin is
/// the request channel: a read on an empty buffer waits for the next
/// request and arms that request's epoch deadline. Stdout assembles frames
/// and sends each complete one back. No file system, no environment.
#[allow(
    clippy::too_many_lines,
    reason = "one shim per import, each a few lines, kept together as the module's whole ABI"
)]
fn wasi_shim(linker: &mut wasmtime::Linker<UnitState>) -> anyhow::Result<()> {
    const W: &str = "wasi_snapshot_preview1";
    linker.func_wrap(
        W,
        "fd_read",
        |mut caller: ShimCaller<'_>, fd: i32, iovs: i32, iovs_len: i32, nread: i32| -> i32 {
            if fd != 0 {
                return ERRNO_BADF;
            }
            if caller.data().input_at >= caller.data().input.len() {
                // Idle: no deadline runs while the unit waits.
                caller.as_context_mut_set_deadline(u64::MAX / 2);
                let Ok((frame, ticks)) = caller.data().requests.recv() else {
                    // The editor dropped the unit: end of input.
                    let Some(mem) = guest_memory(&mut caller) else {
                        return ERRNO_INVAL;
                    };
                    let data = mem.data_mut(&mut caller);
                    return if write_bytes(data, nread as usize, &0u32.to_le_bytes()) {
                        ERRNO_SUCCESS
                    } else {
                        ERRNO_INVAL
                    };
                };
                let state = caller.data_mut();
                state.input = frame;
                state.input_at = 0;
                caller.as_context_mut_set_deadline(ticks);
            }
            let Some(mem) = guest_memory(&mut caller) else {
                return ERRNO_INVAL;
            };
            let (data, state) = mem.data_and_store_mut(&mut caller);
            let mut total = 0usize;
            for i in 0..iovs_len as usize {
                let base = iovs as usize + i * 8;
                let (Some(ptr), Some(len)) = (read_u32(data, base), read_u32(data, base + 4))
                else {
                    return ERRNO_INVAL;
                };
                let left = state.input.len() - state.input_at;
                let n = (len as usize).min(left);
                if n == 0 {
                    break;
                }
                let chunk = &state.input[state.input_at..state.input_at + n];
                if !write_bytes(data, ptr as usize, chunk) {
                    return ERRNO_INVAL;
                }
                state.input_at += n;
                total += n;
            }
            if write_bytes(data, nread as usize, &(total as u32).to_le_bytes()) {
                ERRNO_SUCCESS
            } else {
                ERRNO_INVAL
            }
        },
    )?;
    linker.func_wrap(
        W,
        "fd_write",
        |mut caller: ShimCaller<'_>, fd: i32, iovs: i32, iovs_len: i32, nwritten: i32| -> i32 {
            let Some(mem) = guest_memory(&mut caller) else {
                return ERRNO_INVAL;
            };
            let (data, state) = mem.data_and_store_mut(&mut caller);
            let mut total = 0usize;
            for i in 0..iovs_len as usize {
                let base = iovs as usize + i * 8;
                let (Some(ptr), Some(len)) = (read_u32(data, base), read_u32(data, base + 4))
                else {
                    return ERRNO_INVAL;
                };
                let Some(bytes) = data.get(ptr as usize..ptr as usize + len as usize) else {
                    return ERRNO_INVAL;
                };
                match fd {
                    1 => state.output.extend_from_slice(bytes),
                    2 => {
                        state.stderr.extend_from_slice(bytes);
                        if state.stderr.len() > 16_384 {
                            let cut = state.stderr.len() - 8_192;
                            state.stderr.drain(..cut);
                        }
                    }
                    _ => return ERRNO_BADF,
                }
                total += len as usize;
            }
            if fd == 1 {
                while let Some(n) = complete_frame(&state.output) {
                    let frame: Vec<u8> = state.output.drain(..n).collect();
                    let _ = state.responses.send(frame);
                }
            }
            if write_bytes(data, nwritten as usize, &(total as u32).to_le_bytes()) {
                ERRNO_SUCCESS
            } else {
                ERRNO_INVAL
            }
        },
    )?;
    linker.func_wrap(W, "fd_close", |_: ShimCaller<'_>, _fd: i32| -> i32 {
        ERRNO_SUCCESS
    })?;
    linker.func_wrap(
        W,
        "fd_seek",
        |_: ShimCaller<'_>, _fd: i32, _off: i64, _whence: i32, _new: i32| -> i32 { ERRNO_SPIPE },
    )?;
    linker.func_wrap(
        W,
        "fd_fdstat_get",
        |_: ShimCaller<'_>, _fd: i32, _buf: i32| -> i32 { ERRNO_BADF },
    )?;
    linker.func_wrap(
        W,
        "fd_prestat_get",
        |_: ShimCaller<'_>, _fd: i32, _buf: i32| -> i32 { ERRNO_BADF },
    )?;
    linker.func_wrap(
        W,
        "fd_prestat_dir_name",
        |_: ShimCaller<'_>, _fd: i32, _path: i32, _len: i32| -> i32 { ERRNO_BADF },
    )?;
    for name in ["environ_sizes_get", "args_sizes_get"] {
        linker.func_wrap(
            W,
            name,
            |mut caller: ShimCaller<'_>, count: i32, size: i32| -> i32 {
                let Some(mem) = guest_memory(&mut caller) else {
                    return ERRNO_INVAL;
                };
                let data = mem.data_mut(&mut caller);
                if write_bytes(data, count as usize, &0u32.to_le_bytes())
                    && write_bytes(data, size as usize, &0u32.to_le_bytes())
                {
                    ERRNO_SUCCESS
                } else {
                    ERRNO_INVAL
                }
            },
        )?;
    }
    for name in ["environ_get", "args_get"] {
        linker.func_wrap(W, name, |_: ShimCaller<'_>, _a: i32, _b: i32| -> i32 {
            ERRNO_SUCCESS
        })?;
    }
    linker.func_wrap(
        W,
        "clock_time_get",
        |mut caller: ShimCaller<'_>, id: i32, _precision: i64, out: i32| -> i32 {
            let nanos: u64 = if id == 0 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64)
            } else {
                host().started.elapsed().as_nanos() as u64
            };
            let Some(mem) = guest_memory(&mut caller) else {
                return ERRNO_INVAL;
            };
            let data = mem.data_mut(&mut caller);
            if write_bytes(data, out as usize, &nanos.to_le_bytes()) {
                ERRNO_SUCCESS
            } else {
                ERRNO_INVAL
            }
        },
    )?;
    linker.func_wrap(
        W,
        "random_get",
        |mut caller: ShimCaller<'_>, buf: i32, len: i32| -> i32 {
            let Some(mem) = guest_memory(&mut caller) else {
                return ERRNO_INVAL;
            };
            let (data, state) = mem.data_and_store_mut(&mut caller);
            for i in 0..len as usize {
                state.rng ^= state.rng << 13;
                state.rng ^= state.rng >> 7;
                state.rng ^= state.rng << 17;
                let Some(byte) = data.get_mut(buf as usize + i) else {
                    return ERRNO_INVAL;
                };
                *byte = state.rng as u8;
            }
            ERRNO_SUCCESS
        },
    )?;
    linker.func_wrap(W, "sched_yield", |_: ShimCaller<'_>| -> i32 {
        ERRNO_SUCCESS
    })?;
    linker.func_wrap(
        W,
        "proc_exit",
        |_: ShimCaller<'_>, code: i32| -> anyhow::Result<()> { Err(ProcExit(code).into()) },
    )?;
    Ok(())
}

/// Setting the epoch deadline from inside a host function.
trait SetDeadline {
    fn as_context_mut_set_deadline(&mut self, ticks: u64);
}

impl SetDeadline for ShimCaller<'_> {
    fn as_context_mut_set_deadline(&mut self, ticks: u64) {
        use wasmtime::AsContextMut as _;
        self.as_context_mut().set_epoch_deadline(ticks);
    }
}

/// The length of the first complete frame at the front of `buf`, if one
/// is there: `u32` header length, header, `u32` payload length, payload.
fn complete_frame(buf: &[u8]) -> Option<usize> {
    let head = read_u32(buf, 0)? as usize;
    let body = read_u32(buf, 4 + head)? as usize;
    let total = 8 + head + body;
    (buf.len() >= total).then_some(total)
}

impl Transport for WasmUnit {
    fn call(
        &mut self,
        request: &Request,
        payload: &[u8],
        hard: Option<Duration>,
    ) -> Result<Response, Death> {
        let started = Instant::now();
        let mut frame = Vec::with_capacity(payload.len() + 256);
        if pmacs_parse_unit::write_frame(&mut frame, request, payload).is_err() {
            return Err(Death::Ended("could not encode the request".into()));
        }
        let ticks = hard.map_or(u64::MAX / 2, |d| {
            (d.as_millis() / EPOCH_TICK.as_millis()).max(1) as u64
        });
        let sent = self
            .requests
            .as_ref()
            .is_some_and(|tx| tx.send((frame, ticks)).is_ok());
        if !sent {
            return Err(self.death(
                hard.map(|d| d.saturating_sub(HARD_GRACE)),
                started.elapsed(),
            ));
        }
        match self.responses.recv() {
            Ok(bytes) => match pmacs_parse_unit::read_frame::<_, Response>(&mut bytes.as_slice()) {
                Ok(Some((response, _))) => Ok(response),
                _ => Err(Death::Ended("unreadable response".into())),
            },
            Err(_) => Err(self.death(
                hard.map(|d| d.saturating_sub(HARD_GRACE)),
                started.elapsed(),
            )),
        }
    }

    fn memory_bytes(&self) -> Option<u64> {
        Some(self.memory.load(Ordering::Relaxed) as u64)
    }

    fn id(&self) -> String {
        format!("wasm {}", self.number)
    }
}

impl Drop for WasmUnit {
    fn drop(&mut self) {
        // Closing the request channel ends the guest's read; the instance
        // returns and its store, linear memory included, is dropped.
        self.requests = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
