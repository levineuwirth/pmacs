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
//! one element at a time.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Arc;
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

/// What the editor asks a unit to do.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    /// Bring the unit's text up to date and parse it.
    Parse(ParseCall),
    /// Highlight spans of the installed tree over these byte ranges.
    Spans {
        /// Half-open byte ranges into the installed tree's text.
        ranges: Vec<(u32, u32)>,
    },
    /// The unit's text length and layer count, for a liveness check.
    Stats,
    /// The installed tree's fold candidates at a byte, innermost first, or
    /// its top-level fold targets when `at` is `None`
    /// (`pmacs_syntax::fold`).
    Folds {
        /// The byte, or `None` for the top-level targets.
        at: Option<u64>,
    },
    /// The installed tree's node at `path` (child indices from the root
    /// layer's root) and, with `children`, each child after it, for Lua's
    /// node API (`pmacs_syntax::describe_path`).
    Describe {
        /// Child indices from the root.
        path: Vec<u32>,
        /// Describe the node's children too.
        children: bool,
    },
    /// The s-expression of the installed tree's node at `path`.
    Sexp {
        /// Child indices from the root.
        path: Vec<u32>,
    },
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

/// What a unit answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    /// The parse installed; its layers and the spans over the interest.
    Parsed(Parsed),
    /// Spans of the installed tree over the requested ranges.
    Spans(SpanSet),
    /// The request installed nothing; the previous tree stays installed.
    Failed(Failure),
    /// The unit's text length and installed layer count.
    Stats {
        /// Bytes in the unit's text mirror.
        source_len: u32,
        /// Layers in the installed tree, 0 when none is installed.
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
    /// A spans request reached a unit with no installed tree.
    NoTreeInstalled,
}

/// The installed parse, with each layer's query and local facts resolved
/// as `SyntaxRegistry::resolve_layer_queries` resolves them.
struct Installed {
    bundle: ParseTreeBundle,
    queries: Vec<Option<Arc<tree_sitter::Query>>>,
    facts: Vec<Option<LocalFacts>>,
}

/// One buffer's parse state.
#[derive(Default)]
pub struct Unit {
    source: Vec<u8>,
    language: Option<(String, tree_sitter::Language)>,
    installed: Option<Installed>,
    /// The last request installed nothing, so the installed tree predates
    /// edits that request applied: the next parse starts cold, as
    /// `ParseViewHandle::mark_unparsed` makes the in-process one.
    cold_next: bool,
    highlights: HashMap<String, Option<Arc<tree_sitter::Query>>>,
    locals: HashMap<String, Option<Arc<tree_sitter::Query>>>,
}

impl Unit {
    /// An empty unit: no text, no tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer one request.
    pub fn handle(&mut self, request: Request, payload: &[u8]) -> Response {
        match request {
            Request::Parse(call) => self.parse(&call, payload),
            Request::Spans { ranges } => match self.installed.as_ref() {
                Some(installed) => Response::Spans(spans_over(installed, &ranges)),
                None => Response::Failed(Failure::NoTreeInstalled),
            },
            Request::Folds { at } => match self.installed.as_ref() {
                Some(installed) => Response::Folds(match at {
                    Some(p) => pmacs_syntax::fold::candidates_at(&installed.bundle, p),
                    None => pmacs_syntax::fold::top_level_targets(&installed.bundle),
                }),
                None => Response::Failed(Failure::NoTreeInstalled),
            },
            Request::Describe { path, children } => match self.installed.as_ref() {
                Some(installed) => Response::Nodes(
                    pmacs_syntax::describe_path(&installed.bundle, &path, children)
                        .into_iter()
                        .map(WireNode::from)
                        .collect(),
                ),
                None => Response::Failed(Failure::NoTreeInstalled),
            },
            Request::Sexp { path } => match self.installed.as_ref() {
                Some(installed) => Response::Text(
                    pmacs_syntax::node_at_path(&installed.bundle, &path).map(|n| n.to_sexp()),
                ),
                None => Response::Failed(Failure::NoTreeInstalled),
            },
            Request::Stats => Response::Stats {
                source_len: self.source.len() as u32,
                layers: self
                    .installed
                    .as_ref()
                    .map_or(0, |i| i.bundle.layers.len() as u32),
            },
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

    fn parse(&mut self, call: &ParseCall, payload: &[u8]) -> Response {
        let started = Instant::now();
        if let Err(failure) = self.update_text(call, payload) {
            self.cold_next = true;
            return Response::Failed(failure);
        }
        let language = match &self.language {
            Some((name, language)) if *name == call.language => language.clone(),
            _ => {
                let Some(entry) = BUILTIN_LANGUAGES.iter().find(|e| e.name == call.language) else {
                    return Response::Failed(Failure::Language(format!(
                        "unknown language {}",
                        call.language
                    )));
                };
                let language = (entry.loader)();
                self.language = Some((call.language.clone(), language.clone()));
                self.cold_next = true;
                language
            }
        };
        let prior_tree = if self.cold_next {
            None
        } else {
            self.installed
                .as_ref()
                .map(|i| i.bundle.root_tree().clone())
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
                return Response::Failed(match error {
                    ParseError::DeadlineExceeded { deadline, after } => Failure::Deadline {
                        deadline_ms: deadline.as_millis() as u64,
                        after_ms: after.as_millis() as u64,
                    },
                    ParseError::NoTree => Failure::NoTree,
                    ParseError::Language(message) => Failure::Language(message),
                });
            }
        };
        self.cold_next = false;
        let installed = self.install(bundle);
        let spans = spans_over(&installed, &call.interest);
        let parsed = Parsed {
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
        self.installed = Some(installed);
        Response::Parsed(parsed)
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

/// Serve requests from `input` until it ends, answering on `output`.
pub fn serve<R: Read, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
    let mut unit = Unit::new();
    while let Some((request, payload)) = read_frame::<_, Request>(&mut input)? {
        let response = unit.handle(request, &payload);
        write_frame(&mut output, &response, &[])?;
    }
    Ok(())
}
