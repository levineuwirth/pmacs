use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, ContentMask, Context, Div, FontStyle, FontWeight, Hsla, MouseButton, Pixels, Point,
    Size, Window, canvas, div, fill, font, point, prelude::*, px, rgb, size,
};
use pmacs_attach_client::{
    AttachClient,
    cell_grid::{
        CellRun, Rgb, TerminalPaintPlan, TerminalPalette, hit_test_cell, panel_cell_capacity,
    },
};
use pmacs_protocol::{
    BufferId, CellCoord, CellSize, Modifiers, MouseButton as WireButton, MouseKind,
    PANEL_MAPPING_MIN_VERSION, PANEL_MIN_VERSION, PanelFramePayload, TransportError,
    UnderlineStyle,
};

use crate::{FileView, input};

const FONT: f32 = 14.;
const ROW: f32 = 20.;
const TITLE: f32 = 32.;
const PAD: f32 = 12.;
const BACKGROUND: u32 = 0x181818;
const PALETTE: TerminalPalette = TerminalPalette {
    default_fg: [216, 212, 205],
    default_bg: [24, 24, 24],
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mapping {
    buffer: BufferId,
    panel: u64,
    geometry: u64,
    generation: Option<u64>,
}

impl Mapping {
    fn send(
        self,
        client: &AttachClient,
        cell: CellCoord,
        kind: MouseKind,
        mods: Modifiers,
    ) -> Result<(), TransportError> {
        match self.generation {
            Some(generation) => client.send_panel_pointer_mapped(
                self.geometry,
                self.panel,
                self.buffer,
                cell,
                kind,
                mods,
                generation,
            ),
            None => {
                client.send_panel_pointer(self.geometry, self.panel, self.buffer, cell, kind, mods)
            }
        }
    }
}

struct Presentation {
    mapping: Mapping,
    focused: bool,
    plan: Rc<TerminalPaintPlan>,
}

pub struct Panel {
    version: u32,
    geometry: u64,
    surface: Option<(Size<Pixels>, f32)>,
    total: CellSize,
    advance: Pixels,
    generation: u64,
    visible: Option<Presentation>,
    wheel_remainder: f32,
}

impl Panel {
    pub fn new(version: u32) -> Self {
        Self {
            version,
            geometry: 0,
            surface: None,
            total: CellSize::default(),
            advance: px(0.),
            generation: 0,
            visible: None,
            wheel_remainder: 0.,
        }
    }

    pub fn declare(
        &mut self,
        window: &Window,
        client: &AttachClient,
    ) -> Result<(), TransportError> {
        let surface = (window.viewport_size(), window.scale_factor());
        if self.version < PANEL_MIN_VERSION || self.surface == Some(surface) {
            return Ok(());
        }
        let ts = window.text_system();
        let advance = ts
            .em_advance(ts.resolve_font(&font("JetBrains Mono")), px(FONT))
            .unwrap_or_default();
        // Leave room for the source header, the panel title and the status message.
        // A fixed reservation avoids declaring a different grid as the panel opens.
        let total = panel_cell_capacity(
            f32::from(surface.0.width) - PAD * 2.,
            f32::from(surface.0.height) - 40. - TITLE - 26.,
            advance.into(),
            ROW,
        )
        .unwrap_or_default();
        if self.surface.is_none_or(|(_, scale)| scale != surface.1)
            || self.advance != advance
            || self.total != total
        {
            let Some(epoch) = self.geometry.checked_add(1) else {
                // Disown the old cell range so a late frame cannot restore it.
                self.total = CellSize::default();
                self.visible = None;
                return Ok(());
            };
            client.send_frontend_cell_geometry(epoch, total)?;
            self.geometry = epoch;
            self.total = total;
            self.advance = advance;
            self.visible = None;
            self.wheel_remainder = 0.;
        }
        self.surface = Some(surface);
        Ok(())
    }

    pub fn apply(&mut self, payload: PanelFramePayload) -> bool {
        let (frame, generation) = match payload {
            PanelFramePayload::Absent => {
                self.wheel_remainder = 0.;
                return self.visible.take().is_some();
            }
            PanelFramePayload::Present(frame)
                if (PANEL_MIN_VERSION..PANEL_MAPPING_MIN_VERSION).contains(&self.version) =>
            {
                (frame, None)
            }
            PanelFramePayload::PresentMapped {
                frame,
                mapping_generation,
            } if self.version >= PANEL_MAPPING_MIN_VERSION
                && mapping_generation > 0
                && mapping_generation >= self.generation =>
            {
                (frame, Some(mapping_generation))
            }
            _ => return false,
        };
        if frame.geometry_epoch != self.geometry
            || frame.size.cols != self.total.cols
            || frame.size.rows > self.total.rows
            || frame.validate().is_err()
        {
            return false;
        }
        let mapping = Mapping {
            buffer: frame.buffer_id,
            panel: frame.panel_epoch,
            geometry: frame.geometry_epoch,
            generation,
        };
        if self.visible.as_ref().is_none_or(|old| {
            old.mapping.panel != mapping.panel || old.mapping.buffer != mapping.buffer
        }) {
            self.wheel_remainder = 0.;
        }
        self.generation = generation.unwrap_or_default();
        self.visible = Some(Presentation {
            mapping,
            focused: frame.focused,
            plan: Rc::new(TerminalPaintPlan::build_grid(
                frame.size,
                &frame.cells,
                frame.cursor,
                PALETTE,
            )),
        });
        true
    }

    pub fn is_visible(&self) -> bool {
        self.visible.is_some()
    }

    pub fn focused(&self) -> bool {
        self.visible.as_ref().is_some_and(|p| p.focused)
    }

    pub fn show(&self, active: bool, cx: &Context<FileView>) -> Option<Div> {
        let p = self.visible.as_ref()?;
        let mapping = p.mapping;
        let grid = p.plan.size;
        let advance = self.advance;
        let plan = p.plan.clone();
        let caret = p.focused && active;
        let bounds = Rc::new(Cell::new(None));
        let painted = bounds.clone();
        let surface = canvas(
            move |area, _, _| painted.set(Some(area)),
            move |area, (), window, cx| paint(&plan, area, advance, caret, window, cx),
        )
        .size_full();
        let hit = move |position: Point<Pixels>| {
            let area: Bounds<Pixels> = bounds.get()?;
            hit_test_cell(
                position.x.into(),
                position.y.into(),
                (area.left().into(), area.top().into()),
                advance.into(),
                ROW,
                grid,
            )
        };
        let click_hit = hit.clone();
        let body = div()
            .w_full()
            .h(px(ROW * grid.rows as f32))
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    if let Some(cell) = click_hit(event.position) {
                        view.panel_input(
                            mapping,
                            cell,
                            input::modifiers(event.modifiers),
                            &[
                                MouseKind::Down(WireButton::Left),
                                MouseKind::Up(WireButton::Left),
                            ],
                            cx,
                        );
                    }
                }),
            )
            .on_scroll_wheel(
                cx.listener(move |view, event: &gpui::ScrollWheelEvent, _, cx| {
                    let Some(cell) = hit(event.position) else {
                        return;
                    };
                    if !view.accepts_panel_input(mapping) {
                        return;
                    }
                    let delta = f32::from(event.delta.pixel_delta(px(ROW)).y) / ROW;
                    view.panel.wheel_remainder += delta;
                    if view.panel.wheel_remainder.abs() >= 1. {
                        let kind = if view.panel.wheel_remainder > 0. {
                            MouseKind::ScrollUp
                        } else {
                            MouseKind::ScrollDown
                        };
                        // One daemon wheel gesture scrolls several rows and may publish a new mapping.
                        view.panel.wheel_remainder = view.panel.wheel_remainder.fract();
                        view.panel_input(
                            mapping,
                            cell,
                            input::modifiers(event.modifiers),
                            &[kind],
                            cx,
                        );
                    }
                }),
            )
            .child(surface);
        let title = div()
            .h(px(TITLE))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .border_l_2()
            .border_color(rgb(if p.focused { 0xf6dba2 } else { 0x56544e }))
            .bg(rgb(if p.focused { 0x32302c } else { 0x222221 }))
            .font_family(".SystemUIFont")
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Output"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0xc0bcb3))
                    .child(if p.focused {
                        "C-x o   Return to source"
                    } else {
                        "C-x o   Focus output"
                    }),
            );
        Some(
            div()
                .w_full()
                .flex_shrink_0()
                .overflow_hidden()
                .border_t_1()
                .border_color(rgb(0x494742))
                .bg(rgb(BACKGROUND))
                .child(title)
                .child(div().px(px(PAD)).child(body)),
        )
    }
}

impl FileView {
    fn accepts_panel_input(&self, mapping: Mapping) -> bool {
        self.prompt.is_none()
            && self.notice.is_none()
            && self
                .panel
                .visible
                .as_ref()
                .is_some_and(|p| p.mapping == mapping)
    }

    fn panel_input(
        &mut self,
        mapping: Mapping,
        coord: CellCoord,
        mods: Modifiers,
        kinds: &[MouseKind],
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_panel_input(mapping) {
            return;
        }
        if let Some(client) = &self.client {
            cx.stop_propagation();
            for kind in kinds {
                if let Err(error) = mapping.send(client, coord, *kind, mods) {
                    self.notice = Some(format!("Could not send panel input · {error}"));
                    cx.notify();
                    break;
                }
            }
        }
    }
}

fn color([r, g, b]: Rgb) -> Hsla {
    rgb(u32::from_be_bytes([0, r, g, b])).into()
}

fn paint(
    plan: &TerminalPaintPlan,
    bounds: Bounds<Pixels>,
    advance: Pixels,
    caret: bool,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let rect = |row, start, end| {
        Bounds::new(
            point(
                bounds.left() + advance * start as f32,
                bounds.top() + px(ROW * row as f32),
            ),
            size(advance * (end - start) as f32, px(ROW)),
        )
    };
    let cell_rect = |r: CellRun| rect(r.row, r.start_col, r.end_col);
    for bg in &plan.backgrounds {
        window.paint_quad(fill(cell_rect(bg.run), color(bg.color)));
    }
    for text in &plan.runs {
        if text.text.bytes().all(|b| b == b' ') {
            continue;
        }
        let area = rect(text.row, text.col, text.col + text.cells);
        let mut run = window.text_style().to_run(text.text.len());
        run.font = font("JetBrains Mono");
        if text.bold {
            run.font.weight = FontWeight::BOLD;
        }
        if text.italic {
            run.font.style = FontStyle::Italic;
        }
        run.color = color(text.color);
        let line =
            window
                .text_system()
                .shape_line(text.text.clone().into(), px(FONT), &[run], None);
        window.with_content_mask(Some(ContentMask { bounds: area }), |w| {
            let _ = line.paint(area.origin, px(ROW), w, cx);
        });
    }
    for line in &plan.underlines {
        let area = cell_rect(line.run);
        let y = area.bottom() - px(2.);
        let style = gpui::UnderlineStyle {
            color: Some(color(line.color)),
            thickness: px(1.),
            wavy: line.style == UnderlineStyle::Curly,
        };
        match line.style {
            UnderlineStyle::None => {}
            UnderlineStyle::Dotted | UnderlineStyle::Dashed => {
                let dash = if line.style == UnderlineStyle::Dotted {
                    px(1.)
                } else {
                    px(4.)
                };
                let mut x = area.left();
                while x < area.right() {
                    window.paint_underline(point(x, y), dash.min(area.right() - x), &style);
                    x += dash + px(2.);
                }
            }
            _ => {
                window.paint_underline(point(area.left(), y), area.size.width, &style);
                if line.style == UnderlineStyle::Double {
                    window.paint_underline(point(area.left(), y - px(2.)), area.size.width, &style);
                }
            }
        }
    }
    if let Some(cursor) = plan.cursor.filter(|_| caret) {
        let area = cell_rect(cursor);
        window.paint_quad(fill(
            Bounds::new(area.origin, size(px(2.), area.size.height)),
            rgb(0xf6dba2),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pmacs_protocol::{Cell, PanelFrame};

    #[test]
    fn a_panel_click_can_only_use_the_negotiated_mapping_and_current_geometry() {
        let mut panel = Panel::new(PANEL_MAPPING_MIN_VERSION);
        panel.geometry = 7;
        panel.total = CellSize::new(12, 40);
        let frame = PanelFrame {
            buffer_id: BufferId::from_raw(8),
            panel_epoch: 1,
            geometry_epoch: 7,
            size: CellSize::new(2, 40),
            cells: vec![Cell::default(); 80],
            cursor: None,
            focused: true,
        };
        let mapped = |frame, generation| PanelFramePayload::PresentMapped {
            frame,
            mapping_generation: generation,
        };
        assert!(!panel.apply(PanelFramePayload::Present(frame.clone())));
        assert!(!panel.apply(mapped(frame.clone(), 0)));
        assert!(panel.apply(mapped(frame.clone(), 3)));
        let first = panel.visible.as_ref().unwrap().mapping;
        assert!(panel.focused());
        assert!(panel.apply(mapped(frame.clone(), 4)));
        assert_ne!(first, panel.visible.as_ref().unwrap().mapping);
        assert!(panel.apply(PanelFramePayload::Absent));
        assert!(!panel.focused());
        assert!(!panel.apply(mapped(frame.clone(), 3)));
        assert!(!panel.apply(mapped(
            PanelFrame {
                geometry_epoch: 6,
                ..frame.clone()
            },
            5
        )));
        let mut invalid = frame.clone();
        invalid.cells.pop();
        assert!(!panel.apply(mapped(invalid, 5)));
        assert!(!panel.is_visible());

        panel.version = PANEL_MIN_VERSION;
        assert!(!panel.apply(mapped(frame.clone(), 5)));
        assert!(panel.apply(PanelFramePayload::Present(frame)));
        assert_eq!(panel.visible.as_ref().unwrap().mapping.generation, None);
    }
}
