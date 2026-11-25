//! Lay out the daemon's candidate window against the source row that shaped its anchor.
use gpui::{Bounds, Div, MouseButton, Pixels, Point, div, point, prelude::*, px, rgb, size};
use pmacs_protocol::CompletionPopupRow;

use crate::palette::candidate;

pub struct Completion {
    pub anchor: u64,
    pub rows: Vec<CompletionPopupRow>,
    pub selected: Option<u32>,
}

impl Completion {
    pub fn show(
        &self,
        anchor: Bounds<Pixels>,
        viewport: Bounds<Pixels>,
        origin: Point<Pixels>,
    ) -> Option<Div> {
        let width = viewport.size.width.min(px(580.));
        let narrow = width < px(440.);
        let row_height = if narrow { px(44.) } else { px(30.) };
        let (bounds, count) = placement(anchor, viewport, width, row_height, self.rows.len())?;
        let first = (self.selected.unwrap_or(0) as usize)
            .min(self.rows.len() - 1)
            .saturating_sub(count - 1);
        let offset = bounds.origin - origin;
        let popup = div()
            .id("source-completion")
            .absolute()
            .occlude()
            .left(offset.x)
            .top(offset.y)
            .w(width)
            .h(bounds.size.height)
            .border_1()
            .border_color(rgb(0x58554f))
            .bg(rgb(0x232322))
            .rounded_md()
            .overflow_hidden()
            .font_family(".SystemUIFont")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children((first..first + count).map(|i| {
                let row = &self.rows[i];
                candidate(
                    ("completion-row", i).into(),
                    &row.label,
                    row.detail.as_deref().or_else(|| kind_name(row.kind)),
                    self.selected == Some(i as u32),
                    narrow,
                    row_height,
                )
            }));
        Some(div().relative().size_full().child(popup))
    }
}

fn placement(
    anchor: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    width: Pixels,
    row: Pixels,
    rows: usize,
) -> Option<(Bounds<Pixels>, usize)> {
    let gap = px(2.);
    let below = viewport.bottom() - anchor.bottom() - gap;
    let above = anchor.top() - viewport.top() - gap;
    let desired = row * rows as f32 + px(2.);
    let flip = below < desired && above > below;
    let room = if flip { above } else { below };
    let count = rows.min(((room - px(2.)) / row).floor() as usize);
    if count == 0 || width <= px(2.) {
        return None;
    }
    let height = row * count as f32 + px(2.);
    let x = anchor
        .left()
        .min(viewport.right() - width)
        .max(viewport.left());
    let y = if flip {
        anchor.top() - gap - height
    } else {
        anchor.bottom() + gap
    };
    Some((Bounds::new(point(x, y), size(width, height)), count))
}

// LSP CompletionItemKind. Unknown extension kinds can still render their label.
fn kind_name(kind: u8) -> Option<&'static str> {
    const KINDS: [&str; 25] = [
        "Text",
        "Method",
        "Function",
        "Constructor",
        "Field",
        "Variable",
        "Class",
        "Interface",
        "Module",
        "Property",
        "Unit",
        "Value",
        "Enum",
        "Keyword",
        "Snippet",
        "Color",
        "File",
        "Reference",
        "Folder",
        "Enum member",
        "Constant",
        "Struct",
        "Event",
        "Operator",
        "Type parameter",
    ];
    KINDS.get(usize::from(kind.checked_sub(1)?)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_popup_at_the_right_edge_uses_only_whole_rows_above_the_anchor() {
        let viewport = Bounds::new(point(px(8.), px(40.)), size(px(220.), px(200.)));
        let anchor = Bounds::new(point(px(210.), px(190.)), size(px(0.), px(22.)));
        let (popup, rows) = placement(anchor, viewport, px(180.), px(30.), 9).unwrap();
        assert_eq!(rows, 4);
        assert_eq!(popup.right(), viewport.right());
        assert_eq!(popup.bottom(), anchor.top() - px(2.));
        assert!(popup.top() >= viewport.top());
        assert!(placement(anchor, anchor, px(0.), px(30.), 1).is_none());
    }
}
