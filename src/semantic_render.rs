// semantic_render.rs --- Instance-side semantic projection (T M11.2).

//! The semantic projection seam.
//!
//! [`crate::instance_render::RenderState`] rasterizes the editor to a
//! cell grid and ships [`InstanceMessage::CellDelta`]. `SemanticRenderState`
//! is its sibling for `semantic_render` sessions: it reads the same
//! [`EditorState`] but exits the pipeline *earlier* — it emits the
//! structured byte-range styling the cell painter would otherwise have
//! consumed, mapped through the active [`crate::highlight::Theme`],
//! without the grid-packing step. Styling has one authority per
//! language (policy A): tree-sitter spans from [`crate::syntax`] for
//! grammar-backed languages, LSP semantic tokens
//! ([`crate::lsp::LspManager::semantic_style_context`]) for languages
//! with no bundled grammar (C/C++, …). The frontend lays the styling
//! out locally over rope text it already holds via its `crdt_replica`
//! `BufferMirror`.
//!
//! Contract boundary (see `docs/semantic-frontend-protocol.md`): the
//! instance never learns a pixel. The only spatial fact it consumes is
//! the buffer byte range the frontend declared on screen via
//! [`crate::protocol::FrontendEvent::Viewport`]; styling is scoped to
//! that range so a 100k-line file's styling is never shipped wholesale.
//!
//! Produced families: `StyleSpans` (M11.2; dual authority per above)
//! and `Decorations` (M11.3), both span-granularity diffed (M11.4);
//! `InlineAdornments` (Step 3, from the LSP inlay-hint store,
//! M11.2-level suppression); `FileStyleSummary` (resolving Open Q#2 —
//! per-line dominant style for a minimap, generation-keyed).
//! `BlockAdornments` / `FoldState` / `ResourceOffer` remain wire-
//! declared but unproduced.

use std::collections::HashMap;

use crate::buffer::BufferId;
use crate::cell::Style;
use crate::editor::EditorState;
use crate::protocol::{
    AdornmentContent, AdornmentPlacement, ByteRange, Decoration, DecorationKind, DecorationSegment,
    FrontendId, InlineAdornment, InstanceMessage, StyleSegment, StyleSpan,
};

/// The viewport a `semantic_render` frontend last declared.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DeclaredViewport {
    buffer_id: BufferId,
    visible: ByteRange,
    /// The CRDT generation the frontend computed `visible` against.
    /// Recorded for the M11.4 "ignore a viewport that races a
    /// not-yet-applied edit" refinement; M11.2 always honors the most
    /// recent declaration verbatim.
    frontend_generation: u64,
}

/// The diff baseline for one family on one buffer: the
/// declared-viewport region the set was computed for, the full
/// scoped item set last shipped, and the CRDT generation that set
/// was computed against. The next frame diffs against `items`;
/// `visible` changing (or no entry) forces a `full` resync, and
/// `generation` changing also forces a `full` — see T M11.7.
///
/// **T M11.7 — `generation`-tracked full-resync.** Without this,
/// edits broke the consumer's incremental-update contract:
/// `changed_intervals` only ships dirty-range items on `full=false`
/// frames, but after a text-shift the frontend's cached spans
/// (indexed by *pre-edit* byte positions) need to be replaced
/// wholesale — the post-edit positions have shifted under them.
/// Forcing `full=true` on every generation transition makes the
/// next emission a `replace_*` on the frontend side, which is the
/// correct behavior. Cost: one extra full-viewport ship per edit;
/// negligible on a local Unix socket and bounded by the viewport
/// size.
struct LastFrame<T> {
    visible: ByteRange,
    items: Vec<T>,
    generation: u64,
}

/// Owns one `semantic_render` session's projection state: the last
/// viewport the frontend declared, and the diff baseline per buffer
/// for the `StyleSpans` and `Decorations` families.
pub struct SemanticRenderState {
    /// The session this projection serves. Selection is per-window
    /// (per-frontend) state, so the decoration projection needs the
    /// fid to resolve *this* session's active window via
    /// `active_window_for`. Styling and diagnostics are per-buffer and
    /// do not consult it.
    frontend_id: FrontendId,
    /// `None` until the frontend's first [`Self::set_viewport`]. While
    /// `None`, [`Self::render_frame`] emits nothing: the frontend
    /// bootstraps its rope from `BufferSnapshot`, declares what is on
    /// screen, and only then receives styling for exactly that range.
    viewport: Option<DeclaredViewport>,
    /// Styling diff baseline, keyed by buffer (T M11.4). An unchanged
    /// frame ships nothing; a changed frame ships only the dirty
    /// byte-range segments.
    last_sent: HashMap<BufferId, LastFrame<StyleSpan>>,
    /// Decorations diff baseline, tracked independently of `last_sent`
    /// so a styling change does not force a decorations re-send and
    /// vice versa.
    last_decorations: HashMap<BufferId, LastFrame<Decoration>>,
    /// `InlineAdornments` baseline (T M11 producer arc, Step 3). The
    /// wire variant carries no `generation`/`full`/`segments`, so
    /// unlike the two families above this is only M11.2-level
    /// suppression: a whole-set re-send on any change, nothing when
    /// byte-identical. `LastFrame::items` reuse keeps the shape
    /// uniform even though no segment diffing applies.
    last_adornments: HashMap<BufferId, LastFrame<InlineAdornment>>,
    /// `FileStyleSummary` baseline (post-M11 minimap producer,
    /// resolving design-note Open Q#2). The whole-file dominant-style
    /// summary is expensive to compute on a 100k-line file, so the
    /// producer short-circuits on the last sent CRDT generation: a
    /// buffer at the same generation re-uses what the frontend
    /// already has and emits nothing. First emission happens on the
    /// first frame for a buffer; further emissions only after edits.
    /// `(crdt_generation, diag_epoch)` the last emitted summary was
    /// computed against. Diagnostics arrive without a generation
    /// bump, so the epoch half catches republishes (minimap marks,
    /// T M4.6 GPU parity).
    last_summary: HashMap<BufferId, (u64, u64)>,
    /// `(name, modified, diag_errors, diag_warnings)` last emitted as
    /// `StatusFacts` (Q#S1) — cached-compare suppression.
    last_status: HashMap<BufferId, (String, bool, u32, u32)>,
    /// `StyleSpans` recompute gate (perf). `scoped_style_spans` runs
    /// the tree-sitter highlights query over the *whole declared
    /// viewport* (which the GPU frontend sets to the entire buffer)
    /// and clones the theme — too expensive to repeat on every tick.
    /// The styling depends only on the parse bundle, the CRDT
    /// generation, and the viewport — never the cursor — so a gate
    /// built from those lets cursor-only ticks skip the query entirely.
    /// Only the grammar (tree-sitter) path is gated; the LSP-token path
    /// has no comparably cheap handle and recomputes as before.
    last_style_gate: HashMap<BufferId, StyleGate>,
    /// Cached byte↔line table for the diagnostics projection, keyed
    /// by buffer revision. Building it costs an O(buffer) rope copy
    /// plus a full scan; before this cache, that ran on *every tick*
    /// while diagnostics were on screen (the table is only consulted
    /// when the store is non-stale and non-empty) — a steady-state
    /// CPU burn for a value that changes only when the buffer does.
    diag_line_cache: HashMap<BufferId, DiagLineCache>,
}

/// One [`SemanticRenderState::diag_line_cache`] entry: the line-start
/// offsets and source length of a buffer at `revision`.
struct DiagLineCache {
    revision: u64,
    line_starts: Vec<u64>,
    source_len: u64,
}

/// Recompute gate for [`scoped_style_spans`] on a grammar-backed
/// buffer. Holds the current parse bundle `Arc` so its address stays
/// stable while cached — comparing by `Arc::ptr_eq` then can't be
/// fooled by a freed bundle's address being reused (ABA). Equal gates
/// ⇒ identical spans ⇒ the tree-sitter query can be skipped.
/// `generation` is included so a CRDT edit still forces the M11.7
/// full-resync even when the parse bundle hasn't re-landed yet.
#[derive(Clone)]
struct StyleGate {
    /// Current parse bundle, or `None` when none has landed yet.
    bundle: Option<std::sync::Arc<crate::syntax::ParseTreeBundle>>,
    /// CRDT generation of the buffer.
    generation: u64,
    /// Declared viewport.
    visible: ByteRange,
}

impl StyleGate {
    /// True when both gates would produce identical style spans.
    fn matches(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.visible == other.visible
            && match (&self.bundle, &other.bundle) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl SemanticRenderState {
    /// Fresh session state for frontend `frontend_id`: no viewport
    /// declared, nothing sent.
    #[must_use]
    pub fn new(frontend_id: FrontendId) -> Self {
        Self {
            frontend_id,
            viewport: None,
            last_sent: HashMap::new(),
            last_decorations: HashMap::new(),
            last_adornments: HashMap::new(),
            last_summary: HashMap::new(),
            last_status: HashMap::new(),
            last_style_gate: HashMap::new(),
            diag_line_cache: HashMap::new(),
        }
    }

    /// Record the frontend's declared on-screen byte range. Called by
    /// the dispatcher when it receives
    /// [`crate::protocol::FrontendEvent::Viewport`]. Replaces any
    /// prior declaration wholesale — the latest viewport wins.
    pub fn set_viewport(&mut self, buffer_id: BufferId, visible: ByteRange, generation: u64) {
        self.viewport = Some(DeclaredViewport {
            buffer_id,
            visible,
            frontend_generation: generation,
        });
    }

    /// Project one frame.
    ///
    /// Returns up to three messages — [`InstanceMessage::StyleSpans`]
    /// (T M11.2), [`InstanceMessage::Decorations`] (T M11.3), and
    /// [`InstanceMessage::InlineAdornments`] (Step 3, from the LSP
    /// inlay-hint store) — each scoped to the declared viewport and
    /// each suppressed independently when byte-identical to its last
    /// send. Returns an empty vec before the frontend declares a
    /// viewport.
    ///
    /// `BlockAdornments` / `FoldState` are still deliberately *not*
    /// produced: pmacs has no instance-side blame / lens / fold / diff
    /// source yet. Their wire variants exist (T M11.1); their
    /// producers wire in when those features land — the same
    /// "declared, not yet wired" discipline. Emitting an empty message
    /// every frame would be waste, not honesty, so `InlineAdornments` is
    /// suppressed both when unchanged and when there is simply nothing
    /// to say (no hints, no prior non-empty send).
    #[allow(clippy::too_many_lines)]
    pub fn render_frame(&mut self, state: &EditorState) -> Vec<InstanceMessage> {
        let Some(vp) = self.viewport.clone() else {
            // Emit nothing before the frontend declares a viewport.
            return Vec::new();
        };

        let generation = buffer_generation(state, vp.buffer_id);
        let mut out = Vec::new();

        // --- StyleSpans (T M11.2 producer, T M11.4 diff) ---
        // Perf gate: `scoped_style_spans` runs the tree-sitter query
        // over the whole viewport + clones the theme. For a grammar-
        // backed buffer it's a pure function of (bundle revision,
        // generation, viewport), so a cursor-only tick — same key,
        // already-sent baseline — can skip the whole block. The LSP-
        // token path returns `None` (no cheap revision) and recomputes
        // every tick as before.
        let style_parse_not_ready = grammar_style_parse_not_ready(state, vp.buffer_id);
        // The LSP-token styling authority (grammar-less buffers, e.g.
        // C++) gets the same hold: while the semantic-token store is
        // stale (document edited since the last token response),
        // `lsp_scoped_style_spans` would compute an empty set, and
        // shipping that clears the frontend's colors for the whole
        // stale window — the styling twin of the diagnostics blink.
        let style_tokens_stale = lsp_style_tokens_stale(state, vp.buffer_id);
        let style_hold = style_parse_not_ready || style_tokens_stale;
        let style_gate = (!style_hold).then(|| grammar_style_key(state, &vp, generation));
        let style_gate = style_gate.flatten();
        let style_unchanged = match (&style_gate, self.last_style_gate.get(&vp.buffer_id)) {
            (Some(g), Some(prev)) => g.matches(prev) && self.last_sent.contains_key(&vp.buffer_id),
            _ => false,
        };
        if style_hold || style_unchanged {
            // If the style key is unchanged, styling cannot have
            // changed since the last computation. If a grammar parse is
            // still pending (or the LSP token store is stale), keep the
            // previous spans briefly rather than querying and reshaping
            // stale syntax on every typed byte; the parse-bundle
            // revision (or the next token response) will force a fresh
            // frame as soon as it settles.
        } else {
            match style_gate {
                Some(g) => {
                    self.last_style_gate.insert(vp.buffer_id, g);
                }
                None => {
                    self.last_style_gate.remove(&vp.buffer_id);
                }
            }
            self.emit_style_spans(state, &vp, generation, &mut out);
        }

        // --- Decorations (T M11.3 producer, T M11.4 diff) ---
        let mut decorations = self.scoped_decorations(state, &vp);
        let prev = self.last_decorations.get(&vp.buffer_id);
        // Hold-while-stale, part 2 (selection navigation): while the
        // diag store is stale, CARRY the previously shipped diagnostic
        // items through this frame's set instead of dropping them.
        // A shift+arrow during the post-burst stale window then diffs
        // as a tiny selection-only segment (the carried diag ranges
        // are unchanged, so they fall outside the changed intervals
        // and are never re-shipped at stale positions) — instead of
        // a full frame per keypress that also blinked the frontend's
        // held diagnostics out.
        let diag_hold = diagnostics_store_stale(state, vp.buffer_id);
        if diag_hold && let Some(p) = prev {
            decorations.extend(
                p.items
                    .iter()
                    .filter(|d| is_diagnostic_kind(d.kind))
                    .cloned(),
            );
            decorations.sort_by_key(|d| d.range.start);
        }
        // Hold-while-stale: while the diag store is stale (document
        // edited since the last `publishDiagnostics`), this frame has
        // no authoritative diagnostic positions. The frontend's
        // last-received set — which it translates through its own
        // local edits — is strictly better than anything we can ship:
        // an empty frame wipes it (diagnostics blink out on the first
        // keystroke of every burst and back in after the next publish,
        // one full frontend reshape each way), and re-shipping the
        // store's items would anchor pre-edit positions over post-edit
        // text (the M11.8 artifact). So as long as the
        // *non-diagnostic* part is unchanged, say nothing and leave
        // the baseline untouched — staleness clears on the next
        // publishDiagnostics absorption, and the generation transition
        // since the held baseline forces that frame full.
        let held = diag_hold
            && prev
                .is_some_and(|p| p.visible == vp.visible && decorations.iter().eq(p.items.iter()));
        // During a hold, only the GENERATION trigger for a full frame
        // is suppressed (edits bump it every keystroke; the carried
        // set diffs instead). First-ever frames and viewport changes
        // still resync in full.
        let full = prev
            .is_none_or(|p| p.visible != vp.visible || (!diag_hold && p.generation != generation));
        if held {
            // No new information for the frontend this frame. Keep
            // the baseline's generation current so the eventual
            // unstale frame diffs instead of full-resyncing (the diff
            // covers the diagnostics' post-publish positions).
            if let Some(p) = self.last_decorations.get_mut(&vp.buffer_id) {
                p.generation = generation;
            }
        } else if full {
            let suppress_empty_generation_bump = prev.is_some_and(|p| {
                p.visible == vp.visible && p.items.is_empty() && decorations.is_empty()
            });
            self.last_decorations.insert(
                vp.buffer_id,
                LastFrame {
                    visible: vp.visible,
                    items: decorations.clone(),
                    generation,
                },
            );
            if !suppress_empty_generation_bump {
                out.push(InstanceMessage::Decorations {
                    buffer_id: vp.buffer_id,
                    generation,
                    full: true,
                    segments: vec![DecorationSegment {
                        range: vp.visible,
                        decorations,
                    }],
                });
            }
        } else {
            let prev = prev.expect("checked is_none_or above");
            let intervals = changed_intervals(&prev.items, &decorations, |d| d.range);
            if !intervals.is_empty() {
                let segments = intervals
                    .into_iter()
                    .map(|range| DecorationSegment {
                        range,
                        decorations: clip_decorations(range, &decorations),
                    })
                    .collect();
                self.last_decorations.insert(
                    vp.buffer_id,
                    LastFrame {
                        visible: vp.visible,
                        items: decorations,
                        generation,
                    },
                );
                out.push(InstanceMessage::Decorations {
                    buffer_id: vp.buffer_id,
                    generation,
                    full: false,
                    segments,
                });
            }
        }

        // --- InlineAdornments (Step 3 producer) ---
        out.extend(self.inline_adornments_msg(state, &vp));
        // --- FileStyleSummary (minimap producer; Open Q#2) ---
        out.extend(self.file_style_summary_msg(state, vp.buffer_id, generation));
        // --- StatusFacts (status band; Q#S1, protocol v8) ---
        out.extend(self.status_facts_msg(state, vp.buffer_id));
        out
    }

    /// The `StatusFacts` message for this frame, or `None` when
    /// nothing changed. Carries the facts a semantic frontend cannot
    /// derive locally: buffer name, modified flag, whole-file
    /// diagnostic counts (errors / warnings). Counts freeze at their
    /// last value while the diag store is stale — mid-edit positions
    /// are wrong but *counts* merely lag, and flickering to zero on
    /// every keystroke would be worse. The daemon's write loop keeps
    /// the variant off wires negotiated `< 8`.
    fn status_facts_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        let (name, modified) = {
            let core = state.core.borrow();
            let registry = core.registry.clone();
            let reg = registry.borrow();
            let buf = reg.get(buffer_id).ok()?;
            (buf.name().to_owned(), buf.is_modified())
        };
        let counts = {
            let core = state.core.borrow();
            buffer_file_uri(&core, buffer_id).and_then(|uri| {
                let store = state.lsp_manager.borrow().diag_store();
                let guard = store.lock().expect("diag store mutex poisoned");
                if guard.is_stale(&uri) {
                    None // keep the cached counts
                } else {
                    let mut errors = 0u32;
                    let mut warnings = 0u32;
                    for d in guard.for_uri(&uri) {
                        match d.severity {
                            crate::diag::DiagnosticSeverity::Error => errors += 1,
                            crate::diag::DiagnosticSeverity::Warning => warnings += 1,
                            _ => {}
                        }
                    }
                    Some((errors, warnings))
                }
            })
        };
        let cached = self.last_status.get(&buffer_id);
        let (diag_errors, diag_warnings) =
            counts.unwrap_or_else(|| cached.map_or((0, 0), |c| (c.2, c.3)));
        let facts = (name, modified, diag_errors, diag_warnings);
        if cached == Some(&facts) {
            return None;
        }
        let msg = InstanceMessage::StatusFacts {
            buffer_id,
            name: facts.0.clone(),
            modified: facts.1,
            diag_errors,
            diag_warnings,
        };
        self.last_status.insert(buffer_id, facts);
        Some(msg)
    }

    /// The `InlineAdornments` message for this frame, or `None` when
    /// nothing should be sent. The wire variant has no
    /// `generation`/`full`/`segments`, so this is M11.2-level
    /// suppression only: the whole scoped set re-sends on any change,
    /// nothing when byte-identical, and never an empty frame when
    /// there is simply nothing to say. Updates the baseline on send.
    fn inline_adornments_msg(
        &mut self,
        state: &EditorState,
        vp: &DeclaredViewport,
    ) -> Option<InstanceMessage> {
        // Hold-while-stale — mirrors the Decorations hold in
        // `render_frame`. An empty frame here wipes the frontend's
        // cached virtual text mid-typing-burst, and inline adornments
        // occupy layout space: the wipe visibly shifts real glyphs
        // (and forces a reshape), then the post-refresh re-emit
        // shifts them back. The frontend's locally-translated cache
        // is the better picture until a fresh `inlayHint` response
        // clears the stale flag and re-emits through the diff below.
        if inlay_store_stale(state, vp.buffer_id) {
            return None;
        }
        let adornments = scoped_inline_adornments(state, vp);
        let should_emit = match self.last_adornments.get(&vp.buffer_id) {
            // First sight of this buffer: speak only if there is
            // something to show — no empty-frame spam.
            None => !adornments.is_empty(),
            // Re-send on a real change. `empty → empty` conveys
            // nothing and is suppressed; `non-empty → empty` *is* a
            // change worth sending so the frontend clears its overlay.
            Some(p) => {
                (p.visible != vp.visible || p.items != adornments)
                    && !(adornments.is_empty() && p.items.is_empty())
            }
        };
        if !should_emit {
            return None;
        }
        // Adornments use the same `LastFrame` struct as
        // StyleSpans / Decorations, so we populate `generation` for
        // consistency. Adornments' diff predicate doesn't consult
        // it — the whole-set comparison at line 310 catches post-
        // edit position shifts directly — but tracking it keeps
        // the struct shape uniform.
        let generation = buffer_generation(state, vp.buffer_id);
        self.last_adornments.insert(
            vp.buffer_id,
            LastFrame {
                visible: vp.visible,
                items: adornments.clone(),
                generation,
            },
        );
        Some(InstanceMessage::InlineAdornments {
            buffer_id: vp.buffer_id,
            items: adornments,
        })
    }

    /// The `FileStyleSummary` message for this frame, or `None`. The
    /// summary is keyed on CRDT `generation`: a buffer with an
    /// unchanged generation re-uses the cached summary and emits
    /// nothing. The first frame for a buffer always emits (the
    /// frontend needs the baseline). Updates the baseline on send.
    fn file_style_summary_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
        generation: u64,
    ) -> Option<InstanceMessage> {
        // The summary is a *whole-file* tree-sitter pass (the minimap
        // needs every line). Recomputing it on every edit's generation
        // bump was a per-keystroke O(file) cost — a major part of the
        // typing slowness. For grammar-backed buffers, debounce it to
        // reparse-completion. `pending_edit_count()` alone is not
        // enough: dispatch drains that list immediately, leaving the
        // expensive summary path free to run while a parse job is still
        // in flight. Wait until there is an installed parse, no pending
        // edits, and no recorded parse job for this buffer.
        if grammar_style_parse_not_ready(state, buffer_id) {
            return None;
        }
        // Diagnostics fold into the summary (minimap marks) but
        // publish without a generation bump — key the cache on the
        // diag store's per-URI epoch as well, so a republish
        // refreshes the marks and anything else stays suppressed.
        let diag_epoch = diagnostics_epoch(state, buffer_id);
        if self.last_summary.get(&buffer_id).copied() == Some((generation, diag_epoch)) {
            return None;
        }
        let lines = scoped_file_summary(state, buffer_id);
        self.last_summary
            .insert(buffer_id, (generation, diag_epoch));
        Some(InstanceMessage::FileStyleSummary {
            buffer_id,
            generation,
            lines,
        })
    }

    /// Project the [`Decoration`] set intersecting the declared
    /// viewport: the session's selection (instance-authoritative,
    /// byte-native) and LSP diagnostics (line/col → byte, severity →
    /// kind). Search-hit and current-line decorations are
    /// deliberately absent: pmacs has no instance-side search-hit
    /// store, and current-line is a pure cursor derivation the
    /// frontend already owns (it has `CursorByte`) — emitting it would
    /// couple a visual-motion concern to the instance, against the
    /// contract boundary.
    /// Compute the scoped style spans and push a `StyleSpans` message
    /// (full resync or M11.4 incremental) when they differ from the
    /// last sent baseline. Extracted from `render_frame` so the perf
    /// gate there can skip it wholesale on unchanged ticks.
    fn emit_style_spans(
        &mut self,
        state: &EditorState,
        vp: &DeclaredViewport,
        generation: u64,
        out: &mut Vec<InstanceMessage>,
    ) {
        let spans = scoped_style_spans(state, vp);
        let prev = self.last_sent.get(&vp.buffer_id);
        // Resync when there is no baseline, the declared viewport
        // region moved (scoping window changed), OR the CRDT
        // generation advanced (T M11.7: text edits shift byte
        // positions, so prior spans are stale and incremental
        // updates can't restore the full viewport).
        let full = prev.is_none_or(|p| p.visible != vp.visible || p.generation != generation);
        if full {
            self.last_sent.insert(
                vp.buffer_id,
                LastFrame {
                    visible: vp.visible,
                    items: spans.clone(),
                    generation,
                },
            );
            out.push(InstanceMessage::StyleSpans {
                buffer_id: vp.buffer_id,
                generation,
                full: true,
                segments: vec![StyleSegment {
                    range: vp.visible,
                    spans,
                }],
            });
        } else {
            let prev = prev.expect("checked is_none_or above");
            let intervals = changed_intervals(&prev.items, &spans, |s| s.range);
            if !intervals.is_empty() {
                let segments = intervals
                    .into_iter()
                    .map(|range| StyleSegment {
                        range,
                        spans: clip_style_spans(range, &spans),
                    })
                    .collect();
                self.last_sent.insert(
                    vp.buffer_id,
                    LastFrame {
                        visible: vp.visible,
                        items: spans,
                        generation,
                    },
                );
                out.push(InstanceMessage::StyleSpans {
                    buffer_id: vp.buffer_id,
                    generation,
                    full: false,
                    segments,
                });
            }
            // No dirty interval → styling unchanged → emit nothing.
        }
    }

    fn scoped_decorations(
        &mut self,
        state: &EditorState,
        vp: &DeclaredViewport,
    ) -> Vec<Decoration> {
        let core = state.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        let mut out = Vec::new();

        // Selection is per-window (per-frontend) state. CurrentLine is
        // deliberately not emitted for semantic frontends: the GPU has
        // CursorByte and paints its own caret/current-line affordances.
        // Emitting CurrentLine here forced a whole-buffer line table on
        // every frame even though pmacs-gpu ignores its own current-line
        // wash.
        if let Some(win) = core.active_window_for(self.frontend_id)
            && win.buffer_id == vp.buffer_id
            && let Some((lo, hi)) = win.region()
            && let Some(range) = clip_to_viewport(lo, hi, vp)
        {
            out.push(Decoration {
                range,
                kind: DecorationKind::Selection,
            });
        }

        // Diagnostics — keyed in the shared store by the file URI the
        // Lua LSP glue opened the document under. The URI is derived
        // from `vp.buffer_id` (see [`buffer_file_uri`]'s docs for why
        // not from `core.active_buffer_path()`).
        //
        // T M11.8 — skip emission while the store is stale (the
        // document has been edited since the last `publishDiagnostics`
        // absorption). Without this, the producer ships diagnostics
        // whose byte positions point at pre-edit text — the visible
        // wrong-position color artifact session-5 validation surfaced.
        // The LSP layer's `did_change_full` marks the URI stale; the
        // next `publishDiagnostics` absorbs and clears the flag.
        if let Some(uri) = buffer_file_uri(&core, vp.buffer_id) {
            let (diags, is_stale) = {
                let store = state.lsp_manager.borrow().diag_store();
                let guard = store.lock().expect("diag store mutex poisoned");
                (guard.for_uri(&uri).to_vec(), guard.is_stale(&uri))
            };
            if !is_stale
                && !diags.is_empty()
                && let Ok(buf) = reg.get(vp.buffer_id)
            {
                // Byte<->line mapping, cached per buffer revision —
                // rebuilding it is an O(buffer) rope copy + scan, far
                // too expensive to repeat on every tick a diagnostic
                // is on screen.
                let cache = self
                    .diag_line_cache
                    .entry(vp.buffer_id)
                    .and_modify(|c| {
                        if c.revision != buf.revision() {
                            let s = buffer_source_bytes(buf);
                            c.revision = buf.revision();
                            c.line_starts = line_start_offsets(&s);
                            c.source_len = s.len() as u64;
                        }
                    })
                    .or_insert_with(|| {
                        let s = buffer_source_bytes(buf);
                        DiagLineCache {
                            revision: buf.revision(),
                            line_starts: line_start_offsets(&s),
                            source_len: s.len() as u64,
                        }
                    });
                let (line_starts, source_len) = (&cache.line_starts, cache.source_len);
                for d in &diags {
                    let lo = line_col_to_byte(line_starts, source_len, d.start_line, d.start_col);
                    let hi = line_col_to_byte(line_starts, source_len, d.end_line, d.end_col);
                    let (lo, hi) =
                        widen_zero_width_diag(lo, hi, d.start_line, line_starts, source_len);
                    if let Some(range) = clip_to_viewport(lo, hi, vp) {
                        out.push(Decoration {
                            range,
                            kind: severity_to_kind(d.severity),
                        });
                    }
                }
            }
        }

        out
    }
}

/// Project the LSP inlay-hint set intersecting the declared viewport
/// into [`InlineAdornment`]s. Mirrors the diagnostics half of
/// `scoped_decorations`: same path → URI → store (`for_uri`) lookup.
/// Step 0 established inlay-hint columns are already pmacs byte
/// offsets by the time they reach the store (the absorb path's
/// `inbound_converted` rewrites the `Position`-shaped
/// `InlayHint.position`), so `line_col_to_byte` — which treats the
/// column as a byte offset — is exact here, no per-server encoding
/// needed (unlike semantic-token styling). Takes no `self`: the
/// session id is irrelevant (inlay hints are per-buffer, not
/// per-window like selection), mirroring `scoped_style_spans`.
///
/// Every hint is `AtOffset` (inlay hints are inline by definition)
/// carrying `Text` with the (padding-applied) label and the default
/// style — the instance has no inlay-specific theme face yet, and a
/// fabricated one would be dishonest. Stale store entries are
/// suppressed the same way stale diagnostics / semantic tokens are:
/// a zero-width hint anchored to pre-edit text is still a byte range
/// bug, even though it has no source-byte width of its own.
fn scoped_inline_adornments(state: &EditorState, vp: &DeclaredViewport) -> Vec<InlineAdornment> {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, vp.buffer_id) else {
        return Vec::new();
    };
    let hints = {
        let store = state.lsp_manager.borrow().inlay_hint_store();
        let guard = store.lock().expect("inlay-hint store mutex poisoned");
        if guard.is_stale(&uri) {
            return Vec::new();
        }
        match guard.for_uri(&uri) {
            Some(resp) => resp.hints.clone(),
            None => return Vec::new(),
        }
    };
    if hints.is_empty() {
        return Vec::new();
    }
    let registry = core.registry.clone();
    let reg = registry.borrow();
    let Ok(buf) = reg.get(vp.buffer_id) else {
        return Vec::new();
    };
    let source = buffer_source_bytes(buf);
    let source_len = source.len() as u64;
    let line_starts = line_start_offsets(&source);
    let vis_start = vp.visible.start.min(source_len);
    let vis_end = vp.visible.end.min(source_len);

    let mut out = Vec::new();
    for h in &hints {
        let at = line_col_to_byte(&line_starts, source_len, h.line, h.col);
        // An inlay hint occupies no bytes; include it when its anchor
        // lies within the declared viewport (half-open).
        if at < vis_start || at >= vis_end {
            continue;
        }
        let mut text = String::new();
        if h.padding_left {
            text.push(' ');
        }
        text.push_str(&h.label);
        if h.padding_right {
            text.push(' ');
        }
        out.push(InlineAdornment {
            at,
            placement: AdornmentPlacement::AtOffset,
            content: AdornmentContent::Text {
                text,
                style: Style::default(),
            },
        });
    }
    out
}

/// Intersect `[lo, hi)` with the declared viewport (itself clamped to
/// the source length is the caller's concern for styling; for
/// decorations we clamp against the viewport only). `None` when the
/// intersection is empty or degenerate.
/// Widen a zero-width diagnostic range to one byte so it survives
/// the wire and overlaps a glyph at the frontend (a zero-width range
/// clips to nothing and underlines nothing). Parsers anchor
/// "expected COMMA"-style errors one past the last token —
/// rust-analyzer reports the missing comma as `col N → col N` at end
/// of line — the same shape the TUI's `DiagnosticView` special-cases
/// at its anchor cell (T M4.6). Mid-line anchors widen forward; an
/// anchor at/past the line's content end widens backward instead,
/// because forward would cover only the `\n`, which shapes no glyph.
/// Non-empty ranges pass through untouched.
fn widen_zero_width_diag(
    lo: u64,
    hi: u64,
    start_line: u32,
    line_starts: &[u64],
    source_len: u64,
) -> (u64, u64) {
    if hi > lo {
        return (lo, hi);
    }
    // Content end excludes the trailing newline, same semantics as
    // the summary's per-line ranges.
    let content_end = line_starts
        .get(start_line as usize + 1)
        .map_or(source_len, |&next| next.saturating_sub(1));
    if lo >= content_end {
        (lo.saturating_sub(1), lo)
    } else {
        (lo, (lo + 1).min(source_len))
    }
}

fn clip_to_viewport(lo: u64, hi: u64, vp: &DeclaredViewport) -> Option<ByteRange> {
    let start = lo.max(vp.visible.start);
    let end = hi.min(vp.visible.end);
    if end <= start {
        return None;
    }
    Some(ByteRange { start, end })
}

/// T M11.4 — the dirty byte intervals between two ordered item sets.
///
/// Items are byte-anchored (`range_of` extracts the range). The
/// symmetric difference (items in exactly one set, by `==`) bounds
/// every byte whose covering set changed; its ranges are coalesced
/// into maximal disjoint intervals — the segments the frontend will
/// clear and repaint. Empty result ⇒ unchanged ⇒ the caller emits
/// nothing.
///
/// O(n·m) membership scans: a screenful is a few hundred items, far
/// cheaper than re-shipping the whole viewport every frame, and only
/// runs when the fast `prev == curr` slice check (caller side, via
/// the order-stable producers) would have failed anyway.
fn changed_intervals<T: PartialEq>(
    prev: &[T],
    curr: &[T],
    range_of: impl Fn(&T) -> ByteRange,
) -> Vec<ByteRange> {
    let mut changed: Vec<ByteRange> = Vec::new();
    for p in prev {
        if !curr.contains(p) {
            changed.push(range_of(p));
        }
    }
    for c in curr {
        if !prev.contains(c) {
            changed.push(range_of(c));
        }
    }
    coalesce_ranges(&mut changed)
}

/// Sort and merge overlapping or touching ranges into maximal
/// disjoint intervals. Zero-width ranges are dropped (nothing to
/// repaint). Consumes `ranges` (sorts in place).
fn coalesce_ranges(ranges: &mut Vec<ByteRange>) -> Vec<ByteRange> {
    ranges.retain(|r| r.end > r.start);
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<ByteRange> = Vec::new();
    for r in ranges.iter().copied() {
        match out.last_mut() {
            // Touching (`>=`) merges too: adjacent dirty ranges become
            // one segment rather than two abutting clears.
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Every span intersecting `iv`, clipped to it, order preserved.
fn clip_style_spans(iv: ByteRange, spans: &[StyleSpan]) -> Vec<StyleSpan> {
    spans
        .iter()
        .filter_map(|s| {
            let start = s.range.start.max(iv.start);
            let end = s.range.end.min(iv.end);
            (end > start).then_some(StyleSpan {
                range: ByteRange { start, end },
                style: s.style,
            })
        })
        .collect()
}

/// Every decoration intersecting `iv`, clipped to it, order preserved.
fn clip_decorations(iv: ByteRange, decos: &[Decoration]) -> Vec<Decoration> {
    decos
        .iter()
        .filter_map(|d| {
            let start = d.range.start.max(iv.start);
            let end = d.range.end.min(iv.end);
            (end > start).then_some(Decoration {
                range: ByteRange { start, end },
                kind: d.kind,
            })
        })
        .collect()
}

/// True for the four diagnostic-underline decoration kinds — the
/// family whose emission is gated on diag-store staleness by the
/// hold-while-stale logic in `render_frame`.
fn is_diagnostic_kind(kind: DecorationKind) -> bool {
    matches!(
        kind,
        DecorationKind::DiagnosticError
            | DecorationKind::DiagnosticWarning
            | DecorationKind::DiagnosticInfo
            | DecorationKind::DiagnosticHint
    )
}

/// True when `buffer_id`'s entry in the diagnostics store is stale
/// (the document changed since the last `publishDiagnostics`
/// absorption). Buffers with no file URI are never stale.
fn diagnostics_store_stale(state: &EditorState, buffer_id: BufferId) -> bool {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return false;
    };
    let store = state.lsp_manager.borrow().diag_store();
    let guard = store.lock().expect("diag store mutex poisoned");
    guard.is_stale(&uri)
}

/// The diag store's per-URI change epoch for `buffer_id`'s file, `0`
/// for buffers with no file URI or no diagnostics history. Keys the
/// `FileStyleSummary` cache (see [`SemanticRenderState::last_summary`]).
fn diagnostics_epoch(state: &EditorState, buffer_id: BufferId) -> u64 {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return 0;
    };
    let store = state.lsp_manager.borrow().diag_store();
    let guard = store.lock().expect("diag store mutex poisoned");
    guard.epoch_for(&uri)
}

/// Style-family staleness for the LSP-token authority. True only for
/// a buffer with **no** tree-sitter view (policy A routes those
/// through `lsp_scoped_style_spans`) whose semantic-token store entry
/// is stale. Grammar-backed buffers always return `false` — their
/// styling freshness is `grammar_style_parse_not_ready`'s job.
fn lsp_style_tokens_stale(state: &EditorState, buffer_id: BufferId) -> bool {
    if state.syntax_registry.view(buffer_id).is_some() {
        return false;
    }
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return false;
    };
    let store = state.lsp_manager.borrow().semantic_token_store();
    let guard = store.lock().expect("semantic token store mutex poisoned");
    guard.is_stale(&uri)
}

/// Inlay-hint twin of [`diagnostics_store_stale`].
fn inlay_store_stale(state: &EditorState, buffer_id: BufferId) -> bool {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return false;
    };
    let store = state.lsp_manager.borrow().inlay_hint_store();
    let guard = store.lock().expect("inlay-hint store mutex poisoned");
    guard.is_stale(&uri)
}

/// Map an LSP diagnostic severity onto the wire decoration kind.
fn severity_to_kind(sev: crate::diag::DiagnosticSeverity) -> DecorationKind {
    use crate::diag::DiagnosticSeverity as S;
    match sev {
        S::Error => DecorationKind::DiagnosticError,
        S::Warning => DecorationKind::DiagnosticWarning,
        S::Information => DecorationKind::DiagnosticInfo,
        S::Hint => DecorationKind::DiagnosticHint,
    }
}

/// Resolve `buffer_id` → `file://…` URI, using the same encoding the
/// Lua side uses in `file_uri_for` so the result is byte-identical to
/// the diag-store / inlay-store / semantic-token-store keys.
///
/// **Bug-fix anchor (post-session-5 finding):** the producer used to
/// derive this URI from `core.active_buffer_path()` — the *editor's*
/// active buffer, not the buffer the frame is projecting. In multi-
/// frontend setups (TUI + `pmacs-gpu`) those diverge: each frontend
/// has its own active window, and when the daemon renders frontend B's
/// frame it temporarily flips `active_frontend` to B but B's active
/// buffer may be a scratch buffer with no file path. The diag /
/// inlay / semantic-token lookups would then run against the wrong
/// URI (or `None`) and return empty, so frontend B got no
/// `Decorations` / `InlineAdornments` / LSP-driven `StyleSpans`.
/// Routing the URI through `vp.buffer_id` fixes the multi-frontend
/// case without disturbing the single-frontend one
/// (`active_buffer_id == vp.buffer_id` there, so the resolved path is
/// the same).
fn buffer_file_uri(core: &crate::editor_core::EditorCore, buffer_id: BufferId) -> Option<String> {
    let reg = core.registry.borrow();
    let buf = reg.get(buffer_id).ok()?;
    let path = buf.file_path()?;
    Some(crate::lsp::path_to_file_uri(path))
}

/// Snapshot a buffer's bytes (refcount-cheap rope slice, mirroring
/// `diag.rs`'s render-time snapshot).
fn buffer_source_bytes(buf: &crate::buffer::Buffer) -> Vec<u8> {
    let len = buf.len();
    let mut bytes = vec![0u8; len as usize];
    if !bytes.is_empty() {
        buf.snapshot_rope().slice(0, len, &mut bytes);
    }
    bytes
}

/// Byte offset of the start of each line (index 0 = byte 0; one entry
/// per line, where a line is a maximal run ended by `\n`).
fn line_start_offsets(source: &[u8]) -> Vec<u64> {
    let mut starts = vec![0u64];
    for (i, b) in source.iter().enumerate() {
        if *b == b'\n' {
            starts.push(i as u64 + 1);
        }
    }
    starts
}

/// Translate an LSP `(line, col)` to a byte offset. pmacs v0.1 treats
/// the LSP column as a byte offset within the line (see
/// `crate::diag::Diagnostic`'s field docs); we clamp to the line's
/// end and the source length so a stale diagnostic from before an
/// edit can never index out of range.
fn line_col_to_byte(line_starts: &[u64], source_len: u64, line: u32, col: u32) -> u64 {
    let li = line as usize;
    let Some(&line_start) = line_starts.get(li) else {
        return source_len;
    };
    let line_end = line_starts
        .get(li + 1)
        .map_or(source_len, |&next| next.saturating_sub(1));
    (line_start + u64::from(col)).min(line_end).min(source_len)
}

/// Cheap recompute-gate key for [`scoped_style_spans`] on a grammar-
/// backed buffer. Returns `None` for buffers with no tree-sitter view
/// (the LSP-token path), which has no comparably cheap revision handle
/// and therefore is never gated. `bundle.source_revision` is read via
/// the same `current()` accessor `scoped_style_spans` uses, so the key
/// flips exactly when the spans it would produce can change.
fn grammar_style_key(
    state: &EditorState,
    vp: &DeclaredViewport,
    generation: u64,
) -> Option<StyleGate> {
    let handle = state.syntax_registry.view(vp.buffer_id)?;
    Some(StyleGate {
        bundle: handle.current(),
        generation,
        visible: vp.visible,
    })
}

fn grammar_style_parse_not_ready(state: &EditorState, buffer_id: BufferId) -> bool {
    let Some(handle) = state.syntax_registry.view(buffer_id) else {
        return false;
    };
    handle.current().is_none()
        || handle.pending_edit_count() > 0
        || state.syntax_registry.has_pending_parse_job_for(buffer_id)
}

/// Compute the styled byte runs intersecting the declared viewport,
/// mapped through the active theme. Spans are clipped to the viewport
/// and to the parsed source length; runs that resolve to the default
/// style are dropped (wire economy, and consistent with the grid
/// path, which skips default-style merges).
fn scoped_style_spans(state: &EditorState, vp: &DeclaredViewport) -> Vec<StyleSpan> {
    // Policy A — per-language styling authority. A grammar-backed
    // language (the registry hands out a view only for those) is
    // styled *solely* by tree-sitter; a language with no bundled
    // grammar (C/C++, …) is styled *solely* by LSP semantic tokens.
    // Never both: this is why the no-view branch hands off to the LSP
    // producer while a grammar-backed buffer whose parse isn't ready
    // yet returns empty rather than briefly borrowing LSP styling
    // (which would flicker two authorities on one buffer).
    let Some(handle) = state.syntax_registry.view(vp.buffer_id) else {
        return lsp_scoped_style_spans(state, vp);
    };
    let Some(bundle) = handle.current() else {
        return Vec::new();
    };
    let Some(query) = state
        .syntax_registry
        .highlights_query(&bundle.language_name)
    else {
        return Vec::new();
    };
    let theme = state
        .syntax_registry
        .theme()
        .lock()
        .expect("theme mutex poisoned")
        .clone();

    let source_len = bundle.source.len() as u64;
    let vis_start = vp.visible.start.min(source_len);
    let vis_end = vp.visible.end.min(source_len);
    if vis_end <= vis_start {
        return Vec::new();
    }

    let capture_names = query.capture_names();
    // Scope the tree-sitter capture walk to the visible byte range so
    // re-styling on each edit is O(visible), not O(file) — the typing
    // bottleneck on large files (framing Q#S6). Captures whose nodes
    // intersect the range are returned, then clipped exactly below.
    let highlights = crate::syntax::compute_highlight_spans_in_range(
        &query,
        &bundle,
        Some(vis_start as usize..vis_end as usize),
    );
    let mut out = Vec::new();
    for hs in highlights {
        let s = u64::from(hs.start_byte).max(vis_start);
        let e = u64::from(hs.end_byte).min(vis_end);
        if e <= s {
            continue; // No overlap with the viewport.
        }
        let Some(name) = capture_names.get(hs.capture_index as usize) else {
            continue;
        };
        let style = theme.lookup(name);
        if style == Style::default() {
            continue; // Nothing to render — skip the wire byte.
        }
        out.push(StyleSpan {
            range: ByteRange { start: s, end: e },
            style,
        });
    }
    out
}

/// Policy-A fallback for languages with no bundled tree-sitter
/// grammar (C/C++, …): project the LSP semantic-token store into the
/// same `StyleSpan` shape the tree-sitter path emits, so the existing
/// M11.4 diff pipeline (`render_frame`) consumes it unchanged and the
/// frontend never learns which producer fed it. The instance stays
/// the single styling authority.
///
/// `SemanticToken` `start`/`length` are LSP encoding units (UTF-16
/// for clangd's default) and — unlike inlay hints — are *not*
/// byte-rewritten upstream, so this converts them per line via the
/// owning server's negotiated encoding
/// ([`crate::lsp::LspManager::semantic_style_context`]). Tokens are
/// single-line by the LSP grammar, so per-line conversion is exact.
fn lsp_scoped_style_spans(state: &EditorState, vp: &DeclaredViewport) -> Vec<StyleSpan> {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, vp.buffer_id) else {
        return Vec::new();
    };

    // Styling context (encoding + legend) and the token set resolve
    // the *same* server for `uri` (both via `for_uri`'s lowest-id
    // rule), so they describe one coherent source.
    let mgr = state.lsp_manager.borrow();
    let Some(ctx) = mgr.semantic_style_context(&uri) else {
        return Vec::new();
    };
    let tokens = {
        let store = mgr.semantic_token_store();
        let guard = store.lock().expect("semantic-token store mutex poisoned");
        if guard.is_stale(&uri) {
            return Vec::new();
        }
        match guard.for_uri(&uri) {
            Some((_, resp)) => resp.tokens.clone(),
            None => return Vec::new(),
        }
    };
    drop(mgr);

    let registry = core.registry.clone();
    let reg = registry.borrow();
    let Ok(buf) = reg.get(vp.buffer_id) else {
        return Vec::new();
    };
    let source = buffer_source_bytes(buf);
    let source_len = source.len() as u64;
    let vis_start = vp.visible.start.min(source_len);
    let vis_end = vp.visible.end.min(source_len);
    if vis_end <= vis_start {
        return Vec::new();
    }
    let line_starts = line_start_offsets(&source);

    let theme = state
        .syntax_registry
        .theme()
        .lock()
        .expect("theme mutex poisoned")
        .clone();

    let mut out = Vec::new();
    for t in &tokens {
        let li = t.line as usize;
        let Some(&ls) = line_starts.get(li) else {
            continue; // Token line past EOF (stale response) — skip.
        };
        let le = line_starts
            .get(li + 1)
            .map_or(source_len, |&n| n.saturating_sub(1));
        let Ok(line_text) = std::str::from_utf8(&source[ls as usize..le as usize]) else {
            continue; // Non-UTF-8 line — cannot do encoded conversion.
        };
        let start_b = ls + crate::lsp::char_to_byte(line_text, t.start, ctx.encoding) as u64;
        let end_char = t.start.saturating_add(t.length);
        let end_b = ls + crate::lsp::char_to_byte(line_text, end_char, ctx.encoding) as u64;
        let s = start_b.max(vis_start);
        let e = end_b.min(vis_end);
        if e <= s {
            continue; // Empty, or no overlap with the viewport.
        }
        let Some(name) = ctx
            .legend
            .as_ref()
            .and_then(|lg| lg.type_name(t.token_type))
        else {
            continue; // No legend / unknown type ⇒ cannot name a style.
        };
        let style = theme.lookup(name);
        if style == Style::default() {
            continue; // Nothing to render — skip the wire byte (parity
            // with the tree-sitter path's default-style drop).
        }
        out.push(StyleSpan {
            range: ByteRange { start: s, end: e },
            style,
        });
    }
    out
}

/// Compute the per-line dominant style summary for the whole buffer:
/// one [`Style`] per source line, in line order. The "dominant" style
/// for a line is the one covering the most bytes among the styled
/// runs (`scoped_style_spans` for the full buffer); a line with no
/// styled runs takes [`Style::default`]. Reuses [`scoped_style_spans`]
/// so the policy-A authority choice (tree-sitter for grammar-backed
/// languages, LSP semantic tokens otherwise) is inherited automatically.
///
/// `O(spans × lines)` in the worst case; the caller short-circuits on
/// unchanged CRDT generation so this only runs on first sight of a
/// buffer or after an edit, not per frame.
fn scoped_file_summary(state: &EditorState, buffer_id: BufferId) -> Vec<Style> {
    let source = {
        let core = state.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        match reg.get(buffer_id) {
            Ok(buf) => buffer_source_bytes(buf),
            Err(_) => return Vec::new(),
        }
    };
    let source_len = source.len() as u64;
    let line_starts = line_start_offsets(&source);
    let line_count = line_starts.len();
    let mut out = vec![Style::default(); line_count];

    // Reuse the existing producer with a whole-buffer "viewport". The
    // clip is then a no-op and `scoped_style_spans` yields every
    // styled run; policy A's authority pick (tree-sitter / LSP) is
    // therefore identical to the per-frame styling.
    let vp_all = DeclaredViewport {
        buffer_id,
        visible: ByteRange {
            start: 0,
            end: source_len,
        },
        frontend_generation: 0,
    };
    let spans = scoped_style_spans(state, &vp_all);
    if spans.is_empty() {
        // No styled runs — but diagnostic marks are independent of
        // syntax styling (a plain-text buffer can still have lints).
        overlay_diagnostic_marks(state, buffer_id, &mut out);
        return out;
    }

    // Per-line dominant style: tally bytes per style and pick the
    // winner. `Style` doesn't implement `Hash`, so a small linear-scan
    // tally is fine — the number of distinct styles per line is
    // bounded by the theme's vocabulary (single digits in practice).
    for (li, line_dominant) in out.iter_mut().enumerate() {
        let line_start = line_starts[li];
        // Same trailing-newline semantics as `line_col_to_byte`: the
        // line's byte range excludes the `\n` (`next - 1`), and the
        // trailing line (no next entry) runs to EOF.
        let line_end = line_starts
            .get(li + 1)
            .map_or(source_len, |&next| next.saturating_sub(1));
        let mut tally: Vec<(Style, u64)> = Vec::new();
        for sp in &spans {
            let lo = sp.range.start.max(line_start);
            let hi = sp.range.end.min(line_end);
            if hi > lo {
                let bytes = hi - lo;
                if let Some(entry) = tally.iter_mut().find(|(s, _)| *s == sp.style) {
                    entry.1 += bytes;
                } else {
                    tally.push((sp.style, bytes));
                }
            }
        }
        if let Some(winner) = tally.into_iter().max_by_key(|(_, c)| *c) {
            *line_dominant = winner.0;
        }
    }
    overlay_diagnostic_marks(state, buffer_id, &mut out);
    out
}

/// Fold diagnostics into the file summary: each line a diagnostic
/// touches gets the most severe severity's canonical color in
/// `underline_color` (protocol v6) — the minimap's line marks, the
/// GPU's equivalent of the TUI's column-0 gutter signs (T M4.6).
/// Skipped while the URI's store entry is stale: the positions
/// describe pre-edit text, same discipline as the decorations
/// producer (the marks return on republish, which bumps the diag
/// epoch and recomputes this summary).
fn overlay_diagnostic_marks(state: &EditorState, buffer_id: BufferId, lines: &mut [Style]) {
    let uri = {
        let core = state.core.borrow();
        let Some(uri) = buffer_file_uri(&core, buffer_id) else {
            return;
        };
        uri
    };
    let store = state.lsp_manager.borrow().diag_store();
    let guard = store.lock().expect("diag store mutex poisoned");
    if guard.is_stale(&uri) {
        return;
    }
    // Most severe per line wins; LSP numbering makes that the
    // minimum severity value.
    let mut best: Vec<Option<crate::diag::DiagnosticSeverity>> = vec![None; lines.len()];
    for d in guard.for_uri(&uri) {
        for li in d.start_line..=d.end_line {
            let Some(slot) = best.get_mut(li as usize) else {
                break;
            };
            *slot = Some(slot.map_or(d.severity, |s| s.min(d.severity)));
        }
    }
    for (line, severity) in lines.iter_mut().zip(best) {
        if let Some(s) = severity {
            line.underline_color = s.underline_color();
        }
    }
}

/// The buffer's CRDT version projected to a monotonic scalar — the
/// `generation` anchor for the semantic frame. `0` when the buffer is
/// absent or not CRDT-backed (a `semantic_render` session always
/// negotiates `crdt_replica`, so in practice the buffer is CRDT-backed
/// before any semantic frame is produced; the fallback keeps this
/// total).
#[cfg(feature = "crdt")]
fn buffer_generation(state: &EditorState, buffer_id: BufferId) -> u64 {
    let core = state.core.borrow();
    let registry = core.registry.clone();
    let reg = registry.borrow();
    reg.get(buffer_id)
        .ok()
        .and_then(crate::buffer::Buffer::crdt_state)
        .map_or(0, crate::crdt::CrdtState::version_scalar)
}

/// Non-CRDT builds cannot host a semantic session (the negotiation
/// dependency rule requires `crdt_replica`, gated on the `crdt`
/// feature), so this is never reached with a live viewport; it exists
/// only to keep `render_frame` total across feature flavors.
#[cfg(not(feature = "crdt"))]
#[allow(clippy::missing_const_for_fn)]
fn buffer_generation(_state: &EditorState, _buffer_id: BufferId) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellSize;
    use crate::editor::EditorState;
    use crate::instance_render::RenderState;
    use crate::protocol::FrontendId;

    fn empty_state() -> EditorState {
        EditorState::new()
    }

    fn local() -> SemanticRenderState {
        // FrontendId::LOCAL always has a registered FrontendView
        // (EditorCore invariant), so `active_window_for(LOCAL)` — the
        // selection projection's lookup — resolves in a fresh editor.
        SemanticRenderState::new(FrontendId::LOCAL)
    }

    fn active_buffer(state: &EditorState) -> BufferId {
        state.core.borrow().active_window().buffer_id
    }

    /// All `InstanceMessage` variants the semantic projection may
    /// emit are `StyleSpans`, `Decorations`, `InlineAdornments`,
    /// `FileStyleSummary`, or `StatusFacts` (Q#S1) — never
    /// `CellDelta`, grid `Cursor`, or the still-unwired
    /// `BlockAdornments` / `FoldState` families.
    fn assert_semantic_only(msgs: &[InstanceMessage]) {
        for m in msgs {
            assert!(
                matches!(
                    m,
                    InstanceMessage::StyleSpans { .. }
                        | InstanceMessage::Decorations { .. }
                        | InstanceMessage::InlineAdornments { .. }
                        | InstanceMessage::FileStyleSummary { .. }
                        | InstanceMessage::StatusFacts { .. }
                ),
                "semantic projection emitted an unexpected variant: {m:?}"
            );
        }
    }

    /// Find the `Decorations` message and flatten its segments into
    /// `(full, all decorations across segments)`.
    fn decorations_of(msgs: &[InstanceMessage]) -> Option<(bool, Vec<Decoration>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::Decorations { full, segments, .. } => Some((
                *full,
                segments
                    .iter()
                    .flat_map(|s| s.decorations.clone())
                    .collect(),
            )),
            _ => None,
        })
    }

    /// Find the `StyleSpans` message: `(full, segment ranges)`.
    fn style_segments(msgs: &[InstanceMessage]) -> Option<(bool, Vec<ByteRange>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::StyleSpans { full, segments, .. } => {
                Some((*full, segments.iter().map(|s| s.range).collect()))
            }
            _ => None,
        })
    }

    fn set_selection(state: &EditorState, anchor: u64, cursor: u64) {
        let mut core = state.core.borrow_mut();
        let win = core
            .active_window_mut_for(FrontendId::LOCAL)
            .expect("LOCAL always has a window");
        win.selection = Some(crate::window::Selection { anchor });
        win.cursor = cursor;
    }

    fn seed_diagnostic(state: &EditorState, buffer_id: BufferId) {
        let mut core = state.core.borrow_mut();
        core.registry
            .clone()
            .borrow_mut()
            .get_mut(buffer_id)
            .expect("active buffer")
            .apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: b"abc\nde",
            })
            .expect("seed buffer text");
        core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/m114.rs")));
        drop(core);
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        let store = state.lsp_manager.borrow().diag_store();
        store.lock().expect("diag store").set(
            &uri,
            vec![crate::diag::Diagnostic {
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 2,
                severity: crate::diag::DiagnosticSeverity::Warning,
                message: "x".into(),
                source: None,
                code: None,
            }],
        );
    }

    #[test]
    fn emits_nothing_before_viewport_declared() {
        let mut s = local();
        assert!(
            s.render_frame(&empty_state()).is_empty(),
            "nothing may be emitted before the frontend declares a viewport"
        );
    }

    #[test]
    fn first_post_viewport_frame_is_full_for_both_then_suppresses() {
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(
            buffer_id,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        // Empty scratch: no spans, no selection, no diagnostics — but
        // the first frame is a `full` resync for both diffable families
        // (the frontend clears its viewport), carrying empty segments.
        // FileStyleSummary also emits on the first frame for this buffer
        // (post-M11 minimap producer, generation-keyed), as does
        // StatusFacts (Q#S1, cached-compare).
        let first = s.render_frame(&state);
        assert_eq!(
            first.len(),
            4,
            "first frame ships StyleSpans + Decorations + FileStyleSummary + StatusFacts"
        );
        assert_semantic_only(&first);
        let (style_full, _) = style_segments(&first).expect("StyleSpans present");
        let (deco_full, decos) = decorations_of(&first).expect("Decorations present");
        assert!(style_full, "first styling frame must be full");
        assert!(deco_full, "first decorations frame must be full");
        assert!(decos.is_empty(), "empty scratch has no decorations");

        // Nothing changed → both families suppressed.
        assert!(
            s.render_frame(&state).is_empty(),
            "an unchanged frame must be fully suppressed"
        );
    }

    #[test]
    fn selection_projects_as_a_decoration_clipped_to_viewport() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        // region (2,5) on LOCAL's window; region() compares offsets
        // only, so the empty scratch buffer is fine here.
        set_selection(&state, 2, 5);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 3, end: 64 }, 0);

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let (full, decos) = decorations_of(&msgs).expect("a Decorations message");
        assert!(full, "first frame is a full resync");
        assert_eq!(decos.len(), 1, "exactly the selection decoration");
        assert_eq!(decos[0].kind, DecorationKind::Selection);
        // region (2,5) clipped to viewport [3,64) → [3,5).
        assert_eq!(decos[0].range, ByteRange { start: 3, end: 5 });
    }

    #[test]
    fn semantic_projection_does_not_emit_current_line_decoration() {
        // CurrentLine is a frontend-local visual for semantic sessions;
        // the daemon should not copy the whole buffer to derive it.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos.iter().all(|d| d.kind != DecorationKind::CurrentLine),
            "semantic projection must not emit CurrentLine; got {decos:?}"
        );
    }

    #[test]
    fn semantic_current_line_absence_does_not_depend_on_active_buffer() {
        let state = empty_state();
        let scratch_id = active_buffer(&state);
        let file_id = {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .create_from_bytes("secondary".to_owned(), b"abc\nde")
        };
        assert_ne!(scratch_id, file_id);

        let mut s = local();
        // Project the *non-active* file buffer.
        s.set_viewport(file_id, ByteRange { start: 0, end: 64 }, 0);
        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos.iter().all(|d| d.kind != DecorationKind::CurrentLine),
            "semantic projection must not emit CurrentLine for any viewport; got {decos:?}"
        );
    }

    #[test]
    fn cursor_motion_does_not_re_emit_decorations() {
        // Cursor-only movement should not ship Decorations. Semantic
        // frontends receive CursorByte separately and derive local
        // cursor visuals without daemon decoration churn.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"abcdefghij\nklmno",
                })
                .expect("seed");
        }
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _first = s.render_frame(&state); // initial full
        assert!(
            s.render_frame(&state).is_empty(),
            "steady state must be silent"
        );

        // Move cursor from byte 0 to byte 5 (same line).
        {
            let mut core = state.core.borrow_mut();
            core.active_window_mut().cursor = 5;
        }
        assert!(
            s.render_frame(&state).is_empty(),
            "same-line cursor motion must not re-emit Decorations"
        );

        // Cross a `\n` (byte 10). Still no Decorations frame.
        {
            let mut core = state.core.borrow_mut();
            core.active_window_mut().cursor = 12;
        }
        assert!(
            s.render_frame(&state).is_empty(),
            "line-crossing cursor motion must not re-emit Decorations"
        );
    }

    #[test]
    fn diagnostics_project_with_line_col_to_byte_and_severity() {
        // "abc\nde": line 0 at byte 0, line 1 at byte 4.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        // This test pins the diagnostic projection's byte math without
        // relying on any cursor-line decoration.
        let warning = decos
            .iter()
            .find(|d| d.kind == DecorationKind::DiagnosticWarning)
            .expect("the seeded warning");
        // line 1 starts at byte 4; cols [0,2) → bytes [4,6).
        assert_eq!(warning.range, ByteRange { start: 4, end: 6 });
    }

    /// T M11.8 regression: when the diag store's entry for the URI
    /// is marked stale (an edit has been issued since the last
    /// `publishDiagnostics`), the producer must suppress diagnostic
    /// emission so the frontend doesn't paint colors at pre-edit
    /// byte positions over post-edit text. Closes the bet-#1
    /// surface that session-5 validation exposed.
    #[test]
    fn diagnostics_suppressed_while_diag_store_stale() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);

        // Mark the diag store stale for the seeded URI. The producer
        // should now emit zero diagnostic decorations.
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .mark_stale(uri.clone());

        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos.iter().all(|d| !matches!(
                d.kind,
                DecorationKind::DiagnosticError
                    | DecorationKind::DiagnosticWarning
                    | DecorationKind::DiagnosticInfo
                    | DecorationKind::DiagnosticHint
            )),
            "stale diag store ⇒ no diagnostic decorations emitted; got {decos:?}"
        );

        // Once a fresh `set` clears the stale flag, the decoration
        // re-appears on the next render frame.
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 1,
                    start_col: 0,
                    end_line: 1,
                    end_col: 2,
                    severity: crate::diag::DiagnosticSeverity::Warning,
                    message: "x".into(),
                    source: None,
                    code: None,
                }],
            );

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos
                .iter()
                .any(|d| d.kind == DecorationKind::DiagnosticWarning),
            "fresh set ⇒ stale flag cleared ⇒ decoration re-emitted; got {decos:?}"
        );
    }

    /// Hold-while-stale (diagnostics churn fix): once diagnostics
    /// have shipped, marking the store stale (which happens per edit)
    /// must NOT ship a clearing frame — the frontend keeps its
    /// last-received set, translated through its own local edits,
    /// until the next `publishDiagnostics`. The pre-fix behavior
    /// shipped a full empty frame on the first keystroke of every
    /// burst (diagnostics blinked out, one full frontend reshape) and
    /// re-added them after the next publish (blink in, another
    /// reshape).
    #[test]
    fn diagnostics_hold_emission_while_store_stale_after_shipping() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);

        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("baseline ships the diagnostic");
        assert!(
            decos.iter().any(|d| is_diagnostic_kind(d.kind)),
            "baseline contains the seeded diagnostic; got {decos:?}"
        );

        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .mark_stale(uri.clone());

        assert!(
            decorations_of(&s.render_frame(&state)).is_none(),
            "stale store holds Decorations emission instead of shipping a clearing frame"
        );
        assert!(
            decorations_of(&s.render_frame(&state)).is_none(),
            "the hold is stable across frames"
        );

        // A selection change during the stale window still ships
        // (without diagnostic kinds) — the hold must not pin a dead
        // selection just to protect the diagnostics.
        set_selection(&state, 0, 2);
        let (_full, decos) = decorations_of(&s.render_frame(&state))
            .expect("selection change ships during the stale window");
        assert!(
            decos.iter().any(|d| d.kind == DecorationKind::Selection),
            "fresh selection present; got {decos:?}"
        );
        assert!(
            decos.iter().all(|d| !is_diagnostic_kind(d.kind)),
            "no stale-positioned diagnostics ride along; got {decos:?}"
        );

        // The next publishDiagnostics clears the flag. An IDENTICAL
        // publish stays silent — the carried baseline already matches
        // the frontend's cache. A *changed* diagnostic diffs through.
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 1,
                    start_col: 1,
                    end_line: 1,
                    end_col: 2,
                    severity: crate::diag::DiagnosticSeverity::Warning,
                    message: "x".into(),
                    source: None,
                    code: None,
                }],
            );
        let (_full, decos) = decorations_of(&s.render_frame(&state))
            .expect("post-publish frame ships the moved diagnostic");
        assert!(
            decos
                .iter()
                .any(|d| d.kind == DecorationKind::DiagnosticWarning),
            "diagnostics update once the store is fresh; got {decos:?}"
        );
    }

    /// Regression: in a multi-frontend setup the editor's *active*
    /// buffer (set by `core.active_buffer_id()`, derived from the
    /// active frontend's view) can differ from the buffer a given
    /// frontend's `SemanticRenderState` is projecting. The producer
    /// used to derive the diagnostic URI from `active_buffer_path()`,
    /// which returned the wrong path (or `None`) in that case and
    /// emitted empty decorations — the session-5 manual-validation
    /// finding. Fix routes the URI lookup through `vp.buffer_id` via
    /// `buffer_file_uri`.
    #[test]
    fn decorations_use_vp_buffer_not_active_buffer() {
        let state = empty_state();
        // The default LOCAL frontend's active buffer is a scratch
        // with no file path. We seed text + a file path on a buffer
        // that is NOT the active one, then project that buffer via
        // a separate `SemanticRenderState`. Pre-fix: the producer
        // looks up `active_buffer_path()` (the scratch, no path) and
        // gets `None`, emitting empty decorations. Post-fix: it
        // resolves the URI from `vp.buffer_id`, finds the diagnostic.
        let scratch_id = active_buffer(&state);

        // Create a second buffer with a real file path + a diagnostic.
        let file_id = {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .create_from_bytes("secondary".to_owned(), b"abc\nde")
        };
        {
            let mut core = state.core.borrow_mut();
            core.set_buffer_path(file_id, Some(std::path::PathBuf::from("/tmp/multi-fid.rs")));
        }
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/multi-fid.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 1,
                    start_col: 0,
                    end_line: 1,
                    end_col: 2,
                    severity: crate::diag::DiagnosticSeverity::Error,
                    message: "boom".into(),
                    source: None,
                    code: None,
                }],
            );

        // Sanity: active buffer is still the scratch, not `file_id`.
        assert_eq!(scratch_id, active_buffer(&state));
        assert_ne!(scratch_id, file_id);

        // Project the file buffer through the semantic renderer.
        let mut s = local();
        s.set_viewport(file_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert_eq!(
            decos.len(),
            1,
            "decoration must surface for the projected buffer even when it's not the editor's active buffer"
        );
        assert_eq!(decos[0].kind, DecorationKind::DiagnosticError);
        assert_eq!(decos[0].range, ByteRange { start: 4, end: 6 });
    }

    #[test]
    fn styles_and_decorations_suppress_independently() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state); // first frame: both full
        assert!(s.render_frame(&state).is_empty(), "steady state silent");

        // A selection appears → only Decorations re-emits, and as an
        // incremental (full = false) frame since the viewport region
        // did not move.
        set_selection(&state, 1, 4);
        let msgs = s.render_frame(&state);
        assert_eq!(msgs.len(), 1, "only the changed family re-emits");
        let (full, decos) = decorations_of(&msgs).expect("Decorations re-emitted");
        assert!(!full, "viewport unchanged → incremental, not full");
        assert_eq!(decos.len(), 1);
        assert_eq!(decos[0].kind, DecorationKind::Selection);
    }

    #[test]
    fn full_resync_on_viewport_region_change() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state); // full
        assert!(s.render_frame(&state).is_empty(), "unchanged → silent");

        // Declaring a different on-screen range forces a full resync:
        // prior styling/decorations are positioned for the old window.
        s.set_viewport(
            buffer_id,
            ByteRange {
                start: 200,
                end: 264,
            },
            0,
        );
        let msgs = s.render_frame(&state);
        let (style_full, _) = style_segments(&msgs).expect("StyleSpans");
        let (deco_full, _) = decorations_of(&msgs).expect("Decorations");
        assert!(
            style_full && deco_full,
            "viewport jump must be a full resync"
        );
    }

    /// T M11.7 regression: a CRDT generation transition forces a
    /// `full=true` resync on the next frame for both `StyleSpans`
    /// and `Decorations`, even when the declared viewport is
    /// unchanged. Without this, an edit at byte position N would
    /// leave the frontend with stale spans/decorations indexed at
    /// pre-edit byte positions while only a small dirty-range
    /// increment ships — which violates the incremental contract
    /// (the increment expects the frontend to retain non-dirty
    /// items). The bet-#1 surface for `pmacs-gpu`.
    #[cfg(feature = "crdt")]
    #[test]
    fn full_resync_on_generation_transition() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        set_selection(&state, 0, 1);
        let _ = s.render_frame(&state); // initial full
        assert!(
            s.render_frame(&state).is_empty(),
            "unchanged frame → silent"
        );

        // Upgrade the buffer to CRDT-backed so `buffer_generation`
        // tracks edits — without this, version_scalar stays 0 for a
        // v0.1-style buffer and the producer can't detect transitions.
        // Then bump the buffer's generation by editing it.
        {
            let core = state.core.borrow();
            let mut reg = core.registry.borrow_mut();
            let buf = reg.get_mut(buffer_id).expect("active buffer");
            buf.upgrade_to_crdt(1).expect("upgrade to crdt");
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: b"x",
            })
            .expect("buffer edit");
        }

        // Generation transitioned → next frame must be full for both
        // diff-shaped families when there is state to re-anchor.
        let msgs = s.render_frame(&state);
        let (style_full, _) = style_segments(&msgs).expect("StyleSpans re-emitted");
        let (deco_full, _) = decorations_of(&msgs).expect("Decorations re-emitted");
        assert!(
            style_full,
            "post-edit StyleSpans must be full=true (positions shifted)"
        );
        assert!(
            deco_full,
            "post-edit Decorations must be full=true (positions shifted)"
        );
    }

    #[test]
    fn incremental_decoration_change_ships_only_dirty_intervals() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 256 }, 0);
        set_selection(&state, 10, 12);
        let _ = s.render_frame(&state); // full: selection [10,12)
        assert!(s.render_frame(&state).is_empty());

        // Move the selection far away. The symmetric difference is the
        // old range [10,12) (removed) and the new [40,42) (added);
        // they are disjoint and non-adjacent → two segments.
        set_selection(&state, 40, 42);
        let msgs = s.render_frame(&state);
        let deco_msg = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::Decorations { full, segments, .. } => Some((*full, segments)),
                _ => None,
            })
            .expect("Decorations");
        assert!(!deco_msg.0, "incremental");
        let ranges: Vec<ByteRange> = deco_msg.1.iter().map(|s| s.range).collect();
        assert_eq!(
            ranges,
            vec![
                ByteRange { start: 10, end: 12 },
                ByteRange { start: 40, end: 42 }
            ],
            "two disjoint dirty intervals: old (cleared) + new"
        );
        // The [10,12) segment carries no decorations (selection moved
        // away → frontend clears it); [40,42) carries the new one.
        let s1 = &deco_msg.1[0];
        assert!(s1.decorations.is_empty(), "old selection range cleared");
        let s2 = &deco_msg.1[1];
        assert_eq!(s2.decorations.len(), 1);
        assert_eq!(s2.decorations[0].kind, DecorationKind::Selection);
    }

    #[test]
    fn unchanged_decoration_overlapping_a_dirty_interval_is_reconstructed() {
        // A diagnostic at [4,6) never changes; the selection moves to
        // overlap it. The dirty segment must still carry the (clipped)
        // diagnostic so the frontend, replacing styling within the
        // range, faithfully reconstructs the unchanged decoration.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id); // Warning [4,6)
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        set_selection(&state, 20, 22);
        let _ = s.render_frame(&state); // full: Sel[20,22) + Warn[4,6)
        assert!(s.render_frame(&state).is_empty());

        // Selection moves to [5,7), overlapping the diagnostic.
        set_selection(&state, 5, 7);
        let msgs = s.render_frame(&state);
        let (_full, segs) = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::Decorations { full, segments, .. } => Some((*full, segments)),
                _ => None,
            })
            .expect("Decorations");
        // The segment covering [5,7) must include the unchanged,
        // overlapping diagnostic (clipped into the dirty range),
        // not just the moved selection.
        let overlapping = segs
            .iter()
            .find(|s| s.range.start <= 5 && s.range.end >= 6)
            .expect("a segment covering the diagnostic's bytes");
        assert!(
            overlapping
                .decorations
                .iter()
                .any(|d| d.kind == DecorationKind::DiagnosticWarning),
            "unchanged overlapping diagnostic must be reconstructed in the dirty segment"
        );
    }

    #[test]
    fn block_adornments_and_fold_state_still_never_emitted() {
        // BlockAdornments / FoldState have no instance-side source
        // yet, so the projection never produces them (not even empty).
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        for _ in 0..3 {
            for m in s.render_frame(&state) {
                assert!(
                    !matches!(
                        m,
                        InstanceMessage::BlockAdornments { .. } | InstanceMessage::FoldState { .. }
                    ),
                    "a still-unwired block/fold family was emitted: {m:?}"
                );
            }
        }
    }

    #[test]
    fn inline_adornments_not_emitted_without_hints() {
        // Step 3: InlineAdornments IS wired, but a buffer with no LSP
        // inlay-hint store entry must still never emit an (empty)
        // InlineAdornments frame — no empty-frame spam.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        for _ in 0..3 {
            assert!(
                !s.render_frame(&state)
                    .iter()
                    .any(|m| matches!(m, InstanceMessage::InlineAdornments { .. })),
                "no inlay hints ⇒ no InlineAdornments message"
            );
        }
    }

    #[test]
    fn sibling_of_render_state_reads_same_editor_state() {
        // The dispatcher selects the projection per session, not per
        // buffer: a grid RenderState and a SemanticRenderState observe
        // the same EditorState without interfering.
        let state = empty_state();
        let mut grid = RenderState::new(CellSize::new(24, 80));
        let mut sem = local();
        let buffer_id = active_buffer(&state);
        sem.set_viewport(buffer_id, ByteRange { start: 0, end: 80 }, 0);

        let grid_msgs = grid.render_frame(&state, &[]);
        let sem_msgs = sem.render_frame(&state);
        assert!(
            matches!(grid_msgs[0], InstanceMessage::CellDelta { .. }),
            "grid projection still produces CellDelta"
        );
        assert_semantic_only(&sem_msgs);
        assert!(
            !sem_msgs
                .iter()
                .any(|m| matches!(m, InstanceMessage::CellDelta { .. })),
            "semantic projection never produces CellDelta"
        );
    }

    // --- Step 2: LSP-semantic-token styling authority (policy A) ---

    fn tok(line: u32, start: u32, length: u32) -> crate::semantic_tokens::SemanticToken {
        crate::semantic_tokens::SemanticToken {
            line,
            start,
            length,
            token_type: 0, // index 0 in the seeded legend → "kw"
            token_modifiers: 0,
        }
    }

    /// Overwrite the semantic-token store entry for the test client
    /// `sid` on the `/tmp/x.cpp` URI.
    fn set_tokens(
        state: &EditorState,
        sid: crate::lsp::LspServerId,
        tokens: Vec<crate::semantic_tokens::SemanticToken>,
    ) {
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/x.cpp"));
        let store = state.lsp_manager.borrow().semantic_token_store();
        store.lock().expect("sem token store").set(
            crate::semantic_tokens::SemanticTokenKey::new(sid.raw().to_string(), uri),
            crate::semantic_tokens::SemanticTokensResponse {
                tokens,
                result_id: None,
                raw: Vec::new(),
            },
        );
    }

    /// Seed a grammar-less (`.cpp`) buffer plus an Initialized test
    /// LSP client advertising a one-entry legend (`["kw"]`, UTF-16),
    /// a theme face for `kw`, and `tokens` in the store. Returns the
    /// client id so a test can re-seed the same `(server, uri)`.
    fn seed_lsp_style(
        state: &EditorState,
        buffer_id: BufferId,
        text: &[u8],
        tokens: Vec<crate::semantic_tokens::SemanticToken>,
    ) -> crate::lsp::LspServerId {
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: text,
                })
                .expect("seed buffer text");
            // `.cpp` has no bundled tree-sitter grammar → the registry
            // yields no view → policy A routes to the LSP producer.
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.cpp")));
        }
        state.syntax_registry.theme().lock().expect("theme").insert(
            "kw",
            crate::cell::Style {
                bold: true,
                ..crate::cell::Style::default()
            },
        );
        let sid = state
            .lsp_manager
            .borrow_mut()
            .insert_initialized_test_client(
                serde_json::json!({
                    "semanticTokensProvider": {
                        "legend": { "tokenTypes": ["kw"], "tokenModifiers": [] }
                    }
                }),
                crate::lsp::PositionEncoding::Utf16,
            );
        set_tokens(state, sid, tokens);
        sid
    }

    fn seed_rust_parse_view(
        state: &EditorState,
        buffer_id: BufferId,
        text: &[u8],
    ) -> crate::syntax::ParseViewHandle {
        let language = state
            .syntax_registry
            .language("rust")
            .expect("rust language");
        let mut core = state.core.borrow_mut();
        let registry_handle = core.registry.clone();
        let mut registry = registry_handle.borrow_mut();
        let buf = registry.get_mut(buffer_id).expect("active buffer");
        if !text.is_empty() {
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: text,
            })
            .expect("seed rust text");
        }
        let parse_view = crate::syntax::ParseView::new(buf, language, "rust".to_owned());
        let handle = parse_view.handle();
        let req = handle.make_request();
        let bundle = crate::syntax::run_parse(req).expect("initial rust parse");
        handle.install(std::sync::Arc::new(bundle));
        buf.attach_view(Box::new(parse_view));
        drop(registry);
        core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.rs")));
        drop(core);
        state.syntax_registry.attach_view(buffer_id, handle.clone());
        handle
    }

    #[test]
    fn cpp_style_comes_from_lsp_when_no_tree_sitter_grammar() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "int" on line 0, bytes [0,3).
        seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let spans = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::StyleSpans { full, segments, .. } => {
                    assert!(*full, "first frame is a full resync");
                    Some(
                        segments
                            .iter()
                            .flat_map(|seg| seg.spans.clone())
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .expect("StyleSpans emitted from the LSP authority");
        assert_eq!(spans.len(), 1, "the one LSP token → one span");
        assert_eq!(spans[0].range, ByteRange { start: 0, end: 3 });
        assert!(
            spans[0].style.bold,
            "token_type resolved through legend → theme face"
        );
    }

    #[test]
    fn grammar_style_spans_wait_for_pending_parse() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let handle = seed_rust_parse_view(&state, bid, b"fn main() {}\n");
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let first = s.render_frame(&state);
        assert!(
            style_segments(&first).is_some(),
            "installed parse emits the baseline style frame"
        );

        {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"// editing\n",
                })
                .expect("typing edit");
        }
        assert!(
            handle.pending_edit_count() > 0,
            "attached parse view recorded the edit"
        );
        let pending = s.render_frame(&state);
        assert!(
            style_segments(&pending).is_none(),
            "style query is skipped while edits are waiting for parse dispatch"
        );

        let req = handle.make_request();
        state.syntax_registry.record_parse_job(9001, bid);
        let in_flight = s.render_frame(&state);
        assert!(
            style_segments(&in_flight).is_none(),
            "style query is skipped while the parse job is in flight"
        );

        let bundle = crate::syntax::run_parse(req).expect("settled rust parse");
        handle.install(std::sync::Arc::new(bundle));
        assert_eq!(state.syntax_registry.take_parse_job(9001), Some(bid));
        let settled = s.render_frame(&state);
        assert!(
            style_segments(&settled).is_some(),
            "new parse bundle emits refreshed style spans"
        );
    }

    /// Hold-while-stale for the LSP-token styling authority: once a
    /// grammar-less buffer's colors have shipped, marking the token
    /// store stale (which happens per edit) must NOT ship a clearing
    /// frame — the styling twin of the diagnostics hold. The next
    /// token response clears the flag and re-emits.
    #[test]
    fn lsp_style_holds_while_token_store_stale() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let sid = seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );
        assert!(
            style_segments(&s.render_frame(&state)).is_some(),
            "baseline ships the LSP-token styling"
        );

        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/x.cpp"));
        {
            let store = state.lsp_manager.borrow().semantic_token_store();
            let mut guard = store.lock().expect("semantic token store");
            guard.mark_stale(uri.clone());
        }

        assert!(
            style_segments(&s.render_frame(&state)).is_none(),
            "stale token store holds StyleSpans instead of clearing the colors"
        );
        assert!(
            style_segments(&s.render_frame(&state)).is_none(),
            "the hold is stable across frames"
        );

        // A fresh token response (absorbed via `set`) clears the flag.
        // Identical tokens produce no frame — the frontend's cache was
        // never cleared, so there is nothing to say. Changed tokens
        // diff against the held baseline and ship.
        set_tokens(&state, sid, vec![tok(0, 0, 3)]);
        assert!(
            style_segments(&s.render_frame(&state)).is_none(),
            "fresh-but-identical tokens stay silent (cache was never wiped)"
        );
        set_tokens(&state, sid, vec![tok(0, 0, 5)]);
        assert!(
            style_segments(&s.render_frame(&state)).is_some(),
            "fresh changed tokens re-emit the styling"
        );
    }

    #[test]
    fn lsp_style_suppressed_when_unchanged() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let _ = s.render_frame(&state); // full baseline
        let again = s.render_frame(&state);
        assert!(
            !again
                .iter()
                .any(|m| matches!(m, InstanceMessage::StyleSpans { .. })),
            "unchanged LSP styling reuses the M11.4 byte-identical suppression"
        );
    }

    #[test]
    fn lsp_style_incremental_on_token_change() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let sid = seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );
        let _ = s.render_frame(&state); // full baseline

        // Token now covers a different range ([4,5) = "x").
        set_tokens(&state, sid, vec![tok(0, 4, 1)]);
        let delta = s.render_frame(&state);
        let (full, _) = style_segments(&delta).expect("StyleSpans re-emitted");
        assert!(!full, "a changed token set ships an incremental frame");
    }

    #[test]
    fn lsp_style_empty_when_no_tokens_for_grammarless_buffer() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"int x;\n",
                })
                .expect("seed text");
            core.set_buffer_path(bid, Some(std::path::PathBuf::from("/tmp/x.cpp")));
        }
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let msgs = s.render_frame(&state);
        // Still a full resync (the frontend must be told "nothing
        // here"), but with no spans — no LSP store, no panic, honest
        // empty rather than a fabricated style.
        let styled = msgs.iter().find_map(|m| match m {
            InstanceMessage::StyleSpans { full, segments, .. } => Some((*full, segments.clone())),
            _ => None,
        });
        let (full, segments) = styled.expect("a full StyleSpans resync");
        assert!(full);
        assert!(
            segments.iter().all(|sg| sg.spans.is_empty()),
            "grammar-less buffer with no LSP tokens ⇒ zero spans"
        );
    }

    // --- Step 3: InlineAdornments from the LSP inlay-hint store ---

    fn inlay_uri() -> String {
        crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/h.rs"))
    }

    /// Seed the inlay-hint store. Keyed by `(server, uri)`; `for_uri`
    /// picks the lowest server, so a fixed `"1"` is fine. No LSP
    /// client needed — the inlay path never consults
    /// `semantic_style_context` (cols are already byte offsets, Step 0).
    fn set_inlay_store(state: &EditorState, uri: &str, hints: Vec<crate::inlay_hint::InlayHint>) {
        let store = state.lsp_manager.borrow().inlay_hint_store();
        store.lock().expect("inlay store").set(
            crate::inlay_hint::InlayHintKey::new("1", uri),
            crate::inlay_hint::InlayHintResponse { hints },
        );
    }

    /// Seed buffer text + path and an inlay-hint store entry.
    fn seed_inlay(
        state: &EditorState,
        buffer_id: BufferId,
        hints: Vec<crate::inlay_hint::InlayHint>,
    ) {
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"let x = f();\n",
                })
                .expect("seed buffer text");
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/h.rs")));
        }
        set_inlay_store(state, &inlay_uri(), hints);
    }

    fn hint(line: u32, col: u32, label: &str) -> crate::inlay_hint::InlayHint {
        crate::inlay_hint::InlayHint {
            line,
            col,
            label: label.into(),
            kind: None,
            padding_left: false,
            padding_right: true,
            tooltip: None,
        }
    }

    fn adornments_of(msgs: &[InstanceMessage]) -> Option<Vec<InlineAdornment>> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::InlineAdornments { items, .. } => Some(items.clone()),
            _ => None,
        })
    }

    #[test]
    fn inlay_hints_project_as_inline_adornments_clipped_to_viewport() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // `: i32` after `x` (col 5, in-viewport) and a hint past the
        // declared viewport that must be excluded.
        seed_inlay(&state, bid, vec![hint(0, 5, ": i32"), hint(0, 11, "OUT")]);
        s.set_viewport(bid, ByteRange { start: 0, end: 8 }, 0);

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let items = adornments_of(&msgs).expect("InlineAdornments emitted");
        assert_eq!(items.len(), 1, "the out-of-viewport hint is clipped");
        let a = &items[0];
        assert_eq!(a.at, 5);
        assert_eq!(a.placement, AdornmentPlacement::AtOffset);
        match &a.content {
            AdornmentContent::Text { text, style } => {
                assert_eq!(text, ": i32 ", "padding_right adds a trailing space");
                assert_eq!(*style, Style::default());
            }
            AdornmentContent::Resource { .. } => panic!("expected Text, got Resource"),
        }
    }

    #[test]
    fn inline_adornments_suppressed_then_resync_on_change() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        seed_inlay(&state, bid, vec![hint(0, 5, ": i32")]);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        assert!(
            adornments_of(&s.render_frame(&state)).is_some(),
            "first frame ships the adornments"
        );
        assert!(
            adornments_of(&s.render_frame(&state)).is_none(),
            "byte-identical next frame is suppressed"
        );

        // Hint set changes → whole-set re-send (no segment diffing on
        // this wire variant).
        seed_inlay(&state, bid, vec![hint(0, 5, ": String")]);
        let again = adornments_of(&s.render_frame(&state)).expect("re-sent on change");
        match &again[0].content {
            AdornmentContent::Text { text, .. } => assert_eq!(text, ": String "),
            AdornmentContent::Resource { .. } => panic!("expected Text, got Resource"),
        }
    }

    #[test]
    fn inline_adornments_hold_while_inlay_store_stale() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        seed_inlay(&state, bid, vec![hint(0, 5, ": i32")]);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        assert!(
            adornments_of(&s.render_frame(&state)).is_some(),
            "first frame ships the adornments"
        );

        let uri = inlay_uri();
        let store = state.lsp_manager.borrow().inlay_hint_store();
        store.lock().expect("inlay store").mark_stale(uri.clone());

        // Hold-while-stale: no frame at all. The frontend keeps its
        // last-received hints, translated through its own local
        // edits — an empty frame here would wipe them and visibly
        // shift the line layout on the first keystroke of a burst.
        assert!(
            adornments_of(&s.render_frame(&state)).is_none(),
            "stale store holds emission (frontend keeps its translated cache)"
        );
        assert!(
            adornments_of(&s.render_frame(&state)).is_none(),
            "the hold is stable across frames"
        );

        // A fresh inlayHint response clears the flag and re-emits at
        // the server's (possibly shifted) positions.
        set_inlay_store(&state, &uri, vec![hint(0, 7, ": i32")]);
        let refreshed = adornments_of(&s.render_frame(&state)).expect("fresh hints re-emit");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].at, 7);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn session8_temporal_probe_sustained_edits_hold_stale_inlays_until_refresh() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let path = std::path::PathBuf::from("/tmp/session8-inlay.txt");
        let uri = crate::lsp::path_to_file_uri(&path);

        {
            let mut core = state.core.borrow_mut();
            let mut reg = core.registry.borrow_mut();
            let buf = reg.get_mut(bid).expect("active buffer");
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: b"let x = f();\n",
            })
            .expect("seed buffer text");
            buf.upgrade_to_crdt(1).expect("upgrade to crdt");
            drop(reg);
            core.set_buffer_path(bid, Some(path));
        }

        set_inlay_store(&state, &uri, vec![hint(0, 5, ": i32")]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );
        assert!(
            adornments_of(&s.render_frame(&state)).is_some(),
            "baseline emits the fresh inlay hint"
        );

        let mut clear_frames = 0;
        let mut full_style_frames = 0;
        let mut full_deco_frames = 0;
        for _ in 0..1000 {
            {
                let core = state.core.borrow();
                let mut reg = core.registry.borrow_mut();
                let buf = reg.get_mut(bid).expect("active buffer");
                buf.apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"x",
                })
                .expect("typing edit");
            }
            let store = state.lsp_manager.borrow().inlay_hint_store();
            store.lock().expect("inlay store").mark_stale(uri.clone());

            let msgs = s.render_frame(&state);
            if let Some((full, _)) = style_segments(&msgs)
                && full
            {
                full_style_frames += 1;
            }
            if let Some((full, _)) = decorations_of(&msgs)
                && full
            {
                full_deco_frames += 1;
            }
            if let Some(items) = adornments_of(&msgs) {
                assert!(
                    items.is_empty(),
                    "stale inlay hints must not render during sustained typing"
                );
                clear_frames += 1;
            }
        }

        assert_eq!(
            clear_frames, 0,
            "stale frames hold emission entirely — the frontend keeps \
             its locally-translated hints instead of blinking them out"
        );
        assert_eq!(
            full_style_frames, 1000,
            "each CRDT generation transition forces a StyleSpans full resync"
        );
        assert_eq!(
            full_deco_frames, 0,
            "empty Decorations state stays silent across generation transitions"
        );

        set_inlay_store(&state, &uri, vec![hint(0, 1005, ": i32")]);
        let refreshed = adornments_of(&s.render_frame(&state)).expect("fresh hints re-emit");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].at, 1005);
    }

    // --- M1: FileStyleSummary (minimap producer, Open Q#2) ---

    fn summary_of(msgs: &[InstanceMessage]) -> Option<(u64, Vec<Style>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::FileStyleSummary {
                generation, lines, ..
            } => Some((*generation, lines.clone())),
            _ => None,
        })
    }

    #[test]
    fn file_style_summary_emitted_on_first_frame_and_suppressed_thereafter() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // Tiny buffer with no LSP / grammar setup → no styled runs →
        // per-line dominant style stays `Style::default()`. The point
        // of this test is the emit / suppress lifecycle, not content.
        {
            let core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"a\nb\nc\n",
                })
                .expect("seed");
        }
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        assert_semantic_only(&first);
        let (_gen, lines) = summary_of(&first).expect("first frame ships a summary");
        // 4 lines: "a", "b", "c", "" (trailing empty after final \n).
        assert_eq!(lines.len(), 4);
        assert!(
            lines.iter().all(|s| *s == Style::default()),
            "no styled runs ⇒ every line takes the default style"
        );

        // Same generation → no re-emission.
        let again = s.render_frame(&state);
        assert!(
            summary_of(&again).is_none(),
            "unchanged generation suppresses the summary"
        );
    }

    #[test]
    fn file_style_summary_dominant_style_from_lsp_per_line() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // `.cpp` buffer (no tree-sitter grammar) so styling comes from
        // the LSP semantic-token authority (policy A). Three lines;
        // line 0 has a token spanning bytes 0..3, line 2 has one
        // spanning bytes 8..11. Line 1 has no tokens.
        //
        // Buffer layout (newlines included):
        //   bytes 0..4  "abc\n"   line 0 = [0,3)  ← token [0,3)
        //   bytes 4..8  "def\n"   line 1 = [4,7)  ← no token
        //   bytes 8..12 "ghi\n"   line 2 = [8,11) ← token [8,11)
        let sid = seed_lsp_style(
            &state,
            bid,
            b"abc\ndef\nghi\n",
            vec![tok(0, 0, 3), tok(2, 0, 3)],
        );
        let _ = sid;
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let msgs = s.render_frame(&state);
        let (_, lines) = summary_of(&msgs).expect("summary emitted");
        assert_eq!(lines.len(), 4, "three lines + trailing empty");
        let kw_style = crate::cell::Style {
            bold: true,
            ..crate::cell::Style::default()
        };
        assert_eq!(lines[0], kw_style, "line 0 dominated by the LSP token");
        assert_eq!(lines[1], Style::default(), "line 1 has no token → default");
        assert_eq!(lines[2], kw_style, "line 2 dominated by the LSP token");
        assert_eq!(lines[3], Style::default(), "trailing empty line → default");
    }

    fn facts_of(msgs: &[InstanceMessage]) -> Option<(String, bool, u32, u32)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::StatusFacts {
                name,
                modified,
                diag_errors,
                diag_warnings,
                ..
            } => Some((name.clone(), *modified, *diag_errors, *diag_warnings)),
            _ => None,
        })
    }

    #[test]
    fn status_facts_emit_on_change_and_freeze_counts_while_stale() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "abc\nde" + a Warning diagnostic + file path; the seeding
        // edit flips `modified`.
        seed_diagnostic(&state, bid);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        let (_, modified, errors, warnings) = facts_of(&first).expect("first frame ships facts");
        assert!(modified, "the seeding edit dirtied the buffer");
        assert_eq!((errors, warnings), (0, 1));

        // Nothing changed → suppressed.
        assert!(facts_of(&s.render_frame(&state)).is_none());

        // Republish as an Error → re-emit with new counts.
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        let store = state.lsp_manager.borrow().diag_store();
        store.lock().expect("diag store").set(
            &uri,
            vec![crate::diag::Diagnostic {
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 3,
                severity: crate::diag::DiagnosticSeverity::Error,
                message: "boom".into(),
                source: None,
                code: None,
            }],
        );
        let (_, _, errors, warnings) =
            facts_of(&s.render_frame(&state)).expect("republish re-emits");
        assert_eq!((errors, warnings), (1, 0));

        // Stale store: counts freeze at the cached value instead of
        // flickering to zero, so no re-emission either.
        store.lock().expect("diag store").mark_stale(&uri);
        assert!(facts_of(&s.render_frame(&state)).is_none());
    }

    #[test]
    fn zero_width_diagnostics_widen_to_a_visible_byte() {
        // "abc\nde" — line starts [0, 4], source_len 6; line 0
        // content is bytes [0, 3) (newline excluded).
        let ls = vec![0u64, 4];
        // Mid-line anchor: widen forward.
        assert_eq!(widen_zero_width_diag(1, 1, 0, &ls, 6), (1, 2));
        // End-of-line anchor (the rust-analyzer "expected COMMA"
        // shape): widen backward — forward would cover only the \n.
        assert_eq!(widen_zero_width_diag(3, 3, 0, &ls, 6), (2, 3));
        // End-of-file anchor on the last line.
        assert_eq!(widen_zero_width_diag(6, 6, 1, &ls, 6), (5, 6));
        // Non-empty ranges pass through untouched.
        assert_eq!(widen_zero_width_diag(1, 3, 0, &ls, 6), (1, 3));
    }

    #[test]
    fn file_style_summary_marks_diagnostic_lines_and_refreshes_on_republish() {
        use crate::cell::Color;
        use crate::diag::DiagnosticSeverity;

        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "abc\nde" with a Warning on line 1 (and a file path so the
        // buffer has a URI).
        seed_diagnostic(&state, bid);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        let (_, lines) = summary_of(&first).expect("first frame ships a summary");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0].underline_color,
            Color::Default,
            "clean line carries no mark"
        );
        assert_eq!(
            lines[1].underline_color,
            DiagnosticSeverity::Warning.underline_color(),
            "diagnostic line carries the severity mark"
        );

        // Unchanged generation + diag epoch → suppressed.
        assert!(summary_of(&s.render_frame(&state)).is_none());

        // A republish moves the diagnostic to line 0 and escalates it.
        // No CRDT edit happened — the diag epoch alone must re-emit
        // the summary with refreshed marks.
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 0,
                    start_col: 0,
                    end_line: 0,
                    end_col: 3,
                    severity: DiagnosticSeverity::Error,
                    message: "boom".into(),
                    source: None,
                    code: None,
                }],
            );
        let refreshed = s.render_frame(&state);
        let (_, lines) = summary_of(&refreshed).expect("diag republish re-emits the summary");
        assert_eq!(
            lines[0].underline_color,
            DiagnosticSeverity::Error.underline_color()
        );
        assert_eq!(
            lines[1].underline_color,
            Color::Default,
            "the old warning mark is gone after the republish"
        );
    }
}
