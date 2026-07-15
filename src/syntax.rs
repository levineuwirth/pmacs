// syntax.rs --- T M4.1 tree-sitter integration: parse types, the
// per-buffer ParseView, and the worker-side run_parse function.
// T M4.2 layers bundled grammars; T M4.3 adds highlight-query
// loading and the capture-walk that feeds the highlight view.

//! Tree-sitter integration (T M4.1 -- T M4.3).
//!
//! The async runtime ([`crate::async_runtime`]) already carries the
//! dispatch shape Tree-sitter needs (parse-on-worker, supersede,
//! frame-cadence settle). This module adds:
//!
//! * [`ParseRequest`] / [`ParseTreeBundle`] --- the inputs and outputs
//!   that travel between the main thread and a worker.
//! * [`run_parse`] --- the worker-side body. Synchronous; called from
//!   the parse closure submitted by [`crate::async_runtime::AsyncRuntime::dispatch_parse`].
//! * [`ParseView`] / [`ParseViewHandle`] --- the per-buffer
//!   [`crate::view::View`] implementation that mirrors buffer bytes,
//!   captures every [`Edit`] as a [`tree_sitter::InputEdit`] (with
//!   correct row/col [`tree_sitter::Point`]s), and holds the most
//!   recent [`ParseTreeBundle`]. State lives behind an
//!   [`Arc<Mutex<ParseViewInner>>`] so the buffer-owned `Box<dyn View>`
//!   and the Lua-side glue (which needs to read the tree and feed
//!   back installed bundles) share the same backing store.
//! * [`HighlightSpan`] / [`compute_highlight_spans`] --- T M4.3
//!   capture-walk over a settled tree using a bundled
//!   `highlights.scm`. The resulting spans, sorted "wider first",
//!   feed [`crate::highlight::SyntaxHighlightView`]'s render path.
//!
//! M4.2 wires concrete grammars (`tree-sitter-rust`,
//! `tree-sitter-lua`) on top of this module. M4.1 uses
//! `tree-sitter-rust` only as a `dev-dependency` to drive acceptance
//! tests.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tree_sitter::{Node, Point, Range, StreamingIterator};

use crate::async_runtime::JobId;
use crate::buffer::{Buffer, BufferError, BufferId};
use crate::highlight::{Theme, ThemeHandle};
use crate::rope::Edit;
use crate::view::View;

/// Description of a parse job: the source bytes to parse, the
/// language to parse against, the prior tree (if any) for incremental
/// re-parse, and the [`tree_sitter::InputEdit`] descriptions
/// accumulated since that prior tree was produced.
///
/// All fields are owned ([R31]) so the closure submitted to a worker
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
    /// Edits accumulated by the [`ParseView`] since `prior_tree` was
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
}

/// Output of [`run_parse`]. The runtime's parse-handoff side map
/// holds these by [`Arc`]; `Lua` introspection ([`crate::lua_bindings`])
/// resolves a buffer id to its current bundle and walks the tree.
#[derive(Debug)]
pub struct ParseTreeBundle {
    /// Injection layers (framing Q#IJ1). `layers[0]` is the root layer
    /// (the whole buffer, parsed with the buffer's own grammar);
    /// subsequent entries are injected child layers in depth-ascending
    /// order. Always non-empty — a parse produces at least the root, so
    /// [`Self::root_tree`] never panics.
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
}

impl ParseTreeBundle {
    /// The root layer's tree (`layers[0]`) — the whole-buffer parse.
    /// Never panics: [`run_parse`] always seeds the root layer.
    #[must_use]
    pub fn root_tree(&self) -> &tree_sitter::Tree {
        &self.layers[0].tree
    }
}

/// Run a parse. This is the worker-side body that the runtime's
/// `dispatch_parse` closure invokes after pulling a job from the
/// queue. Always synchronous --- there is no internal yielding.
///
/// Returns `Err` if the language is rejected by [`tree_sitter::Parser`]
/// (ABI mismatch, almost always a build issue) or if the parser
/// itself returns no tree (cancellation flag flipped, exhausted
/// timeout --- neither wired in M4.1, so under M4.1 contracts this
/// path is unreachable in practice).
pub fn run_parse(req: ParseRequest) -> Result<ParseTreeBundle, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&req.language)
        .map_err(|e| format!("set_language: {e}"))?;
    let mut prior = req.prior_tree;
    if let Some(tree) = prior.as_mut() {
        for edit in &req.edits {
            tree.edit(edit);
        }
    }
    let started = Instant::now();
    let root_tree = parser
        .parse(req.source.as_ref(), prior.as_ref())
        .ok_or_else(|| "parser produced no tree".to_owned())?;
    // `parse_duration` measures the root parse only — the metric the M4.1
    // acceptance gates are stated in. Injection layer building (below) is an
    // additive phase separately guarded by the settle-time budget test; it
    // must not retroactively inflate this metric.
    let parse_duration = started.elapsed();

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
    }];
    let injection_capped =
        build_injection_layers(&mut layers, req.source.as_ref(), &req.injection_aliases);
    Ok(ParseTreeBundle {
        layers,
        source: req.source,
        language_name: req.language_name,
        parse_duration,
        injection_capped,
    })
}

// ---------------------------------------------------------------------------
// Injection layers (framing Q#IJ2 -- Q#IJ5). Worker-side: this runs on a
// parse worker, so it touches no `Rc` registry and no Lua — it resolves
// injected languages by indexing the `&'static BUILTIN_LANGUAGES` table
// (loaders + `injections_query` sources are `Send`) and case-folds fence
// names through the `ParseRequest`'s alias snapshot.
// ---------------------------------------------------------------------------

/// Max injection nesting depth (framing Q#IJ3). markdown→rust is depth 1.
const MAX_INJECTION_DEPTH: u16 = 3;
/// Runaway backstop on total layers per buffer (framing Q#IJ3) — set well
/// above any real document (a markdown doc's one-inline-layer-per-paragraph
/// sits far under this). Purely anti-runaway; the perf bound is the
/// settle-time acceptance guard, not this number. If hit, tail layers are
/// dropped (degraded highlighting on a pathological file only).
const MAX_INJECTION_LAYERS: usize = 4096;

/// The default fence-name → canonical-language alias map (framing Q#IJ4).
/// Keys are lowercase; the resolver case-folds before lookup. Seeded into
/// the registry and snapshotted into each [`ParseRequest`]; also handy for
/// tests that build a request without the registry.
#[must_use]
pub fn default_injection_aliases() -> HashMap<String, String> {
    [
        ("js", "javascript"),
        ("jsx", "javascriptreact"),
        ("ts", "typescript"),
        ("tsx", "typescriptreact"),
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
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect()
}

/// One injection region resolved from a parent layer's `injections.scm`.
/// Ranges are already child-excluded and normalized (Q#IJ5) but not yet
/// intersected with the parent layer's ranges (that happens per-parent in
/// [`build_injection_layers`], which knows the parent's included ranges).
struct InjectionMatch {
    /// Raw language name — dynamic capture text or a static `#set!` value.
    language: String,
    /// Child-excluded content ranges for this match, sorted/non-overlapping.
    ranges: Vec<Range>,
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
) -> bool {
    let mut query_cache: HashMap<String, Option<Arc<tree_sitter::Query>>> = HashMap::new();
    let mut visited: HashSet<(String, Vec<(usize, usize)>)> = HashSet::new();
    // Frontier entries are (layer index, that layer's included ranges).
    let mut frontier: Vec<(usize, Vec<Range>)> = vec![(0, vec![whole_source_range(source)])];
    let mut depth: u16 = 0;
    let mut capped = false;

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
                let Some(tree) = parse_child(child_lang, &ranges, source) else {
                    continue; // child parse failed — skip this child only
                };
                children.push((
                    Layer {
                        language_name: child_lang.to_owned(),
                        tree,
                        depth: depth + 1,
                        highlight_query: None,
                    },
                    ranges,
                ));
            }
        }
        if children.is_empty() {
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
    capped
}

/// Compile (once, cached) the `injections.scm` for `lang` from the static
/// [`BUILTIN_LANGUAGES`] table, or `None` if the language ships none.
fn injection_query_cached(
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
fn collect_injection_matches(
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
fn content_node_ranges(node: Node, include_children: bool) -> Vec<Range> {
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
fn parse_child(lang: &str, ranges: &[Range], source: &[u8]) -> Option<tree_sitter::Tree> {
    let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == lang)?;
    let language = (entry.loader)();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    parser.set_included_ranges(ranges).ok()?;
    parser.parse(source, None)
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

/// Mutable state shared between the buffer-attached [`ParseView`]
/// and any external [`ParseViewHandle`] clones.
struct ParseViewInner {
    language: tree_sitter::Language,
    language_name: String,
    /// Source bytes mirror, kept in sync with the buffer. Updated
    /// inside `on_edit`.
    source: Vec<u8>,
    /// Edits accumulated since `current` was produced. Drained on
    /// `make_request`; cleared on `install`.
    pending: Vec<tree_sitter::InputEdit>,
    /// Most recent settled parse, or `None` if no parse has run yet.
    current: Option<Arc<ParseTreeBundle>>,
}

/// Per-buffer parse-tree state. Attached to a [`Buffer`] as a
/// [`View`]; `on_edit` mirrors the rope edit into a parallel
/// `Vec<u8>` source buffer and pushes a corresponding
/// [`tree_sitter::InputEdit`] onto a pending list.
///
/// Internally a thin wrapper over `Arc<Mutex<ParseViewInner>>` so
/// callers (Lua bindings, dispatch glue) can hold a
/// [`ParseViewHandle`] clone and read/modify the same state without
/// having to detach the view from the buffer.
pub struct ParseView {
    inner: Arc<Mutex<ParseViewInner>>,
}

/// External handle to a [`ParseView`]'s state. Cheap to clone
/// (`Arc` bump). Used by [`crate::lua_bindings`] to (a) build a
/// [`ParseRequest`] before dispatch, (b) install the produced
/// [`ParseTreeBundle`] after settle, (c) introspect the tree from
/// Lua.
#[derive(Clone)]
pub struct ParseViewHandle {
    inner: Arc<Mutex<ParseViewInner>>,
}

impl ParseView {
    /// Construct a view by snapshotting the buffer's current bytes.
    /// The snapshot becomes the view's source mirror, so the first
    /// dispatched parse has byte-accurate input even before any edit
    /// is observed.
    #[must_use]
    pub fn new(buf: &Buffer, language: tree_sitter::Language, language_name: String) -> Self {
        let len = buf.len();
        let mut source = Vec::with_capacity(len as usize);
        if len > 0 {
            for chunk in buf.snapshot_rope().chunks(0, len) {
                source.extend_from_slice(chunk);
            }
        }
        let inner = ParseViewInner {
            language,
            language_name,
            source,
            pending: Vec::new(),
            current: None,
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Cheap clone of the shared state handle.
    #[must_use]
    pub fn handle(&self) -> ParseViewHandle {
        ParseViewHandle {
            inner: self.inner.clone(),
        }
    }
}

impl ParseViewHandle {
    /// Language this view parses against.
    #[must_use]
    pub fn language(&self) -> tree_sitter::Language {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .language
            .clone()
    }

    /// Human-readable language label.
    #[must_use]
    pub fn language_name(&self) -> String {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .language_name
            .clone()
    }

    /// Most recent parse, if any has settled.
    #[must_use]
    pub fn current(&self) -> Option<Arc<ParseTreeBundle>> {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .current
            .clone()
    }

    /// Number of pending edits waiting for the next parse dispatch.
    #[must_use]
    pub fn pending_edit_count(&self) -> usize {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .pending
            .len()
    }

    /// Snapshot of the source mirror's current contents. Test
    /// helper; the worker receives the same bytes as `req.source`
    /// when a parse is dispatched.
    #[must_use]
    pub fn source_snapshot(&self) -> Vec<u8> {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .source
            .clone()
    }

    /// Build a [`ParseRequest`] reflecting the current state. Drains
    /// the pending-edit list. Caller is expected to dispatch the
    /// request and feed the settled bundle back via [`Self::install`].
    pub fn make_request(&self) -> ParseRequest {
        let mut inner = self.inner.lock().expect("ParseView mutex poisoned");
        let edits = std::mem::take(&mut inner.pending);
        let prior_tree = inner.current.as_ref().map(|b| b.root_tree().clone());
        ParseRequest {
            source: Arc::from(inner.source.clone()),
            language: inner.language.clone(),
            language_name: inner.language_name.clone(),
            prior_tree,
            edits,
            // Empty by default; the dispatch binding overrides with the
            // registry's alias snapshot (framing Q#IJ4). Callers that need
            // injections and bypass the registry set this themselves.
            injection_aliases: Arc::new(HashMap::new()),
        }
    }

    /// Install a freshly-parsed bundle. The caller is responsible
    /// for matching the bundle to the request that produced it ---
    /// installing a stale bundle would desynchronize the source
    /// mirror from the tree.
    pub fn install(&self, bundle: Arc<ParseTreeBundle>) {
        self.inner.lock().expect("ParseView mutex poisoned").current = Some(bundle);
    }
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
    /// Canonical language name. Used by [`SyntaxRegistry::language`]
    /// lookups, surfaced through Lua as the grammar label.
    pub name: &'static str,
    /// File extensions (without the leading dot) that should auto-
    /// attach this grammar's [`ParseView`] when a file is opened.
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
pub const BUILTIN_LANGUAGES: &[LanguageEntry] = &[
    LanguageEntry {
        name: "rust",
        extensions: &["rs"],
        loader: || tree_sitter_rust::LANGUAGE.into(),
        highlights_query: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
        injections_query: &[tree_sitter_rust::INJECTIONS_QUERY],
    },
    LanguageEntry {
        name: "lua",
        extensions: &["lua"],
        loader: || tree_sitter_lua::LANGUAGE.into(),
        highlights_query: &[tree_sitter_lua::HIGHLIGHTS_QUERY],
        injections_query: &[],
    },
    // T M9.7: markdown block grammar (`tree_sitter_md::LANGUAGE`) — headers,
    // lists, fenced code blocks, blockquotes. Its `injections.scm` (framing
    // Q#IJ10) drives two layer kinds: fenced code blocks inject the fence's
    // named language, and paragraph/heading text injects `markdown_inline`
    // (the entry below) — so inline emphasis/links are now highlighted, and
    // the former M9.7 "block-only, inline unhighlighted" floor is retired.
    // Note the constant name: `HIGHLIGHT_QUERY_BLOCK` (singular) is
    // the markdown crate's idiom; `tree-sitter-rust` and
    // `tree-sitter-lua` use `HIGHLIGHTS_QUERY` (plural).
    LanguageEntry {
        name: "markdown",
        extensions: &["md", "markdown"],
        loader: || tree_sitter_md::LANGUAGE.into(),
        highlights_query: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
        injections_query: &[tree_sitter_md::INJECTION_QUERY_BLOCK],
    },
    // markdown_inline (framing Q#IJ10) — the inline grammar the block
    // grammar injects for paragraph/heading text (`#set! injection.language
    // "markdown_inline"`). No file extension: it is injection-only, never
    // opened directly by name. Ships an inline highlights query (emphasis,
    // links, code spans) and its own injections (e.g. inline HTML), so it
    // recurses like any other layer. Retires the M9.7 block-only floor.
    LanguageEntry {
        name: "markdown_inline",
        extensions: &[],
        loader: || tree_sitter_md::INLINE_LANGUAGE.into(),
        highlights_query: &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
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
        injections_query: &[],
    },
    LanguageEntry {
        name: "cpp",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp", "inl", "cppm"],
        loader: || tree_sitter_cpp::LANGUAGE.into(),
        highlights_query: &[tree_sitter_cpp::HIGHLIGHT_QUERY],
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
        injections_query: &[],
    },
    LanguageEntry {
        name: "make",
        extensions: &["mk", "make"],
        loader: || tree_sitter_make::LANGUAGE.into(),
        highlights_query: &[tree_sitter_make::HIGHLIGHTS_QUERY],
        injections_query: &[],
    },
    LanguageEntry {
        name: "cmake",
        extensions: &["cmake"],
        loader: || tree_sitter_cmake::LANGUAGE.into(),
        highlights_query: &[tree_sitter_cmake::HIGHLIGHTS_QUERY],
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
        injections_query: &[],
    },
    LanguageEntry {
        name: "go",
        extensions: &["go"],
        loader: || tree_sitter_go::LANGUAGE.into(),
        highlights_query: &[tree_sitter_go::HIGHLIGHTS_QUERY],
        injections_query: &[],
    },
    // JavaScript / TypeScript. One `tree-sitter-javascript` grammar parses
    // both `.js` and `.jsx`; `tree-sitter-typescript` ships two grammars
    // (`LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`). Highlights inherit: the TS
    // query is a ~5-capture delta over JavaScript, and JSX is a further
    // `JSX_HIGHLIGHT_QUERY` delta — so the `*react` and `typescript*`
    // entries compose base-first (js → jsx → ts), the same pattern as
    // `cuda` over C/C++. The four names mirror the LSP filetype map
    // (typescriptreact/javascriptreact) so tsserver enables the JSX parser.
    LanguageEntry {
        name: "javascript",
        extensions: &["js", "mjs", "cjs"],
        loader: || tree_sitter_javascript::LANGUAGE.into(),
        highlights_query: &[tree_sitter_javascript::HIGHLIGHT_QUERY],
        injections_query: &[],
    },
    LanguageEntry {
        name: "javascriptreact",
        extensions: &["jsx"],
        loader: || tree_sitter_javascript::LANGUAGE.into(),
        highlights_query: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        injections_query: &[],
    },
    LanguageEntry {
        name: "typescript",
        extensions: &["ts", "mts", "cts"],
        loader: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlights_query: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        injections_query: &[],
    },
    LanguageEntry {
        name: "typescriptreact",
        extensions: &["tsx"],
        loader: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlights_query: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        injections_query: &[],
    },
    LanguageEntry {
        name: "toml",
        extensions: &["toml"],
        loader: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights_query: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
        injections_query: &[],
    },
    LanguageEntry {
        name: "zig",
        extensions: &["zig", "zon"],
        loader: || tree_sitter_zig::LANGUAGE.into(),
        highlights_query: &[tree_sitter_zig::HIGHLIGHTS_QUERY],
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
        injections_query: &[],
    },
    LanguageEntry {
        name: "yaml",
        extensions: &["yaml", "yml"],
        loader: || tree_sitter_yaml::LANGUAGE.into(),
        highlights_query: &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
        injections_query: &[],
    },
];

/// Registry that the Lua surface ([`crate::lua_bindings::install_parse`])
/// reads to map language names to grammars and buffer ids to attached
/// [`ParseView`] handles. Held by `Rc<...>` --- main-thread state, no
/// cross-thread sharing.
pub struct SyntaxRegistry {
    languages: RefCell<HashMap<String, tree_sitter::Language>>,
    views: RefCell<HashMap<BufferId, ParseViewHandle>>,
    /// Job id → buffer id mapping populated when a parse is dispatched
    /// for a buffer. The Lua-side install path looks up the buffer id
    /// from the settled job's id so it can drain the parse-handoff
    /// bundle into the right view.
    parse_jobs: RefCell<HashMap<JobId, BufferId>>,
    /// Custom (non-builtin) `extension → language name` mappings
    /// registered at runtime. Hosts that want to wire an extra
    /// grammar without touching [`BUILTIN_LANGUAGES`] can call
    /// [`SyntaxRegistry::register_extension`] alongside
    /// [`SyntaxRegistry::register_language`]. Looked up *after*
    /// [`BUILTIN_LANGUAGES`] so users can't accidentally shadow a
    /// builtin extension.
    extra_extensions: RefCell<HashMap<String, String>>,
    /// Compiled `highlights.scm` query per language (T M4.3). Lazy-
    /// compiled from [`LanguageEntry::highlights_query`] on first
    /// access; cached for the registry's lifetime. The result of a
    /// compilation failure (e.g. grammar / query ABI skew) is
    /// cached as `Err(message)` so we don't burn cycles re-trying.
    queries: RefCell<HashMap<String, Result<Arc<tree_sitter::Query>, String>>>,
    /// Fence-name → canonical-language alias map (framing Q#IJ4). Seeded
    /// with [`default_injection_aliases`]; Lua adds to it through
    /// [`Self::register_injection_alias`]. Snapshotted into each
    /// [`ParseRequest`] at dispatch so the worker reads a `Send` copy.
    injection_aliases: RefCell<HashMap<String, String>>,
    /// Active theme (T M4.3). Shared with every
    /// [`crate::highlight::SyntaxHighlightView`] attached through
    /// this registry --- editing the theme through Lua updates all
    /// attached views in lock-step.
    theme: ThemeHandle,
}

/// Cheaply-cloneable shared handle to a [`SyntaxRegistry`]. Same
/// `Rc<RefCell<...>>` shape as the other shared registries in
/// [`crate::lua_bindings`].
pub type SharedSyntaxRegistry = Rc<SyntaxRegistry>;

impl SyntaxRegistry {
    /// Construct an empty registry. Languages are registered by the
    /// host (Rust startup) before Lua scripts run, or lazy-loaded
    /// from [`BUILTIN_LANGUAGES`] on first lookup. Theme starts at
    /// the [`Theme::default_dark`] palette so an opening rust file
    /// gets a usable highlight without any Lua configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            languages: RefCell::new(HashMap::new()),
            views: RefCell::new(HashMap::new()),
            parse_jobs: RefCell::new(HashMap::new()),
            extra_extensions: RefCell::new(HashMap::new()),
            queries: RefCell::new(HashMap::new()),
            injection_aliases: RefCell::new(default_injection_aliases()),
            theme: Arc::new(Mutex::new(Theme::default_dark())),
        }
    }

    /// Shared theme handle. Cheap clone (an [`Arc`] bump). T M4.3:
    /// every [`crate::highlight::SyntaxHighlightView`] holds a clone
    /// of this same `Arc<Mutex<Theme>>`, so a Lua-driven theme edit
    /// is observable on the next render.
    #[must_use]
    pub fn theme(&self) -> ThemeHandle {
        self.theme.clone()
    }

    /// Register a tree-sitter [`tree_sitter::Language`] under `name`.
    /// Subsequent registrations under the same name overwrite the
    /// prior entry. Called from Rust startup; Lua scripts cannot
    /// construct `tree_sitter::Language` values directly.
    pub fn register_language(&self, name: impl Into<String>, lang: tree_sitter::Language) {
        self.languages.borrow_mut().insert(name.into(), lang);
    }

    /// Register a runtime extension → language mapping. The name
    /// must already exist (either from a manual
    /// [`Self::register_language`] or in [`BUILTIN_LANGUAGES`]).
    /// Custom mappings sit *after* [`BUILTIN_LANGUAGES`] in the
    /// lookup order, so users can't accidentally shadow a builtin.
    /// T M4.2: lets a host wire an out-of-tree grammar without
    /// touching the builtin table.
    pub fn register_extension(&self, ext: impl Into<String>, lang_name: impl Into<String>) {
        self.extra_extensions
            .borrow_mut()
            .insert(ext.into(), lang_name.into());
    }

    /// Look up a language by name. On miss, consults
    /// [`BUILTIN_LANGUAGES`] and lazy-loads the entry on demand
    /// (caching it for the rest of the registry's lifetime).
    #[must_use]
    pub fn language(&self, name: &str) -> Option<tree_sitter::Language> {
        if let Some(lang) = self.languages.borrow().get(name).cloned() {
            return Some(lang);
        }
        let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == name)?;
        let lang = (entry.loader)();
        self.languages
            .borrow_mut()
            .insert(name.to_owned(), lang.clone());
        Some(lang)
    }

    /// True if `name` is a registered or builtin language.
    #[must_use]
    pub fn has_language(&self, name: &str) -> bool {
        self.languages.borrow().contains_key(name)
            || BUILTIN_LANGUAGES.iter().any(|e| e.name == name)
    }

    /// Resolve a file extension to a language name. Checks
    /// [`BUILTIN_LANGUAGES`] first, then runtime-registered extras.
    /// Match is case-sensitive (file extensions traditionally are);
    /// extension is the part *after* the last `.`, with no leading
    /// dot. T M4.2.
    #[must_use]
    pub fn language_name_for_extension(&self, ext: &str) -> Option<&'static str> {
        BUILTIN_LANGUAGES
            .iter()
            .find(|e| e.extensions.contains(&ext))
            .map(|e| e.name)
    }

    /// Resolve a file path to a language name. Strips the path to
    /// its extension and delegates to
    /// [`Self::language_name_for_extension`]. Returns `None` for
    /// extensionless paths and unrecognized extensions.
    #[must_use]
    pub fn language_name_for_path(&self, path: &str) -> Option<String> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|os| os.to_str())?;
        if let Some(name) = self.language_name_for_extension(ext) {
            return Some(name.to_owned());
        }
        self.extra_extensions.borrow().get(ext).cloned()
    }

    /// Record the [`ParseViewHandle`] attached to a buffer.
    pub fn attach_view(&self, buffer: BufferId, handle: ParseViewHandle) {
        self.views.borrow_mut().insert(buffer, handle);
    }

    /// Retrieve the handle for `buffer` if one is attached.
    #[must_use]
    pub fn view(&self, buffer: BufferId) -> Option<ParseViewHandle> {
        self.views.borrow().get(&buffer).cloned()
    }

    /// Forget the handle for `buffer`. Called when a buffer is
    /// removed.
    pub fn detach_view(&self, buffer: BufferId) {
        self.views.borrow_mut().remove(&buffer);
    }

    /// Record that `job_id` was dispatched for `buffer`. The settle
    /// path looks the buffer up from the job id so it can install the
    /// settled bundle into the right view.
    pub fn record_parse_job(&self, job_id: JobId, buffer: BufferId) {
        self.parse_jobs.borrow_mut().insert(job_id, buffer);
    }

    /// Drain the recorded buffer-id for `job_id`.
    #[must_use]
    pub fn take_parse_job(&self, job_id: JobId) -> Option<BufferId> {
        self.parse_jobs.borrow_mut().remove(&job_id)
    }

    /// Number of unsettled parse-job → buffer mappings. Test helper.
    #[must_use]
    pub fn pending_parse_job_count(&self) -> usize {
        self.parse_jobs.borrow().len()
    }

    /// True when a dispatched parse job for `buffer` has not yet been
    /// installed or drained. The syntax Lua glue records jobs here at
    /// dispatch time and removes them in `_install_settled`, so this
    /// is the main-thread "parse in flight" bit for render producers
    /// that need to avoid stale whole-file work while typing.
    #[must_use]
    pub fn has_pending_parse_job_for(&self, buffer: BufferId) -> bool {
        self.parse_jobs.borrow().values().any(|&bid| bid == buffer)
    }

    /// Lazy-compile and cache the bundled `highlights.scm` query for
    /// `lang_name`. Returns `None` if the language is unknown, the
    /// language entry has an empty query (no highlights shipped),
    /// or compilation failed (the failure is cached so subsequent
    /// calls don't re-attempt). T M4.3.
    #[must_use]
    pub fn highlights_query(&self, lang_name: &str) -> Option<Arc<tree_sitter::Query>> {
        if let Some(slot) = self.queries.borrow().get(lang_name) {
            return slot.as_ref().ok().cloned();
        }
        let language = self.language(lang_name)?;
        let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == lang_name);
        // Fragments are joined with a newline, never bare-concatenated: a
        // fragment can end mid-`; comment` or without a trailing newline,
        // and abutting it against the next fragment's first token would
        // corrupt the query (e.g. `@variable; Functions` swallows the
        // next line into a comment).
        let source = entry.map_or_else(String::new, |e| e.highlights_query.join("\n"));
        if source.trim().is_empty() {
            self.queries
                .borrow_mut()
                .insert(lang_name.to_owned(), Err("no highlights query".to_owned()));
            return None;
        }
        let compiled = tree_sitter::Query::new(&language, &source)
            .map(Arc::new)
            .map_err(|e| format!("compile {lang_name} highlights: {e:?}"));
        let result = compiled.as_ref().ok().cloned();
        self.queries
            .borrow_mut()
            .insert(lang_name.to_owned(), compiled);
        result
    }

    /// Add or override a fence-name → language alias (framing Q#IJ4). The
    /// alias key is case-folded to match the resolver. Called from Lua via
    /// `pmacs.parse.injection_aliases`.
    pub fn register_injection_alias(&self, alias: impl Into<String>, lang: impl Into<String>) {
        self.injection_aliases
            .borrow_mut()
            .insert(alias.into().to_ascii_lowercase(), lang.into());
    }

    /// A `Send` snapshot of the alias map for a [`ParseRequest`] (Q#IJ4).
    #[must_use]
    pub fn injection_alias_snapshot(&self) -> Arc<HashMap<String, String>> {
        Arc::new(self.injection_aliases.borrow().clone())
    }

    /// Stage 2 of the injection handoff (framing Q#IJ2): fill each layer's
    /// `highlight_query` from this registry's cache and return the resolved
    /// bundle. The worker leaves the queries `None`; this runs on the main
    /// thread at settle where the `Rc` query cache lives. Tree clones are
    /// cheap (`ts_tree_copy` shares subtrees), so rebuilding the bundle is
    /// near-free.
    #[must_use]
    pub fn resolve_layer_queries(&self, raw: &ParseTreeBundle) -> Arc<ParseTreeBundle> {
        let layers = raw
            .layers
            .iter()
            .map(|l| Layer {
                language_name: l.language_name.clone(),
                tree: l.tree.clone(),
                depth: l.depth,
                highlight_query: self.highlights_query(&l.language_name),
            })
            .collect();
        Arc::new(ParseTreeBundle {
            layers,
            source: raw.source.clone(),
            language_name: raw.language_name.clone(),
            parse_duration: raw.parse_duration,
            injection_capped: raw.injection_capped,
        })
    }
}

impl Default for SyntaxRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl View for ParseView {
    fn on_edit(&mut self, _buf: &Buffer, edit: &Edit) -> Result<(), BufferError> {
        let start_byte = edit.range.start as usize;
        let old_end_byte = edit.range.end as usize;
        let new_end_byte = start_byte + edit.inserted_len as usize;

        let mut inner = self.inner.lock().expect("ParseView mutex poisoned");

        // Compute pre-edit Points BEFORE mutating the source mirror.
        let start_position = byte_to_point(&inner.source, start_byte);
        let old_end_position = byte_to_point(&inner.source, old_end_byte);

        // Splice: replace [start..old_end] with the inserted bytes
        // pulled from the new rope. The inserted bytes are at
        // [start_byte, new_end_byte) in the new rope.
        let mut new_bytes = vec![0u8; edit.inserted_len as usize];
        if !new_bytes.is_empty() {
            edit.new_rope
                .slice(start_byte as u64, new_end_byte as u64, &mut new_bytes);
        }
        inner.source.splice(start_byte..old_end_byte, new_bytes);

        // Now compute the new_end Point against the updated source.
        let new_end_position = byte_to_point(&inner.source, new_end_byte);

        inner.pending.push(tree_sitter::InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        });
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// T M4.3: highlight-span extraction
// ---------------------------------------------------------------------------

/// One highlighted byte range produced by a `highlights.scm` query
/// run against a [`ParseTreeBundle`]. The `capture_index` is into
/// [`tree_sitter::Query::capture_names`]; resolving to a style
/// happens inside [`crate::highlight::SyntaxHighlightView`] using
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
    compute_highlight_spans_for(
        query,
        bundle.root_tree(),
        bundle.source.as_ref(),
        byte_range,
    )
}

/// Like [`compute_highlight_spans_in_range`] but over an explicit
/// `(tree, source)` — the per-layer form the producers call for each
/// injection layer (framing Q#IJ7). `source` is the whole buffer; a
/// child layer's tree carries absolute offsets into it, so the same
/// capture walk works unchanged.
#[must_use]
pub fn compute_highlight_spans_for(
    query: &tree_sitter::Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
    byte_range: Option<std::ops::Range<usize>>,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    if let Some(range) = byte_range {
        cursor.set_byte_range(range);
    }
    let root = tree.root_node();
    let mut iter = cursor.captures(query, root, source);
    while let Some((qmatch, capture_idx)) = iter.next() {
        // Fail-closed on the locals property predicate. The capture
        // iterator already applies text predicates (`#eq?`/`#match?`/
        // `#any-of?`), but `#is? local` / `#is-not? local` are *property*
        // predicates (`Query::property_predicates`) that need a scope map
        // built from the grammar's LOCALS_QUERY, which pmacs does not run.
        // Applying such a capture regardless mis-styles shadowed locals —
        // e.g. a local `console`/`require` in JS/TS would still capture as
        // `@variable.builtin`/`@function.builtin`. Until locals processing
        // exists, drop captures whose pattern carries one; the identifier
        // falls back to its non-builtin capture. `#set!` (property
        // *settings*) is a different API and is not consulted here.
        if query
            .property_predicates(qmatch.pattern_index)
            .iter()
            .any(|(prop, _)| &*prop.key == "local")
        {
            continue;
        }
        let cap = qmatch.captures[*capture_idx];
        spans.push(HighlightSpan {
            start_byte: cap.node.start_byte() as u32,
            end_byte: cap.node.end_byte() as u32,
            capture_index: cap.index,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, BufferId, EditOp};

    fn fresh_buffer(name: &str) -> Buffer {
        Buffer::new(BufferId::next(), name)
    }

    fn rust_view(buf: &Buffer) -> (ParseView, ParseViewHandle) {
        let view = ParseView::new(buf, tree_sitter_rust::LANGUAGE.into(), "rust".to_owned());
        let handle = view.handle();
        (view, handle)
    }

    fn parse_synchronously(handle: &ParseViewHandle) -> Arc<ParseTreeBundle> {
        let req = handle.make_request();
        let bundle = Arc::new(run_parse(req).expect("parse succeeds"));
        handle.install(bundle.clone());
        bundle
    }

    /// Parse `src` as `lang` through `reg` with injection layers expanded
    /// (worker) and each layer's highlight query resolved (settle) — the
    /// full framing Q#IJ2 handoff. Returns the resolved bundle.
    fn parse_layered(reg: &SyntaxRegistry, lang: &str, src: &[u8]) -> Arc<ParseTreeBundle> {
        let language = reg.language(lang).expect("grammar loads");
        let mut buf = fresh_buffer("doc");
        buf.apply_edit(EditOp::Insert { pos: 0, bytes: src })
            .unwrap();
        let view = ParseView::new(&buf, language, lang.to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let mut req = handle.make_request();
        req.injection_aliases = reg.injection_alias_snapshot();
        let bundle = run_parse(req).expect("parse succeeds");
        reg.resolve_layer_queries(&bundle)
    }

    #[test]
    fn injection_query_block_compiles() {
        // ABI guard (framing acceptance #1): the markdown block + inline
        // injection queries must compile against their grammars. A crate
        // bump that drifted query and grammar apart surfaces here.
        let mut cache = HashMap::new();
        assert!(
            injection_query_cached(&mut cache, "markdown").is_some(),
            "markdown ships a compilable injection query"
        );
        assert!(
            injection_query_cached(&mut cache, "markdown_inline").is_some(),
            "markdown_inline ships a compilable injection query"
        );
        // A language with no injections resolves to None, not an error.
        assert!(injection_query_cached(&mut cache, "toml").is_none());
    }

    #[test]
    fn markdown_fence_builds_rust_child_layer() {
        // Framing acceptance #2.
        let reg = SyntaxRegistry::new();
        let src = b"# Title\n\n```rust\nfn demo() { let x = 1; }\n```\n\nText.\n";
        let bundle = parse_layered(&reg, "markdown", src);
        assert!(
            bundle.layers.len() >= 2,
            "fenced markdown is layered; got {} layer(s)",
            bundle.layers.len()
        );
        assert_eq!(
            bundle.layers[0].language_name, "markdown",
            "root is markdown"
        );
        let rust = bundle
            .layers
            .iter()
            .find(|l| l.language_name == "rust")
            .expect("a rust child layer for the ```rust fence");
        assert_eq!(
            rust.tree.root_node().kind(),
            "source_file",
            "rust root kind"
        );
        assert_eq!(rust.depth, 1, "the fence child is at depth 1");
    }

    #[test]
    fn child_layer_offsets_are_absolute() {
        // Framing acceptance #3 / mechanic #1: a node inside the fence has
        // byte offsets absolute into the FULL markdown source.
        let reg = SyntaxRegistry::new();
        let prefix = "# Title\n\n```rust\n";
        let src = format!("{prefix}fn demo() {{}}\n```\n");
        let bundle = parse_layered(&reg, "markdown", src.as_bytes());
        let rust = bundle
            .layers
            .iter()
            .find(|l| l.language_name == "rust")
            .expect("rust child layer");
        let fi = rust.tree.root_node().child(0).expect("function_item");
        assert!(
            fi.start_byte() >= prefix.len(),
            "child offset {} is absolute (>= prefix len {})",
            fi.start_byte(),
            prefix.len()
        );
        let text = &bundle.source[fi.start_byte()..fi.end_byte()];
        assert!(
            std::str::from_utf8(text).unwrap().contains("fn demo"),
            "absolute offsets index the rust code within the full source"
        );
    }

    #[test]
    fn dynamic_alias_resolves_and_unknown_skips() {
        // Framing acceptance #4: case-folded alias resolution + graceful
        // skip of an unknown fence language.
        let reg = SyntaxRegistry::new();
        for (fence, lang) in [("py", "python"), ("rs", "rust"), ("JS", "javascript")] {
            let src = format!("```{fence}\nvalue\n```\n");
            let bundle = parse_layered(&reg, "markdown", src.as_bytes());
            assert!(
                bundle.layers.iter().any(|l| l.language_name == lang),
                "fence ```{fence} resolves to {lang}"
            );
        }
        let bundle = parse_layered(&reg, "markdown", b"```nonsense\nvalue\n```\n");
        assert!(
            !bundle.layers.iter().any(|l| l.language_name == "nonsense"),
            "unknown fence language produces no child layer"
        );
        assert_eq!(bundle.layers[0].language_name, "markdown", "root intact");
        assert_eq!(bundle.root_tree().root_node().kind(), "document");
    }

    #[test]
    fn registry_alias_override_resolves_via_snapshot() {
        // Framing acceptance #5 (Rust-level bridge): a registered alias is
        // snapshotted into the request and resolved by the worker. The full
        // Lua-async path is covered in `tests/injection_acceptance.rs`.
        let reg = SyntaxRegistry::new();
        reg.register_injection_alias("MyLang", "rust"); // case-folded to `mylang`
        let bundle = parse_layered(&reg, "markdown", b"```mylang\nfn f() {}\n```\n");
        assert!(
            bundle.layers.iter().any(|l| l.language_name == "rust"),
            "a registered alias resolves the fence to its target grammar"
        );
    }

    #[test]
    fn inline_layer_multi_range_excludes_block_continuation() {
        // Framing acceptance #6 (round-2 finding 3): the multi-range path.
        // A one-line paragraph would give a SINGLE range (a link/emphasis
        // are child-grammar structures, not block-grammar named children),
        // so it can't prove multi-range. A MULTI-LINE blockquote's inline
        // node carries a named `block_continuation` child (the `> ` marker),
        // which `content_node_ranges` excludes — yielding MORE THAN ONE
        // included range, the genuine path markdown_inline depends on.
        let reg = SyntaxRegistry::new();
        let src = b"> first *one*\n> second *two*\n";
        let language = reg.language("markdown").expect("markdown");
        let mut buf = fresh_buffer("doc");
        buf.apply_edit(EditOp::Insert { pos: 0, bytes: src })
            .unwrap();
        let view = ParseView::new(&buf, language, "markdown".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let mut req = handle.make_request();
        req.injection_aliases = reg.injection_alias_snapshot();
        let raw = run_parse(req).expect("parse");

        // The inline injection match collects more than one included range.
        let mut cache = HashMap::new();
        let query = injection_query_cached(&mut cache, "markdown").expect("md injections");
        let inline_match = collect_injection_matches(&query, &raw.layers[0].tree, src)
            .into_iter()
            .find(|m| m.language == "markdown_inline")
            .expect("an inline injection match");
        assert!(
            inline_match.ranges.len() >= 2,
            "the multi-line inline node yields >1 range (block_continuation \
             excluded); got {:?}",
            inline_match
                .ranges
                .iter()
                .map(|r| (r.start_byte, r.end_byte))
                .collect::<Vec<_>>()
        );

        // Both ranges parse and highlight: emphasis is recognized on BOTH
        // lines, and the resolved layer produces spans.
        let bundle = reg.resolve_layer_queries(&raw);
        let inline = bundle
            .layers
            .iter()
            .find(|l| l.language_name == "markdown_inline")
            .expect("inline layer");
        let sexp = inline.tree.root_node().to_sexp();
        assert!(
            sexp.matches("emphasis").count() >= 2,
            "the inline grammar parsed emphasis in both ranges: {sexp}"
        );
        let hquery = inline
            .highlight_query
            .as_ref()
            .expect("inline highlights resolved at settle");
        let spans = compute_highlight_spans_for(hquery, &inline.tree, &bundle.source, None);
        assert!(
            !spans.is_empty(),
            "the inline layer produces highlight spans across both ranges"
        );
    }

    #[test]
    fn recursion_bounds_terminate() {
        // Framing acceptance #7: rust self-injects into macro token-trees, so
        // nested injections recurse. This proves the depth bound and, most
        // importantly, TERMINATION — a completing test (vs a hang) is the
        // observable guarantee. (The total-layer backstop is exercised
        // separately by `injection_layer_cap_surfaces_and_preserves_root`;
        // the `(lang, ranges)` visited guard is a defensive early-out that
        // no bundled grammar's self-injection-over-a-fixed-range can trip,
        // so it is covered by inspection + this termination check, not an
        // isolated positive case.)
        let reg = SyntaxRegistry::new();
        let src =
            b"macro_rules! m { () => { println!(\"{}\", vec![1, 2, 3]); }; }\nfn f() { m!(); }\n";
        let bundle = parse_layered(&reg, "rust", src);
        assert!(
            bundle.layers.len() <= MAX_INJECTION_LAYERS,
            "layer count within the backstop"
        );
        let max_depth = bundle.layers.iter().map(|l| l.depth).max().unwrap_or(0);
        assert!(
            max_depth <= MAX_INJECTION_DEPTH,
            "max depth {max_depth} within cap {MAX_INJECTION_DEPTH}"
        );
        assert_eq!(bundle.layers[0].language_name, "rust", "root is rust");
        assert!(
            bundle.layers.iter().all(|l| l.language_name == "rust"),
            "all layers are rust (self-injection)"
        );
    }

    #[test]
    fn injection_layer_cap_surfaces_and_preserves_root() {
        // Round-2 finding 4: hitting the total-layer backstop must set the
        // surfaced `injection_capped` flag (not drop silently), bound the
        // layer count, and keep the root intact.
        let reg = SyntaxRegistry::new();
        let fences = MAX_INJECTION_LAYERS + 8; // just over the backstop
        let mut src = String::with_capacity(fences * 15);
        for _ in 0..fences {
            src.push_str("```rust\nx\n```\n\n");
        }
        let language = reg.language("markdown").expect("markdown");
        let mut buf = fresh_buffer("doc");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: src.as_bytes(),
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "markdown".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let mut req = handle.make_request();
        req.injection_aliases = reg.injection_alias_snapshot();
        let bundle = run_parse(req).expect("root parse");

        assert!(
            bundle.injection_capped,
            "hitting the backstop sets the surfaced flag"
        );
        assert!(
            bundle.layers.len() <= MAX_INJECTION_LAYERS,
            "layer count is bounded by the backstop; got {}",
            bundle.layers.len()
        );
        assert_eq!(bundle.layers[0].language_name, "markdown", "root intact");
        assert_eq!(bundle.root_tree().root_node().kind(), "document");
    }

    #[test]
    fn non_injecting_buffer_single_layer() {
        // Framing acceptance #13: a plain rust file with no macros produces
        // exactly one layer — no behavior change for non-injecting content.
        let reg = SyntaxRegistry::new();
        let bundle = parse_layered(&reg, "rust", b"fn main() { let x = 1; }\n");
        assert_eq!(
            bundle.layers.len(),
            1,
            "no injections yields the single root layer"
        );
        assert_eq!(bundle.layers[0].depth, 0);
    }

    #[test]
    fn incremental_edit_reflects_in_child_and_new_fence_adds_layer() {
        // Framing acceptance #11: editing inside a fence reflects in the
        // child layer after reparse; a NEW fence adds a layer.
        let reg = SyntaxRegistry::new();
        let language = reg.language("markdown").expect("markdown");
        let mut buf = fresh_buffer("doc");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"```rust\nfn a() {}\n```\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "markdown".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));

        let reparse = |handle: &ParseViewHandle| -> Arc<ParseTreeBundle> {
            let mut req = handle.make_request();
            req.injection_aliases = reg.injection_alias_snapshot();
            let bundle = reg.resolve_layer_queries(&run_parse(req).expect("parse"));
            handle.install(bundle.clone());
            bundle
        };
        let count_fns = |b: &ParseTreeBundle| -> usize {
            b.layers
                .iter()
                .find(|l| l.language_name == "rust")
                .map_or(0, |l| {
                    l.tree
                        .root_node()
                        .to_sexp()
                        .matches("function_item")
                        .count()
                })
        };

        let b0 = reparse(&handle);
        assert_eq!(
            b0.layers
                .iter()
                .filter(|l| l.language_name == "rust")
                .count(),
            1,
            "initial: one rust fence layer"
        );
        assert_eq!(count_fns(&b0), 1, "initial rust child has one function");

        // Edit inside the fence: add a second function before the closer.
        let src = handle.source_snapshot();
        let at = src
            .windows(4)
            .position(|w| w == b"\n```")
            .expect("closing fence");
        buf.apply_edit(EditOp::Insert {
            pos: at as u64,
            bytes: b"\nfn b() {}",
        })
        .unwrap();
        let b1 = reparse(&handle);
        assert!(
            count_fns(&b1) >= 2,
            "an edit inside the fence is reflected in the rust child layer"
        );

        // Append a NEW python fence → a new layer appears.
        let end = buf.len();
        buf.apply_edit(EditOp::Insert {
            pos: end,
            bytes: b"\n```py\nx = 1\n```\n",
        })
        .unwrap();
        let b2 = reparse(&handle);
        assert!(
            b2.layers.iter().any(|l| l.language_name == "python"),
            "a newly-added fence adds its child layer"
        );
    }

    #[test]
    fn builtin_languages_include_c_and_cpp() {
        // M_B3 regression guard: a future refactor must not silently
        // drop C / C++ tree-sitter coverage — the dual-authority
        // styling in the TUI depends on these entries existing and
        // claiming their canonical extensions.
        let c = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "c")
            .expect("`c` language entry must be present");
        assert!(c.extensions.contains(&"c"), "`c` claims `.c`");
        assert!(
            c.extensions.contains(&"h"),
            "`c` claims `.h` (matches the LSP filetype map's default)"
        );
        assert!(
            !c.highlights_query.is_empty(),
            "`c` ships a non-empty highlights query"
        );
        let cpp = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "cpp")
            .expect("`cpp` language entry must be present");
        for ext in ["cpp", "cc", "cxx", "hpp", "hh", "hxx"] {
            assert!(cpp.extensions.contains(&ext), "`cpp` claims `.{ext}`");
        }
        assert!(
            !cpp.highlights_query.is_empty(),
            "`cpp` ships a non-empty highlights query"
        );
    }

    #[test]
    fn builtin_languages_include_cuda() {
        // Regression guard mirroring `builtin_languages_include_c_and_cpp`:
        // the CUDA entry must keep claiming its canonical extensions and,
        // because its bundled query is a `; inherits: cpp` delta, prepend
        // the C and C++ base queries explicitly (see the entry comment and
        // `cuda_highlights_resolve_c_and_cpp_captures`).
        let cuda = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "cuda")
            .expect("`cuda` language entry must be present");
        assert!(cuda.extensions.contains(&"cu"), "`cuda` claims `.cu`");
        assert!(cuda.extensions.contains(&"cuh"), "`cuda` claims `.cuh`");
        assert!(
            cuda.highlights_query
                .contains(&tree_sitter_c::HIGHLIGHT_QUERY),
            "`cuda` prepends the C base highlights (it does not resolve `inherits:`)"
        );
        assert!(
            cuda.highlights_query
                .contains(&tree_sitter_cpp::HIGHLIGHT_QUERY),
            "`cuda` prepends the C++ base highlights"
        );
        assert!(
            cuda.highlights_query
                .contains(&tree_sitter_cuda::HIGHLIGHTS_QUERY),
            "`cuda` carries its own CUDA-specific highlights delta"
        );
    }

    #[test]
    fn cuda_highlights_resolve_c_and_cpp_captures() {
        // Finding-2 regression: the bundled CUDA `highlights.scm` is only
        // a two-capture `; inherits: cpp` delta (launch brackets + CUDA
        // modifiers). pmacs does not resolve `inherits:`, so the entry
        // prepends the C and C++ base queries; assert the COMPILED query
        // actually carries ordinary C/C++ captures (the C base's
        // `@variable`) and far more than the delta's two capture classes —
        // not merely that some query is non-empty.
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("cuda")
            .expect("cuda highlights compile");
        let names = query.capture_names();
        assert!(
            names.contains(&"variable"),
            "combined query carries the C base `@variable` capture; got {names:?}"
        );
        assert!(
            names.len() >= 8,
            "combined C+C+++CUDA query resolves many capture classes, not the \
             CUDA delta's two; got {} ({names:?})",
            names.len()
        );
    }

    #[test]
    fn cuda_grammar_loads_and_parses_kernel_launch() {
        // ABI acceptance: a `tree-sitter-cuda` 0.21 grammar must be
        // accepted by our `tree-sitter` 0.26 core — `set_language`
        // succeeds and a tree is produced. This is the runtime check the
        // compile step cannot give us (a too-old grammar ABI fails only
        // here, at parse time). Grammar identity: `<<<grid, block>>>`
        // kernel-launch syntax is CUDA-specific; the C++ grammar parses
        // it as chained comparison/shift operators and flags an error, so
        // an error-free parse proves the entry wired the CUDA grammar,
        // not a C++ fallback.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("cuda")
            .expect("`cuda` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("kernel.cu");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"__global__ void add(int *c) { c[threadIdx.x] = 1; }\n\
                     int main() { add<<<1, 256>>>(0); return 0; }\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "cuda".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "translation_unit",
            "CUDA grammar (C-derived) roots at translation_unit"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "CUDA grammar parses the `<<<...>>>` kernel launch without error"
        );
    }

    #[test]
    fn language_for_path_resolves_cuda_extensions() {
        // `.cu`/`.cuh` resolve to the CUDA grammar through the same
        // extension-detection path as every other bundled language, so
        // the LSP filetype fallback in `lsp.lua` is never consulted for
        // them in practice.
        let reg = SyntaxRegistry::new();
        assert_eq!(
            reg.language_name_for_path("kernel.cu").as_deref(),
            Some("cuda")
        );
        assert_eq!(
            reg.language_name_for_path("device.cuh").as_deref(),
            Some("cuda")
        );
    }

    #[test]
    fn builtin_languages_include_bash() {
        // Regression guard: the bash entry claims the wider shell family
        // and ships a highlights query, so shell scripts get lexical color
        // (they had LSP via bash-language-server but no grammar before).
        let bash = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "bash")
            .expect("`bash` language entry must be present");
        for ext in ["sh", "bash", "zsh", "ksh", "ash", "bats"] {
            assert!(bash.extensions.contains(&ext), "`bash` claims `.{ext}`");
        }
        assert!(
            !bash.highlights_query.is_empty(),
            "`bash` ships a highlights query"
        );
    }

    #[test]
    fn bash_grammar_loads_and_parses_script() {
        // ABI acceptance: the `tree-sitter-bash` 0.25 grammar must be
        // accepted by our `tree-sitter` 0.26 core — `set_language`
        // succeeds and a tree is produced. A representative script
        // (shebang, `set`, parameter expansion, function, `if`) parses
        // without error, so the entry wired a working grammar.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("bash")
            .expect("`bash` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("deploy.sh");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"#!/usr/bin/env bash\nset -euo pipefail\nname=${1:-world}\n\
                     greet() { echo \"hello, $name\"; }\nif [ -n \"$name\" ]; then greet; fi\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "bash".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "program",
            "bash grammar roots at `program`"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "bash grammar parses a representative script without error"
        );
    }

    #[test]
    fn bash_highlights_compile_with_captures() {
        // The self-contained bash `highlights.scm` must compile against
        // the grammar and yield real capture classes (a crate bump that
        // drifted query and grammar apart would surface here).
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("bash")
            .expect("bash highlights compile");
        assert!(
            query.capture_names().len() >= 5,
            "bash highlights resolve several capture classes; got {}",
            query.capture_names().len()
        );
    }

    #[test]
    fn language_for_path_resolves_bash_extensions() {
        // The whole shell family resolves to the `bash` grammar via
        // extension detection; `.zsh`/`.ksh`/`.ash`/`.bats` are new here
        // (only `.sh`/`.bash` were covered by the LSP filetype map before).
        let reg = SyntaxRegistry::new();
        for path in [
            "deploy.sh",
            "lib.bash",
            "prompt.zsh",
            "script.ksh",
            "init.ash",
            "test_cli.bats",
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some("bash"),
                "{path} resolves to bash"
            );
        }
    }

    #[test]
    fn builtin_languages_include_json_and_yaml() {
        // Framing acceptance #1: both entries present, claim their
        // extensions, ship non-empty highlights.
        let json = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "json")
            .expect("`json` entry present");
        assert!(json.extensions.contains(&"json"), "`json` claims `.json`");
        assert!(!json.highlights_query.is_empty(), "`json` ships highlights");
        let yaml = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "yaml")
            .expect("`yaml` entry present");
        assert!(yaml.extensions.contains(&"yaml"), "`yaml` claims `.yaml`");
        assert!(yaml.extensions.contains(&"yml"), "`yaml` claims `.yml`");
        assert!(!yaml.highlights_query.is_empty(), "`yaml` ships highlights");
    }

    #[test]
    fn json_grammar_loads_and_parses() {
        // Framing acceptance #2 / ABI pin: `tree-sitter-json` 0.24 is
        // accepted by our tree-sitter 0.26 core; a JSON object parses to a
        // `document` root without error.
        let reg = SyntaxRegistry::new();
        let language = reg.language("json").expect("`json` loads");
        let mut buf = fresh_buffer("data.json");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"{\n  \"name\": \"pmacs\",\n  \"nums\": [1, 2, 3],\n  \"ok\": true\n}\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "json".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "document",
            "json grammar roots at `document`"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "json grammar parses an object without error"
        );
    }

    #[test]
    fn yaml_grammar_loads_and_parses() {
        // Framing acceptance #3 / ABI pin: `tree-sitter-yaml` 0.7 loads and
        // a YAML mapping parses to a `stream` root without error.
        let reg = SyntaxRegistry::new();
        let language = reg.language("yaml").expect("`yaml` loads");
        let mut buf = fresh_buffer("config.yaml");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"name: pmacs\nversion: 1\ntags:\n  - a\n  - b\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "yaml".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "stream",
            "yaml grammar roots at `stream`"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "yaml grammar parses a mapping without error"
        );
    }

    #[test]
    fn json_yaml_highlights_compile() {
        // Framing acceptance #4: both highlights queries compile against
        // their grammars and resolve capture classes.
        let reg = SyntaxRegistry::new();
        let json = reg
            .highlights_query("json")
            .expect("json highlights compile");
        assert!(
            json.capture_names().len() >= 3,
            "json highlights resolve capture classes; got {}",
            json.capture_names().len()
        );
        let yaml = reg
            .highlights_query("yaml")
            .expect("yaml highlights compile");
        assert!(
            yaml.capture_names().len() >= 3,
            "yaml highlights resolve capture classes; got {}",
            yaml.capture_names().len()
        );
    }

    #[test]
    fn language_for_path_resolves_json_yaml() {
        // Framing acceptance #5.
        let reg = SyntaxRegistry::new();
        assert_eq!(
            reg.language_name_for_path("tsconfig.json").as_deref(),
            Some("json")
        );
        assert_eq!(
            reg.language_name_for_path("config.yaml").as_deref(),
            Some("yaml")
        );
        assert_eq!(
            reg.language_name_for_path("ci.yml").as_deref(),
            Some("yaml")
        );
    }

    #[test]
    fn yaml_frontmatter_injects_in_markdown() {
        // Framing acceptance #7 — THE headline synergy with #122: a markdown
        // `---` frontmatter block (a `minus_metadata` node) is injected as
        // yaml by the bundled markdown injection query, so registering the
        // yaml grammar lights it up with no extra wiring.
        let reg = SyntaxRegistry::new();
        let src = b"---\ntitle: Hello\ntags: [a, b]\n---\n\n# Body\n";
        let bundle = parse_layered(&reg, "markdown", src);
        let yaml = bundle
            .layers
            .iter()
            .find(|l| l.language_name == "yaml")
            .expect("`---` frontmatter yields a yaml child layer");
        assert_eq!(
            yaml.tree.root_node().kind(),
            "stream",
            "yaml layer roots at stream"
        );
        let query = yaml
            .highlight_query
            .as_ref()
            .expect("yaml highlights resolved");
        let spans = compute_highlight_spans_for(query, &yaml.tree, &bundle.source, None);
        assert!(!spans.is_empty(), "the yaml frontmatter layer highlights");
    }

    #[test]
    fn json_fence_injects_in_markdown() {
        // Framing acceptance #8: a ```json fence yields a json child layer
        // through the #122 engine.
        let reg = SyntaxRegistry::new();
        let src = b"# Doc\n\n```json\n{\"a\": 1, \"b\": [2, 3]}\n```\n";
        let bundle = parse_layered(&reg, "markdown", src);
        let json = bundle
            .layers
            .iter()
            .find(|l| l.language_name == "json")
            .expect("a ```json fence yields a json child layer");
        assert_eq!(
            json.tree.root_node().kind(),
            "document",
            "json layer roots at document"
        );
    }

    #[test]
    fn builtin_languages_include_dockerfile_make_cmake() {
        for (name, exts) in [
            ("dockerfile", &["dockerfile", "containerfile"][..]),
            ("make", &["mk", "make"][..]),
            ("cmake", &["cmake"][..]),
        ] {
            let entry = BUILTIN_LANGUAGES
                .iter()
                .find(|l| l.name == name)
                .unwrap_or_else(|| panic!("`{name}` language entry must be present"));
            for ext in exts {
                assert!(entry.extensions.contains(ext), "`{name}` claims `.{ext}`");
            }
            assert!(
                entry.highlights_query.iter().any(|q| !q.is_empty()),
                "`{name}` ships a highlights query"
            );
        }
    }

    #[test]
    fn filename_grammars_load_and_parse() {
        // ABI acceptance: each 0.x/1.x grammar must be accepted by our
        // tree-sitter 0.26 core (set_language succeeds at runtime) and
        // parse a representative snippet without error, at its own root.
        let reg = SyntaxRegistry::new();
        let cases: &[(&str, &str, &[u8])] = &[
            (
                "dockerfile",
                "source_file",
                b"FROM alpine:3\nRUN apk add curl\nCMD [\"sh\"]\n",
            ),
            (
                "make",
                "makefile",
                b"all: build\n\tcc -o app main.c\n.PHONY: all\n",
            ),
            (
                "cmake",
                "source_file",
                b"cmake_minimum_required(VERSION 3.10)\nproject(demo)\n",
            ),
        ];
        for (lang, root_kind, src) in cases {
            let language = reg
                .language(lang)
                .unwrap_or_else(|| panic!("`{lang}` loads from BUILTIN_LANGUAGES"));
            let mut buf = fresh_buffer(&format!("probe.{lang}"));
            buf.apply_edit(EditOp::Insert { pos: 0, bytes: src })
                .unwrap();
            let view = ParseView::new(&buf, language, (*lang).to_owned());
            let handle = view.handle();
            let _vid = buf.attach_view(Box::new(view));
            let bundle = parse_synchronously(&handle);
            assert_eq!(
                bundle.root_tree().root_node().kind(),
                *root_kind,
                "`{lang}` roots at `{root_kind}`"
            );
            assert!(
                !bundle.root_tree().root_node().has_error(),
                "`{lang}` parses its snippet without error"
            );
        }
    }

    #[test]
    fn language_for_path_resolves_dockerfile_make_cmake_extensions() {
        let reg = SyntaxRegistry::new();
        for (path, lang) in [
            ("app.dockerfile", "dockerfile"),
            ("svc.containerfile", "dockerfile"),
            ("rules.mk", "make"),
            ("common.make", "make"),
            ("toolchain.cmake", "cmake"),
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some(lang),
                "{path} resolves to {lang}"
            );
        }
    }

    #[test]
    fn builtin_languages_include_gap_grammars() {
        for (name, exts) in [
            ("python", &["py", "pyi"][..]),
            ("go", &["go"][..]),
            ("javascript", &["js", "mjs", "cjs"][..]),
            ("javascriptreact", &["jsx"][..]),
            ("typescript", &["ts", "mts", "cts"][..]),
            ("typescriptreact", &["tsx"][..]),
            ("toml", &["toml"][..]),
            ("zig", &["zig", "zon"][..]),
        ] {
            let entry = BUILTIN_LANGUAGES
                .iter()
                .find(|l| l.name == name)
                .unwrap_or_else(|| panic!("`{name}` language entry must be present"));
            for ext in exts {
                assert!(entry.extensions.contains(ext), "`{name}` claims `.{ext}`");
            }
            assert!(
                entry.highlights_query.iter().any(|q| !q.is_empty()),
                "`{name}` ships a highlights query"
            );
        }
    }

    #[test]
    fn gap_grammars_load_and_parse() {
        // ABI acceptance for each new grammar (set_language succeeds at
        // runtime) + a snippet that parses without error at the expected
        // root. Covers both `tree-sitter-typescript` grammars.
        let reg = SyntaxRegistry::new();
        let cases: &[(&str, &str, &[u8])] = &[
            ("python", "module", b"def f(x):\n    return x + 1\n"),
            ("go", "source_file", b"package main\nfunc main() {}\n"),
            ("javascript", "program", b"const x = 1;\nlet y = [x];\n"),
            (
                "javascriptreact",
                "program",
                b"const e = <div id=\"a\"/>;\n",
            ),
            ("typescript", "program", b"const x: number = 1;\n"),
            ("typescriptreact", "program", b"const e = <div/>;\n"),
            ("toml", "document", b"[pkg]\nname = \"x\"\n"),
            ("zig", "source_file", b"const std = @import(\"std\");\n"),
        ];
        for (lang, root_kind, src) in cases {
            let language = reg
                .language(lang)
                .unwrap_or_else(|| panic!("`{lang}` loads from BUILTIN_LANGUAGES"));
            let mut buf = fresh_buffer(&format!("probe_{lang}"));
            buf.apply_edit(EditOp::Insert { pos: 0, bytes: src })
                .unwrap();
            let view = ParseView::new(&buf, language, (*lang).to_owned());
            let handle = view.handle();
            let _vid = buf.attach_view(Box::new(view));
            let bundle = parse_synchronously(&handle);
            assert_eq!(
                bundle.root_tree().root_node().kind(),
                *root_kind,
                "`{lang}` roots at `{root_kind}`"
            );
            assert!(
                !bundle.root_tree().root_node().has_error(),
                "`{lang}` parses its snippet without error"
            );
        }
    }

    #[test]
    fn typescript_highlights_compose_the_javascript_base() {
        // The bundled TypeScript highlights are a ~5-capture delta over
        // JavaScript; the entries prepend the JS query (and JSX for tsx).
        // Assert the COMPILED query resolves far more than the delta — the
        // JS base is really there, not just the ts-specific captures.
        let reg = SyntaxRegistry::new();
        for lang in ["typescript", "typescriptreact"] {
            let query = reg
                .highlights_query(lang)
                .unwrap_or_else(|| panic!("`{lang}` highlights compile"));
            assert!(
                query.capture_names().len() >= 15,
                "`{lang}` composes the JavaScript base (got {} captures, delta alone is ~5)",
                query.capture_names().len()
            );
        }
    }

    #[test]
    fn javascript_shadowed_builtin_is_not_mislabeled() {
        // `#is-not? local` (JS/TS use it for console/require/etc.) needs a
        // scope map from the LOCALS_QUERY we don't run, so
        // `compute_highlight_spans` drops captures guarded by it. Here
        // `console` is a LOCAL declaration — it must not surface as a
        // `*.builtin` capture (which is what a naive run of the shared JS
        // query would produce).
        let reg = SyntaxRegistry::new();
        let language = reg.language("javascript").expect("javascript loads");
        let query = reg
            .highlights_query("javascript")
            .expect("javascript highlights compile");
        let mut buf = fresh_buffer("shadow.js");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"const console = 5;\nconsole;\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "javascript".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        let spans = compute_highlight_spans(&query, &bundle);
        assert!(!spans.is_empty(), "the JS query produced highlight spans");
        let names = query.capture_names();
        let builtin: Vec<&str> = spans
            .iter()
            .map(|s| names[s.capture_index as usize])
            .filter(|n| n.contains("builtin"))
            .collect();
        assert!(
            builtin.is_empty(),
            "a locally-shadowed `console` must not get a *.builtin capture; got {builtin:?}"
        );
        // ...and dropping the builtin pattern must not strip *all* styling:
        // each `console` occurrence still keeps its ordinary `@variable`
        // capture (the fallback), so the loss is only the `.builtin` refine.
        let src = "const console = 5;\nconsole;\n";
        for (pos, _) in src.match_indices("console") {
            let (start, end) = (pos as u32, (pos + "console".len()) as u32);
            let caps: Vec<&str> = spans
                .iter()
                .filter(|s| s.start_byte == start && s.end_byte == end)
                .map(|s| names[s.capture_index as usize])
                .collect();
            assert!(
                caps.iter().any(|n| n.starts_with("variable")),
                "`console` at byte {pos} keeps a variable capture; got {caps:?}"
            );
        }
    }

    #[test]
    fn gap_grammar_extensions_resolve() {
        let reg = SyntaxRegistry::new();
        for (path, lang) in [
            ("main.py", "python"),
            ("stub.pyi", "python"),
            ("server.go", "go"),
            ("app.js", "javascript"),
            ("mod.mjs", "javascript"),
            ("view.jsx", "javascriptreact"),
            ("index.ts", "typescript"),
            ("types.mts", "typescript"),
            ("App.tsx", "typescriptreact"),
            ("Cargo.toml", "toml"),
            ("build.zig", "zig"),
            ("config.zon", "zig"),
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some(lang),
                "{path} resolves to {lang}"
            );
        }
    }

    #[test]
    fn byte_to_point_handles_first_line() {
        let src = b"hello world";
        assert_eq!(byte_to_point(src, 0), tree_sitter::Point::new(0, 0));
        assert_eq!(byte_to_point(src, 5), tree_sitter::Point::new(0, 5));
        assert_eq!(byte_to_point(src, 11), tree_sitter::Point::new(0, 11));
    }

    #[test]
    fn byte_to_point_counts_newlines() {
        let src = b"a\nbb\nccc";
        assert_eq!(byte_to_point(src, 0), tree_sitter::Point::new(0, 0));
        assert_eq!(byte_to_point(src, 1), tree_sitter::Point::new(0, 1));
        assert_eq!(byte_to_point(src, 2), tree_sitter::Point::new(1, 0));
        assert_eq!(byte_to_point(src, 4), tree_sitter::Point::new(1, 2));
        assert_eq!(byte_to_point(src, 5), tree_sitter::Point::new(2, 0));
        assert_eq!(byte_to_point(src, 8), tree_sitter::Point::new(2, 3));
    }

    #[test]
    fn byte_to_point_clamps_past_end() {
        let src = b"abc\ndef";
        assert_eq!(byte_to_point(src, 999), byte_to_point(src, src.len()));
    }

    #[test]
    fn parse_view_records_insert_input_edit() {
        let mut buf = fresh_buffer("scratch.rs");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"fn main() {}\n",
        })
        .unwrap();
        let (view, handle) = rust_view(&buf);
        let _vid = buf.attach_view(Box::new(view));
        // Insert " let x = 1;" between `{` and `}`. Position 11 is
        // the byte after `{` in "fn main() {}\n".
        buf.apply_edit(EditOp::Insert {
            pos: 11,
            bytes: b" let x = 1;",
        })
        .unwrap();
        assert_eq!(handle.pending_edit_count(), 1);
        assert_eq!(handle.source_snapshot(), b"fn main() { let x = 1;}\n");
    }

    #[test]
    fn parse_view_round_trips_initial_then_incremental_parse() {
        let mut buf = fresh_buffer("scratch.rs");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"fn main() {}\n",
        })
        .unwrap();
        let (view, handle) = rust_view(&buf);
        let _vid = buf.attach_view(Box::new(view));

        // Cold parse.
        let bundle = parse_synchronously(&handle);
        assert_eq!(bundle.root_tree().root_node().kind(), "source_file");
        assert_eq!(handle.pending_edit_count(), 0);

        // One incremental edit, then re-parse with the new source +
        // accumulated InputEdits.
        buf.apply_edit(EditOp::Insert {
            pos: 11,
            bytes: b" let _ = 1;",
        })
        .unwrap();
        assert_eq!(handle.pending_edit_count(), 1);
        let bundle = parse_synchronously(&handle);
        assert_eq!(bundle.root_tree().root_node().kind(), "source_file");
        assert_eq!(
            bundle.source.as_ref(),
            b"fn main() { let _ = 1;}\n",
            "bundle source must reflect post-edit bytes"
        );
        // Pending list cleared on make_request.
        assert_eq!(handle.pending_edit_count(), 0);
    }

    #[test]
    fn registry_tracks_inflight_parse_jobs_by_buffer() {
        let registry = SyntaxRegistry::new();
        let a = BufferId::next();
        let b = BufferId::next();

        registry.record_parse_job(11, a);
        registry.record_parse_job(12, b);

        assert!(registry.has_pending_parse_job_for(a));
        assert!(registry.has_pending_parse_job_for(b));

        assert_eq!(registry.take_parse_job(11), Some(a));
        assert!(!registry.has_pending_parse_job_for(a));
        assert!(registry.has_pending_parse_job_for(b));
    }
}
