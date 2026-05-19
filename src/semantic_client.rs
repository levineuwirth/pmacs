// semantic_client.rs --- Headless consumer of the SemanticFrame wire (T M11.5).

//! The frontend↔instance glue for the semantic projection.
//!
//! `docs/semantic-frontend-protocol.md` deliberately moves rendering
//! correctness (shaping, wrap, hit-testing) into a GPU frontend the
//! instance test harness cannot exercise, and bounds the *testable*
//! surface to "the frontend↔instance glue, not all rendering."
//! `SemanticClient` is exactly that glue, made headless and
//! self-contained: no terminal, no GPU, no pixels.
//!
//! It composes the two replica layers a `semantic_render` session
//! needs:
//!
//! - [`BufferMirror`] — the rope replica (M10.10). The semantic frame
//!   ships *no text*; the client holds the document locally via
//!   `BufferSnapshot` + `CrdtOp`, exactly as the grid TUI does.
//! - A [`SemanticModel`] per family — the *interpretation* layer:
//!   byte-anchored styling / decorations, reconstructed from the
//!   `full` + dirty-segment deltas (M11.4).
//!
//! The client also produces the one frontend→instance message the
//! protocol adds — [`FrontendEvent::Viewport`] — declaring the byte
//! range it has "on screen" so the instance scopes its projection.
//!
//! Read-back accessors (`text`, `effective_style_at`,
//! `decoration_kinds_at`) exist so a test can assert the
//! reconstruction equals the instance's intent — the
//! "reconstruction-equivalence" golden discipline (no snapshot crate;
//! matches the repo's explicit-assertion style).

use std::collections::HashMap;

use crate::buffer::BufferId;
use crate::buffer_mirror::BufferMirror;
use crate::cell::Style;
use crate::overlay::merge_styles;
use crate::protocol::{
    ByteRange, Decoration, DecorationKind, DecorationSegment, FrontendEvent, FrontendId,
    InstanceMessage, StyleSegment, StyleSpan,
};

/// An item the model can restrict to a sub-range. `range` is where it
/// applies; `clipped` is the item narrowed to `bounds` (or `None`
/// when disjoint). The semantic frame's items are byte-anchored, so
/// both families implement this uniformly.
trait Clip: Clone {
    fn range(&self) -> ByteRange;
    fn clipped(&self, bounds: ByteRange) -> Option<Self>;
}

fn intersect(a: ByteRange, b: ByteRange) -> Option<ByteRange> {
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    (end > start).then_some(ByteRange { start, end })
}

impl Clip for StyleSpan {
    fn range(&self) -> ByteRange {
        self.range
    }
    fn clipped(&self, bounds: ByteRange) -> Option<Self> {
        intersect(self.range, bounds).map(|range| Self {
            range,
            style: self.style,
        })
    }
}

impl Clip for Decoration {
    fn range(&self) -> ByteRange {
        self.range
    }
    fn clipped(&self, bounds: ByteRange) -> Option<Self> {
        intersect(self.range, bounds).map(|range| Self {
            range,
            kind: self.kind,
        })
    }
}

/// One reconstructed dirty region. The M11.4 contract — *each segment
/// carries every current item intersecting its range* — makes a tile
/// self-contained: rendering any byte in `range` consults only this
/// tile's `items`, never a neighbour's. That is what lets incremental
/// application be a clean per-tile replacement instead of fragile
/// cross-span surgery.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Tile<T> {
    range: ByteRange,
    items: Vec<T>,
}

/// One family's reconstructed view of one buffer: disjoint tiles
/// ordered by start. Bytes covered by no tile have no styling /
/// decoration (default), exactly as the instance intends for regions
/// outside the declared viewport.
struct SemanticModel<T> {
    tiles: Vec<Tile<T>>,
}

// Manual `Default` — the derive would wrongly require `T: Default`
// (a `StyleSpan`/`Decoration` has no meaningful default); an empty
// model is just no tiles regardless of `T`.
impl<T> Default for SemanticModel<T> {
    fn default() -> Self {
        Self { tiles: Vec::new() }
    }
}

impl<T: Clip> SemanticModel<T> {
    /// Apply one frame. `full` discards everything first (resync);
    /// otherwise each segment replaces only its own byte range —
    /// tiles straddling a segment are split, keeping the parts
    /// outside it (clipped), and the segment's items become the new
    /// tile for the region.
    fn apply(&mut self, full: bool, segments: &[(ByteRange, Vec<T>)]) {
        if full {
            self.tiles = segments
                .iter()
                .map(|(range, items)| Tile {
                    range: *range,
                    items: items.clone(),
                })
                .collect();
        } else {
            for (range, items) in segments {
                self.replace_region(*range, items.clone());
            }
        }
        self.tiles.sort_by_key(|t| (t.range.start, t.range.end));
    }

    fn replace_region(&mut self, region: ByteRange, items: Vec<T>) {
        let mut next: Vec<Tile<T>> = Vec::with_capacity(self.tiles.len() + 1);
        for t in std::mem::take(&mut self.tiles) {
            if intersect(t.range, region).is_none() {
                next.push(t);
                continue;
            }
            // Keep the parts of `t` outside `region`, each carrying
            // only the items that survive the narrower range.
            if t.range.start < region.start {
                let left = ByteRange {
                    start: t.range.start,
                    end: region.start,
                };
                next.push(Tile {
                    range: left,
                    items: t.items.iter().filter_map(|i| i.clipped(left)).collect(),
                });
            }
            if t.range.end > region.end {
                let right = ByteRange {
                    start: region.end,
                    end: t.range.end,
                };
                next.push(Tile {
                    range: right,
                    items: t.items.iter().filter_map(|i| i.clipped(right)).collect(),
                });
            }
            // The overlapped middle is dropped — `items` re-supplies it.
        }
        next.push(Tile {
            range: region,
            items,
        });
        self.tiles = next;
    }

    /// Items covering `byte`, in instance order (the order they were
    /// shipped — wider-first for styling, so a fold via
    /// [`merge_styles`] reproduces the grid path's layering).
    fn items_at(&self, byte: u64) -> impl Iterator<Item = &T> {
        self.tiles
            .iter()
            .find(|t| t.range.start <= byte && byte < t.range.end)
            .into_iter()
            .flat_map(move |t| {
                t.items
                    .iter()
                    .filter(move |i| i.range().start <= byte && byte < i.range().end)
            })
    }

    fn tile_ranges(&self) -> Vec<ByteRange> {
        self.tiles.iter().map(|t| t.range).collect()
    }
}

/// A headless `semantic_render` session: rope replica + the styling
/// and decoration interpretation layers, plus the `Viewport` event it
/// emits. Drive it by feeding every [`InstanceMessage`] through
/// [`Self::apply`]; read it back through the accessors.
pub struct SemanticClient {
    frontend_id: FrontendId,
    mirror: BufferMirror,
    styles: HashMap<BufferId, SemanticModel<StyleSpan>>,
    decos: HashMap<BufferId, SemanticModel<Decoration>>,
}

impl SemanticClient {
    /// Construct a client for the session assigned `frontend_id`
    /// (the id the daemon stamped in `Hello`).
    #[must_use]
    pub fn new(frontend_id: FrontendId) -> Self {
        Self {
            frontend_id,
            mirror: BufferMirror::new(frontend_id),
            styles: HashMap::new(),
            decos: HashMap::new(),
        }
    }

    /// The session's assigned frontend id.
    #[must_use]
    pub fn frontend_id(&self) -> FrontendId {
        self.frontend_id
    }

    /// Build the [`FrontendEvent::Viewport`] declaring `visible` for
    /// `buffer_id`. The caller writes it to the daemon; the instance
    /// scopes its projection to this range. `generation` is the CRDT
    /// version the frontend computed the range against (M11.4 records
    /// it for the future viewport-race refinement).
    #[must_use]
    pub fn viewport_event(
        &self,
        buffer_id: BufferId,
        visible: ByteRange,
        generation: u64,
    ) -> FrontendEvent {
        FrontendEvent::Viewport {
            frontend_id: self.frontend_id,
            buffer_id,
            visible,
            generation,
        }
    }

    /// Route one instance message into the replica/interpretation
    /// layers. Unrelated variants (grid `CellDelta`/`Cursor`,
    /// presence, and the not-yet-produced adornment/fold/resource
    /// families) are ignored — a semantic session lays out locally
    /// and never consumes the grid projection.
    pub fn apply(&mut self, msg: &InstanceMessage) {
        match msg {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => {
                // `AlreadyInitialized` means a duplicate bootstrap for
                // a buffer we already mirror — benign for a consumer.
                let _ = self.mirror.init_from_snapshot(*buffer_id, crdt_snapshot);
            }
            InstanceMessage::CrdtOp { buffer_id, op } => {
                // A pure consumer never edits, so it is never the
                // op's source — no echo to filter (the daemon also
                // excludes the sender). Drop a non-applying op
                // silently, as the test Observer does.
                let _ = self.mirror.apply_remote_op(*buffer_id, &op.bytes);
            }
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => {
                self.mirror
                    .set_cursor_byte_pos(*buffer_id, *byte_pos as usize);
            }
            InstanceMessage::StyleSpans {
                buffer_id,
                full,
                segments,
                ..
            } => {
                let segs: Vec<(ByteRange, Vec<StyleSpan>)> = segments
                    .iter()
                    .map(|s: &StyleSegment| (s.range, s.spans.clone()))
                    .collect();
                self.styles
                    .entry(*buffer_id)
                    .or_default()
                    .apply(*full, &segs);
            }
            InstanceMessage::Decorations {
                buffer_id,
                full,
                segments,
                ..
            } => {
                let segs: Vec<(ByteRange, Vec<Decoration>)> = segments
                    .iter()
                    .map(|s: &DecorationSegment| (s.range, s.decorations.clone()))
                    .collect();
                self.decos
                    .entry(*buffer_id)
                    .or_default()
                    .apply(*full, &segs);
            }
            // Grid projection, presence, and the honest-stub families
            // (InlineAdornments / BlockAdornments / FoldState /
            // ResourceOffer) — a semantic session does not consume
            // these. ModeLine / Signal / Goodbye are session control,
            // handled by the attach loop, not the model.
            _ => {}
        }
    }

    /// The reconstructed document text for `buffer_id` (the rope
    /// replica materialized), or `None` if not yet bootstrapped.
    #[must_use]
    pub fn text(&self, buffer_id: BufferId) -> Option<String> {
        self.mirror.materialize(buffer_id)
    }

    /// Whether the rope replica for `buffer_id` has been bootstrapped.
    #[must_use]
    pub fn is_ready(&self, buffer_id: BufferId) -> bool {
        self.mirror.is_ready(buffer_id)
    }

    /// The cursor byte position the instance last reported.
    #[must_use]
    pub fn cursor_byte_pos(&self, buffer_id: BufferId) -> Option<usize> {
        self.mirror.cursor_byte_pos(buffer_id)
    }

    /// The effective style at `byte`: every reconstructed span
    /// covering it, folded via [`merge_styles`] in instance order.
    /// `Style::default()` when nothing covers it (outside the
    /// declared viewport, or no styling there).
    #[must_use]
    pub fn effective_style_at(&self, buffer_id: BufferId, byte: u64) -> Style {
        self.styles.get(&buffer_id).map_or_else(Style::default, |m| {
            m.items_at(byte)
                .fold(Style::default(), |acc, s| merge_styles(acc, s.style))
        })
    }

    /// The decoration kinds covering `byte`, in instance order
    /// (duplicates preserved — a byte can carry, e.g., both a
    /// selection and a diagnostic).
    #[must_use]
    pub fn decoration_kinds_at(&self, buffer_id: BufferId, byte: u64) -> Vec<DecorationKind> {
        self.decos.get(&buffer_id).map_or_else(Vec::new, |m| {
            m.items_at(byte).map(|d| d.kind).collect()
        })
    }

    /// Reconstructed styling tile ranges for `buffer_id` — for
    /// invariant assertions (disjointness, in-viewport bounds).
    #[must_use]
    pub fn style_tile_ranges(&self, buffer_id: BufferId) -> Vec<ByteRange> {
        self.styles
            .get(&buffer_id)
            .map(SemanticModel::tile_ranges)
            .unwrap_or_default()
    }

    /// Reconstructed decoration tile ranges for `buffer_id`.
    #[must_use]
    pub fn decoration_tile_ranges(&self, buffer_id: BufferId) -> Vec<ByteRange> {
        self.decos
            .get(&buffer_id)
            .map(SemanticModel::tile_ranges)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn br(start: u64, end: u64) -> ByteRange {
        ByteRange { start, end }
    }

    fn styled(fg_bold: bool) -> Style {
        Style {
            bold: fg_bold,
            ..Style::default()
        }
    }

    fn span(start: u64, end: u64, bold: bool) -> StyleSpan {
        StyleSpan {
            range: br(start, end),
            style: styled(bold),
        }
    }

    fn deco(start: u64, end: u64, kind: DecorationKind) -> Decoration {
        Decoration {
            range: br(start, end),
            kind,
        }
    }

    #[test]
    fn full_frame_replaces_the_whole_model() {
        let mut m: SemanticModel<StyleSpan> = SemanticModel::default();
        m.apply(true, &[(br(0, 10), vec![span(2, 5, true)])]);
        assert_eq!(m.tile_ranges(), vec![br(0, 10)]);
        // A second full frame discards the first entirely.
        m.apply(true, &[(br(0, 4), vec![span(0, 4, false)])]);
        assert_eq!(m.tile_ranges(), vec![br(0, 4)]);
        assert_eq!(m.items_at(2).count(), 1);
        assert!(m.items_at(8).next().is_none(), "byte 8 no longer covered");
    }

    #[test]
    fn incremental_segment_splits_a_straddling_tile_and_keeps_the_edges() {
        let mut m: SemanticModel<StyleSpan> = SemanticModel::default();
        // One wide tile spanning [0,30) with a span over [0,30).
        m.apply(true, &[(br(0, 30), vec![span(0, 30, true)])]);
        // A dirty segment repaints the middle [10,20).
        m.apply(false, &[(br(10, 20), vec![span(10, 20, false)])]);
        // Edges [0,10) and [20,30) survive (clipped), middle replaced.
        assert_eq!(
            m.tile_ranges(),
            vec![br(0, 10), br(10, 20), br(20, 30)],
            "straddling tile split into left edge / new middle / right edge"
        );
        // Edge styling preserved (bold); middle replaced (not bold).
        assert!(m.items_at(5).next().unwrap().style.bold);
        assert!(!m.items_at(15).next().unwrap().style.bold);
        assert!(m.items_at(25).next().unwrap().style.bold);
    }

    #[test]
    fn bytes_outside_all_tiles_have_default_style() {
        let c = SemanticClient::new(FrontendId(7));
        let b = BufferId::next();
        assert_eq!(c.effective_style_at(b, 3), Style::default());
        assert!(c.decoration_kinds_at(b, 3).is_empty());
    }

    #[test]
    fn overlapping_spans_fold_in_order_via_merge_styles() {
        let mut m: SemanticModel<StyleSpan> = SemanticModel::default();
        // Wider span (bold) then a nested non-bold span — instance
        // ships wider-first; merge_styles overlays in that order.
        let wide = StyleSpan {
            range: br(0, 10),
            style: Style {
                bold: true,
                ..Style::default()
            },
        };
        let inner = StyleSpan {
            range: br(4, 6),
            style: Style {
                italic: true,
                ..Style::default()
            },
        };
        m.apply(true, &[(br(0, 10), vec![wide, inner])]);
        let folded = m
            .items_at(5)
            .fold(Style::default(), |acc, s| merge_styles(acc, s.style));
        assert!(folded.bold && folded.italic, "both layers apply at byte 5");
        let only_wide = m
            .items_at(1)
            .fold(Style::default(), |acc, s| merge_styles(acc, s.style));
        assert!(only_wide.bold && !only_wide.italic);
    }

    #[test]
    fn decoration_model_tracks_kinds_at_byte() {
        let mut m: SemanticModel<Decoration> = SemanticModel::default();
        m.apply(
            true,
            &[(
                br(0, 20),
                vec![
                    deco(2, 8, DecorationKind::Selection),
                    deco(5, 6, DecorationKind::DiagnosticError),
                ],
            )],
        );
        let at5: Vec<_> = m.items_at(5).map(|d| d.kind).collect();
        assert_eq!(
            at5,
            vec![DecorationKind::Selection, DecorationKind::DiagnosticError]
        );
        assert_eq!(
            m.items_at(3).map(|d| d.kind).collect::<Vec<_>>(),
            vec![DecorationKind::Selection]
        );
        assert!(m.items_at(15).next().is_none());
    }

    #[test]
    fn client_ignores_grid_and_stub_families() {
        let mut c = SemanticClient::new(FrontendId(2));
        let b = BufferId::next();
        // None of these should panic or affect the model.
        c.apply(&InstanceMessage::Cursor(None));
        c.apply(&InstanceMessage::FoldState {
            buffer_id: b,
            folds: vec![br(0, 1)],
        });
        c.apply(&InstanceMessage::ResourceOffer {
            handle: 1,
            mime: "image/png".into(),
            body: crate::protocol::ResourceBody::Inline(vec![1, 2]),
        });
        assert!(c.style_tile_ranges(b).is_empty());
        assert!(c.text(b).is_none());
    }

    #[test]
    fn viewport_event_carries_the_sessions_fid() {
        let c = SemanticClient::new(FrontendId(9));
        let b = BufferId::next();
        match c.viewport_event(b, br(0, 64), 3) {
            FrontendEvent::Viewport {
                frontend_id,
                buffer_id,
                visible,
                generation,
            } => {
                assert_eq!(frontend_id, FrontendId(9));
                assert_eq!(buffer_id, b);
                assert_eq!(visible, br(0, 64));
                assert_eq!(generation, 3);
            }
            other => panic!("expected Viewport, got {other:?}"),
        }
    }
}
