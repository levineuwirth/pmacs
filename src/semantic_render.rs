// semantic_render.rs --- Instance-side semantic projection (T M11.2).

//! The semantic projection seam.
//!
//! [`crate::instance_render::RenderState`] rasterizes the editor to a
//! cell grid and ships [`InstanceMessage::CellDelta`]. `SemanticRenderState`
//! is its sibling for `semantic_render` sessions: it reads the same
//! [`EditorState`] but exits the pipeline *earlier* — it emits the
//! structured byte-range styling the cell painter would otherwise have
//! consumed (tree-sitter spans from [`crate::syntax`] mapped through
//! the active [`crate::highlight::Theme`]), without the grid-packing
//! step. The frontend lays the styling out locally over rope text it
//! already holds via its `crdt_replica` `BufferMirror`.
//!
//! Contract boundary (see `docs/semantic-frontend-protocol.md`): the
//! instance never learns a pixel. The only spatial fact it consumes is
//! the buffer byte range the frontend declared on screen via
//! [`crate::protocol::FrontendEvent::Viewport`]; styling is scoped to
//! that range so a 100k-line file's styling is never shipped wholesale.
//!
//! M11.2 scope: `StyleSpans` only. `Decorations` / `InlineAdornments` /
//! `BlockAdornments` / `FoldState` / `ResourceOffer` are M11.3; true
//! span-granularity diffing (this module currently suppresses only
//! byte-identical frames) is M11.4.

use std::collections::HashMap;

use crate::buffer::BufferId;
use crate::cell::Style;
use crate::editor::EditorState;
use crate::protocol::{
    ByteRange, Decoration, DecorationKind, DecorationSegment, FrontendId, InstanceMessage,
    StyleSegment, StyleSpan,
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
/// declared-viewport region the set was computed for, and the full
/// scoped item set last shipped. The next frame diffs against
/// `items`; `visible` changing (or no entry) forces a `full` resync.
/// The frame's `generation` is recomputed each tick and carried on
/// the wire, so it is not retained here.
struct LastFrame<T> {
    visible: ByteRange,
    items: Vec<T>,
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
    /// Returns up to two messages — an [`InstanceMessage::StyleSpans`]
    /// (T M11.2) and an [`InstanceMessage::Decorations`] (T M11.3) —
    /// each scoped to the declared viewport and each suppressed
    /// independently when byte-identical to its last send at the same
    /// generation. Returns an empty vec before the frontend declares a
    /// viewport.
    ///
    /// `InlineAdornments` / `BlockAdornments` / `FoldState` are
    /// deliberately *not* produced: pmacs has no instance-side inlay-
    /// hint / blame / lens / fold / diff source yet. The wire variants
    /// exist (T M11.1); their producers are wired when those features
    /// land — the same "declared, not yet wired" discipline M11.1
    /// applied to the whole family. Emitting empty messages every
    /// frame would be waste, not honesty.
    pub fn render_frame(&mut self, state: &EditorState) -> Vec<InstanceMessage> {
        let Some(vp) = self.viewport.clone() else {
            // Emit nothing before the frontend declares a viewport.
            return Vec::new();
        };

        let generation = buffer_generation(state, vp.buffer_id);
        let mut out = Vec::new();

        // --- StyleSpans (T M11.2 producer, T M11.4 diff) ---
        let spans = scoped_style_spans(state, &vp);
        let prev = self.last_sent.get(&vp.buffer_id);
        // Resync when there is no baseline, or the declared viewport
        // region moved (the scoping window changed, so prior styling
        // is no longer positioned correctly).
        let full = prev.is_none_or(|p| p.visible != vp.visible);
        if full {
            // The first frame for this buffer/viewport. One segment
            // covering the declared viewport carries the whole scoped
            // set (possibly empty → frontend clears the viewport).
            self.last_sent.insert(
                vp.buffer_id,
                LastFrame {
                    visible: vp.visible,
                    items: spans.clone(),
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

        // --- Decorations (T M11.3 producer, T M11.4 diff) ---
        let decorations = self.scoped_decorations(state, &vp);
        let prev = self.last_decorations.get(&vp.buffer_id);
        let full = prev.is_none_or(|p| p.visible != vp.visible);
        if full {
            self.last_decorations.insert(
                vp.buffer_id,
                LastFrame {
                    visible: vp.visible,
                    items: decorations.clone(),
                },
            );
            out.push(InstanceMessage::Decorations {
                buffer_id: vp.buffer_id,
                generation,
                full: true,
                segments: vec![DecorationSegment {
                    range: vp.visible,
                    decorations,
                }],
            });
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

        out
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
    fn scoped_decorations(&self, state: &EditorState, vp: &DeclaredViewport) -> Vec<Decoration> {
        let core = state.core.borrow();
        let mut out = Vec::new();

        // Selection — per-window (per-frontend) state, already byte
        // offsets. Only this session's active window for the declared
        // buffer contributes.
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
        // Lua LSP glue opened the document under. `core.file_path` is
        // the editor's active file path; encoding it with the shared
        // `path_to_file_uri` reproduces that exact key (the Lua
        // `file_uri_for` is byte-identical). A buffer with no file
        // path, or no diagnostics under its URI, contributes nothing.
        if let Some(path) = core.active_buffer_path() {
            let uri = crate::lsp::path_to_file_uri(&path);
            let diags = {
                let store = state.lsp_manager.borrow().diag_store();
                let guard = store.lock().expect("diag store mutex poisoned");
                guard.for_uri(&uri).to_vec()
            };
            if !diags.is_empty() {
                let registry = core.registry.clone();
                let reg = registry.borrow();
                if let Ok(buf) = reg.get(vp.buffer_id) {
                    let source = buffer_source_bytes(buf);
                    let line_starts = line_start_offsets(&source);
                    for d in &diags {
                        let lo = line_col_to_byte(
                            &line_starts,
                            source.len() as u64,
                            d.start_line,
                            d.start_col,
                        );
                        let hi = line_col_to_byte(
                            &line_starts,
                            source.len() as u64,
                            d.end_line,
                            d.end_col,
                        );
                        if let Some(range) = clip_to_viewport(lo, hi, vp) {
                            out.push(Decoration {
                                range,
                                kind: severity_to_kind(d.severity),
                            });
                        }
                    }
                }
            }
        }

        out
    }
}

/// Intersect `[lo, hi)` with the declared viewport (itself clamped to
/// the source length is the caller's concern for styling; for
/// decorations we clamp against the viewport only). `None` when the
/// intersection is empty or degenerate.
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

/// Compute the styled byte runs intersecting the declared viewport,
/// mapped through the active theme. Spans are clipped to the viewport
/// and to the parsed source length; runs that resolve to the default
/// style are dropped (wire economy, and consistent with the grid
/// path, which skips default-style merges).
fn scoped_style_spans(state: &EditorState, vp: &DeclaredViewport) -> Vec<StyleSpan> {
    let Some(handle) = state.syntax_registry.view(vp.buffer_id) else {
        return Vec::new();
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
    let highlights = crate::syntax::compute_highlight_spans(&query, &bundle);
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
    /// emit are `StyleSpans` or `Decorations` — never `CellDelta`,
    /// grid `Cursor`, or the not-yet-wired adornment/fold families.
    fn assert_semantic_only(msgs: &[InstanceMessage]) {
        for m in msgs {
            assert!(
                matches!(
                    m,
                    InstanceMessage::StyleSpans { .. } | InstanceMessage::Decorations { .. }
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
        // the first frame is a `full` resync for both families (the
        // frontend clears its viewport), carrying empty segments.
        let first = s.render_frame(&state);
        assert_eq!(first.len(), 2, "first frame ships StyleSpans + Decorations");
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
    fn diagnostics_project_with_line_col_to_byte_and_severity() {
        // "abc\nde": line 0 at byte 0, line 1 at byte 4.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert_eq!(decos.len(), 1);
        assert_eq!(decos[0].kind, DecorationKind::DiagnosticWarning);
        // line 1 starts at byte 4; cols [0,2) → bytes [4,6).
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
    fn adornment_and_fold_families_are_never_emitted() {
        // M11.3 honest-stub contract: InlineAdornments / BlockAdornments
        // / FoldState have no instance-side source yet, so the
        // projection never produces them (not even empty ones).
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        for _ in 0..3 {
            for m in s.render_frame(&state) {
                assert!(
                    !matches!(
                        m,
                        InstanceMessage::InlineAdornments { .. }
                            | InstanceMessage::BlockAdornments { .. }
                            | InstanceMessage::FoldState { .. }
                    ),
                    "a not-yet-wired adornment/fold family was emitted: {m:?}"
                );
            }
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
}
