//! Source hit testing and selection use the same shaped row as the caret.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::{
    Bounds, Div, Pixels, Point, ShapedLine, SharedString, canvas, div, fill, point, prelude::*, px,
    rgb, size,
};
use pmacs_protocol::{ByteRange, DecorationKind, DecorationSegment};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct Selection(Vec<ByteRange>);

impl Selection {
    pub fn apply(&mut self, full: bool, segments: Vec<DecorationSegment>) -> bool {
        let mut ranges = if full { Vec::new() } else { self.0.clone() };
        for segment in segments {
            let dirty = segment.range;
            ranges = ranges
                .into_iter()
                .flat_map(|range| {
                    [
                        ByteRange {
                            start: range.start,
                            end: range.end.min(dirty.start),
                        },
                        ByteRange {
                            start: range.start.max(dirty.end),
                            end: range.end,
                        },
                    ]
                })
                .filter(|r| r.start < r.end)
                .collect();
            ranges.extend(
                segment
                    .decorations
                    .into_iter()
                    .filter(|d| d.kind == DecorationKind::Selection)
                    .map(|d| d.range),
            );
        }
        ranges.sort_unstable_by_key(|r| r.start);
        ranges.dedup_by(|next, prev| {
            if next.start <= prev.end {
                prev.end = prev.end.max(next.end);
                true
            } else {
                false
            }
        });
        if ranges == self.0 {
            return false;
        }
        self.0 = ranges;
        true
    }

    fn on_row(&self, start: u64, len: usize) -> Vec<std::ops::Range<usize>> {
        let end = start + len as u64;
        self.0
            .iter()
            .filter(|r| r.start < end && r.end > start)
            .map(|r| (r.start.saturating_sub(start) as usize)..((r.end.min(end) - start) as usize))
            .collect()
    }
}

struct Projection {
    text: SharedString,
    // ASCII rows need no table. Other rows keep source and display offsets
    // at grapheme boundaries, including both ends of each expanded tab.
    stops: Vec<(usize, usize)>,
}
impl Projection {
    fn new(mut source: String) -> Self {
        if source.ends_with('\n') {
            source.pop();
            if source.ends_with('\r') {
                source.pop();
            }
        }
        if source.is_ascii() && !source.contains('\t') {
            return Self {
                text: source.into(),
                stops: Vec::new(),
            };
        }
        let mut text = String::with_capacity(source.len());
        let mut stops = Vec::new();
        let mut column = 0;
        for (i, cluster) in source.grapheme_indices(true) {
            stops.push((i, text.len()));
            for ch in cluster.chars() {
                let next = pmacs_protocol::columns::advance_char(column, ch);
                if ch == '\t' {
                    text.extend(std::iter::repeat_n(' ', (next - column) as usize));
                } else {
                    text.push(ch);
                }
                column = next;
            }
        }
        stops.push((source.len(), text.len()));
        Self {
            text: text.into(),
            stops,
        }
    }

    fn display(&self, source: usize) -> usize {
        if self.stops.is_empty() {
            return source.min(self.text.len());
        }
        let i = self.stops.partition_point(|&(s, _)| s <= source) - 1;
        let (s, d) = self.stops[i];
        (d + source - s).min(self.text.len())
    }

    fn hit(&self, line: &ShapedLine, x: Pixels) -> usize {
        // Find the glyph first, then choose the nearer legal insertion edge.
        let index = line.index_for_x(x).unwrap_or(self.text.len());
        let (left, right) = if self.stops.is_empty() {
            (
                (index, index),
                (
                    (index + 1).min(self.text.len()),
                    (index + 1).min(self.text.len()),
                ),
            )
        } else {
            let i = self.stops.partition_point(|&(_, d)| d <= index) - 1;
            (self.stops[i], self.stops[(i + 1).min(self.stops.len() - 1)])
        };
        if x - line.x_for_index(left.1) > line.x_for_index(right.1) - x {
            right.0
        } else {
            left.0
        }
    }
}

struct PaintedRow {
    visible: Bounds<Pixels>,
    origin: Pixels,
    start: u64,
    projection: Projection,
    line: ShapedLine,
}

#[derive(Clone, Default)]
pub struct PaintedRows {
    rows: Rc<RefCell<Vec<PaintedRow>>>,
    // Retain the active line's visible suffix while the caret is inside it.
    horizontal: Rc<Cell<Option<(u64, Pixels)>>>,
}

impl PaintedRows {
    pub fn clear(&self) {
        self.rows.borrow_mut().clear();
    }

    pub fn byte(&self, position: Point<Pixels>, clamp: bool) -> Option<u64> {
        let rows = self.rows.borrow();
        let row = rows
            .iter()
            .find(|r| position.y >= r.visible.top() && position.y < r.visible.bottom())
            .or_else(|| {
                if !clamp {
                    None
                } else {
                    rows.iter().min_by_key(|r| {
                        (r.visible.top() - position.y).max(position.y - r.visible.bottom())
                    })
                }
            })?;
        Some(row.start + row.projection.hit(&row.line, position.x - row.origin) as u64)
    }

    pub fn row(
        &self,
        source: String,
        start: u64,
        caret: Option<usize>,
        selection: &Selection,
    ) -> Div {
        let selected = selection.on_row(start, source.len());
        let projection = Projection::new(source);
        let painted = self.clone();
        let horizontal = self.horizontal.clone();
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .cursor_text()
            .child(
                canvas(
                    move |bounds, window, _| {
                        let style = window.text_style();
                        let line = window.text_system().shape_line(
                            projection.text.clone(),
                            style.font_size.to_pixels(window.rem_size()),
                            &[style.to_run(projection.text.len())],
                            None,
                        );
                        let caret = caret.map(|i| line.x_for_index(projection.display(i)));
                        let shift = caret.map_or(px(0.), |x| {
                            let previous = horizontal
                                .get()
                                .filter(|(row, _)| *row == start)
                                .map_or(px(0.), |(_, shift)| shift);
                            let width = bounds.size.width;
                            let shift = previous
                                .min(x)
                                .min(line.width + px(2.) - width)
                                .max(x + px(2.) - width)
                                .max(px(0.));
                            horizontal.set(Some((start, shift)));
                            shift
                        });
                        (projection, line, caret, bounds.left() - shift)
                    },
                    move |bounds, (projection, line, caret, origin), window, cx| {
                        let visible = bounds.intersect(&window.content_mask().bounds);
                        if visible.is_empty() {
                            return;
                        }
                        for range in selected {
                            let left = line.x_for_index(projection.display(range.start));
                            let mut right = line.x_for_index(projection.display(range.end));
                            let source_end = projection
                                .stops
                                .last()
                                .map_or(projection.text.len(), |s| s.0);
                            if range.end > source_end {
                                right += px(8.);
                            }
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(origin + left, bounds.top()),
                                    size(right - left, bounds.size.height),
                                ),
                                rgb(0x354968),
                            ));
                        }
                        let _ =
                            line.paint(point(origin, bounds.top()), bounds.size.height, window, cx);
                        if let Some(x) = caret {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(origin + x, bounds.top() + px(2.)),
                                    size(px(1.5), bounds.size.height - px(4.)),
                                ),
                                rgb(0xf6dba2),
                            ));
                        }
                        painted.rows.borrow_mut().push(PaintedRow {
                            visible,
                            origin,
                            start,
                            projection,
                            line,
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pmacs_protocol::Decoration;

    #[test]
    fn tab_stops_and_graphemes_map_to_original_utf8_and_exclude_crlf() {
        let source = "界\t👩‍💻e\u{301}\r\n";
        let line = Projection::new(source.into());
        assert_eq!(line.text.as_ref(), "界      👩‍💻e\u{301}");
        assert_eq!(line.stops, [(0, 0), (3, 3), (4, 9), (15, 20), (18, 23)]);
        for (source, display) in [
            (0, 0),
            (3, 3),
            (4, 9),
            (15, 20),
            (16, 21),
            (18, 23),
            (19, 23),
        ] {
            assert_eq!(line.display(source), display);
        }
        let empty = Projection::new("\r\n".into());
        assert_eq!(empty.display(2), 0);
    }

    #[test]
    fn selection_patches_keep_both_edges_and_span_empty_rows() {
        let bytes = |start, end| ByteRange { start, end };
        let segment = |range, selected: Option<ByteRange>| DecorationSegment {
            range,
            decorations: selected
                .into_iter()
                .map(|range| Decoration {
                    kind: DecorationKind::Selection,
                    range,
                })
                .collect(),
        };
        let mut selection = Selection::default();
        selection.apply(true, vec![segment(bytes(0, 30), Some(bytes(2, 24)))]);
        selection.apply(false, vec![segment(bytes(5, 20), None)]);
        assert_eq!(selection.0, [bytes(2, 5), bytes(20, 24)]);
        selection.apply(false, vec![segment(bytes(5, 20), Some(bytes(5, 20)))]);
        assert_eq!(selection.0, [bytes(2, 24)]);
        let blank = selection.on_row(9, 1);
        assert_eq!(blank.len(), 1);
        assert_eq!(blank[0], 0..1);
        assert!(selection.on_row(24, 5).is_empty());
        assert!(selection.apply(true, vec![]));
    }
}
