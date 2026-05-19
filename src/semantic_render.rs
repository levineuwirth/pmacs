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
use crate::protocol::{ByteRange, InstanceMessage, StyleSpan};

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
/// viewport the frontend declared, and the last `StyleSpans` payload
/// shipped per buffer (for byte-identical-frame suppression).
#[derive(Default)]
pub struct SemanticRenderState {
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
}

impl SemanticRenderState {
    /// Fresh session state: no viewport declared, nothing sent.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
    /// Returns at most one [`InstanceMessage::StyleSpans`], scoped to
    /// the declared viewport. Returns an empty vec when: no viewport
    /// has been declared yet; the buffer has no parse view or settled
    /// tree; the language has no highlights query; or the scoped span
    /// set is byte-identical to the last one shipped at the same
    /// generation (the unchanged-frame fast path).
    pub fn render_frame(&mut self, state: &EditorState) -> Vec<InstanceMessage> {
        let Some(vp) = self.viewport.clone() else {
            // Emit nothing before the frontend declares a viewport.
            return Vec::new();
        };

        let generation = buffer_generation(state, vp.buffer_id);
        let spans = scoped_style_spans(state, &vp);

        // Unchanged-frame suppression (M11.4 replaces this with
        // per-span delta encoding). A buffer with an empty scoped set
        // still suppresses correctly: the first empty frame ships once
        // (clearing any prior styling on the frontend), subsequent
        // identical empty frames are squelched.
        if let Some((last_gen, last_spans)) = self.last_sent.get(&vp.buffer_id)
            && *last_gen == generation
            && *last_spans == spans
        {
            return Vec::new();
        }
        self.last_sent
            .insert(vp.buffer_id, (generation, spans.clone()));

        vec![InstanceMessage::StyleSpans {
            buffer_id: vp.buffer_id,
            generation,
            spans,
        }]
    }

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

    #[test]
    fn emits_nothing_before_viewport_declared() {
        let mut s = SemanticRenderState::new();
        assert!(
            s.render_frame(&empty_state()).is_empty(),
            "no StyleSpans may be emitted before the frontend declares a viewport"
        );
    }

    #[test]
    fn after_viewport_emits_style_spans_message_then_suppresses() {
        let state = empty_state();
        let mut s = SemanticRenderState::new();
        // Pick whatever buffer the fresh editor's active window holds.
        let buffer_id = {
            let core = state.core.borrow();
            core.active_window().buffer_id
        };
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 4096 }, 0);

        let first = s.render_frame(&state);
        assert_eq!(first.len(), 1, "first post-viewport frame emits once");
        match &first[0] {
            InstanceMessage::StyleSpans {
                buffer_id: b,
                generation,
                ..
            } => {
                assert_eq!(*b, buffer_id);
                assert_eq!(*generation, 0, "fresh scratch buffer has generation 0");
            }
            other => panic!("expected StyleSpans, got {other:?}"),
        }

        // Nothing changed → byte-identical frame is suppressed.
        assert!(
            s.render_frame(&state).is_empty(),
            "an unchanged frame must be suppressed"
        );
    }

    #[test]
    fn viewport_with_zero_width_range_yields_empty_span_set() {
        let state = empty_state();
        let mut s = SemanticRenderState::new();
        let buffer_id = {
            let core = state.core.borrow();
            core.active_window().buffer_id
        };
        // Degenerate viewport: end <= start after clamping.
        s.set_viewport(buffer_id, ByteRange { start: 10, end: 10 }, 0);
        let msgs = s.render_frame(&state);
        // First frame still ships once (clears any prior styling),
        // carrying an empty span set.
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            InstanceMessage::StyleSpans { spans, .. } => assert!(spans.is_empty()),
            other => panic!("expected StyleSpans, got {other:?}"),
        }
    }

    #[test]
    fn semantic_state_default_constructs() {
        // The dispatcher relies on `Default`/`new` parity.
        let _ = SemanticRenderState::default();
        let _ = SemanticRenderState::new();
    }

    #[test]
    fn sibling_of_render_state_reads_same_editor_state() {
        // Documents the M11.2 contract: a grid RenderState and a
        // SemanticRenderState observe the same EditorState without
        // interfering — the dispatcher selects the projection per
        // session, not per buffer.
        let state = empty_state();
        let mut grid = RenderState::new(CellSize::new(24, 80));
        let mut sem = SemanticRenderState::new();
        let buffer_id = {
            let core = state.core.borrow();
            core.active_window().buffer_id
        };
        sem.set_viewport(buffer_id, ByteRange { start: 0, end: 80 }, 0);

        let grid_msgs = grid.render_frame(&state, &[]);
        let sem_msgs = sem.render_frame(&state);
        assert!(
            matches!(grid_msgs[0], InstanceMessage::CellDelta { .. }),
            "grid projection still produces CellDelta"
        );
        assert!(
            sem_msgs
                .iter()
                .all(|m| matches!(m, InstanceMessage::StyleSpans { .. })),
            "semantic projection produces only StyleSpans, never CellDelta"
        );
        let _ = FrontendId::LOCAL; // import anchor for future fid-scoped tests
    }
}
