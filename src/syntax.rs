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
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub use pmacs_syntax::*;

use crate::async_runtime::JobId;
use crate::buffer::{Buffer, BufferError, BufferId};
use crate::highlight::{Theme, ThemeHandle};
use crate::rope::Edit;
use crate::view::View;

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
    /// Each pending edit's inserted bytes, in step with `pending`, which
    /// an isolated parse sends instead of the whole text (E7i). Drained
    /// with `pending`.
    pending_inserted: Vec<Vec<u8>>,
    /// Most recent settled parse, or `None` if no parse has run yet.
    current: Option<Arc<ParseTreeBundle>>,
    /// Set when a dispatched request produced no bundle (E7h.2: a parse
    /// cancelled at its deadline, or failed). That request drained the
    /// edits made since `current`, so `current` can no longer be carried
    /// forward incrementally: the next request parses cold. Cleared by
    /// `install`.
    cold_next: bool,
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
            pending_inserted: Vec::new(),
            current: None,
            cold_next: false,
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
        inner.pending_inserted.clear();
        // After a request that installed nothing, `current` predates edits
        // that request drained; carrying it forward would edit it with an
        // incomplete list. Parse cold instead (E7h.2).
        let prior_tree = if inner.cold_next {
            None
        } else {
            // A tree held by a parse unit (E7i) has no in-process copy.
            inner
                .current
                .as_ref()
                .filter(|b| b.isolated.is_none())
                .map(|b| b.root_tree().clone())
        };
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
            // Unbounded by default; the dispatch binding sets the editor's
            // `syntax.parse-deadline-ms` (E7h.2).
            deadline: None,
        }
    }

    /// [`Self::make_request`] for a parse unit (E7i): the same request and,
    /// beside it, each drained edit's inserted bytes, so the unit can apply
    /// the edits to its own copy of the text instead of receiving it whole.
    pub fn make_isolated_request(&self) -> (ParseRequest, Vec<Vec<u8>>) {
        let inserted = std::mem::take(
            &mut self
                .inner
                .lock()
                .expect("ParseView mutex poisoned")
                .pending_inserted,
        );
        (self.make_request(), inserted)
    }

    /// Record that the last request this view made produced no bundle
    /// (E7h.2): the view keeps `current`, and the next request parses
    /// cold because the drained edits are gone with the failed request.
    pub fn mark_unparsed(&self) {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .cold_next = true;
    }

    /// Install a freshly-parsed bundle. The caller is responsible
    /// for matching the bundle to the request that produced it ---
    /// installing a stale bundle would desynchronize the source
    /// mirror from the tree.
    pub fn install(&self, bundle: Arc<ParseTreeBundle>) {
        let mut inner = self.inner.lock().expect("ParseView mutex poisoned");
        inner.current = Some(bundle);
        inner.cold_next = false;
    }
}

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
    /// Compiled `locals.scm` query per language. Like `queries`, both
    /// compilation failures and absent query sources are cached.
    local_queries: RefCell<HashMap<String, Result<Arc<tree_sitter::Query>, String>>>,
    /// Fence-name → canonical-language alias map (framing Q#IJ4). Seeded
    /// with [`default_injection_aliases`]; Lua adds to it through
    /// [`Self::register_injection_alias`]. Snapshotted into each
    /// [`ParseRequest`] at dispatch so the worker reads a `Send` copy.
    injection_aliases: RefCell<HashMap<String, String>>,
    /// The byte ranges each buffer's renderers last showed (E7i): an
    /// isolated parse returns the spans over these, so a frame needs no
    /// second trip to the unit.
    interest: RefCell<HashMap<BufferId, Vec<(u32, u32)>>>,
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
            local_queries: RefCell::new(HashMap::new()),
            injection_aliases: RefCell::new(default_injection_aliases()),
            interest: RefCell::new(HashMap::new()),
            theme: Arc::new(Mutex::new(Theme::default_dark())),
        }
    }

    /// Remember that a renderer showed `range` of `buffer` (E7i).
    pub fn note_interest(&self, buffer: BufferId, range: (u32, u32)) {
        let mut map = self.interest.borrow_mut();
        let ranges = map.entry(buffer).or_default();
        if !ranges.contains(&range) {
            ranges.push(range);
            if ranges.len() > 4 {
                ranges.remove(0);
            }
        }
    }

    /// The ranges an isolated parse of `buffer` should return spans for:
    /// what its renderers last showed, widened by `margin` bytes on each
    /// side for the scroll an edit can cause, or the file's first `margin`
    /// bytes when nothing has been shown yet (E7i).
    #[must_use]
    pub fn interest_for(&self, buffer: BufferId, margin: u32) -> Vec<(u32, u32)> {
        match self.interest.borrow().get(&buffer) {
            Some(ranges) if !ranges.is_empty() => ranges
                .iter()
                .map(|&(s, e)| (s.saturating_sub(margin), e.saturating_add(margin)))
                .collect(),
            _ => vec![(0, margin)],
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

    /// Test-only: register a grammar the table does not ship, with its
    /// highlights and locals queries, as `name`.
    #[cfg(test)]
    pub(crate) fn register_fixture(
        &self,
        name: &str,
        language: tree_sitter::Language,
        highlights: &str,
        locals: &str,
    ) {
        let compile = |src: &str| {
            tree_sitter::Query::new(&language, src)
                .map(Arc::new)
                .map_err(|e| format!("{e:?}"))
        };
        let (h, l) = (compile(highlights), compile(locals));
        self.queries.borrow_mut().insert(name.to_owned(), h);
        self.local_queries.borrow_mut().insert(name.to_owned(), l);
        self.register_language(name, language);
    }

    /// Lazy-compile and cache the bundled `locals.scm` query for
    /// `lang_name`. Empty sources and compilation failures are cached.
    #[must_use]
    pub fn locals_query(&self, lang_name: &str) -> Option<Arc<tree_sitter::Query>> {
        if let Some(slot) = self.local_queries.borrow().get(lang_name) {
            return slot.as_ref().ok().cloned();
        }
        let language = self.language(lang_name)?;
        let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == lang_name);
        let source = entry.map_or_else(String::new, |e| e.locals_query.join("\n"));
        if source.trim().is_empty() {
            self.local_queries
                .borrow_mut()
                .insert(lang_name.to_owned(), Err("no locals query".to_owned()));
            return None;
        }
        let compiled = tree_sitter::Query::new(&language, &source)
            .map(Arc::new)
            .map_err(|e| format!("compile {lang_name} locals: {e:?}"));
        let result = compiled.as_ref().ok().cloned();
        self.local_queries
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
            .map(|layer| {
                let highlight_query = self.highlights_query(&layer.language_name);
                let local_facts = highlight_query
                    .as_deref()
                    .filter(|query| query_uses_local_predicates(query))
                    .and_then(|_| self.locals_query(&layer.language_name))
                    .map(|query| {
                        Arc::new(compute_local_facts(
                            &query,
                            &layer.tree,
                            raw.source.as_ref(),
                        ))
                    });
                Layer {
                    language_name: layer.language_name.clone(),
                    tree: layer.tree.clone(),
                    depth: layer.depth,
                    highlight_query,
                    local_facts,
                }
            })
            .collect();
        Arc::new(ParseTreeBundle {
            layers,
            source: raw.source.clone(),
            language_name: raw.language_name.clone(),
            parse_duration: raw.parse_duration,
            injection_capped: raw.injection_capped,
            layers_cut_by_deadline: raw.layers_cut_by_deadline,
            isolated: raw.isolated.clone(),
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
        inner.pending_inserted.push(new_bytes.clone());
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
        for (fence, lang) in [("py", "python"), ("rs", "rust"), ("SH", "bash")] {
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
        let spans = compute_highlight_spans_for(
            hquery,
            &inline.tree,
            &bundle.source,
            inline.local_facts.as_deref(),
            None,
        );
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
    fn builtin_languages_include_latex() {
        // The LaTeX entry claims its four extensions and — uniquely among
        // BUILTIN_LANGUAGES — drives highlighting from the in-repo overlay
        // `builtin/queries/latex/highlights.scm` (`LATEX_HIGHLIGHTS`), because
        // the grammar crate exports no query constant (framing Q#LX2).
        let latex = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "latex")
            .expect("`latex` language entry must be present");
        for ext in ["tex", "latex", "sty", "cls"] {
            assert!(latex.extensions.contains(&ext), "`latex` claims `.{ext}`");
        }
        assert!(
            latex.highlights_query.contains(&LATEX_HIGHLIGHTS),
            "`latex` carries the in-repo highlights overlay"
        );
        assert!(
            !LATEX_HIGHLIGHTS.trim().is_empty(),
            "the vendored LaTeX highlights overlay is non-empty"
        );
    }

    #[test]
    fn latex_grammar_loads_and_parses() {
        // ABI acceptance: `codebook-tree-sitter-latex` (LanguageFn over
        // `tree-sitter-language 0.1`) must be accepted by our `tree-sitter`
        // 0.26 core. The `verbatim` environment exercises the grammar's
        // external scanner (`scanner.c`) — the exact surface the squatted,
        // scanner-less `tree-sitter-latex` 0.1.0 crate lacked — so an
        // error-free parse proves the linkable republish is wired, not a
        // partial grammar.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("latex")
            .expect("`latex` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("paper.tex");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"\\documentclass{article}\n\
                     \\begin{document}\n\
                     Hello $x^2$ and text.\n\
                     \\begin{verbatim}\n\
                     raw $ text\n\
                     \\end{verbatim}\n\
                     \\end{document}\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "latex".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "source_file",
            "LaTeX grammar roots at source_file"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "LaTeX grammar parses a document with a verbatim environment \
             (external scanner) without error"
        );
    }

    #[test]
    fn latex_highlights_resolve() {
        // The vendored overlay must COMPILE against the bundled grammar —
        // simultaneously the grammar/query node-name compatibility gate
        // (framing Q#LX2): a query referencing a node the grammar version
        // lacks fails here. Assert the reconciled captures are the ones pmacs
        // recognizes, and that no upstream fall-through capture survived.
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("latex")
            .expect("latex highlights compile against the grammar");
        let names = query.capture_names();
        for expected in ["function", "keyword", "comment"] {
            assert!(
                names.contains(&expected),
                "reconciled query carries the recognized `@{expected}` capture; got {names:?}"
            );
        }
        for stray in [
            "module",
            "label",
            "markup.heading",
            "markup.link",
            "markup.math",
        ] {
            assert!(
                !names.contains(&stray),
                "reconciliation removed the fall-through `@{stray}` capture; got {names:?}"
            );
        }
    }

    #[test]
    fn language_for_path_resolves_latex_extensions() {
        // `.tex`/`.latex`/`.sty`/`.cls` all resolve to the LaTeX grammar via
        // the same extension path as every bundled language — the single
        // `extensions` field wires detection ahead of the LSP filetype map.
        let reg = SyntaxRegistry::new();
        for path in ["paper.tex", "slides.latex", "mypkg.sty", "myclass.cls"] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some("latex"),
                "{path} resolves to latex"
            );
        }
    }

    #[test]
    fn builtin_languages_include_lean4() {
        // Framing acceptance 1/3 (`docs/archive/framings/lean4-mode-framing.md`). The entry is
        // named `lean4` because that name becomes the `didOpen` language_id
        // (Q#LN2), and it claims `.lean` ONLY: `.olean` is a compiled binary
        // artifact and `.ilean` is JSON metadata (Q#LN3).
        let lean = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "lean4")
            .expect("`lean4` language entry must be present");
        assert!(lean.extensions.contains(&"lean"), "`lean4` claims `.lean`");
        for unclaimed in ["olean", "ilean"] {
            assert!(
                !lean.extensions.contains(&unclaimed),
                "`lean4` must not claim `.{unclaimed}`"
            );
        }
        assert!(
            lean.highlights_query
                .contains(&arborium_lean::HIGHLIGHTS_QUERY),
            "`lean4` drives highlighting from the crate's query constant, not an overlay"
        );
        assert!(
            lean.locals_query.is_empty() && lean.injections_query.is_empty(),
            "`lean4` ships neither locals nor injections (Q#LN1)"
        );
    }

    #[test]
    fn lean4_grammar_loads_and_parses() {
        // Framing acceptance 2 and the open half of Q#LN1: `arborium-lean`
        // exports `const fn language() -> LanguageFn` (not the `LANGUAGE`
        // const every other entry uses) over `tree-sitter-language 0.1`, and
        // its README demonstrates usage against a `tree_sitter_patched_
        // arborium` core. Neither is supposed to matter — the LanguageFn ABI
        // is shared — but "supposed to" is not evidence, so this pins that
        // OUR `tree-sitter` 0.26 core accepts it and produces a real tree.
        //
        // The fixture exercises the grammar's external scanner (`scanner.c`
        // supplies a NEWLINE token, so layout-sensitive `def`/`theorem`
        // bodies depend on it) and the Unicode operators that make Lean
        // Lean — `→`, `∀`, `≥` — which a byte-oriented misbuild would shred.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("lean4")
            .expect("`lean4` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("Basic.lean");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: "-- a comment\n\
                    def fibonacci : Nat → Nat\n\
                    \x20 | 0 => 0\n\
                    \x20 | n + 1 => n\n\
                    \n\
                    theorem fib_nonneg : ∀ n, fibonacci n ≥ 0 := by\n\
                    \x20 intro n\n\
                    \x20 exact Nat.zero_le _\n"
                .as_bytes(),
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "lean4".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "module",
            "Lean grammar roots at module"
        );
        let sexp = bundle.root_tree().root_node().to_sexp();
        // This specific committed fixture parses cleanly. The claim is
        // scoped to the fixture on purpose: Lean's syntax is user-extensible
        // via macros, so a static grammar necessarily mis-parses some legal
        // input (the upstream grammar says so itself, and the framing scores
        // it as bet 3). What a clean parse HERE proves is that the crate is
        // wired correctly, not that Lean is fully parseable.
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "the fixture parses without error; got {sexp}"
        );
        // `def` and `theorem` sit under a `declaration` wrapper, not directly
        // under `module`.
        for expected in ["(comment)", "(def ", "(theorem "] {
            assert!(
                sexp.contains(expected),
                "expected `{expected}` in the tree; got {sexp}"
            );
        }
        // The load-bearing part of this test. A grammar built against a
        // mismatched core, or one whose scanner mis-handles multibyte input,
        // does not fail loudly — it produces a tree that silently degrades on
        // exactly the characters Lean is made of. `→` must become an `arrow`,
        // `∀` a `forall`, and `≥` a `comparison`; if these three hold, the
        // UTF-8 path through the parser is sound.
        for expected in ["(arrow ", "(forall ", "(comparison "] {
            assert!(
                sexp.contains(expected),
                "Unicode operator did not produce `{expected}`; got {sexp}"
            );
        }
    }

    #[test]
    fn lean4_highlights_resolve() {
        // The crate's 213-line query must COMPILE against the grammar it
        // ships with — the node-name compatibility gate. A query referencing
        // a node this grammar version lacks fails here rather than silently
        // producing no spans at runtime.
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("lean4")
            .expect("lean4 highlights compile against the grammar");
        let names = query.capture_names();
        // The four capture names Q#LN4 adds to the GLOBAL theme table are
        // present here — this is the forward direction of that decision; the
        // reverse direction (what they do to other languages) is pinned in
        // `highlight.rs`.
        for expected in ["constructor", "character", "keyword.conditional", "warning"] {
            assert!(
                names.contains(&expected),
                "lean4 query uses `@{expected}`, which Q#LN4 adds to the theme; got {names:?}"
            );
        }
    }

    #[test]
    fn language_for_path_resolves_lean_extension() {
        let reg = SyntaxRegistry::new();
        assert_eq!(
            reg.language_name_for_path("Mathlib/Data/Nat/Basic.lean")
                .as_deref(),
            Some("lean4"),
            "`.lean` resolves to the lean4 grammar"
        );
        for unclaimed in ["Basic.olean", "Basic.ilean"] {
            assert_ne!(
                reg.language_name_for_path(unclaimed).as_deref(),
                Some("lean4"),
                "{unclaimed} must not resolve to lean4"
            );
        }
    }

    #[test]
    fn builtin_languages_include_haskell() {
        // Aside E7e. The crate's three query constants, `.hs` only.
        let hs = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "haskell")
            .expect("`haskell` language entry must be present");
        assert_eq!(
            hs.extensions,
            &["hs"],
            "`haskell` claims `.hs` and not `.lhs`"
        );
        assert_eq!(
            hs.highlights_query,
            &[tree_sitter_haskell::HIGHLIGHTS_QUERY]
        );
        assert_eq!(hs.locals_query, &[tree_sitter_haskell::LOCALS_QUERY]);
        assert_eq!(
            hs.injections_query,
            &[tree_sitter_haskell::INJECTIONS_QUERY]
        );
    }

    #[test]
    fn haskell_grammar_loads_and_parses() {
        // The crate rides `tree-sitter-language 0.1`, so `LANGUAGE.into()`
        // must yield a language our 0.26 core accepts. The fixture leans on
        // the external scanner (layout: `where` and `do` blocks close by
        // indentation, not braces), which a misbuilt scanner shreds.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("haskell")
            .expect("`haskell` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("Main.hs");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: "module Main (main) where\n\
                    \n\
                    import qualified Data.Map as Map\n\
                    \n\
                    -- | A greeting.\n\
                    greet :: String -> String\n\
                    greet name = \"hello, \" ++ name\n\
                    \n\
                    main :: IO ()\n\
                    main = do\n\
                    \x20 let m = Map.fromList [(1 :: Int, 'a')]\n\
                    \x20 putStrLn (greet \"world\")\n\
                    \x20 print (Map.size m)\n"
                .as_bytes(),
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "haskell".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        let root = bundle.root_tree().root_node();
        let sexp = root.to_sexp();
        assert_eq!(root.kind(), "haskell", "Haskell grammar roots at haskell");
        assert!(
            !root.has_error(),
            "the fixture parses without error; got {sexp}"
        );
        for expected in ["(header ", "(import ", "(signature ", "(function ", "(do "] {
            assert!(
                sexp.contains(expected),
                "expected `{expected}` in the tree; got {sexp}"
            );
        }
    }

    #[test]
    fn haskell_highlights_locals_and_injections_resolve() {
        // All three crate queries must compile against the grammar they ship
        // with; the highlights use supertype patterns (`decl/function`) that
        // an older core would refuse.
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("haskell")
            .expect("haskell highlights compile against the grammar");
        let names = query.capture_names();
        for expected in ["keyword", "type", "string", "comment", "function"] {
            assert!(
                names.contains(&expected),
                "haskell query uses `@{expected}`; got {names:?}"
            );
        }
        assert!(
            reg.locals_query("haskell").is_some(),
            "haskell locals compile"
        );
        let language = reg.language("haskell").expect("grammar loads");
        tree_sitter::Query::new(&language, tree_sitter_haskell::INJECTIONS_QUERY)
            .expect("haskell injections compile");
    }

    #[test]
    fn language_for_path_resolves_hs_and_not_lhs() {
        let reg = SyntaxRegistry::new();
        assert_eq!(
            reg.language_name_for_path("app/Main.hs").as_deref(),
            Some("haskell"),
            "`.hs` resolves to the haskell grammar"
        );
        assert_ne!(
            reg.language_name_for_path("app/Main.lhs").as_deref(),
            Some("haskell"),
            "`.lhs` must not resolve to haskell"
        );
    }

    #[test]
    fn lhs_is_not_haskell_to_this_grammar() {
        // Why `.lhs` is unclaimed. Literate Haskell is prose; the code is
        // either Bird-tracked (`> ` at column 0) or between `\begin{code}` and
        // `\end{code}`. GHC unliterates before it lexes; this grammar has no
        // such pass, so both styles parse as errors. If a grammar bump makes
        // either parse clean, this fails and `.lhs` is worth revisiting.
        let reg = SyntaxRegistry::new();
        let language = reg.language("haskell").expect("grammar loads");
        for (style, src) in [
            (
                "Bird tracks",
                "A literate module.\n\n> module Main where\n> main :: IO ()\n> main = pure ()\n",
            ),
            (
                "LaTeX style",
                "\\documentclass{article}\n\\begin{document}\n\\begin{code}\nmain :: IO ()\nmain = pure ()\n\\end{code}\n\\end{document}\n",
            ),
        ] {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).unwrap();
            let tree = parser.parse(src, None).expect("parse");
            assert!(
                tree.root_node().has_error(),
                "{style} literate source parsed clean: {}",
                tree.root_node().to_sexp()
            );
        }
    }

    #[test]
    fn a_hs_fence_in_markdown_injects_haskell() {
        let reg = SyntaxRegistry::new();
        for fence in ["hs", "haskell"] {
            let src = format!("```{fence}\nmain = pure ()\n```\n");
            let bundle = parse_layered(&reg, "markdown", src.as_bytes());
            assert!(
                bundle.layers.iter().any(|l| l.language_name == "haskell"),
                "fence ```{fence} resolves to haskell"
            );
        }
    }

    #[test]
    fn builtin_languages_include_html_and_css() {
        // Both crate grammars export their query constants (no overlay). HTML
        // additionally carries an injections query (script/style); CSS does not.
        let html = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "html")
            .expect("`html` language entry must be present");
        for ext in ["html", "htm", "xhtml"] {
            assert!(html.extensions.contains(&ext), "`html` claims `.{ext}`");
        }
        assert!(
            !html.highlights_query.is_empty(),
            "`html` ships a highlights query"
        );
        assert!(
            !html.injections_query.is_empty(),
            "`html` ships an injections query (script/style)"
        );
        let css = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "css")
            .expect("`css` language entry must be present");
        assert!(css.extensions.contains(&"css"), "`css` claims `.css`");
        assert!(
            !css.highlights_query.is_empty(),
            "`css` ships a highlights query"
        );
    }

    #[test]
    fn html_grammar_loads_and_parses() {
        // ABI acceptance: `tree-sitter-html` (LanguageFn over
        // `tree-sitter-language 0.1`) is accepted by our `tree-sitter` 0.26 core.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("html")
            .expect("`html` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("index.html");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"<!DOCTYPE html>\n<html><body><a href=\"x\">Hi</a></body></html>\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "html".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "document",
            "HTML grammar roots at document"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "HTML grammar parses a document without error"
        );
    }

    #[test]
    fn css_grammar_loads_and_parses() {
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("css")
            .expect("`css` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("style.css");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"a { color: red; }\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "css".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.root_tree().root_node().kind(),
            "stylesheet",
            "CSS grammar roots at stylesheet"
        );
        assert!(
            !bundle.root_tree().root_node().has_error(),
            "CSS grammar parses a rule without error"
        );
    }

    #[test]
    fn html_and_css_highlights_resolve() {
        // The crate-exported queries compile against their grammars (node-name
        // compatibility gate), and both use the `@tag` capture this lane teaches
        // the highlighter (Q#WEB4).
        let reg = SyntaxRegistry::new();
        for lang in ["html", "css"] {
            let query = reg
                .highlights_query(lang)
                .unwrap_or_else(|| panic!("{lang} highlights compile against the grammar"));
            let names = query.capture_names();
            assert!(
                names.contains(&"tag"),
                "{lang} highlights use the @tag capture; got {names:?}"
            );
        }
    }

    #[test]
    fn language_for_path_resolves_web_extensions() {
        let reg = SyntaxRegistry::new();
        for (path, lang) in [
            ("index.html", "html"),
            ("page.htm", "html"),
            ("doc.xhtml", "html"),
            ("style.css", "css"),
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some(lang),
                "{path} resolves to {lang}"
            );
        }
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
        let spans = compute_highlight_spans_for(
            query,
            &yaml.tree,
            &bundle.source,
            yaml.local_facts.as_deref(),
            None,
        );
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

    /// A registry holding JavaScript as a test fixture: the local-facts
    /// machinery's witnesses parse fixed JavaScript, the one grammar whose
    /// highlights use `local` predicates, which E7g unshipped.
    fn javascript_fixture() -> SyntaxRegistry {
        let registry = SyntaxRegistry::new();
        registry.register_fixture(
            "javascript",
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        );
        registry
    }

    #[test]
    fn javascript_family_is_not_bundled() {
        // E7g. tree-sitter-javascript 0.25.0 never returns from a 24-byte
        // file of unclosed brackets, and the TypeScript and TSX grammars do
        // the same on it; `e7g_the_javascript_family_stays_unshipped` in
        // tests/e7g_grammar_fuzz_acceptance.rs names it. No entry, no
        // extension, no fence alias brings them back unfuzzed.
        let family = [
            "javascript",
            "javascriptreact",
            "typescript",
            "typescriptreact",
        ];
        assert!(BUILTIN_LANGUAGES.iter().all(|l| !family.contains(&l.name)));
        let reg = SyntaxRegistry::new();
        for path in [
            "a.js", "a.mjs", "a.cjs", "a.jsx", "a.ts", "a.mts", "a.cts", "a.tsx",
        ] {
            assert_eq!(reg.language_name_for_path(path), None, "{path}");
        }
        assert!(
            default_injection_aliases()
                .values()
                .all(|lang| !family.contains(&lang.as_str()))
        );
    }

    #[test]
    fn builtin_languages_include_gap_grammars() {
        for (name, exts) in [
            ("python", &["py", "pyi"][..]),
            ("go", &["go"][..]),
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
        // root.
        let reg = SyntaxRegistry::new();
        let cases: &[(&str, &str, &[u8])] = &[
            ("python", "module", b"def f(x):\n    return x + 1\n"),
            ("go", "source_file", b"package main\nfunc main() {}\n"),
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
    fn local_sensitive_builtin_highlights_have_compilable_locals_queries() {
        let registry = SyntaxRegistry::new();
        for entry in BUILTIN_LANGUAGES {
            let Some(highlights) = registry.highlights_query(entry.name) else {
                continue;
            };
            if !query_uses_local_predicates(&highlights) {
                continue;
            }
            assert!(
                entry
                    .locals_query
                    .iter()
                    .any(|fragment| !fragment.trim().is_empty()),
                "`{}` highlights use a local predicate but ship no locals query",
                entry.name
            );
            assert!(
                registry.locals_query(entry.name).is_some(),
                "`{}` highlights use a local predicate but its locals query does not compile",
                entry.name
            );
        }
    }

    #[test]
    fn javascript_local_predicates_distinguish_lexical_scope() {
        let registry = javascript_fixture();
        let source = b"console.log('outer');\n\
                       require('outer');\n\
                       function f(console, require) {\n\
                         console.log('inner');\n\
                         require('inner');\n\
                       }\n\
                       window.alert('outer');\n";
        let bundle = parse_layered(&registry, "javascript", source);
        let layer = &bundle.layers[0];
        let query = layer
            .highlight_query
            .as_deref()
            .expect("javascript highlights compile");
        assert!(
            layer.local_facts.is_some(),
            "a local-sensitive highlight query must settle lexical facts"
        );
        let spans = compute_highlight_spans(query, &bundle);
        let names = query.capture_names();
        let captures_at = |start: usize, len: usize| -> Vec<&str> {
            spans
                .iter()
                .filter(|span| {
                    span.start_byte == start as u32 && span.end_byte == (start + len) as u32
                })
                .map(|span| names[span.capture_index as usize])
                .collect()
        };

        for identifier in ["console", "require"] {
            let positions: Vec<usize> = std::str::from_utf8(source)
                .expect("fixture is UTF-8")
                .match_indices(identifier)
                .map(|(position, _)| position)
                .collect();
            assert_eq!(positions.len(), 3, "fixture has three `{identifier}` uses");
            assert!(
                captures_at(positions[0], identifier.len())
                    .iter()
                    .any(|name| name.ends_with(".builtin")),
                "unshadowed outer `{identifier}` keeps its builtin refinement"
            );
            for position in &positions[1..] {
                let captures = captures_at(*position, identifier.len());
                assert!(
                    !captures.iter().any(|name| name.ends_with(".builtin")),
                    "local `{identifier}` at byte {position} is not builtin: {captures:?}"
                );
                assert!(
                    captures.iter().any(|name| name.starts_with("variable")),
                    "local `{identifier}` keeps an ordinary variable capture: {captures:?}"
                );
            }
        }

        let window = std::str::from_utf8(source)
            .expect("fixture is UTF-8")
            .find("window")
            .expect("window fixture");
        assert!(
            captures_at(window, "window".len())
                .iter()
                .any(|name| name == &"variable.builtin"),
            "an unresolved builtin after the function remains builtin"
        );
    }

    #[test]
    fn positive_and_capture_qualified_local_predicates_use_resolved_facts() {
        let registry = javascript_fixture();
        let source = b"let f = () => {};\nf();\ng();\n";
        let bundle = parse_layered(&registry, "javascript", source);
        let language = registry.language("javascript").expect("javascript loads");
        let facts = bundle.layers[0]
            .local_facts
            .as_deref()
            .expect("javascript local facts settle");

        let positive = tree_sitter::Query::new(&language, "((identifier) @local-id (#is? local))")
            .expect("positive local predicate compiles");
        let positive_spans =
            compute_highlight_spans_for(&positive, bundle.root_tree(), source, Some(facts), None);
        let f_positions: Vec<usize> = std::str::from_utf8(source)
            .expect("fixture is UTF-8")
            .match_indices('f')
            .map(|(position, _)| position)
            .collect();
        assert_eq!(f_positions.len(), 2);
        for position in f_positions {
            assert!(
                positive_spans.iter().any(|span| {
                    span.start_byte == position as u32 && span.end_byte == (position + 1) as u32
                }),
                "definition/reference `f` at byte {position} is local"
            );
        }
        let g_position = std::str::from_utf8(source)
            .expect("fixture is UTF-8")
            .find("g()")
            .expect("g call");
        assert!(
            positive_spans
                .iter()
                .all(|span| span.start_byte != g_position as u32),
            "unresolved `g` does not satisfy #is? local"
        );

        let qualified = tree_sitter::Query::new(
            &language,
            "((call_expression function: (identifier) @callee) @call \
             (#is? @callee local))",
        )
        .expect("capture-qualified local predicate compiles");
        let qualified_spans =
            compute_highlight_spans_for(&qualified, bundle.root_tree(), source, Some(facts), None);
        let qualified_names = qualified.capture_names();
        assert!(
            qualified_spans.iter().any(|span| {
                qualified_names[span.capture_index as usize] == "call"
                    && span.start_byte
                        == source
                            .windows(4)
                            .position(|window| window == b"f();")
                            .expect("f call") as u32
            }),
            "the call whose @callee is local satisfies the qualified predicate"
        );
        assert!(
            qualified_spans
                .iter()
                .all(|span| span.start_byte != g_position as u32),
            "the call whose @callee is unresolved fails the qualified predicate"
        );
    }

    #[test]
    fn local_definition_value_and_scope_inheritance_control_resolution() {
        let registry = javascript_fixture();
        let language = registry.language("javascript").expect("javascript loads");

        let value_source = b"let x = x;\nx;\n";
        let value_tree = {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&language)
                .expect("set javascript language");
            parser
                .parse(value_source, None)
                .expect("parse value fixture")
        };
        let value_locals = tree_sitter::Query::new(
            &language,
            "(variable_declarator \
               name: (identifier) @local.definition \
               value: (identifier) @local.definition-value) \
             (identifier) @local.reference",
        )
        .expect("definition-value locals query compiles");
        let value_facts = compute_local_facts(&value_locals, &value_tree, value_source);
        let x_positions: Vec<usize> = std::str::from_utf8(value_source)
            .expect("fixture is UTF-8")
            .match_indices('x')
            .map(|(position, _)| position)
            .collect();
        assert_eq!(x_positions.len(), 3);
        assert!(value_facts.contains(x_positions[0], x_positions[0] + 1));
        assert!(
            !value_facts.contains(x_positions[1], x_positions[1] + 1),
            "a definition is not visible inside its own value"
        );
        assert!(value_facts.contains(x_positions[2], x_positions[2] + 1));

        let scope_source = b"let x = 1;\nfunction f() { x; }\nx;\n";
        let scope_tree = {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&language)
                .expect("set javascript language");
            parser
                .parse(scope_source, None)
                .expect("parse scope fixture")
        };
        let scope_locals = tree_sitter::Query::new(
            &language,
            "((function_declaration) @local.scope \
                (#set! local.scope-inherits false)) \
             (variable_declarator name: (identifier) @local.definition) \
             (identifier) @local.reference",
        )
        .expect("non-inheriting locals query compiles");
        let scope_facts = compute_local_facts(&scope_locals, &scope_tree, scope_source);
        let x_positions: Vec<usize> = std::str::from_utf8(scope_source)
            .expect("fixture is UTF-8")
            .match_indices('x')
            .map(|(position, _)| position)
            .collect();
        assert_eq!(x_positions.len(), 3);
        assert!(scope_facts.contains(x_positions[0], x_positions[0] + 1));
        assert!(
            !scope_facts.contains(x_positions[1], x_positions[1] + 1),
            "a non-inheriting scope cannot see the outer `x`"
        );
        assert!(
            scope_facts.contains(x_positions[2], x_positions[2] + 1),
            "leaving the scope restores outer resolution"
        );
    }

    #[test]
    fn gap_grammar_extensions_resolve() {
        let reg = SyntaxRegistry::new();
        for (path, lang) in [
            ("main.py", "python"),
            ("stub.pyi", "python"),
            ("server.go", "go"),
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
