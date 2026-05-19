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
    ByteRange, Decoration, DecorationKind, FrontendId, InstanceMessage, StyleSpan,
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

/// Owns one `semantic_render` session's projection state: the last
/// viewport the frontend declared, and the last `StyleSpans` /
/// `Decorations` payloads shipped per buffer (for byte-identical-frame
/// suppression).
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
    /// Last `(generation, spans)` shipped, keyed by buffer. A frame
    /// whose scoped span set and generation match the last send emits
    /// nothing — the steady-state cost between edits is one map
    /// lookup. True per-span delta encoding is M11.4.
    last_sent: HashMap<BufferId, (u64, Vec<StyleSpan>)>,
    /// Same unchanged-frame suppression for the `Decorations` family
    /// (T M11.3), tracked independently of `last_sent` so a styling
    /// change does not force a decorations re-send and vice versa.
    last_decorations: HashMap<BufferId, (u64, Vec<Decoration>)>,
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

        // --- StyleSpans (T M11.2) ---
        let spans = scoped_style_spans(state, &vp);
        // Unchanged-frame suppression (M11.4 replaces this with
        // per-span delta encoding). A buffer with an empty scoped set
        // still suppresses correctly: the first empty frame ships once
        // (clearing any prior styling on the frontend), subsequent
        // identical empty frames are squelched.
        let style_unchanged = self
            .last_sent
            .get(&vp.buffer_id)
            .is_some_and(|(g, s)| *g == generation && *s == spans);
        if !style_unchanged {
            self.last_sent
                .insert(vp.buffer_id, (generation, spans.clone()));
            out.push(InstanceMessage::StyleSpans {
                buffer_id: vp.buffer_id,
                generation,
                spans,
            });
        }

        // --- Decorations (T M11.3) ---
        let decorations = self.scoped_decorations(state, &vp);
        let deco_unchanged = self
            .last_decorations
            .get(&vp.buffer_id)
            .is_some_and(|(g, d)| *g == generation && *d == decorations);
        if !deco_unchanged {
            self.last_decorations
                .insert(vp.buffer_id, (generation, decorations.clone()));
            out.push(InstanceMessage::Decorations {
                buffer_id: vp.buffer_id,
                decorations,
            });
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
    fn scoped_decorations(
        &self,
        state: &EditorState,
        vp: &DeclaredViewport,
    ) -> Vec<Decoration> {
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
        if let Some(path) = core.file_path.as_ref() {
            let uri = crate::lsp::path_to_file_uri(path);
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

    #[test]
    fn emits_nothing_before_viewport_declared() {
        let mut s = local();
        assert!(
            s.render_frame(&empty_state()).is_empty(),
            "nothing may be emitted before the frontend declares a viewport"
        );
    }

    #[test]
    fn first_post_viewport_frame_ships_styles_and_decorations_then_suppresses() {
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 4096 }, 0);

        // Empty scratch buffer: no syntax spans, no selection, no
        // diagnostics — but the first frame ships both messages once
        // (each clears any prior frontend state), carrying empty sets.
        let first = s.render_frame(&state);
        assert_eq!(first.len(), 2, "first frame ships StyleSpans + Decorations");
        assert_semantic_only(&first);
        let has = |pred: fn(&InstanceMessage) -> bool| first.iter().any(pred);
        assert!(has(|m| matches!(m, InstanceMessage::StyleSpans { generation: 0, .. })));
        assert!(has(|m| matches!(m, InstanceMessage::Decorations { .. })));

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
        // Put a selection on LOCAL's active window: anchor 2, cursor
        // 5 → region (2, 5). region() compares offsets only, so the
        // empty scratch buffer is fine for this projection test.
        {
            let mut core = state.core.borrow_mut();
            let win = core
                .active_window_mut_for(FrontendId::LOCAL)
                .expect("LOCAL always has a window");
            win.selection = Some(crate::window::Selection { anchor: 2 });
            win.cursor = 5;
        }
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 3, end: 64 }, 0);

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let deco = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::Decorations { decorations, .. } => Some(decorations),
                _ => None,
            })
            .expect("a Decorations message");
        assert_eq!(deco.len(), 1, "exactly the selection decoration");
        assert_eq!(deco[0].kind, DecorationKind::Selection);
        // region (2,5) clipped to viewport [3,64) → [3,5).
        assert_eq!(deco[0].range, ByteRange { start: 3, end: 5 });
    }

    #[test]
    fn diagnostics_project_with_line_col_to_byte_and_severity() {
        // Buffer with two short lines so line/col → byte is exercised.
        // "abc\nde" → line 0 starts at byte 0, line 1 at byte 4.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        {
            let mut core = state.core.borrow_mut();
            let reg = core.registry.clone();
            reg.borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"abc\nde",
                })
                .expect("seed buffer text");
            // The diag store is keyed by file URI; point the editor's
            // active file path at one and seed a diagnostic there.
            core.file_path = Some(std::path::PathBuf::from("/tmp/m113.rs"));
        }
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m113.rs"));
        {
            let store = state.lsp_manager.borrow().diag_store();
            let mut g = store.lock().expect("diag store");
            g.set(
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
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let msgs = s.render_frame(&state);
        let deco = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::Decorations { decorations, .. } => Some(decorations),
                _ => None,
            })
            .expect("a Decorations message");
        assert_eq!(deco.len(), 1);
        assert_eq!(deco[0].kind, DecorationKind::DiagnosticWarning);
        // line 1 starts at byte 4; cols [0,2) → bytes [4,6).
        assert_eq!(deco[0].range, ByteRange { start: 4, end: 6 });
    }

    #[test]
    fn styles_and_decorations_suppress_independently() {
        // A selection change must re-send Decorations without forcing
        // a StyleSpans re-send (and the empty scratch styling stays
        // suppressed).
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state); // first frame: both shipped
        assert!(s.render_frame(&state).is_empty(), "steady state silent");

        // Introduce a selection → only Decorations should re-emit.
        {
            let mut core = state.core.borrow_mut();
            let win = core
                .active_window_mut_for(FrontendId::LOCAL)
                .expect("LOCAL window");
            win.selection = Some(crate::window::Selection { anchor: 1 });
            win.cursor = 4;
        }
        let msgs = s.render_frame(&state);
        assert_eq!(msgs.len(), 1, "only the changed family re-emits");
        assert!(matches!(msgs[0], InstanceMessage::Decorations { .. }));
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
