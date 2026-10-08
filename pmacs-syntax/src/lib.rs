// pmacs-syntax --- the parse logic that needs no editor state: the
// request and its bundle, run_parse with its injection layers, the
// bundled-grammar table, local facts and the highlight capture walk.
// Moved verbatim out of `src/syntax.rs` at E7i so the same code runs in
// the editor (the native arm) and inside a parse unit, the worker
// process, where no editor state exists.

//! Tree-sitter parse logic shared by the editor and its parse units.
//!
//! `pmacs::syntax` re-exports every public item here, so the editor's
//! paths are unchanged; see that module for the per-buffer `ParseView`
//! and the registry, which hold editor state and stay there.

#![forbid(unsafe_code)]

pub mod fold;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tree_sitter::{Node, ParseOptions, ParseState, Point, Range, StreamingIterator};

/// Description of a parse job: the source bytes to parse, the
/// language to parse against, the prior tree (if any) for incremental
/// re-parse, and the [`tree_sitter::InputEdit`] descriptions
/// accumulated since that prior tree was produced.
///
/// All fields are owned (R31) so the closure submitted to a worker
/// holds nothing borrowed from the main thread.
#[derive(Clone, Debug)]
pub struct ParseRequest {
    /// Bytes to parse. Materialized from the buffer's rope on the
    /// main thread before dispatch.
    pub source: Arc<[u8]>,
    /// Grammar language. `tree_sitter::Language` is a cheap
    /// pointer-to-static and `Send + Sync + Clone`.
    pub language: tree_sitter::Language,
    /// Human-readable language label, surfaced through Lua and the
    /// `*workers*` buffer ([T M3.7]).
    pub language_name: String,
    /// Tree from a prior parse of the same buffer, or `None` for a
    /// cold parse. The worker calls [`tree_sitter::Tree::edit`] for
    /// every entry in [`Self::edits`] before re-parsing.
    pub prior_tree: Option<tree_sitter::Tree>,
    /// Edits accumulated by the editor's `ParseView` since `prior_tree` was
    /// produced. Empty for cold parses; non-empty drives incremental
    /// re-parse.
    pub edits: Vec<tree_sitter::InputEdit>,
    /// Snapshot of the injection alias map (framing Q#IJ4). The worker
    /// resolves a dynamic `@injection.language` fence-name (`py`, `ts`,
    /// `c++`) through this map — case-folded — before matching it against
    /// [`BUILTIN_LANGUAGES`]. Snapshotted from the registry at dispatch so
    /// the worker never touches the main-thread `Rc` registry or a Lua
    /// table. Empty for the non-layered/legacy callers (no injections
    /// resolve, root parse unaffected).
    pub injection_aliases: Arc<HashMap<String, String>>,
    /// How long the parse may run before it is cancelled (E7h.2): the
    /// root parse and every injection layer's together, measured from
    /// when [`run_parse`] starts, and enforced only where tree-sitter
    /// calls its progress callback ([`run_parse`] names the work it cannot
    /// reach). `None` is unbounded, as every parse was before E7h; the
    /// editor's dispatch fills it from `syntax.parse-deadline-ms`.
    pub deadline: Option<Duration>,
}

/// Why [`run_parse`] produced no bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The parser refused the grammar: an ABI mismatch, a build problem.
    Language(String),
    /// The parser returned no tree although nothing cancelled it.
    NoTree,
    /// The root parse ran past [`ParseRequest::deadline`] and was
    /// cancelled through tree-sitter's progress callback (E7h.2). The
    /// buffer keeps whatever tree it had.
    DeadlineExceeded {
        /// The deadline the request carried.
        deadline: Duration,
        /// How long the parse had run when it returned.
        after: Duration,
    },
}

/// How a [`ParseError::DeadlineExceeded`] message begins. The async
/// runtime carries a worker's error as text, so the settle path tells a
/// cancelled parse from a failed one by [`is_deadline_message`].
pub const PARSE_DEADLINE_MESSAGE: &str = "parse ran past its deadline";

/// Whether a parse job's failure text is a [`ParseError::DeadlineExceeded`].
#[must_use]
pub fn is_deadline_message(message: &str) -> bool {
    message.starts_with(PARSE_DEADLINE_MESSAGE)
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(e) => write!(f, "set_language: {e}"),
            Self::NoTree => f.write_str("parser produced no tree"),
            Self::DeadlineExceeded { deadline, after } => write!(
                f,
                "{PARSE_DEADLINE_MESSAGE} of {} ms and was cancelled after {} ms",
                deadline.as_millis(),
                after.as_millis()
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// Output of [`run_parse`]. The runtime's parse-handoff side map
/// holds these by [`Arc`]; `Lua` introspection (`pmacs::lua_bindings`)
/// resolves a buffer id to its current bundle and walks the tree.
#[derive(Debug)]
pub struct ParseTreeBundle {
    /// Injection layers (framing Q#IJ1). `layers[0]` is the root layer
    /// (the whole buffer, parsed with the buffer's own grammar);
    /// subsequent entries are injected child layers in depth-ascending
    /// order. Non-empty for an in-process parse --- a parse produces at
    /// least the root, so [`Self::root_tree`] never panics there --- and
    /// empty when the trees live in a parse unit ([`Self::isolated`]).
    pub layers: Vec<Layer>,
    /// Source bytes every layer's tree was parsed against. Co-owned with
    /// the request so node-byte-range lookups can read the underlying
    /// text (T M4.1 acceptance: "parse tree introspectable via Lua"
    /// implies the source the tree references). Child layers parse the
    /// *same* full source via `set_included_ranges`, so their node
    /// offsets are absolute into these bytes (framing mechanic #1).
    pub source: Arc<[u8]>,
    /// Root language label (`layers[0].language_name`). Kept here so Lua
    /// and the `*workers*` buffer can ask "what grammar produced this?"
    /// without indexing the layer vec.
    pub language_name: String,
    /// Wall-clock duration of the **root** parse (excludes injection layer
    /// building and dispatch/materialization/bus overhead). The M4.1
    /// acceptance perf gates are stated in this metric, so it stays the
    /// single-tree cost even as injection layers are added on top.
    pub parse_duration: Duration,
    /// True if injection expansion hit the total-layer backstop (framing
    /// Q#IJ3) and dropped some regions. Surfaced (not silent) at settle via
    /// `pmacs.error`; only a pathological file (thousands of embedded
    /// regions) can set it.
    pub injection_capped: bool,
    /// True if the deadline ([`ParseRequest::deadline`]) ran out while
    /// injection layers were being built: the layers not yet parsed were
    /// dropped and the root and the rest installed, surfaced once per
    /// buffer at settle (E7h.2). A root parse past the deadline is
    /// [`ParseError::DeadlineExceeded`] instead and installs nothing.
    pub layers_cut_by_deadline: bool,
    /// E7i: set when the parse ran in a parse unit, the worker process
    /// that holds the trees; `layers` is then empty and the editor reads
    /// highlight spans, folds and nodes through this instead of walking a
    /// tree.
    pub isolated: Option<Arc<dyn IsolatedTree>>,
}

/// Highlight spans of a tree that lives in a parse unit (E7i), as the
/// editor holds them: per layer, in layer order, over the byte ranges in
/// `covered`.
#[derive(Debug, Default)]
pub struct IsolatedSpans {
    /// The byte ranges these spans cover, sorted and disjoint.
    pub covered: Vec<(u32, u32)>,
    /// One entry per layer with a highlight query, in layer order.
    pub layers: Vec<IsolatedLayerSpans>,
}

/// One layer's spans in an [`IsolatedSpans`].
#[derive(Debug)]
pub struct IsolatedLayerSpans {
    /// The layer's index among the unit's layers (0 is the root).
    pub layer: usize,
    /// The capture names the spans' `capture_index` indexes.
    pub capture_names: Arc<[String]>,
    /// Spans sorted as [`compute_highlight_spans_for`] sorts them.
    pub spans: Vec<HighlightSpan>,
}

impl IsolatedSpans {
    /// Whether `range` lies inside one of the covered ranges.
    #[must_use]
    pub fn covers(&self, range: &std::ops::Range<usize>) -> bool {
        self.covered
            .iter()
            .any(|&(s, e)| s as usize <= range.start && range.end <= e as usize)
    }
}

/// The editor's handle on a tree held by a parse unit (E7i). The spans
/// that came back with the parse are always there; a range they do not
/// cover is asked of the unit when it is idle and holds this tree, or of
/// a fresh unit the handle re-parses this tree's text into when the old
/// one was discarded.
pub trait IsolatedTree: Send + Sync + fmt::Debug {
    /// Spans covering `range`, or `None` when the unit cannot answer now.
    fn spans_for(&self, range: std::ops::Range<usize>) -> Option<Arc<IsolatedSpans>>;
    /// The layers the unit installed, the root first.
    fn layer_languages(&self) -> Vec<String>;
    /// The fold candidates at `pos`, innermost first, as
    /// [`fold::candidates_at`] finds them in the unit's tree; `None` when
    /// the unit cannot answer now.
    fn fold_candidates(&self, pos: u64) -> Option<Vec<(u64, u64)>>;
    /// The top-level fold targets, as [`fold::top_level_targets`] finds
    /// them; `None` when the unit cannot answer now.
    fn top_level_folds(&self) -> Option<Vec<(u64, u64)>>;
    /// The node at `path` (child indices from the root layer's root) and,
    /// with `children`, each of its children after it, in order: what
    /// Lua's node API reads, so a script walks a tree in a unit with one
    /// request per node whose children it reads. `None` when the unit
    /// cannot answer now (busy, or holding a newer tree); empty when the
    /// path names no node.
    fn describe(&self, path: &[u32], children: bool) -> Option<Vec<NodeFacts>>;
    /// The s-expression of the node at `path`; `None` when the unit cannot
    /// answer now or the path names no node.
    fn sexp(&self, path: &[u32]) -> Option<String>;
}

/// One node's fields as a parse unit describes it (E7i), the ones Lua's
/// node API reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeFacts {
    /// The node's kind.
    pub kind: String,
    /// First byte, absolute into the bundle's source.
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
    /// The node is named (not anonymous syntax).
    pub named: bool,
    /// The parser inserted it to recover from an error.
    pub missing: bool,
    /// It or a descendant is an error.
    pub has_error: bool,
}

impl NodeFacts {
    /// The facts of `node`.
    #[must_use]
    pub fn of(node: Node<'_>) -> Self {
        let point = |p: Point| (p.row as u64, p.column as u64);
        Self {
            kind: node.kind().to_owned(),
            start_byte: node.start_byte() as u64,
            end_byte: node.end_byte() as u64,
            start: point(node.start_position()),
            end: point(node.end_position()),
            child_count: node.child_count() as u32,
            named_child_count: node.named_child_count() as u32,
            named: node.is_named(),
            missing: node.is_missing(),
            has_error: node.has_error(),
        }
    }
}

/// The node at `path` in `bundle`'s root layer, and with `children` each of
/// its children after it: the answer to [`IsolatedTree::describe`], read
/// where the tree is. Empty when the path names no node.
#[must_use]
pub fn describe_path(bundle: &ParseTreeBundle, path: &[u32], children: bool) -> Vec<NodeFacts> {
    let Some(node) = node_at_path(bundle, path) else {
        return Vec::new();
    };
    let mut out = vec![NodeFacts::of(node)];
    if children {
        let mut cursor = node.walk();
        out.extend(node.children(&mut cursor).map(NodeFacts::of));
    }
    out
}

/// The node at `path`, child indices from the root layer's root.
#[must_use]
pub fn node_at_path<'tree>(bundle: &'tree ParseTreeBundle, path: &[u32]) -> Option<Node<'tree>> {
    let mut node = bundle.layers.first()?.tree.root_node();
    for &index in path {
        node = node.child(index)?;
    }
    Some(node)
}

/// Lexically-local identifier ranges derived from a grammar's bundled
/// `locals.scm` query. Ranges are sorted and deduplicated so highlight
/// predicate checks are allocation-free binary searches.
#[derive(Debug, Default)]
pub struct LocalFacts {
    ranges: Box<[(u32, u32)]>,
}

/// One injection layer within a [`ParseTreeBundle`] (framing Q#IJ1). A
/// layer pairs a parse tree with the language that produced it and the
/// injection-nesting depth (root = 0). `highlight_query` is resolved on
/// the main thread at settle from the registry cache (framing Q#IJ2) —
/// the worker leaves it `None`.
#[derive(Debug)]
pub struct Layer {
    /// Canonical language name of the grammar that produced `tree`.
    pub language_name: String,
    /// The layer's parse tree. Node offsets are absolute into the
    /// bundle's `source` (child layers use `set_included_ranges`).
    pub tree: tree_sitter::Tree,
    /// Injection depth: 0 for the root, 1 for a direct injection, etc.
    pub depth: u16,
    /// Compiled `highlights.scm` for `language_name`, resolved at settle.
    /// `None` when the language ships no highlights, or on the worker
    /// (pre-settle). Producers read it to style this layer.
    pub highlight_query: Option<Arc<tree_sitter::Query>>,
    /// Lexically-local definitions and resolved references for this tree.
    /// Present only when the highlight query asks about the `local`
    /// property; computed once when the bundle settles.
    pub local_facts: Option<Arc<LocalFacts>>,
}

impl ParseTreeBundle {
    /// The layers' languages, the root first, wherever the trees live: the
    /// in-process layers, or the ones a parse unit reported (E7i).
    #[must_use]
    pub fn layer_languages(&self) -> Vec<String> {
        match self.isolated.as_ref() {
            Some(isolated) => isolated.layer_languages(),
            None => self
                .layers
                .iter()
                .map(|l| l.language_name.clone())
                .collect(),
        }
    }

    /// The root layer's tree (`layers[0]`) — the whole-buffer parse.
    /// [`run_parse`] always seeds the root layer; a bundle whose trees live
    /// in a parse unit ([`Self::isolated`]) has none, and this panics on it.
    #[must_use]
    pub fn root_tree(&self) -> &tree_sitter::Tree {
        &self.layers[0].tree
    }
}

/// Run a parse. This is the worker-side body that the runtime's
/// `dispatch_parse` closure invokes after pulling a job from the
/// queue. Always synchronous --- there is no internal yielding.
///
/// Since E7h.2 a parse is bounded in time where tree-sitter calls its
/// progress callback, and only there: with [`ParseRequest::deadline`]
/// set, the callback (about every hundred parser operations) cancels the
/// root parse once the deadline has passed and this returns
/// [`ParseError::DeadlineExceeded`]; injection layers share the same
/// deadline, and one it cuts short drops the layers not yet parsed. The
/// callback runs in the runtime's advance loop, so it bounds a grammar
/// whose error recovery never terminates (the JavaScript family, E7g).
/// It does not bound work done between two callbacks: an external scanner
/// that never returns to the runtime; `ts_parser__accept`, which at the
/// end of the input pops every stack path and builds a root for each with
/// no callback at all (a markdown paragraph of underscore runs spends
/// 29.5 s and 9.8 GB there at 32 KB under a 5 s deadline, #296); or
/// `ts_parser__condense_stack`, whose merging of stack versions runs
/// exponentially long in nested image and link openers (37 s at 28 of them
/// under a 5 s deadline, #301). Isolating the grammar alone does not bound
/// that: this is the runtime's own C, which tree-sitter's `WasmStore` leaves
/// native; a bound on the whole parse is E7i's to rule.
///
/// Returns `Err` if the language is rejected by [`tree_sitter::Parser`]
/// (ABI mismatch, almost always a build issue), if the deadline cut the
/// root parse short, or if the parser returns no tree otherwise.
pub fn run_parse(req: ParseRequest) -> Result<ParseTreeBundle, ParseError> {
    run_parse_observed(req, &mut |_| {})
}

/// [`run_parse`], calling `on_layer` with an injected layer's language
/// just before that layer's parse starts (E7i fix round 2): the parse
/// unit says which grammar it is running, so a crash inside an injected
/// layer is told under that layer's grammar and not the buffer's.
pub fn run_parse_observed(
    req: ParseRequest,
    on_layer: &mut dyn FnMut(&str),
) -> Result<ParseTreeBundle, ParseError> {
    let started = Instant::now();
    let deadline_at = req.deadline.map(|d| started + d);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&req.language)
        .map_err(|e| ParseError::Language(e.to_string()))?;
    let mut prior = req.prior_tree;
    if let Some(tree) = prior.as_mut() {
        for edit in &req.edits {
            tree.edit(edit);
        }
    }
    let root_started = Instant::now();
    let root_tree = parse_bounded(
        &mut parser,
        req.source.as_ref(),
        prior.as_ref(),
        deadline_at,
    )
    .ok_or_else(|| match (req.deadline, deadline_at) {
        (Some(deadline), Some(at)) if Instant::now() >= at => ParseError::DeadlineExceeded {
            deadline,
            after: started.elapsed(),
        },
        _ => ParseError::NoTree,
    })?;
    // `parse_duration` measures the root parse only — the metric the M4.1
    // acceptance gates are stated in. Injection layer building (below) is an
    // additive phase separately guarded by the settle-time budget test; it
    // must not retroactively inflate this metric.
    let parse_duration = root_started.elapsed();

    // Seed the root layer, then expand injection layers (framing Q#IJ1).
    // Injection expansion is best-effort and isolated to the child
    // (Q#IJ3): a failed/unknown/over-budget child drops that child only —
    // the root always installs, so this returns `Ok` whenever the root
    // parsed.
    let mut layers = vec![Layer {
        language_name: req.language_name.clone(),
        tree: root_tree,
        depth: 0,
        highlight_query: None,
        local_facts: None,
    }];
    let (injection_capped, layers_cut_by_deadline) = build_injection_layers(
        &mut layers,
        req.source.as_ref(),
        &req.injection_aliases,
        deadline_at,
        on_layer,
    );
    Ok(ParseTreeBundle {
        layers,
        source: req.source,
        language_name: req.language_name,
        parse_duration,
        injection_capped,
        layers_cut_by_deadline,
        isolated: None,
    })
}

/// Parse `source` with `parser`, cancelling through the progress callback
/// once `deadline_at` has passed. `None` from a bounded parse after the
/// deadline is the cancellation; the parser is dropped with it, so no
/// half-finished parse is ever resumed.
fn parse_bounded(
    parser: &mut tree_sitter::Parser,
    source: &[u8],
    old_tree: Option<&tree_sitter::Tree>,
    deadline_at: Option<Instant>,
) -> Option<tree_sitter::Tree> {
    let len = source.len();
    let mut read = |i: usize, _: Point| if i < len { &source[i..] } else { &[][..] };
    let Some(at) = deadline_at else {
        return parser.parse_with_options(&mut read, old_tree, None);
    };
    let mut progress = |_: &ParseState| {
        if Instant::now() >= at {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    parser.parse_with_options(&mut read, old_tree, Some(options))
}

// ---------------------------------------------------------------------------
// Injection layers (framing Q#IJ2 -- Q#IJ5). Worker-side: this runs on a
// parse worker, so it touches no `Rc` registry and no Lua — it resolves
// injected languages by indexing the `&'static BUILTIN_LANGUAGES` table
// (loaders + `injections_query` sources are `Send`) and case-folds fence
// names through the `ParseRequest`'s alias snapshot.
// ---------------------------------------------------------------------------

/// Max injection nesting depth (framing Q#IJ3). markdown→rust is depth 1.
#[doc(hidden)]
pub const MAX_INJECTION_DEPTH: u16 = 3;
/// Runaway backstop on total layers per buffer (framing Q#IJ3) — set well
/// above any real document (a markdown doc's one-inline-layer-per-paragraph
/// sits far under this). Purely anti-runaway; the perf bound is the
/// settle-time acceptance guard, not this number. If hit, tail layers are
/// dropped (degraded highlighting on a pathological file only).
#[doc(hidden)]
pub const MAX_INJECTION_LAYERS: usize = 4096;

/// The default fence-name → canonical-language alias map (framing Q#IJ4).
/// Keys are lowercase; the resolver case-folds before lookup. Seeded into
/// the registry and snapshotted into each [`ParseRequest`]; also handy for
/// tests that build a request without the registry.
#[must_use]
pub fn default_injection_aliases() -> HashMap<String, String> {
    [
        ("py", "python"),
        ("py3", "python"),
        ("python3", "python"),
        ("rs", "rust"),
        ("sh", "bash"),
        ("shell", "bash"),
        ("shellscript", "bash"),
        ("zsh", "bash"),
        ("c++", "cpp"),
        ("cxx", "cpp"),
        ("cc", "cpp"),
        ("golang", "go"),
        ("yml", "yaml"),
        ("md", "markdown"),
        // Lean 4 (framing Q#LN17). A ```lean fence is overwhelmingly Lean 4
        // in practice, so the Lean 3 spelling is deliberately mapped forward
        // rather than left unresolved. `lean4` needs no alias — it is the
        // entry name. `lean4-mode` does the equivalent through
        // `markdown-code-lang-modes`.
        ("lean", "lean4"),
        // A ```hs fence is as common as ```haskell, which needs no alias.
        ("hs", "haskell"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect()
}

/// One injection region resolved from a parent layer's `injections.scm`.
/// Ranges are already child-excluded and normalized (Q#IJ5) but not yet
/// intersected with the parent layer's ranges (that happens per-parent in
/// [`build_injection_layers`], which knows the parent's included ranges).
#[doc(hidden)]
pub struct InjectionMatch {
    /// Raw language name — dynamic capture text or a static `#set!` value.
    pub language: String,
    /// Child-excluded content ranges for this match, sorted/non-overlapping.
    pub ranges: Vec<Range>,
}

/// Expand injection layers under the already-parsed root (`layers[0]`),
/// appending children in depth-ascending order (Q#IJ1, Q#IJ6 rely on this
/// ordering). Bounded by depth, total layer count, and a
/// `(language, ranges)` visited guard (Q#IJ3). BFS by depth so siblings
/// at a level are grouped before descending. Returns `true` if the
/// total-layer backstop was hit and some regions were dropped (surfaced at
/// settle, framing Q#IJ3).
fn build_injection_layers(
    layers: &mut Vec<Layer>,
    source: &[u8],
    aliases: &HashMap<String, String>,
    deadline_at: Option<Instant>,
    on_layer: &mut dyn FnMut(&str),
) -> (bool, bool) {
    let mut query_cache: HashMap<String, Option<Arc<tree_sitter::Query>>> = HashMap::new();
    let mut visited: HashSet<(String, Vec<(usize, usize)>)> = HashSet::new();
    // Frontier entries are (layer index, that layer's included ranges).
    let mut frontier: Vec<(usize, Vec<Range>)> = vec![(0, vec![whole_source_range(source)])];
    let mut depth: u16 = 0;
    let mut capped = false;
    let mut cut_by_deadline = false;

    while depth < MAX_INJECTION_DEPTH && !frontier.is_empty() {
        // Children discovered this level: (layer, its ranges) to append and
        // (if any injections themselves) descend into next level.
        let mut children: Vec<(Layer, Vec<Range>)> = Vec::new();
        'parents: for (parent_idx, parent_ranges) in &frontier {
            let parent_lang = layers[*parent_idx].language_name.clone();
            let Some(query) = injection_query_cached(&mut query_cache, &parent_lang) else {
                continue;
            };
            for m in collect_injection_matches(&query, &layers[*parent_idx].tree, source) {
                if layers.len() + children.len() >= MAX_INJECTION_LAYERS {
                    capped = true;
                    break 'parents; // runaway backstop; tail dropped
                }
                let Some(child_lang) = resolve_injected_language(&m.language, aliases) else {
                    continue; // unknown/unaliased language — skip this child only
                };
                let mut ranges = intersect_ranges(&m.ranges, parent_ranges, source);
                normalize_ranges(&mut ranges);
                if ranges.is_empty() {
                    continue;
                }
                let key = (child_lang.to_owned(), ranges_key(&ranges));
                if !visited.insert(key) {
                    continue; // same (language, ranges) already parsed — cycle guard
                }
                if deadline_at.is_some_and(|at| Instant::now() >= at) {
                    cut_by_deadline = true;
                    break 'parents; // E7h.2: the deadline is spent; tail dropped
                }
                on_layer(child_lang);
                let Some(tree) = parse_child(child_lang, &ranges, source, deadline_at) else {
                    if deadline_at.is_some_and(|at| Instant::now() >= at) {
                        cut_by_deadline = true;
                        break 'parents; // cancelled mid-child: the tail goes with it
                    }
                    continue; // child parse failed — skip this child only
                };
                children.push((
                    Layer {
                        language_name: child_lang.to_owned(),
                        tree,
                        depth: depth + 1,
                        highlight_query: None,
                        local_facts: None,
                    },
                    ranges,
                ));
            }
        }
        if children.is_empty() || cut_by_deadline {
            for (layer, _) in children {
                layers.push(layer);
            }
            break;
        }
        let mut next_frontier = Vec::with_capacity(children.len());
        for (layer, ranges) in children {
            let idx = layers.len();
            layers.push(layer);
            next_frontier.push((idx, ranges));
        }
        frontier = next_frontier;
        depth += 1;
    }
    (capped, cut_by_deadline)
}

/// Compile (once, cached) the `injections.scm` for `lang` from the static
/// [`BUILTIN_LANGUAGES`] table, or `None` if the language ships none.
#[doc(hidden)]
#[allow(
    clippy::implicit_hasher,
    reason = "private until E7i moved it here for the editor's tests; one caller's map"
)]
pub fn injection_query_cached(
    cache: &mut HashMap<String, Option<Arc<tree_sitter::Query>>>,
    lang: &str,
) -> Option<Arc<tree_sitter::Query>> {
    if let Some(slot) = cache.get(lang) {
        return slot.clone();
    }
    let compiled = BUILTIN_LANGUAGES
        .iter()
        .find(|e| e.name == lang)
        .and_then(|entry| {
            let source = entry.injections_query.join("\n");
            if source.trim().is_empty() {
                return None;
            }
            let language = (entry.loader)();
            tree_sitter::Query::new(&language, &source)
                .ok()
                .map(Arc::new)
        });
    cache.insert(lang.to_owned(), compiled.clone());
    compiled
}

/// Run `query` over `tree` and return each injection region: its raw
/// language name (dynamic `@injection.language` node text, or static
/// `#set! injection.language`) and its child-excluded content ranges.
#[doc(hidden)]
pub fn collect_injection_matches(
    query: &tree_sitter::Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
) -> Vec<InjectionMatch> {
    let names = query.capture_names();
    let content_cap = names.iter().position(|n| *n == "injection.content");
    let Some(content_cap) = content_cap.map(|i| i as u32) else {
        return Vec::new();
    };
    let lang_cap = names
        .iter()
        .position(|n| *n == "injection.language")
        .map(|i| i as u32);

    let mut out = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut it = cursor.matches(query, tree.root_node(), source);
    while let Some(m) = it.next() {
        // Static language + include-children from `#set!` property settings.
        let mut static_lang: Option<String> = None;
        let mut include_children = false;
        for prop in query.property_settings(m.pattern_index) {
            match &*prop.key {
                "injection.language" => {
                    static_lang = prop.value.as_deref().map(str::to_owned);
                }
                "injection.include-children" => include_children = true,
                _ => {}
            }
        }
        let mut dyn_lang: Option<String> = None;
        let mut ranges: Vec<Range> = Vec::new();
        for cap in m.captures {
            if Some(cap.index) == lang_cap {
                if let Ok(text) = cap.node.utf8_text(source) {
                    dyn_lang = Some(text.to_owned());
                }
            } else if cap.index == content_cap {
                ranges.extend(content_node_ranges(cap.node, include_children));
            }
        }
        let Some(language) = static_lang.or(dyn_lang) else {
            continue;
        };
        normalize_ranges(&mut ranges);
        if ranges.is_empty() {
            continue;
        }
        out.push(InjectionMatch { language, ranges });
    }
    out
}

/// The included ranges for one `@injection.content` node (framing Q#IJ5 /
/// mechanic #3). With `include_children`, the whole node span; otherwise
/// the node's extent minus its **named** children's ranges. Anonymous
/// token children are *kept* — they are the injected text itself, not
/// structure to exclude. (This matches `tree-sitter-md`'s own inline
/// splitter, `bindings/rust/parser.rs:410`, which filters on `is_named()`:
/// excluding a block `inline` node's anonymous text tokens would shred the
/// paragraph into unparseable fragments. Our real injection sites — a
/// childless `code_fence_content`, an `inline` with only anonymous
/// children, an `include-children` macro `token_tree` — all resolve
/// correctly under this rule.) A node with no named children yields its
/// whole span.
#[doc(hidden)]
pub fn content_node_ranges(node: Node, include_children: bool) -> Vec<Range> {
    if include_children {
        return vec![node.range()];
    }
    let mut ranges = Vec::new();
    let mut start_byte = node.start_byte();
    let mut start_point = node.start_position();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() {
                if child.start_byte() > start_byte {
                    ranges.push(Range {
                        start_byte,
                        end_byte: child.start_byte(),
                        start_point,
                        end_point: child.start_position(),
                    });
                }
                start_byte = child.end_byte();
                start_point = child.end_position();
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    if node.end_byte() > start_byte {
        ranges.push(Range {
            start_byte,
            end_byte: node.end_byte(),
            start_point,
            end_point: node.end_position(),
        });
    }
    ranges
}

/// Clip `candidate` ranges to `parent` ranges (framing Q#IJ5): a nested
/// injection cannot reintroduce bytes its parent excluded. Points are
/// recomputed only for a clipped edge (unclipped edges keep the node's
/// exact point). At depth 1 the parent is the whole buffer, so this is a
/// pass-through.
fn intersect_ranges(candidate: &[Range], parent: &[Range], source: &[u8]) -> Vec<Range> {
    let mut out = Vec::new();
    for c in candidate {
        for p in parent {
            let start = c.start_byte.max(p.start_byte);
            let end = c.end_byte.min(p.end_byte);
            if end > start {
                out.push(Range {
                    start_byte: start,
                    end_byte: end,
                    start_point: if start == c.start_byte {
                        c.start_point
                    } else {
                        byte_to_point(source, start)
                    },
                    end_point: if end == c.end_byte {
                        c.end_point
                    } else {
                        byte_to_point(source, end)
                    },
                });
            }
        }
    }
    out
}

/// Sort, drop empty, and merge overlapping ranges so the result satisfies
/// `set_included_ranges`' sorted/non-overlapping/non-empty contract.
fn normalize_ranges(ranges: &mut Vec<Range>) {
    ranges.retain(|r| r.end_byte > r.start_byte);
    ranges.sort_by_key(|r| r.start_byte);
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for r in ranges.drain(..) {
        if let Some(last) = merged.last_mut()
            && r.start_byte < last.end_byte
        {
            if r.end_byte > last.end_byte {
                last.end_byte = r.end_byte;
                last.end_point = r.end_point;
            }
            continue;
        }
        merged.push(r);
    }
    *ranges = merged;
}

/// A hashable identity for a range set (framing Q#IJ3 visited guard).
fn ranges_key(ranges: &[Range]) -> Vec<(usize, usize)> {
    ranges.iter().map(|r| (r.start_byte, r.end_byte)).collect()
}

/// Case-fold `raw`, apply the alias map, then resolve against the bundled
/// table (framing Q#IJ4). Returns the canonical `&'static` name, or `None`
/// for an unknown language.
fn resolve_injected_language(raw: &str, aliases: &HashMap<String, String>) -> Option<&'static str> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let candidate: &str = aliases.get(&lower).map_or(lower.as_str(), String::as_str);
    BUILTIN_LANGUAGES
        .iter()
        .find(|e| e.name == candidate)
        .map(|e| e.name)
}

/// Cold-parse `source` restricted to `ranges` with `lang`'s grammar. Node
/// offsets in the returned tree are absolute into `source` (mechanic #1).
fn parse_child(
    lang: &str,
    ranges: &[Range],
    source: &[u8],
    deadline_at: Option<Instant>,
) -> Option<tree_sitter::Tree> {
    let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == lang)?;
    let language = (entry.loader)();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    parser.set_included_ranges(ranges).ok()?;
    parse_bounded(&mut parser, source, None, deadline_at)
}

/// The whole-buffer range, the root layer's parent range.
fn whole_source_range(source: &[u8]) -> Range {
    Range {
        start_byte: 0,
        end_byte: source.len(),
        start_point: Point::new(0, 0),
        end_point: byte_to_point(source, source.len()),
    }
}

/// Convert a byte offset within `source` to a tree-sitter
/// `(row, column)` [`tree_sitter::Point`]. `byte` is clamped to
/// `source.len()`.
///
/// O(byte) on a linear scan. For 5000-line files (~150 KB) this is
/// tens of microseconds per call --- well under the 5 ms incremental
/// budget. A precomputed line-start index would be the obvious
/// follow-up if profiling argues for it.
#[must_use]
pub fn byte_to_point(source: &[u8], byte: usize) -> tree_sitter::Point {
    let bounded = byte.min(source.len());
    let mut row: usize = 0;
    let mut last_nl: Option<usize> = None;
    for (i, b) in source[..bounded].iter().enumerate() {
        if *b == b'\n' {
            row += 1;
            last_nl = Some(i);
        }
    }
    let column = match last_nl {
        Some(nl) => bounded - nl - 1,
        None => bounded,
    };
    tree_sitter::Point::new(row, column)
}

/// One row of the bundled-grammar config (T M4.2). Adding a new
/// grammar is a one-line addition to [`BUILTIN_LANGUAGES`] (plus the
/// matching `tree-sitter-foo` line in `Cargo.toml`).
///
/// The `loader` is a function pointer rather than a pre-materialized
/// [`tree_sitter::Language`] so the C-side grammar object isn't
/// touched until the first buffer of that language is opened ---
/// "load grammar lazily" per the M4.2 acceptance criterion. The
/// [`Self::highlights_query`] fragments ship as `&'static str`
/// constants in the binary; T M4.3 concatenates and compiles them
/// into a [`tree_sitter::Query`] on first highlight attach.
pub struct LanguageEntry {
    /// Canonical language name. Used by `SyntaxRegistry::language`
    /// lookups, surfaced through Lua as the grammar label.
    pub name: &'static str,
    /// File extensions (without the leading dot) that should auto-
    /// attach this grammar's `ParseView` when a file is opened.
    /// First match wins; ordering inside [`BUILTIN_LANGUAGES`] is
    /// the tiebreaker for ambiguous extensions.
    pub extensions: &'static [&'static str],
    /// Producer for the [`tree_sitter::Language`]. Called at most
    /// once per registry lifetime --- the result is cached under
    /// `name` after the first invocation.
    pub loader: fn() -> tree_sitter::Language,
    /// Bundled `highlights.scm` query fragments (T M4.3), concatenated
    /// in order (base grammar first) to form the effective query. Most
    /// grammars ship one self-contained fragment. A grammar whose
    /// bundled query is a tree-sitter `; inherits: <lang>` delta lists
    /// the inherited base queries ahead of its own, because pmacs does
    /// not resolve `inherits:` directives — CUDA, for instance, ships a
    /// two-capture delta over C++ and must carry the C and C++ queries
    /// explicitly or ordinary C/C++ syntax goes unhighlighted. An empty
    /// slice (or all-empty fragments) means no highlights: the view
    /// runs but emits nothing.
    pub highlights_query: &'static [&'static str],
    /// Bundled `locals.scm` query fragments, composed base-first like
    /// [`Self::highlights_query`]. The query supplies lexical scopes,
    /// definitions, values, and references for `local` property predicates.
    pub locals_query: &'static [&'static str],
    /// Bundled `injections.scm` fragments (framing Q#IJ2), joined with a
    /// newline and compiled on the parse worker to find embedded-language
    /// regions. Empty for the many grammars that ship none (or don't
    /// inject). Names are inconsistent across crates — markdown exposes
    /// `INJECTION_QUERY_BLOCK`, rust `INJECTIONS_QUERY`, most none — the
    /// same shape `highlights_query` already absorbs.
    pub injections_query: &'static [&'static str],
}

/// Bundled grammars (T M4.2 + M4.3). The order is significant only
/// for extensions that map to multiple languages --- none of the
/// v0.1 entries collide.
///
/// Adding a grammar:
/// 1. Add `tree-sitter-foo = "X.Y"` to `Cargo.toml`.
/// 2. Add one [`LanguageEntry`] here, with
///    `highlights_query: &[tree_sitter_foo::HIGHLIGHTS_QUERY]` (or the
///    inherited base queries ahead of it, if `foo`'s bundled query is
///    a `; inherits:` delta — see the `cuda` entry).
/// 3. (Done.) The Lua side picks up the new grammar through the
///    `buffer.after-load` hook automatically and the highlight
///    overlay attaches in the same step.
/// 4. Give it a row in `fuzz/corpora.tsv` (its upstream corpus, or real
///    files) and run `scripts/fuzz-grammars --grammar foo --seconds 600`
///    clean before it ships; CLAUDE.md states the rule and E7g why: a
///    grammar is C, and tree-sitter-haskell's aborted the editor on a
///    real file its author's test file never reached.
pub const BUILTIN_LANGUAGES: &[LanguageEntry] = &[
    LanguageEntry {
        name: "rust",
        extensions: &["rs"],
        loader: || tree_sitter_rust::LANGUAGE.into(),
        highlights_query: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[tree_sitter_rust::INJECTIONS_QUERY],
    },
    LanguageEntry {
        name: "lua",
        extensions: &["lua"],
        loader: || tree_sitter_lua::LANGUAGE.into(),
        highlights_query: &[tree_sitter_lua::HIGHLIGHTS_QUERY],
        locals_query: &[tree_sitter_lua::LOCALS_QUERY],
        injections_query: &[],
    },
    // Haskell (aside E7e). The crate exports all three query constants, in
    // the plural rust/lua idiom. Its injections inject quasiquote bodies by
    // quoter (`[hamlet|…|]` -> html, `[aesonQQ|…|]` -> json, `[sql|…|]`) and
    // tag comments as `comment`; a language this table does not register
    // resolves to nothing and is skipped. `.lhs` is deliberately unclaimed:
    // literate Haskell is prose with code in Bird tracks (`> `) or
    // `\begin{code}` blocks, which needs an unliterate pass this grammar does
    // not have, and it parses both as errors
    // (`lhs_is_not_haskell_to_this_grammar` pins it). Unshipped at E7g, its
    // vendored `array.h` being aliasing UB that GCC 16 at -O2 compiled into
    // a heap overflow, and restored at E7h under `.cargo/config.toml`'s
    // `-fno-strict-aliasing` after a clean 600 s fuzz run on GCC 16. E7h's
    // fix round 1 built it from a copy carrying the conforming header,
    // because the flag reaches only builds cargo starts at the repository
    // root; since fix round 2 it is tree-sitter-haskell 0.24.1 from
    // crates.io, published on that header.
    LanguageEntry {
        name: "haskell",
        extensions: &["hs"],
        loader: || tree_sitter_haskell::LANGUAGE.into(),
        highlights_query: &[tree_sitter_haskell::HIGHLIGHTS_QUERY],
        locals_query: &[tree_sitter_haskell::LOCALS_QUERY],
        injections_query: &[tree_sitter_haskell::INJECTIONS_QUERY],
    },
    // T M9.7: markdown block grammar (`tree_sitter_md::LANGUAGE`) — headers,
    // lists, fenced code blocks, blockquotes. Its `injections.scm` (framing
    // Q#IJ10) drives two layer kinds: fenced code blocks inject the fence's
    // named language, and paragraph/heading text injects `markdown_inline`
    // (the entry below) — so inline emphasis/links are now highlighted, and
    // the former M9.7 "block-only, inline unhighlighted" floor is retired.
    // Its highlights are the in-repo overlay `MARKDOWN_HIGHLIGHTS` below
    // (#310): the crate's `HIGHLIGHT_QUERY_BLOCK` speaks `@text.*`, which the
    // theme does not, so it painted plain; the injections are the crate's.
    LanguageEntry {
        name: "markdown",
        extensions: &["md", "markdown"],
        loader: || tree_sitter_md::LANGUAGE.into(),
        highlights_query: &[MARKDOWN_HIGHLIGHTS],
        locals_query: &[],
        injections_query: &[tree_sitter_md::INJECTION_QUERY_BLOCK],
    },
    // markdown_inline (framing Q#IJ10) — the inline grammar the block
    // grammar injects for paragraph/heading text (`#set! injection.language
    // "markdown_inline"`). No file extension: it is injection-only, never
    // opened directly by name. Ships an inline highlights query (emphasis,
    // links, code spans) and its own injections (e.g. inline HTML), so it
    // recurses like any other layer. Retires the M9.7 block-only floor.
    // Like the block grammar's, its highlights are an in-repo overlay of
    // the crate's, `MARKDOWN_INLINE_HIGHLIGHTS` (#310).
    LanguageEntry {
        name: "markdown_inline",
        extensions: &[],
        loader: || tree_sitter_md::INLINE_LANGUAGE.into(),
        highlights_query: &[MARKDOWN_INLINE_HIGHLIGHTS],
        locals_query: &[],
        injections_query: &[tree_sitter_md::INJECTION_QUERY_INLINE],
    },
    // T M_B3 — C / C++. Lexical highlighting (keywords / strings /
    // operators) so the grid TUI shows code-shaped C++ on first open.
    // `LspStyleView` layers on top with semantic refinement
    // (functions / types / macros / namespaces) from clangd's
    // semantic tokens — the two views' styles merge via
    // `crate::overlay::merge_styles`.
    //
    // `.h` is ambiguous C / C++; the `c` entry below claims it
    // (matches the LSP filetype map's default in `lsp.lua`). Users
    // who want `.h` parsed as C++ can override via Lua.
    // Note the const names: `tree-sitter-c` and `tree-sitter-cpp`
    // expose `HIGHLIGHT_QUERY` (singular), matching the `tree-sitter-md`
    // crate's `HIGHLIGHT_QUERY_BLOCK` style; `tree-sitter-rust` and
    // `tree-sitter-lua` use `HIGHLIGHTS_QUERY` (plural). No semantic
    // difference — same bundled `highlights.scm` either way.
    LanguageEntry {
        name: "c",
        extensions: &["c", "h"],
        loader: || tree_sitter_c::LANGUAGE.into(),
        highlights_query: &[tree_sitter_c::HIGHLIGHT_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    LanguageEntry {
        name: "cpp",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp", "inl", "cppm"],
        loader: || tree_sitter_cpp::LANGUAGE.into(),
        highlights_query: &[tree_sitter_cpp::HIGHLIGHT_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // CUDA (`.cu` source, `.cuh` header). A dedicated grammar rather
    // than reusing `cpp`: CUDA extends C++ with `__global__`/`__device__`
    // qualifiers, `<<<grid, block>>>` kernel-launch syntax, and builtin
    // types the C++ grammar misparses. Neither extension collides with
    // an entry above, so ordering is irrelevant here. `LspStyleView`
    // layers clangd's CUDA semantic tokens on top, exactly as for C/C++.
    //
    // Note the const name: `tree-sitter-cuda` exposes `HIGHLIGHTS_QUERY`
    // (plural, the `tree-sitter-rust`/`tree-sitter-lua` idiom), NOT the
    // singular `HIGHLIGHT_QUERY` that `tree-sitter-c`/`-cpp`/`-md` use.
    //
    // The CUDA `highlights.scm` opens with `; inherits: cpp` and defines
    // only the CUDA-specific captures (`<<<...>>>` launch brackets, the
    // `__global__`/`__device__` modifiers) — two capture classes on its
    // own. pmacs does not resolve `inherits:`, so the C and C++ base
    // queries are prepended explicitly; the three compile together into
    // ~16 capture classes against the CUDA grammar (which is a superset
    // of C++). Order is base-first (C, then C++, then CUDA) so later
    // fragments refine earlier ones. Without this, ordinary C/C++ syntax
    // in a `.cu` file would go almost entirely unhighlighted.
    LanguageEntry {
        name: "cuda",
        extensions: &["cu", "cuh"],
        loader: || tree_sitter_cuda::LANGUAGE.into(),
        highlights_query: &[
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            tree_sitter_cuda::HIGHLIGHTS_QUERY,
        ],
        locals_query: &[],
        injections_query: &[],
    },
    // Shell / bash. Lexical highlighting for the shell family; the LSP
    // half (bash-language-server) was already wired in `lsp.lua`. Unlike
    // `cuda`, bash's `highlights.scm` is self-contained (no `; inherits:`
    // delta), so a single fragment suffices. The extension set is wider
    // than the `.sh`/`.bash` the LSP filetype map covered: `.zsh`/`.ksh`/
    // `.ash` are close-enough dialects and `.bats` is bash. None collide
    // with an entry above. Because the language name is `bash` — matching
    // the `pmacs.lsp.config.bash` key — opening any of these also
    // auto-attaches bash-language-server. Extensionless shell scripts are
    // resolved by shebang, and rc dotfiles (`.bashrc`, `PKGBUILD`) by the
    // filename map — both in `builtin/runtime/syntax.lua`.
    LanguageEntry {
        name: "bash",
        extensions: &["sh", "bash", "zsh", "ksh", "ash", "bats"],
        loader: || tree_sitter_bash::LANGUAGE.into(),
        highlights_query: &[tree_sitter_bash::HIGHLIGHT_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // Filename-identified languages. These files usually have no useful
    // extension (`Dockerfile`, `Makefile`, `CMakeLists.txt`), so the bulk
    // of detection is the filename map in `syntax.lua`; the extensions
    // here catch the `.dockerfile`/`.mk`/`.cmake` variants. All three ship
    // self-contained highlights (no `; inherits:`), so single fragments.
    //
    // Dockerfile uses the `tree-sitter-containerfile` crate (the
    // ABI-current grammar; also covers Containerfile); its root node is
    // `source_file`. Make roots at `makefile`, CMake at `source_file`.
    LanguageEntry {
        name: "dockerfile",
        extensions: &["dockerfile", "containerfile"],
        loader: || tree_sitter_containerfile::LANGUAGE.into(),
        highlights_query: &[tree_sitter_containerfile::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    LanguageEntry {
        name: "make",
        extensions: &["mk", "make"],
        loader: || tree_sitter_make::LANGUAGE.into(),
        highlights_query: &[tree_sitter_make::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    LanguageEntry {
        name: "cmake",
        extensions: &["cmake"],
        loader: || tree_sitter_cmake::LANGUAGE.into(),
        highlights_query: &[tree_sitter_cmake::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // Grammar-gap languages — these already had LSP configs but no
    // grammar, so they rendered without lexical color. Each language name
    // matches its existing `pmacs.lsp.config.<name>` key, so grammar
    // detection (which wins over the filetype map) resolves the same id
    // the server keys off. Root kinds: python `module`, go/zig
    // `source_file`, js/ts family `program`, toml `document`.
    LanguageEntry {
        name: "python",
        extensions: &["py", "pyi"],
        loader: || tree_sitter_python::LANGUAGE.into(),
        highlights_query: &[tree_sitter_python::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    LanguageEntry {
        name: "go",
        extensions: &["go"],
        loader: || tree_sitter_go::LANGUAGE.into(),
        highlights_query: &[tree_sitter_go::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // No JavaScript or TypeScript (E7g): `tree-sitter-javascript` 0.25.0
    // never returns from a 24-byte file of unclosed brackets and grows
    // without bound while it tries, and the TypeScript and TSX grammars do
    // the same on it (`javascript_family_is_not_bundled`). The four
    // languages reach tsserver through `pmacs.lsp.filetypes`, uncolored.
    LanguageEntry {
        name: "toml",
        extensions: &["toml"],
        loader: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights_query: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    LanguageEntry {
        name: "zig",
        extensions: &["zig", "zon"],
        loader: || tree_sitter_zig::LANGUAGE.into(),
        highlights_query: &[tree_sitter_zig::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // JSON + YAML — config formats, both self-contained highlights and no
    // injections of their own. Registering `yaml` also lights up markdown
    // `---` frontmatter via the #122 injection engine (the markdown block
    // injection query sets `injection.language "yaml"` for `minus_metadata`;
    // `+++` TOML frontmatter already works). Root kinds: json `document`,
    // yaml `stream`. `.jsonc`/`.json5` (comments / trailing commas) are a
    // deferred variant — the plain JSON grammar rejects them.
    LanguageEntry {
        name: "json",
        extensions: &["json"],
        loader: || tree_sitter_json::LANGUAGE.into(),
        highlights_query: &[tree_sitter_json::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // YAML: unshipped at E7g (its scanner wrote past the runtime's
    // 1024-byte serialization buffer at 254 levels and the runtime aborted
    // the editor) and shipped again at E7h from `vendor/tree-sitter-yaml`
    // with that bound fixed (D36 as amended), after a clean 600 s fuzz run.
    LanguageEntry {
        name: "yaml",
        extensions: &["yaml", "yml"],
        loader: || tree_sitter_yaml::LANGUAGE.into(),
        highlights_query: &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // LaTeX / TeX. The grammar crate exports no query constants (unlike every
    // entry above), so the highlights query is the in-repo overlay
    // `builtin/queries/latex/highlights.scm`, `include_str!`'d as
    // `LATEX_HIGHLIGHTS` below — the first such overlay in the tree (framing
    // Q#LX2; the `audit-rules.scm` include is the precedent). Locals and
    // injections are empty for v0; `(math_environment) @math` injection
    // detection is deferred to the inline-math arc.
    LanguageEntry {
        name: "latex",
        extensions: &["tex", "latex", "sty", "cls"],
        loader: || codebook_tree_sitter_latex::LANGUAGE.into(),
        highlights_query: &[LATEX_HIGHLIGHTS],
        locals_query: &[],
        injections_query: &[],
    },
    // HTML + CSS (framing `docs/archive/framings/web-grammars-html-css-framing.md`). Both crates
    // export their query constants (no overlay). HTML's `INJECTIONS_QUERY`
    // wires `<style>` -> css (below), riding the #122 injection engine; `css`
    // must be registered here for that injection to resolve. Its `<script>`
    // -> javascript injection resolves to nothing since E7g unshipped
    // JavaScript, so script bodies stay plain. The `tag`/`attribute` captures these
    // queries use are taught to the highlighter in `crate::highlight` (Q#WEB4).
    LanguageEntry {
        name: "html",
        extensions: &["html", "htm", "xhtml"],
        loader: || tree_sitter_html::LANGUAGE.into(),
        highlights_query: &[tree_sitter_html::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[tree_sitter_html::INJECTIONS_QUERY],
    },
    LanguageEntry {
        name: "css",
        extensions: &["css"],
        loader: || tree_sitter_css::LANGUAGE.into(),
        highlights_query: &[tree_sitter_css::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
    // Lean 4 (framing `docs/archive/framings/lean4-mode-framing.md`, Arc 8 Stage 1).
    //
    // The entry is named `lean4`, not `lean` (Q#LN2): this name becomes the
    // `language_id` sent in `didOpen` — `ensure_server` at
    // `builtin/runtime/lsp.lua:540` passes it straight through — and the
    // Lean ecosystem's id is `lean4` (`lean` is Lean 3, which is
    // end-of-life). The grammar's own C symbol is `tree_sitter_lean`; that
    // is arborium's business, not ours. Stage 3 adds
    // `pmacs.lsp.config.lean4` against this name.
    //
    // Note the loader shape: `arborium-lean` exports `const fn language() ->
    // LanguageFn` rather than a `LANGUAGE` const, so this is the one entry
    // that calls a function to get the `LanguageFn` before `.into()`.
    //
    // `.olean` (compiled artifacts) and `.ilean` (JSON metadata) are
    // deliberately unclaimed (Q#LN3). Locals and injections are empty
    // because the crate ships both as empty strings — Lean has no embedded
    // sublanguage worth injecting, and its scoping is far beyond what a
    // tree-sitter locals query could model.
    LanguageEntry {
        name: "lean4",
        extensions: &["lean"],
        loader: || arborium_lean::language().into(),
        highlights_query: &[arborium_lean::HIGHLIGHTS_QUERY],
        locals_query: &[],
        injections_query: &[],
    },
];

/// LaTeX highlights overlay (framing Q#LX2). The chosen grammar crate
/// (`codebook-tree-sitter-latex`) ships no query constants, so — unlike every
/// other [`BUILTIN_LANGUAGES`] entry, which references a crate-exported
/// `HIGHLIGHTS_QUERY` — LaTeX highlighting is driven by this vendored query,
/// reconciled onto pmacs' recognized capture set (`crate::highlight`). The
/// `include_str!` path mirrors the sole prior `.scm` precedent,
/// `crate::audit`'s `audit-rules.scm`.
#[doc(hidden)]
pub const LATEX_HIGHLIGHTS: &str = include_str!("../../builtin/queries/latex/highlights.scm");

/// Markdown block highlights overlay (#310): `tree_sitter_md`'s
/// `HIGHLIGHT_QUERY_BLOCK` reconciled onto the recognized capture set, as
/// [`LATEX_HIGHLIGHTS`] is. The crate's query speaks `@text.*`, which the
/// default theme resolves to nothing, so a markdown buffer painted plain.
#[doc(hidden)]
pub const MARKDOWN_HIGHLIGHTS: &str = include_str!("../../builtin/queries/markdown/highlights.scm");

/// Markdown inline highlights overlay (#310): `tree_sitter_md`'s
/// `HIGHLIGHT_QUERY_INLINE` reconciled onto the recognized capture set.
#[doc(hidden)]
pub const MARKDOWN_INLINE_HIGHLIGHTS: &str =
    include_str!("../../builtin/queries/markdown_inline/highlights.scm");

#[doc(hidden)]
pub fn query_uses_local_predicates(query: &tree_sitter::Query) -> bool {
    (0..query.pattern_count()).any(|pattern| {
        query
            .property_predicates(pattern)
            .iter()
            .any(|(property, _)| property.key.as_ref() == "local")
    })
}

#[derive(Debug)]
struct LocalDefinition {
    name_range: std::ops::Range<usize>,
    value_range: std::ops::Range<usize>,
}

#[derive(Debug)]
struct LocalScope {
    inherits: bool,
    range: std::ops::Range<usize>,
    definitions: Vec<LocalDefinition>,
}

impl LocalFacts {
    #[doc(hidden)]
    pub fn contains(&self, start_byte: usize, end_byte: usize) -> bool {
        let (Ok(start_byte), Ok(end_byte)) = (u32::try_from(start_byte), u32::try_from(end_byte))
        else {
            return false;
        };
        self.ranges.binary_search(&(start_byte, end_byte)).is_ok()
    }
}

/// Resolve lexical definitions and references according to Tree-sitter's
/// standard `locals.scm` capture conventions.
#[doc(hidden)]
pub fn compute_local_facts(
    query: &tree_sitter::Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
) -> LocalFacts {
    let scope_capture = query.capture_index_for_name("local.scope");
    let definition_capture = query.capture_index_for_name("local.definition");
    let value_capture = query.capture_index_for_name("local.definition-value");
    let reference_capture = query.capture_index_for_name("local.reference");

    let mut scopes = vec![LocalScope {
        inherits: false,
        range: 0..source.len(),
        definitions: Vec::new(),
    }];
    let mut ranges = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut captures = cursor.captures(query, tree.root_node(), source);

    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        let node_range = capture.node.byte_range();
        while scopes.len() > 1
            && node_range.start > scopes.last().expect("root scope exists").range.end
        {
            scopes.pop();
        }

        if Some(capture.index) == scope_capture {
            let mut inherits = true;
            for property in query.property_settings(query_match.pattern_index) {
                if property.key.as_ref() == "local.scope-inherits" {
                    inherits = property
                        .value
                        .as_deref()
                        .is_none_or(|value| value == "true");
                }
            }
            scopes.push(LocalScope {
                inherits,
                range: node_range,
                definitions: Vec::new(),
            });
            continue;
        }

        if Some(capture.index) == definition_capture {
            let Some(_) = source.get(node_range.clone()) else {
                continue;
            };
            let value_range = query_match
                .captures
                .iter()
                .find(|candidate| Some(candidate.index) == value_capture)
                .map_or(0..0, |candidate| candidate.node.byte_range());
            scopes
                .last_mut()
                .expect("root scope exists")
                .definitions
                .push(LocalDefinition {
                    name_range: node_range.clone(),
                    value_range,
                });
            if let (Ok(start), Ok(end)) = (
                u32::try_from(node_range.start),
                u32::try_from(node_range.end),
            ) {
                ranges.push((start, end));
            }
            continue;
        }

        if Some(capture.index) != reference_capture {
            continue;
        }
        let Some(name) = source.get(node_range.clone()) else {
            continue;
        };
        let mut resolved = false;
        for scope in scopes.iter().rev() {
            if scope.definitions.iter().rev().any(|definition| {
                node_range.start >= definition.value_range.end
                    && source.get(definition.name_range.clone()) == Some(name)
            }) {
                resolved = true;
                break;
            }
            if !scope.inherits {
                break;
            }
        }
        if resolved
            && let (Ok(start), Ok(end)) = (
                u32::try_from(node_range.start),
                u32::try_from(node_range.end),
            )
        {
            ranges.push((start, end));
        }
    }

    ranges.sort_unstable();
    ranges.dedup();
    LocalFacts {
        ranges: ranges.into_boxed_slice(),
    }
}

// ---------------------------------------------------------------------------
// T M4.3: highlight-span extraction
// ---------------------------------------------------------------------------

/// One highlighted byte range produced by a `highlights.scm` query
/// run against a [`ParseTreeBundle`]. The `capture_index` is into
/// [`tree_sitter::Query::capture_names`]; resolving to a style
/// happens inside `pmacs::highlight::SyntaxHighlightView` using
/// the active theme.
///
/// `start_byte`/`end_byte` are byte offsets into the bundle's
/// `source`. Stored as `u32` because pmacs files cap at 4 GiB
/// (rope domain) and downstream coordinates (rows, cols) are also
/// `u32` --- avoids signed/unsigned conversion noise on the render
/// hot path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    /// First byte covered by the span.
    pub start_byte: u32,
    /// One past the last byte covered.
    pub end_byte: u32,
    /// Index into [`tree_sitter::Query::capture_names`] for the
    /// capture that produced this span.
    pub capture_index: u32,
}

/// Walk every capture produced by `query` over `bundle.tree`,
/// collect them as [`HighlightSpan`]s, and sort them so that wider
/// (outer) ranges come *before* narrower (inner) ones at the same
/// start byte. The render path then applies them in order, so inner
/// captures end up overriding outer ones --- matches typical
/// editor highlight precedence ("the most specific node wins").
///
/// O(captures · log captures) for the sort; O(query work) for the
/// capture walk itself (the dominant cost; see M4.3 acceptance).
#[must_use]
pub fn compute_highlight_spans(
    query: &tree_sitter::Query,
    bundle: &ParseTreeBundle,
) -> Vec<HighlightSpan> {
    compute_highlight_spans_in_range(query, bundle, None)
}

/// Like [`compute_highlight_spans`], but restricts the query to nodes
/// intersecting `byte_range` when `Some`. tree-sitter's
/// `QueryCursor::set_byte_range` makes the capture walk proportional to
/// the range, not the whole tree — the semantic producer passes the
/// declared viewport so styling a screenful of a huge file is
/// O(visible), not O(file) (the per-edit typing cost; framing Q#S6).
#[must_use]
pub fn compute_highlight_spans_in_range(
    query: &tree_sitter::Query,
    bundle: &ParseTreeBundle,
    byte_range: Option<std::ops::Range<usize>>,
) -> Vec<HighlightSpan> {
    // E7i: a bundle whose tree lives in a parse unit has no layers here.
    let Some(root) = bundle.layers.first() else {
        return Vec::new();
    };
    compute_highlight_spans_for(
        query,
        &root.tree,
        bundle.source.as_ref(),
        root.local_facts.as_deref(),
        byte_range,
    )
}

/// Like [`compute_highlight_spans_in_range`] but over an explicit
/// `(tree, source, local_facts)` layer tuple — the form producers call for
/// each injection layer (framing Q#IJ7 and Q#LQ5). `source` is the whole
/// buffer; a child layer's tree carries absolute offsets into it, so the same
/// capture walk works unchanged.
#[must_use]
pub fn compute_highlight_spans_for(
    query: &tree_sitter::Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
    local_facts: Option<&LocalFacts>,
    byte_range: Option<std::ops::Range<usize>>,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    if let Some(range) = byte_range {
        cursor.set_byte_range(range);
    }
    let root = tree.root_node();
    let mut iter = cursor.captures(query, root, source);
    while let Some((query_match, capture_index)) = iter.next() {
        let capture = query_match.captures[*capture_index];
        let local_predicates_match = query
            .property_predicates(query_match.pattern_index)
            .iter()
            .filter(|(property, _)| property.key.as_ref() == "local")
            .all(|(property, positive)| {
                let node = property.capture_id.map_or(Some(capture.node), |target| {
                    query_match
                        .captures
                        .iter()
                        .find(|candidate| candidate.index as usize == target)
                        .map(|candidate| candidate.node)
                });
                let is_local = node.is_some_and(|node| {
                    local_facts
                        .is_some_and(|facts| facts.contains(node.start_byte(), node.end_byte()))
                });
                is_local == *positive
            });
        if !local_predicates_match {
            continue;
        }
        spans.push(HighlightSpan {
            start_byte: capture.node.start_byte() as u32,
            end_byte: capture.node.end_byte() as u32,
            capture_index: capture.index,
        });
    }
    // Wider-first ordering at equal start: later writes (the
    // narrower / more specific spans) override earlier ones in the
    // overlay merge.
    spans.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then_with(|| b.end_byte.cmp(&a.end_byte))
            .then_with(|| a.capture_index.cmp(&b.capture_index))
    });
    spans
}
