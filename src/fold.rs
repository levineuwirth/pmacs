// fold.rs --- Structural code-folding engine (Arc 6, Stage 1).

//! The instance-side fold engine: a per-buffer fold store, a structural
//! fold source over the tree-sitter parse, and the state-aware fold
//! operations the Lua command/data surface drives. No rendering lives
//! here — Stage 2 (grid) and Stage 3 (GPU) consume the store; the
//! semantic producer ships it as `FoldState`.
//!
//! Design (see `docs/archive/framings/folding-framing.md`, approved rev 5):
//!
//! - **Store = a set of byte ranges** attached to the buffer as a
//!   [`View`] so it translates across every edit, provenance-blind
//!   (Q#FD2/FD3/FD5). Stored range = `[end of head line, end of the
//!   last hidden line]`; containment is **start-exclusive,
//!   end-inclusive** `(start, end]` so a point at the end of the head
//!   line is *outside* (typing there shifts the fold right, landing the
//!   character visible on the head line) while a point at the end of the
//!   last hidden line is *inside* (typing there unfolds).
//! - **Source = structural node folding** (Q#FD1): the nearest enclosing
//!   block-like node ≥ 2 source lines → resolve introducer↔body → the
//!   head line is *the line immediately above the first hidden line*
//!   (so a rustfmt-wrapped signature or a `where` clause stays visible —
//!   hideshow / LSP `foldingRange` parity) → a **closer-aware tail**
//!   keeps a closing-delimiter line visible (`} else {`, `}, [deps])`).
//! - **Translate + drop only** (Q#FD6): an edit strictly inside the
//!   interior shifts the fold's end; an edit that crosses the head or
//!   tail boundary drops the fold. The *interactive* unfold-on-typing is
//!   a dispatch-layer pre-edit step (see `EditorCore`), not here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use pmacs_protocol::{BufferId, ByteRange};
use tree_sitter::Node;

use crate::buffer::{Buffer, BufferError, ViewId};
use crate::rope::Edit;
use crate::syntax::ParseTreeBundle;
use crate::view::View;

const OPEN_DELIMS: &[u8] = b"{[(";
const CLOSE_DELIMS: &[u8] = b"}])";

// ---------------------------------------------------------------------------
// Line math (a self-contained copy of the `highlight.rs` scan — kept private
// so the fold source has no cross-module coupling).
// ---------------------------------------------------------------------------

/// `out[n]` = start byte of line `n`; `out` always begins with `0`. The
/// number of lines is `out.len()` (a trailing entry past the final `\n`
/// is included, mirroring `highlight::compute_line_offsets`).
fn compute_line_offsets(source: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(source.len() / 32 + 1);
    out.push(0);
    for (i, b) in source.iter().enumerate() {
        if *b == b'\n' {
            out.push(i as u32 + 1);
        }
    }
    out
}

/// Index of the line containing byte `offset`.
fn line_at_offset(line_offsets: &[u32], offset: u32) -> usize {
    match line_offsets.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// The byte offset just past line `row`'s last *visible* character — i.e.
/// the position of the row's terminating `\n`, or `source.len()` for the
/// final unterminated line. This is the "end of line" the stored range
/// uses for both its head and tail.
fn line_content_end(source: &[u8], line_offsets: &[u32], row: usize) -> u64 {
    let start = line_offsets
        .get(row)
        .copied()
        .unwrap_or(source.len() as u32) as usize;
    let next = line_offsets
        .get(row + 1)
        .copied()
        .unwrap_or(source.len() as u32) as usize;
    let mut end = next.min(source.len());
    if end > start && source[end - 1] == b'\n' {
        end -= 1;
    }
    end as u64
}

/// True iff line `row`'s first non-whitespace byte is a closing delimiter
/// (`}`, `)`, `]`) — the closer-aware tail test.
fn line_starts_with_closer(source: &[u8], line_offsets: &[u32], row: usize) -> bool {
    let Some(&ls) = line_offsets.get(row) else {
        return false;
    };
    let mut i = ls as usize;
    while i < source.len() && (source[i] == b' ' || source[i] == b'\t') {
        i += 1;
    }
    i < source.len() && CLOSE_DELIMS.contains(&source[i])
}

// ---------------------------------------------------------------------------
// FoldStore — the per-buffer set of collapsed ranges.
// ---------------------------------------------------------------------------

/// A buffer's set of currently-collapsed ranges. Kept sorted by
/// `(start, end)`; nested folds are allowed; exact duplicates are not.
#[derive(Debug, Default)]
pub struct FoldStore {
    folds: Vec<ByteRange>,
}

impl FoldStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self { folds: Vec::new() }
    }

    /// Whether the store holds no folds.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.folds.is_empty()
    }

    /// The current folds, sorted and stable — the form the producer diffs
    /// and `pmacs.fold.folds` returns.
    #[must_use]
    pub fn folds(&self) -> Vec<ByteRange> {
        self.folds.clone()
    }

    /// Whether an exactly-equal fold range is already stored.
    #[must_use]
    pub fn contains_exact(&self, r: ByteRange) -> bool {
        self.folds.contains(&r)
    }

    /// Add a fold. Rejects an empty/inverted range or an exact duplicate;
    /// returns whether it was added.
    pub fn insert(&mut self, r: ByteRange) -> bool {
        if r.end <= r.start || self.contains_exact(r) {
            return false;
        }
        self.folds.push(r);
        self.normalize();
        true
    }

    /// Remove an exact fold; returns whether one was removed.
    pub fn remove(&mut self, r: ByteRange) -> bool {
        let before = self.folds.len();
        self.folds.retain(|f| *f != r);
        self.folds.len() != before
    }

    /// Drop every fold; returns whether anything was cleared.
    pub fn clear(&mut self) -> bool {
        let had = !self.folds.is_empty();
        self.folds.clear();
        had
    }

    /// Folds whose interior contains `p` under `(start, end]` containment,
    /// **innermost first** (a more deeply nested fold has the larger start).
    #[must_use]
    pub fn containing(&self, p: u64) -> Vec<ByteRange> {
        let mut v: Vec<ByteRange> = self
            .folds
            .iter()
            .copied()
            .filter(|f| f.start < p && p <= f.end)
            .collect();
        v.sort_by(|a, b| b.start.cmp(&a.start).then(a.end.cmp(&b.end)));
        v
    }

    /// Remove every fold containing `p` (the dispatch-layer pre-edit
    /// unfold, and the org-TAB "open all" leg). Returns the count removed.
    pub fn unfold_containing(&mut self, p: u64) -> usize {
        let before = self.folds.len();
        self.folds.retain(|f| !(f.start < p && p <= f.end));
        before - self.folds.len()
    }

    fn normalize(&mut self) {
        self.folds
            .sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        self.folds.dedup();
    }

    /// Translate every fold across `edit`, dropping any whose head or tail
    /// the edit crosses (Q#FD6). Provenance-blind: `Edit` carries no source
    /// frontend, so this cannot (and must not) unfold-on-typing — that is
    /// the dispatch layer's pre-edit job.
    ///
    /// Boundary handling mirrors `BufferStyleSpanTranslator`'s right-bias:
    /// an insert exactly at the start (end of the head line) shifts the
    /// whole fold right, so the character lands visible on the head line;
    /// an insert exactly at the end is left outside.
    pub fn translate(&mut self, edit: &Edit) {
        let os = edit.range.start;
        let oe = edit.range.end;
        let old_len = oe - os;
        let new_len = edit.inserted_len;
        // Buffers broadcast no-op edits; nothing moved.
        if old_len == 0 && new_len == 0 {
            return;
        }
        // Shift a byte offset by the edit's signed length delta, in u64
        // arithmetic (no `as i64` wrap): grow by `new_len - old_len` or
        // shrink by `old_len - new_len`, saturating at 0.
        let shift = |x: u64| -> u64 {
            if new_len >= old_len {
                x + (new_len - old_len)
            } else {
                x.saturating_sub(old_len - new_len)
            }
        };
        let mut kept = Vec::with_capacity(self.folds.len());
        for f in self.folds.drain(..) {
            let (s, e) = (f.start, f.end);
            let next = if oe <= s {
                // Strictly before the fold (an insert at exactly `s` lands
                // here, shifting the fold right — the head-line right-bias).
                Some(ByteRange {
                    start: shift(s),
                    end: shift(e),
                })
            } else if os > e || (os == e && old_len == 0) {
                // Strictly after the fold, OR a pure insert at exactly `e`
                // (left outside). A *delete* starting at `e` removes the
                // `\n` that `e` names — the terminator of the last hidden
                // line — so it destroys the tail and falls through to the
                // drop arm below, symmetric with the head side.
                Some(ByteRange { start: s, end: e })
            } else if os > s && oe < e {
                // Strictly inside the interior — the fold still hides a
                // valid interior; shift its end by the delta.
                let e2 = shift(e);
                if e2 > s {
                    Some(ByteRange { start: s, end: e2 })
                } else {
                    None
                }
            } else {
                // The edit crosses the head or tail boundary (or engulfs
                // the fold) — the head/tail it named is gone. Drop it.
                None
            };
            if let Some(r) = next {
                kept.push(r);
            }
        }
        self.folds = kept;
        self.normalize();
    }
}

// ---------------------------------------------------------------------------
// FoldStoreTranslator — the buffer-attached View that keeps the store in
// sync with edits.
// ---------------------------------------------------------------------------

struct FoldStoreTranslator {
    store: Arc<Mutex<FoldStore>>,
}

impl View for FoldStoreTranslator {
    fn on_edit(&mut self, _buf: &Buffer, edit: &Edit) -> Result<(), BufferError> {
        self.store
            .lock()
            .expect("fold store mutex poisoned")
            .translate(edit);
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "fold_store_translator"
    }
}

// ---------------------------------------------------------------------------
// FoldRegistry — per-buffer stores, keyed by BufferId (the SyntaxRegistry
// model), each paired with a translator View over the same Arc.
// ---------------------------------------------------------------------------

/// Shared, cloneable handle to the process's fold stores. Held by
/// `EditorCore` (for the pre-edit unfold), by `EditorState` and the
/// semantic producer (to ship `FoldState`), and by the `pmacs.fold` Lua
/// bindings (via Lua app-data) — all the same `Rc`.
pub type SharedFoldRegistry = Rc<FoldRegistry>;

struct FoldEntry {
    store: Arc<Mutex<FoldStore>>,
    view: ViewId,
}

/// One fold store per buffer. Interior-mutable so a `&SharedFoldRegistry`
/// suffices everywhere.
#[derive(Default)]
pub struct FoldRegistry {
    stores: RefCell<HashMap<BufferId, FoldEntry>>,
}

/// Build a fresh, empty fold registry.
#[must_use]
pub fn make_shared_fold_registry() -> SharedFoldRegistry {
    Rc::new(FoldRegistry::default())
}

impl FoldRegistry {
    /// The buffer's store if one exists — the lookup used by read-only
    /// callers (the pre-edit unfold and the producer) that must not
    /// materialize a store or attach a view.
    #[must_use]
    pub fn store(&self, buf: BufferId) -> Option<Arc<Mutex<FoldStore>>> {
        self.stores.borrow().get(&buf).map(|e| Arc::clone(&e.store))
    }

    /// The buffer's folds (sorted; empty when it has no store).
    #[must_use]
    pub fn folds(&self, buf: BufferId) -> Vec<ByteRange> {
        self.store(buf)
            .map(|s| s.lock().expect("fold store mutex poisoned").folds())
            .unwrap_or_default()
    }

    /// Get-or-create the store for `buffer`, attaching the translator view
    /// on first materialization so every later edit is tracked.
    pub fn store_or_attach(&self, buffer: &mut Buffer) -> Arc<Mutex<FoldStore>> {
        let id = buffer.id();
        if let Some(existing) = self.stores.borrow().get(&id) {
            return Arc::clone(&existing.store);
        }
        let store = Arc::new(Mutex::new(FoldStore::new()));
        let view = buffer.attach_view(Box::new(FoldStoreTranslator {
            store: Arc::clone(&store),
        }));
        self.stores.borrow_mut().insert(
            id,
            FoldEntry {
                store: Arc::clone(&store),
                view,
            },
        );
        store
    }

    /// Drop the buffer's store and detach its translator view — the
    /// content-replacement (revert/reload) reset, where the buffer survives
    /// but its bytes are replaced wholesale, so the view must come off too
    /// (a later fold re-attaches a fresh one). Named bytes no longer exist,
    /// so revalidation is not attempted (Q#FD8, framing acceptance 8).
    pub fn forget(&self, buffer: &mut Buffer) {
        if let Some(entry) = self.stores.borrow_mut().remove(&buffer.id()) {
            buffer.detach_view(entry.view);
        }
    }

    /// Drop the store for a buffer that has already been removed (the
    /// `pmacs.buffer.kill` path). The buffer — and its attached translator
    /// view — is gone, so only the map entry needs clearing; there is no
    /// view to detach.
    pub fn forget_buffer(&self, buf: BufferId) {
        self.stores.borrow_mut().remove(&buf);
    }

    /// Unfold every fold in `buf` containing `p`. The pre-edit hook the
    /// six `EditorCore` edit primitives call; a no-op when the buffer has
    /// no store (hence no folds). Returns the count unfolded.
    pub fn unfold_containing(&self, buf: BufferId, p: u64) -> usize {
        match self.store(buf) {
            Some(s) => s
                .lock()
                .expect("fold store mutex poisoned")
                .unfold_containing(p),
            None => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Structural fold source.
// ---------------------------------------------------------------------------

/// The innermost foldable region at `pos`, or `None` — the fold target the
/// data-API `toggle` and a bare "fold this" use.
#[must_use]
pub fn fold_target_at(bundle: &ParseTreeBundle, pos: u64) -> Option<ByteRange> {
    candidates_at(bundle, pos).into_iter().next()
}

/// Every foldable region enclosing `pos`, **innermost first**. The
/// state-aware commands walk this list against the store to decide what to
/// close (innermost open) or open (outermost closed).
#[must_use]
pub fn candidates_at(bundle: &ParseTreeBundle, pos: u64) -> Vec<ByteRange> {
    let source: &[u8] = &bundle.source;
    let line_offsets = compute_line_offsets(source);
    let Some(node) = innermost_named_node(bundle, pos) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(r) = fold_from_node(n, source, &line_offsets)
            && !out.contains(&r)
        {
            out.push(r);
        }
        cur = n.parent();
    }
    out
}

/// The top-level foldable regions in the buffer — what `fold.close-all`
/// collapses (Emacs `hs-hide-all`: top level only, nested not auto-folded).
#[must_use]
pub fn top_level_fold_targets(bundle: &ParseTreeBundle) -> Vec<ByteRange> {
    let source: &[u8] = &bundle.source;
    let line_offsets = compute_line_offsets(source);
    let root = bundle.root_tree().root_node();
    let mut out = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if let Some(r) = fold_from_node(child, source, &line_offsets)
            && !out.contains(&r)
        {
            out.push(r);
        }
    }
    out
}

/// The innermost named node at `pos`, resolved through injection layers:
/// the deepest layer whose root span covers `pos` wins (a fenced code block
/// inside markdown resolves to the inner block, not the markdown node).
fn innermost_named_node(bundle: &ParseTreeBundle, pos: u64) -> Option<Node<'_>> {
    let p = pos as usize;
    let mut best: Option<&crate::syntax::Layer> = None;
    for layer in &bundle.layers {
        let root = layer.tree.root_node();
        if root.start_byte() <= p && p <= root.end_byte() {
            best = match best {
                Some(b) if b.depth >= layer.depth => Some(b),
                _ => Some(layer),
            };
        }
    }
    best?.tree.root_node().named_descendant_for_byte_range(p, p)
}

/// Compute the fold range a single node yields, or `None` if it is not a
/// foldable structure (< 2 source lines, no block-like body, or a
/// normalized interior with < 1 hidden line).
fn fold_from_node(n: Node<'_>, source: &[u8], line_offsets: &[u32]) -> Option<ByteRange> {
    // Match condition: the node spans >= 2 source lines.
    if n.end_position().row <= n.start_position().row {
        return None;
    }
    let (b, introduced) = resolve_body(n, source)?;

    let b_start_row = b.start_position().row;
    let b_end_row = b.end_position().row;
    let b_start_byte = b.start_byte();
    if b_start_byte >= source.len() {
        return None;
    }
    let is_brace = OPEN_DELIMS.contains(&source[b_start_byte]);

    // Head line = the line immediately above the first hidden line. For a
    // brace body that is the `{` line (the introducer's own line, or the
    // `) -> bool {` line when a signature wraps). For an *introduced*
    // delimiter-less body (a Python `block`) the introducer's header ends
    // on the line above, so it is `b_start_row - 1`.
    let head_row = if is_brace {
        b_start_row
    } else if introduced && b_start_row > 0 {
        b_start_row - 1
    } else {
        b_start_row
    };

    // Tail: a closing-delimiter line stays visible (`} else {`); a
    // delimiter-less body hides through its last line.
    let last_hidden_row = if line_starts_with_closer(source, line_offsets, b_end_row) {
        if b_end_row == 0 {
            return None;
        }
        b_end_row - 1
    } else {
        b_end_row
    };

    // Foldability = the normalized interior has >= 1 hidden line.
    if last_hidden_row < head_row + 1 {
        return None;
    }
    let start = line_content_end(source, line_offsets, head_row);
    let end = line_content_end(source, line_offsets, last_hidden_row);
    if end <= start {
        return None;
    }
    Some(ByteRange { start, end })
}

/// Resolve the interior-defining body `B` and whether it is *introduced*
/// (its parent is an introducer whose body field is `B`). If `n` is itself
/// a body, use it; if it is an introducer with a block-like body child,
/// descend to that child (Q#FD1 step 2 — matching/`close-all` association).
fn resolve_body<'tree>(n: Node<'tree>, source: &[u8]) -> Option<(Node<'tree>, bool)> {
    if is_body_kind(n, source) {
        return Some((n, is_introduced(n)));
    }
    if let Some(b) = body_child(n)
        && is_body_kind(b, source)
    {
        return Some((b, true));
    }
    None
}

fn body_child(n: Node) -> Option<Node> {
    n.child_by_field_name("body")
        .or_else(|| n.child_by_field_name("consequence"))
}

fn is_introduced(n: Node<'_>) -> bool {
    if let Some(p) = n.parent()
        && let Some(b) = body_child(p)
    {
        return b.id() == n.id();
    }
    false
}

/// A node is a fold *body* if it opens with a bracket delimiter (a brace
/// body) or is a grammar block node (an indentation body). The delimiter
/// probe generalizes across grammars without a per-language kind list.
fn is_body_kind(n: Node<'_>, source: &[u8]) -> bool {
    let sb = n.start_byte();
    if sb < source.len() && OPEN_DELIMS.contains(&source[sb]) {
        return true;
    }
    matches!(
        n.kind(),
        "block"
            | "statement_block"
            | "declaration_list"
            | "field_declaration_list"
            | "enum_variant_list"
            | "block_mapping"
            | "block_sequence"
    ) || n.kind().ends_with("_body")
}

// ---------------------------------------------------------------------------
// State-aware operations (Q#FD4 shared-head ordering). Pure over a store
// (+ parse bundle); the Lua bindings drive them and move the point.
// ---------------------------------------------------------------------------

/// Close the innermost still-open foldable region at `p`; returns the newly
/// folded range. Repeated calls walk outward.
pub fn close_at(store: &mut FoldStore, bundle: &ParseTreeBundle, p: u64) -> Option<ByteRange> {
    for c in candidates_at(bundle, p) {
        if !store.contains_exact(c) {
            store.insert(c);
            return Some(c);
        }
    }
    None
}

/// Open the outermost currently-closed fold at `p`; returns the removed
/// range. Repeated calls walk inward.
pub fn open_at(store: &mut FoldStore, p: u64) -> Option<ByteRange> {
    let mut containing = store.containing(p);
    // `containing` is innermost-first; the outermost has the smallest start.
    containing.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let outer = containing.into_iter().next()?;
    store.remove(outer);
    Some(outer)
}

/// The result of an org-TAB-style toggle cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleOutcome {
    /// Closed one more (innermost open) fold on the head.
    Closed(ByteRange),
    /// Every fold on the head was already closed; opened them all.
    OpenedAll(usize),
    /// Nothing foldable and nothing folded at the point.
    Nothing,
}

/// `fold.toggle`: org-TAB cycle. While any foldable region at `p` is open,
/// close the innermost open one; once all are closed, one more press opens
/// them all. Every press has a visible effect (Q#FD4/R3-2).
pub fn cycle_at(store: &mut FoldStore, bundle: &ParseTreeBundle, p: u64) -> CycleOutcome {
    let candidates = candidates_at(bundle, p);
    if candidates.iter().any(|c| !store.contains_exact(*c)) {
        for c in &candidates {
            if !store.contains_exact(*c) {
                store.insert(*c);
                return CycleOutcome::Closed(*c);
            }
        }
    }
    let n = store.unfold_containing(p);
    if n > 0 {
        CycleOutcome::OpenedAll(n)
    } else {
        CycleOutcome::Nothing
    }
}

/// The result of a data-API `toggle(buffer, pos)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleOutcome {
    /// Folded the innermost tree target at the point.
    Folded(ByteRange),
    /// Unfolded the stored fold(s) containing the point.
    Unfolded(usize),
    /// Nothing foldable and nothing folded at the point.
    Nothing,
}

/// Data-API `toggle`: unfold if a stored fold contains `pos`, else fold the
/// innermost tree target at `pos`.
pub fn toggle_at(store: &mut FoldStore, bundle: &ParseTreeBundle, p: u64) -> ToggleOutcome {
    if !store.containing(p).is_empty() {
        return ToggleOutcome::Unfolded(store.unfold_containing(p));
    }
    match fold_target_at(bundle, p) {
        Some(t) => {
            store.insert(t);
            ToggleOutcome::Folded(t)
        }
        None => ToggleOutcome::Nothing,
    }
}

/// Normalize an arbitrary data-API range (no node, so no introducer/closer
/// inference — the caller names exactly what to hide). Head line = the line
/// containing `range.start`; the hidden lines are the full lines strictly
/// after it through the line containing `range.end` (or the previous line
/// when `range.end` sits at a line start). `None` if that is < 1 hidden
/// line.
#[must_use]
pub fn normalize_arbitrary_range(source: &[u8], range: ByteRange) -> Option<ByteRange> {
    if range.start > source.len() as u64 || range.end > source.len() as u64 {
        return None;
    }
    let line_offsets = compute_line_offsets(source);
    let head_row = line_at_offset(&line_offsets, range.start as u32);
    let end_row_raw = line_at_offset(&line_offsets, range.end as u32);
    let end_at_line_start = line_offsets.get(end_row_raw).copied() == Some(range.end as u32);
    let last_hidden_row = if end_at_line_start && end_row_raw > 0 {
        end_row_raw - 1
    } else {
        end_row_raw
    };
    if last_hidden_row < head_row + 1 {
        return None;
    }
    let start = line_content_end(source, &line_offsets, head_row);
    let end = line_content_end(source, &line_offsets, last_hidden_row);
    if end <= start {
        return None;
    }
    Some(ByteRange { start, end })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rope::{Edit, Range, Rope};

    fn edit(range: Range, inserted_len: u64) -> Edit {
        Edit {
            new_rope: Rope::new(),
            range,
            inserted_len,
            crdt_op: None,
        }
    }

    fn r(start: u64, end: u64) -> ByteRange {
        ByteRange { start, end }
    }

    #[test]
    fn insert_at_head_boundary_shifts_fold_right() {
        // `(start, end]` containment: an insert exactly at the end of the
        // head line lands *before* the fold, shifting it right so the
        // character stays visible on the head line.
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(10, 10), 1));
        assert_eq!(store.folds(), vec![r(11, 31)]);
    }

    #[test]
    fn insert_strictly_inside_grows_the_end() {
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(20, 20), 3));
        assert_eq!(store.folds(), vec![r(10, 33)]);
    }

    #[test]
    fn insert_at_tail_boundary_leaves_fold_untouched() {
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(30, 30), 4));
        assert_eq!(store.folds(), vec![r(10, 30)]);
    }

    #[test]
    fn edit_before_fold_shifts_whole_range() {
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(2, 5), 0)); // delete 3 bytes before
        assert_eq!(store.folds(), vec![r(7, 27)]);
    }

    #[test]
    fn edit_crossing_head_boundary_drops_fold() {
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        // A delete starting at the head boundary destroys the head.
        store.translate(&edit(Range::new(10, 15), 0));
        assert!(store.is_empty());
    }

    #[test]
    fn edit_crossing_tail_boundary_drops_fold() {
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(25, 40), 0));
        assert!(store.is_empty());
    }

    #[test]
    fn delete_starting_at_tail_boundary_drops_fold() {
        // A delete beginning exactly at `end` removes the `\n` that `end`
        // names (the last hidden line's terminator), destroying the tail —
        // it must drop, not survive with a mid-line end. Mirror of
        // `insert_at_tail_boundary_leaves_fold_untouched` for a delete.
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(30, 31), 0)); // delete one byte at `end`
        assert!(store.is_empty());

        // Same class: a delete starting at the boundary and extending past.
        let mut store = FoldStore::new();
        store.insert(r(10, 30));
        store.translate(&edit(Range::new(30, 45), 0));
        assert!(store.is_empty());
    }

    #[test]
    fn containment_is_start_exclusive_end_inclusive() {
        let store = {
            let mut s = FoldStore::new();
            s.insert(r(10, 30));
            s
        };
        assert!(store.containing(10).is_empty(), "start is exclusive");
        assert_eq!(store.containing(11), vec![r(10, 30)]);
        assert_eq!(store.containing(30), vec![r(10, 30)], "end is inclusive");
        assert!(store.containing(31).is_empty());
    }

    #[test]
    fn containing_is_innermost_first() {
        let mut store = FoldStore::new();
        store.insert(r(10, 100)); // outer
        store.insert(r(20, 60)); // inner
        assert_eq!(store.containing(30), vec![r(20, 60), r(10, 100)]);
    }

    #[test]
    fn unfold_containing_removes_all_nested() {
        let mut store = FoldStore::new();
        store.insert(r(10, 100));
        store.insert(r(20, 60));
        store.insert(r(200, 300)); // unrelated
        assert_eq!(store.unfold_containing(30), 2);
        assert_eq!(store.folds(), vec![r(200, 300)]);
    }

    #[test]
    fn insert_rejects_empty_and_duplicate() {
        let mut store = FoldStore::new();
        assert!(store.insert(r(10, 30)));
        assert!(!store.insert(r(10, 30)), "duplicate rejected");
        assert!(!store.insert(r(5, 5)), "empty rejected");
        assert!(!store.insert(r(9, 8)), "inverted rejected");
    }

    #[test]
    fn open_at_takes_outermost_closed() {
        let mut store = FoldStore::new();
        store.insert(r(10, 100));
        store.insert(r(20, 60));
        assert_eq!(open_at(&mut store, 30), Some(r(10, 100)));
        assert_eq!(open_at(&mut store, 30), Some(r(20, 60)));
        assert_eq!(open_at(&mut store, 30), None);
    }

    #[test]
    fn line_content_end_excludes_newline() {
        let src = b"abc\ndef\nghi";
        let off = compute_line_offsets(src);
        assert_eq!(line_content_end(src, &off, 0), 3); // "abc"
        assert_eq!(line_content_end(src, &off, 1), 7); // "def"
        assert_eq!(line_content_end(src, &off, 2), 11); // "ghi" (no newline)
    }

    #[test]
    fn normalize_arbitrary_range_basic() {
        //          0123 4567 89012
        let src = b"aaa\nbbb\nccc\nddd";
        // range covering into line 1 and line 2 -> hidden lines 1..2
        let out = normalize_arbitrary_range(src, r(1, 9)).expect("foldable");
        assert_eq!(out, r(3, 11)); // [end of line0, end of line2]
    }

    #[test]
    fn normalize_arbitrary_range_end_at_line_start_drops_a_line() {
        let src = b"aaa\nbbb\nccc\nddd";
        // end exactly at start of line 2 (byte 8) -> last hidden line is 1.
        let out = normalize_arbitrary_range(src, r(1, 8)).expect("foldable");
        assert_eq!(out, r(3, 7));
    }

    #[test]
    fn normalize_arbitrary_range_rejects_sub_one_line() {
        let src = b"aaa\nbbb\nccc";
        // start and end on the same line -> zero hidden lines.
        assert!(normalize_arbitrary_range(src, r(1, 2)).is_none());
    }
}
