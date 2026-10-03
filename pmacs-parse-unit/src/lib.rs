// pmacs-parse-unit --- one buffer's parse behind a boundary (E7i).

//! The parse unit: one buffer's tree-sitter state, its parse and its
//! capture walks, run outside the editor.
//!
//! The editor talks to a unit, the worker process `pmacs-parse-unit` it
//! spawns per buffer, over a byte stream: a request frame, then a response
//! frame. The parse is `pmacs_syntax::run_parse`, the code the editor ran
//! in-process; what the unit adds is the text mirror, the installed tree
//! and the batching: a parse answers with the highlight spans for the byte
//! ranges the editor is showing, and a read of the tree (more spans, folds,
//! a node and its children) is one request, so no capture walk and no node
//! crosses the boundary one at a time.
//!
//! A frame is a little-endian `u32` length, a postcard header of that
//! length, a second `u32` length and that many raw payload bytes. Text
//! travels in the payload, so a file's bytes are copied, not serialized
//! one element at a time. A request's header is `(id, Request)` and its
//! answer's `(id, Response)`, the same id; id 0 is the unit's own word,
//! [`Response::Resident`], sent while it parses when the editor asks for
//! it (`--report-memory`).
//!
//! A parse runs on a thread of its own, so a read of the installed tree
//! (spans, folds, a node) is answered while a newer parse runs: the editor
//! never waits on a parse to read the tree it shows. The unit keeps its
//! two most recently installed trees, each with the generation its parse
//! answered with, and a read names the generation it wants; one the unit
//! no longer keeps answers [`Failure::Stale`].

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use pmacs_syntax::{
    BUILTIN_LANGUAGES, LocalFacts, ParseError, ParseRequest, ParseTreeBundle,
    compute_highlight_spans_for, compute_local_facts, query_uses_local_predicates, run_parse,
};

/// The largest header or payload either side accepts, so a corrupt length
/// cannot make a reader allocate without bound.
pub const MAX_FRAME_PART: u32 = 1 << 30;

/// The protocol this unit speaks, answered to [`Request::Hello`]. A worker
/// beside a `pmacs` built from another tree speaks another one, and the
/// editor refuses it rather than misreading its frames.
pub const PROTOCOL: u32 = 2;

/// The exit status of a worker its memory watch stopped.
pub const MEMORY_WATCH_EXIT: i32 = 86;

/// Write one frame: `header`, then `payload` as raw bytes.
pub fn write_frame<W: Write, T: Serialize>(
    w: &mut W,
    header: &T,
    payload: &[u8],
) -> io::Result<()> {
    let head = postcard::to_stdvec(header).map_err(io::Error::other)?;
    let head_len = u32::try_from(head.len()).map_err(|_| io::Error::other("header too large"))?;
    let body_len =
        u32::try_from(payload.len()).map_err(|_| io::Error::other("payload too large"))?;
    w.write_all(&head_len.to_le_bytes())?;
    w.write_all(&head)?;
    w.write_all(&body_len.to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// Read one frame, or `None` at a clean end of stream before a frame began.
pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<(T, Vec<u8>)>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let head = read_part(r, u32::from_le_bytes(len))?;
    r.read_exact(&mut len)?;
    let payload = read_part(r, u32::from_le_bytes(len))?;
    let header = postcard::from_bytes(&head).map_err(io::Error::other)?;
    Ok(Some((header, payload)))
}

fn read_part<R: Read>(r: &mut R, len: u32) -> io::Result<Vec<u8>> {
    if len > MAX_FRAME_PART {
        return Err(io::Error::other(format!("frame part of {len} bytes")));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// What the editor asks a unit to do. `Hello` stays the first variant in
/// every protocol, so any two builds agree on how to ask which one the
/// other speaks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    /// Which protocol the unit speaks.
    Hello,
    /// Bring the unit's text up to date and parse it.
    Parse(ParseCall),
    /// Highlight spans of an installed tree over these byte ranges.
    Spans {
        /// The tree's generation, as its parse answered.
        generation: u64,
        /// Half-open byte ranges into that tree's text.
        ranges: Vec<(u32, u32)>,
    },
    /// The unit's text length and layer count, for a liveness check.
    Stats,
    /// An installed tree's fold candidates at a byte, innermost first, or
    /// its top-level fold targets when `at` is `None`
    /// (`pmacs_syntax::fold`).
    Folds {
        /// The tree's generation.
        generation: u64,
        /// The byte, or `None` for the top-level targets.
        at: Option<u64>,
    },
    /// An installed tree's node at `path` (child indices from the root
    /// layer's root) and, with `children`, each child after it, for Lua's
    /// node API (`pmacs_syntax::describe_path`).
    Describe {
        /// The tree's generation.
        generation: u64,
        /// Child indices from the root.
        path: Vec<u32>,
        /// Describe the node's children too.
        children: bool,
    },
    /// The s-expression of an installed tree's node at `path`.
    Sexp {
        /// The tree's generation.
        generation: u64,
        /// Child indices from the root.
        path: Vec<u32>,
    },
}

impl Request {
    /// Whether the unit answers this on its parse thread, in order.
    fn is_parse(&self) -> bool {
        matches!(self, Self::Parse(_))
    }
}

/// How a [`ParseCall`]'s payload brings the unit's text up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextUpdate {
    /// The payload is the whole text; the unit parses it cold.
    Full,
    /// The payload is the inserted bytes of [`ParseCall::edits`],
    /// concatenated in order; each edit's are `new_end - start` long.
    Edits,
}

/// One `tree_sitter::InputEdit`, on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEdit {
    /// First byte the edit touched.
    pub start_byte: u32,
    /// End of the replaced range in the text before the edit.
    pub old_end_byte: u32,
    /// End of the inserted range in the text after the edit.
    pub new_end_byte: u32,
    /// `(row, column)` of `start_byte`.
    pub start: (u32, u32),
    /// `(row, column)` of `old_end_byte` before the edit.
    pub old_end: (u32, u32),
    /// `(row, column)` of `new_end_byte` after the edit.
    pub new_end: (u32, u32),
}

impl WireEdit {
    /// The edit as tree-sitter takes it.
    #[must_use]
    pub fn input_edit(&self) -> tree_sitter::InputEdit {
        let point =
            |(row, column): (u32, u32)| tree_sitter::Point::new(row as usize, column as usize);
        tree_sitter::InputEdit {
            start_byte: self.start_byte as usize,
            old_end_byte: self.old_end_byte as usize,
            new_end_byte: self.new_end_byte as usize,
            start_position: point(self.start),
            old_end_position: point(self.old_end),
            new_end_position: point(self.new_end),
        }
    }

    /// The wire form of `edit`.
    #[must_use]
    pub fn from_input_edit(edit: &tree_sitter::InputEdit) -> Self {
        let point = |p: tree_sitter::Point| (p.row as u32, p.column as u32);
        Self {
            start_byte: edit.start_byte as u32,
            old_end_byte: edit.old_end_byte as u32,
            new_end_byte: edit.new_end_byte as u32,
            start: point(edit.start_position),
            old_end: point(edit.old_end_position),
            new_end: point(edit.new_end_position),
        }
    }
}

/// A parse request: the text update, the language, the deadline and the
/// ranges whose spans should come back with the tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParseCall {
    /// Canonical language name, a `BUILTIN_LANGUAGES` entry.
    pub language: String,
    /// What the payload holds.
    pub text: TextUpdate,
    /// The edits since the unit's last request, for [`TextUpdate::Edits`].
    pub edits: Vec<WireEdit>,
    /// The text's length after the update; a unit whose mirror disagrees
    /// answers [`Failure::Desync`] and the editor sends the whole text.
    pub expect_len: u32,
    /// The injection alias snapshot (fence name to language).
    pub aliases: Vec<(String, String)>,
    /// `syntax.parse-deadline-ms`, enforced inside the unit where
    /// tree-sitter calls its progress callback, exactly as in-process.
    pub deadline_ms: Option<u64>,
    /// Byte ranges of the new text whose highlight spans to return.
    pub interest: Vec<(u32, u32)>,
}

/// What a unit answers. `Hello` stays the first variant in every protocol.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    /// The protocol the unit speaks, [`PROTOCOL`].
    Hello {
        /// The protocol.
        protocol: u32,
    },
    /// The parse installed; its layers and the spans over the interest.
    Parsed(Parsed),
    /// Spans of the installed tree over the requested ranges.
    Spans(SpanSet),
    /// The request installed or read nothing; for a parse, the previous
    /// tree stays installed.
    Failed(Failure),
    /// The unit's text length and installed layer count.
    Stats {
        /// Bytes in the unit's text mirror.
        source_len: u32,
        /// Layers in the newest installed tree, 0 when none is installed.
        layers: u32,
    },
    /// Fold ranges, `(start, end)`, as [`Request::Folds`] asked.
    Folds(Vec<(u64, u64)>),
    /// Nodes, as [`Request::Describe`] asked; empty when the path names no
    /// node.
    Nodes(Vec<WireNode>),
    /// Text, as [`Request::Sexp`] asked; `None` when the path names no
    /// node.
    Text(Option<String>),
    /// The unit's resident memory now, bytes, sent unasked (id 0) while it
    /// parses when started with `--report-memory`: what the editor's total
    /// watchdog sums where no cgroup holds the workers.
    Resident(u64),
}

/// `pmacs_syntax::NodeFacts` on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNode {
    /// The node's kind.
    pub kind: String,
    /// First byte.
    pub start_byte: u64,
    /// End byte, exclusive.
    pub end_byte: u64,
    /// `(row, column)` of the start.
    pub start: (u64, u64),
    /// `(row, column)` of the end.
    pub end: (u64, u64),
    /// Children, named and anonymous.
    pub child_count: u32,
    /// Named children.
    pub named_child_count: u32,
    /// Named, missing, has-error.
    pub flags: (bool, bool, bool),
}

impl From<pmacs_syntax::NodeFacts> for WireNode {
    fn from(f: pmacs_syntax::NodeFacts) -> Self {
        Self {
            kind: f.kind,
            start_byte: f.start_byte,
            end_byte: f.end_byte,
            start: f.start,
            end: f.end,
            child_count: f.child_count,
            named_child_count: f.named_child_count,
            flags: (f.named, f.missing, f.has_error),
        }
    }
}

impl From<WireNode> for pmacs_syntax::NodeFacts {
    fn from(w: WireNode) -> Self {
        Self {
            kind: w.kind,
            start_byte: w.start_byte,
            end_byte: w.end_byte,
            start: w.start,
            end: w.end,
            child_count: w.child_count,
            named_child_count: w.named_child_count,
            named: w.flags.0,
            missing: w.flags.1,
            has_error: w.flags.2,
        }
    }
}

/// A parse that installed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Parsed {
    /// The installed tree's generation, which a read of it names.
    pub generation: u64,
    /// The unit's peak resident memory after the parse, bytes, as the
    /// kernel counts it (`getrusage`); 0 where it cannot be read.
    pub peak_bytes: u64,
    /// The installed layers: the root first, then injections in depth
    /// order, as `ParseTreeBundle::layers` holds them.
    pub layers: Vec<LayerInfo>,
    /// Spans over the call's interest ranges.
    pub spans: SpanSet,
    /// The root parse alone, as `ParseTreeBundle::parse_duration`.
    pub root_parse_us: u64,
    /// The whole request inside the unit: update, parse, layers, spans.
    pub total_us: u64,
    /// The injection layer backstop dropped regions.
    pub injection_capped: bool,
    /// The deadline dropped injection layers not yet parsed.
    pub layers_cut_by_deadline: bool,
}

/// One installed layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerInfo {
    /// Canonical language name of the layer's grammar.
    pub language: String,
    /// Injection depth, 0 for the root.
    pub depth: u16,
}

/// Highlight spans over a set of byte ranges, per layer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SpanSet {
    /// The ranges these spans cover, normalized (sorted, disjoint).
    pub covered: Vec<(u32, u32)>,
    /// Spans per layer that has a highlight query, in layer order.
    pub layers: Vec<LayerSpans>,
    /// Each language's capture names, indexed by a span's capture index.
    pub capture_names: Vec<(String, Vec<String>)>,
}

/// One layer's spans, sorted as `compute_highlight_spans_for` sorts them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerSpans {
    /// Index into [`Parsed::layers`].
    pub layer: u32,
    /// `(start_byte, end_byte, capture_index)`.
    pub spans: Vec<(u32, u32, u32)>,
}

/// Why a request installed nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Failure {
    /// The parse ran past its deadline and was cancelled through the
    /// progress callback, as in-process.
    Deadline {
        /// The deadline the call carried.
        deadline_ms: u64,
        /// How long the parse had run.
        after_ms: u64,
    },
    /// The parser returned no tree.
    NoTree,
    /// The grammar is unknown to the unit or refused by the parser.
    Language(String),
    /// The edits did not bring the mirror to the expected length.
    Desync {
        /// The mirror's length after the edits.
        have: u32,
        /// The length the editor expected.
        want: u32,
    },
    /// A read reached a unit with no installed tree.
    NoTreeInstalled,
    /// A read named a generation the unit no longer keeps: two newer
    /// parses have installed since.
    Stale,
}

/// An installed parse, with each layer's query and local facts resolved
/// as `SyntaxRegistry::resolve_layer_queries` resolves them.
struct Installed {
    bundle: ParseTreeBundle,
    queries: Vec<Option<Arc<tree_sitter::Query>>>,
    facts: Vec<Option<LocalFacts>>,
}

/// The installed trees a read can name: the newest, and the one before it,
/// which the editor may still be showing between the unit installing the
/// newest and the editor settling it.
#[derive(Default)]
struct Kept {
    newest: Option<(u64, Arc<Installed>)>,
    before: Option<(u64, Arc<Installed>)>,
}

impl Kept {
    fn get(&self, generation: u64) -> Result<Arc<Installed>, Failure> {
        for (g, installed) in [&self.newest, &self.before].into_iter().flatten() {
            if *g == generation {
                return Ok(installed.clone());
            }
        }
        if self.newest.is_none() {
            Err(Failure::NoTreeInstalled)
        } else {
            Err(Failure::Stale)
        }
    }

    /// Install `installed` as `generation`; the tree it evicts comes back,
    /// for the caller to drop after answering (dropping a large tree takes
    /// milliseconds the editor need not wait for).
    fn install(&mut self, generation: u64, installed: Arc<Installed>) -> Option<Arc<Installed>> {
        let evicted = self.before.take().map(|(_, i)| i);
        self.before = self.newest.replace((generation, installed));
        evicted
    }
}

/// One buffer's parse state: the text mirror and what a parse needs, and
/// the installed trees, which reads share.
#[derive(Default)]
pub struct Unit {
    source: Vec<u8>,
    language: Option<(String, tree_sitter::Language)>,
    /// The last request installed nothing, so the installed tree predates
    /// edits that request applied: the next parse starts cold, as
    /// `ParseViewHandle::mark_unparsed` makes the in-process one.
    cold_next: bool,
    highlights: HashMap<String, Option<Arc<tree_sitter::Query>>>,
    locals: HashMap<String, Option<Arc<tree_sitter::Query>>>,
    /// The last generation installed.
    generation: u64,
    kept: Arc<Mutex<Kept>>,
}

impl Unit {
    /// An empty unit: no text, no tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer one request, in order: a parse here, a read from the trees
    /// kept. [`serve`] answers reads while a parse runs instead.
    pub fn handle(&mut self, request: Request, payload: &[u8]) -> Response {
        match request {
            Request::Parse(call) => self.parse(&call, payload).0,
            read => answer_read(&self.kept, read),
        }
    }

    /// Bring the mirror up to date from `call` and its payload.
    fn update_text(&mut self, call: &ParseCall, payload: &[u8]) -> Result<(), Failure> {
        let desync = |have: usize| Failure::Desync {
            have: have as u32,
            want: call.expect_len,
        };
        match call.text {
            TextUpdate::Full => {
                self.source.clear();
                self.source.extend_from_slice(payload);
                self.cold_next = true;
            }
            TextUpdate::Edits => {
                let mut at = 0usize;
                for edit in &call.edits {
                    let n = (edit.new_end_byte - edit.start_byte) as usize;
                    let (start, old_end) = (edit.start_byte as usize, edit.old_end_byte as usize);
                    if at + n > payload.len() || start > old_end || old_end > self.source.len() {
                        return Err(desync(self.source.len()));
                    }
                    self.source
                        .splice(start..old_end, payload[at..at + n].iter().copied());
                    at += n;
                }
            }
        }
        if self.source.len() == call.expect_len as usize {
            Ok(())
        } else {
            Err(desync(self.source.len()))
        }
    }

    /// Parse, install, and answer; the tree the install evicted comes back
    /// with the answer, to be dropped after it is sent.
    fn parse(&mut self, call: &ParseCall, payload: &[u8]) -> (Response, Option<Arc<Installed>>) {
        let started = Instant::now();
        if let Err(failure) = self.update_text(call, payload) {
            self.cold_next = true;
            return (Response::Failed(failure), None);
        }
        let language = match &self.language {
            Some((name, language)) if *name == call.language => language.clone(),
            _ => {
                let Some(entry) = BUILTIN_LANGUAGES.iter().find(|e| e.name == call.language) else {
                    let failure = Failure::Language(format!("unknown language {}", call.language));
                    return (Response::Failed(failure), None);
                };
                let language = (entry.loader)();
                self.language = Some((call.language.clone(), language.clone()));
                self.cold_next = true;
                language
            }
        };
        // The prior tree is a copy (`ts_tree_copy`): the parse edits it,
        // and a read may be walking the installed one on another thread.
        let prior_tree = if self.cold_next {
            None
        } else {
            self.kept
                .lock()
                .expect("kept trees poisoned")
                .newest
                .as_ref()
                .map(|(_, i)| i.bundle.root_tree().clone())
        };
        let edits = if prior_tree.is_some() {
            call.edits.iter().map(WireEdit::input_edit).collect()
        } else {
            Vec::new()
        };
        let request = ParseRequest {
            source: Arc::from(self.source.as_slice()),
            language,
            language_name: call.language.clone(),
            prior_tree,
            edits,
            injection_aliases: Arc::new(call.aliases.iter().cloned().collect()),
            deadline: call
                .deadline_ms
                .filter(|&ms| ms > 0)
                .map(Duration::from_millis),
        };
        let bundle = match run_parse(request) {
            Ok(bundle) => bundle,
            Err(error) => {
                self.cold_next = true;
                return (
                    Response::Failed(match error {
                        ParseError::DeadlineExceeded { deadline, after } => Failure::Deadline {
                            deadline_ms: deadline.as_millis() as u64,
                            after_ms: after.as_millis() as u64,
                        },
                        ParseError::NoTree => Failure::NoTree,
                        ParseError::Language(message) => Failure::Language(message),
                    }),
                    None,
                );
            }
        };
        self.cold_next = false;
        let installed = Arc::new(self.install(bundle));
        let spans = spans_over(&installed, &call.interest);
        self.generation += 1;
        let parsed = Parsed {
            generation: self.generation,
            peak_bytes: peak_resident_bytes(),
            layers: installed
                .bundle
                .layers
                .iter()
                .map(|l| LayerInfo {
                    language: l.language_name.clone(),
                    depth: l.depth,
                })
                .collect(),
            spans,
            root_parse_us: installed.bundle.parse_duration.as_micros() as u64,
            total_us: started.elapsed().as_micros() as u64,
            injection_capped: installed.bundle.injection_capped,
            layers_cut_by_deadline: installed.bundle.layers_cut_by_deadline,
        };
        let evicted = self
            .kept
            .lock()
            .expect("kept trees poisoned")
            .install(self.generation, installed);
        (Response::Parsed(parsed), evicted)
    }

    fn install(&mut self, bundle: ParseTreeBundle) -> Installed {
        let mut queries = Vec::with_capacity(bundle.layers.len());
        let mut facts = Vec::with_capacity(bundle.layers.len());
        for layer in &bundle.layers {
            let query = cached_query(&mut self.highlights, &layer.language_name, |e| {
                e.highlights_query
            });
            let local = query
                .as_deref()
                .filter(|q| query_uses_local_predicates(q))
                .and_then(|_| {
                    cached_query(&mut self.locals, &layer.language_name, |e| e.locals_query)
                })
                .map(|q| compute_local_facts(&q, &layer.tree, bundle.source.as_ref()));
            queries.push(query);
            facts.push(local);
        }
        Installed {
            bundle,
            queries,
            facts,
        }
    }
}

/// Answer a read from the trees `kept`, holding the lock only to take the
/// named tree: the walk itself runs beside any parse.
fn answer_read(kept: &Mutex<Kept>, request: Request) -> Response {
    let tree = |generation: u64| kept.lock().expect("kept trees poisoned").get(generation);
    let answer = |generation: u64, read: &dyn Fn(&Installed) -> Response| match tree(generation) {
        Ok(installed) => read(&installed),
        Err(failure) => Response::Failed(failure),
    };
    match request {
        Request::Hello => Response::Hello { protocol: PROTOCOL },
        Request::Stats => {
            let kept = kept.lock().expect("kept trees poisoned");
            let newest = kept.newest.as_ref().map(|(_, i)| i);
            Response::Stats {
                source_len: newest.map_or(0, |i| i.bundle.source.len() as u32),
                layers: newest.map_or(0, |i| i.bundle.layers.len() as u32),
            }
        }
        Request::Spans { generation, ranges } => answer(generation, &|installed| {
            Response::Spans(spans_over(installed, &ranges))
        }),
        Request::Folds { generation, at } => answer(generation, &|installed| {
            Response::Folds(match at {
                Some(p) => pmacs_syntax::fold::candidates_at(&installed.bundle, p),
                None => pmacs_syntax::fold::top_level_targets(&installed.bundle),
            })
        }),
        Request::Describe {
            generation,
            path,
            children,
        } => answer(generation, &|installed| {
            Response::Nodes(
                pmacs_syntax::describe_path(&installed.bundle, &path, children)
                    .into_iter()
                    .map(WireNode::from)
                    .collect(),
            )
        }),
        Request::Sexp { generation, path } => answer(generation, &|installed| {
            Response::Text(
                pmacs_syntax::node_at_path(&installed.bundle, &path).map(|n| n.to_sexp()),
            )
        }),
        Request::Parse(_) => Response::Failed(Failure::Language(
            "a parse reached the read path".to_owned(),
        )),
    }
}

/// Compile and cache a language's query from its `BUILTIN_LANGUAGES`
/// fragments, joined with a newline as the registry joins them.
fn cached_query(
    cache: &mut HashMap<String, Option<Arc<tree_sitter::Query>>>,
    language: &str,
    fragments: fn(&pmacs_syntax::LanguageEntry) -> &'static [&'static str],
) -> Option<Arc<tree_sitter::Query>> {
    if let Some(slot) = cache.get(language) {
        return slot.clone();
    }
    let compiled = BUILTIN_LANGUAGES
        .iter()
        .find(|e| e.name == language)
        .and_then(|entry| {
            let source = fragments(entry).join("\n");
            if source.trim().is_empty() {
                return None;
            }
            tree_sitter::Query::new(&(entry.loader)(), &source)
                .ok()
                .map(Arc::new)
        });
    cache.insert(language.to_owned(), compiled.clone());
    compiled
}

/// Sort and merge byte ranges into disjoint ones.
#[must_use]
pub fn normalize(ranges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut sorted: Vec<(u32, u32)> = ranges.iter().copied().filter(|(s, e)| s < e).collect();
    sorted.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(sorted.len());
    for (s, e) in sorted {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// The installed tree's spans over `ranges`, per layer with a query.
fn spans_over(installed: &Installed, ranges: &[(u32, u32)]) -> SpanSet {
    let len = installed.bundle.source.len() as u32;
    let clamped: Vec<(u32, u32)> = ranges
        .iter()
        .map(|&(s, e)| (s.min(len), e.min(len)))
        .collect();
    let covered = normalize(&clamped);
    let source = installed.bundle.source.as_ref();
    let mut layers = Vec::new();
    let mut names: Vec<(String, Vec<String>)> = Vec::new();
    for (index, layer) in installed.bundle.layers.iter().enumerate() {
        let Some(query) = installed.queries[index].as_ref() else {
            continue;
        };
        let mut spans: Vec<(u32, u32, u32)> = Vec::new();
        for &(s, e) in &covered {
            for span in compute_highlight_spans_for(
                query,
                &layer.tree,
                source,
                installed.facts[index].as_ref(),
                Some(s as usize..e as usize),
            ) {
                spans.push((span.start_byte, span.end_byte, span.capture_index));
            }
        }
        if covered.len() > 1 {
            spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
            spans.dedup();
        }
        if !names.iter().any(|(n, _)| *n == layer.language_name) {
            names.push((
                layer.language_name.clone(),
                query
                    .capture_names()
                    .iter()
                    .map(|n| (*n).to_owned())
                    .collect(),
            ));
        }
        layers.push(LayerSpans {
            layer: index as u32,
            spans,
        });
    }
    SpanSet {
        covered,
        layers,
        capture_names: names,
    }
}

/// What [`serve`] watches and reports about the unit's own memory.
#[derive(Clone, Copy, Debug, Default)]
pub struct ServeOptions {
    /// Stop the process ([`MEMORY_WATCH_EXIT`]) once its peak resident
    /// memory passes what it held at start plus this many bytes: the memory
    /// limit where `RLIMIT_AS` is refused (macOS), checked every
    /// [`WATCH_EVERY`] while a parse runs. Reactive where `RLIMIT_AS` is
    /// preventive: it overshoots by what the parse grows between two checks.
    pub watch_growth: Option<u64>,
    /// Send [`Response::Resident`] every [`REPORT_EVERY`] while a parse runs
    /// and once after it, for the editor's total watchdog.
    pub report_memory: bool,
}

/// How often the memory watch reads the unit's peak while a parse runs.
pub const WATCH_EVERY: Duration = Duration::from_millis(1);

/// How often a parsing unit reports its resident memory when asked to.
pub const REPORT_EVERY: Duration = Duration::from_millis(10);

/// Whether a parse is running, and whether the unit is stopping.
#[derive(Default)]
struct Parsing {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl Parsing {
    fn set(&self, parsing: bool) {
        self.state.lock().expect("parsing flag poisoned").0 = parsing;
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.state.lock().expect("parsing flag poisoned").1 = true;
        self.changed.notify_all();
    }

    /// Wait until a parse runs (`true`) or the unit stops (`false`).
    fn wait(&self) -> bool {
        let mut state = self.state.lock().expect("parsing flag poisoned");
        loop {
            if state.1 {
                return false;
            }
            if state.0 {
                return true;
            }
            state = self.changed.wait(state).expect("parsing flag poisoned");
        }
    }

    fn running(&self) -> bool {
        let state = self.state.lock().expect("parsing flag poisoned");
        state.0 && !state.1
    }
}

/// Serve requests from `input` until it ends, answering on `output`. A
/// parse runs on a thread of its own, in arrival order; every other request
/// is answered here at once, so reads never wait on a parse.
pub fn serve<R: Read, W: Write + Send + 'static>(
    mut input: R,
    output: W,
    options: ServeOptions,
) -> io::Result<()> {
    let out = Arc::new(Mutex::new(output));
    let parsing = Arc::new(Parsing::default());
    let mut unit = Unit::new();
    let kept = unit.kept.clone();
    let (parses, queue) = mpsc::channel::<(u64, ParseCall, Vec<u8>)>();
    let parser = {
        let out = out.clone();
        let parsing = parsing.clone();
        thread::Builder::new()
            .name("pmacs-parse".into())
            .spawn(move || {
                for (id, call, payload) in queue {
                    parsing.set(true);
                    let (response, evicted) = unit.parse(&call, &payload);
                    parsing.set(false);
                    let mut out = out.lock().expect("output poisoned");
                    let mut sent = write_frame(&mut *out, &(id, &response), &[]);
                    if options.report_memory && sent.is_ok() {
                        sent = write_frame(
                            &mut *out,
                            &(0u64, &Response::Resident(resident_bytes())),
                            &[],
                        );
                    }
                    drop(out);
                    drop(evicted);
                    if sent.is_err() {
                        break;
                    }
                }
            })?
    };
    let watcher = if options.watch_growth.is_some() || options.report_memory {
        let out = out.clone();
        let parsing = parsing.clone();
        Some(
            thread::Builder::new()
                .name("pmacs-memory-watch".into())
                .spawn(move || watch_memory(&parsing, &out, options))?,
        )
    } else {
        None
    };
    let served = (|| {
        while let Some(((id, request), payload)) = read_frame::<_, (u64, Request)>(&mut input)? {
            if request.is_parse() {
                let Request::Parse(call) = request else {
                    unreachable!("is_parse")
                };
                if parses.send((id, call, payload)).is_err() {
                    break;
                }
                continue;
            }
            let response = answer_read(&kept, request);
            write_frame(
                &mut *out.lock().expect("output poisoned"),
                &(id, &response),
                &[],
            )?;
        }
        Ok(())
    })();
    drop(parses);
    let _ = parser.join();
    parsing.stop();
    if let Some(watcher) = watcher {
        let _ = watcher.join();
    }
    served
}

/// The memory watch: while a parse runs, stop the process once its peak
/// passes the allowance, and report its resident memory when asked to.
fn watch_memory<W: Write>(parsing: &Parsing, out: &Mutex<W>, options: ServeOptions) {
    let limit = options
        .watch_growth
        .map(|growth| peak_resident_bytes().saturating_add(growth));
    while parsing.wait() {
        let mut last_report: Option<Instant> = None;
        while parsing.running() {
            if let Some(limit) = limit {
                let peak = peak_resident_bytes();
                if peak > limit {
                    eprintln!(
                        "pmacs-parse-unit: memory watch: peak resident {peak} bytes passed the limit {limit}"
                    );
                    std::process::exit(MEMORY_WATCH_EXIT);
                }
            }
            if options.report_memory && last_report.is_none_or(|t| t.elapsed() >= REPORT_EVERY) {
                last_report = Some(Instant::now());
                let report = (0u64, Response::Resident(resident_bytes()));
                if let Ok(mut out) = out.lock() {
                    let _ = write_frame(&mut *out, &report, &[]);
                }
            }
            thread::sleep(WATCH_EVERY);
        }
    }
}

/// This process's peak resident memory, bytes, as the kernel counts it
/// (`getrusage`: kibibytes on Linux, bytes on macOS); 0 where unread.
#[must_use]
pub fn peak_resident_bytes() -> u64 {
    #[cfg(unix)]
    {
        use nix::sys::resource::{UsageWho, getrusage};
        let Ok(usage) = getrusage(UsageWho::RUSAGE_SELF) else {
            return 0;
        };
        let max = u64::try_from(usage.max_rss()).unwrap_or(0);
        if cfg!(target_os = "macos") {
            max
        } else {
            max * 1024
        }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// This process's resident memory now, bytes: `VmRSS` where `/proc` has it
/// (Linux), else the peak, which is never below it.
#[must_use]
pub fn resident_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.split_whitespace().next())
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map_or_else(peak_resident_bytes, |kb| kb * 1024)
}
