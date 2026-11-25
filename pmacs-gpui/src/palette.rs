use gpui::{
    Div, ElementId, FontWeight, Pixels, ScrollStrategy, SharedString, UniformListScrollHandle,
    Window, div, prelude::*, px, rgb, uniform_list,
};
use gpui_component::list::ListItem;
use pmacs_protocol::MinibufferRow;

use crate::text_with_caret;

pub struct Prompt {
    pub label: SharedString,
    pub input: SharedString,
    pub cursor: u32,
    pub rows: Vec<MinibufferRow>,
    pub selected: Option<u32>,
    pub total: u32,
}

impl Prompt {
    pub fn show(
        &self,
        window: &Window,
        scroll: &UniformListScrollHandle,
        cx: &gpui::Context<crate::FileView>,
    ) -> gpui::Stateful<Div> {
        let viewport = window.viewport_size();
        let width = (viewport.width - px(24.)).max(px(1.)).min(px(760.));
        let narrow = width < px(540.);
        let row_height = if narrow { px(54.) } else { px(34.) };
        let available = (viewport.height - px(120.)).max(row_height);
        let cursor = self
            .input
            .char_indices()
            .nth(self.cursor as usize)
            .map_or(self.input.len(), |(offset, _)| offset);
        let header = div()
            .h(px(48.))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .text_size(px(16.))
            .text_color(rgb(0xeeeae3))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(0xb5b1a8))
                    .child(self.label.clone()),
            )
            .child(
                text_with_caret(self.input.clone(), Some(cursor))
                    .flex_1()
                    .min_w_0(),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(rgb(0xb5b1a8))
                    .child(format!(
                        "{} {}",
                        self.total,
                        if self.total == 1 { "match" } else { "matches" }
                    )),
            );
        let mut panel = div()
            .id("command-palette")
            .absolute()
            .occlude()
            .left((viewport.width - width) / 2.)
            .top(px(48.))
            .w(width)
            .rounded_md()
            .border_1()
            .border_color(rgb(0x58554f))
            .bg(rgb(0x232322))
            .font_family(".SystemUIFont")
            .overflow_hidden()
            .shadow_lg()
            .child(header);
        if !self.rows.is_empty() {
            let rows = uniform_list(
                "command-rows",
                self.rows.len(),
                cx.processor(move |view, range: std::ops::Range<usize>, _, _| {
                    let Some(prompt) = &view.prompt else {
                        return Vec::new();
                    };
                    range
                        .filter_map(|i| {
                            prompt.rows.get(i).map(|row| {
                                candidate(
                                    ("command-row", i).into(),
                                    &row.label,
                                    row.detail.as_deref(),
                                    prompt.selected == Some(i as u32),
                                    narrow,
                                    row_height,
                                )
                            })
                        })
                        .collect()
                }),
            )
            .h((row_height * self.rows.len() as f32).min(available))
            .track_scroll(scroll.clone());
            panel = panel.child(div().border_t_1().border_color(rgb(0x44423e)).child(rows));
        }
        panel
    }

    pub fn reveal_selected(&self, scroll: &UniformListScrollHandle) {
        if let Some(i) = self.selected
            && (i as usize) < self.rows.len()
        {
            scroll.scroll_to_item(i as usize, ScrollStrategy::Center);
        }
    }
}

pub fn candidate(
    id: ElementId,
    label: &str,
    detail: Option<&str>,
    selected: bool,
    narrow: bool,
    height: Pixels,
) -> ListItem {
    let name = div()
        .truncate()
        .font_family("JetBrains Mono")
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(0xe6e3dc))
        .when(!narrow && detail.is_some(), |d| {
            d.w(gpui::relative(0.43)).flex_shrink_0()
        })
        .child(label.to_owned());
    let content = div()
        .w_full()
        .flex()
        .overflow_hidden()
        .when(narrow, |d| d.flex_col().justify_center())
        .when(!narrow, |d| d.items_center().gap_3())
        .child(name)
        .when_some(detail, |d, detail| {
            d.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(0xbcb8b0))
                    .child(detail.to_owned()),
            )
        });
    ListItem::new(id)
        .selected(selected)
        .w_full()
        .h(height)
        .px_4()
        .py_0()
        .child(content)
}
