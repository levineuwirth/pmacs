#![forbid(unsafe_code)]

mod document;
mod input;
mod palette;
mod panel;

use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use async_channel::Receiver;
use document::Document;
use gpui::{
    Application, Bounds, Context, Div, FocusHandle, KeyDownEvent, Render, ScrollStrategy,
    SharedString, UniformListScrollHandle, Window, WindowBounds, WindowOptions, canvas, div, fill,
    point, prelude::*, px, rgb, size, uniform_list,
};
use palette::Prompt;
use pmacs_attach_client::{
    AttachClient, AttachEvent, InitialTargetPaths, connect_with_target_and_sink,
};
use pmacs_protocol::{BufferId, ByteRange, InstanceMessage, MinibufferRow};

const SOURCE_FONT: &[u8] = include_bytes!("../../pmacs-gpu/fonts/JetBrainsMono-Regular.ttf");
const USAGE: &str = "Usage: pmacs-gpui --attach SOCKET FILE";

struct FileView {
    id: BufferId,
    name: SharedString,
    modified: bool,
    notice: Option<String>,
    document: Document,
    client: Option<AttachClient>,
    focus: FocusHandle,
    cursor: Option<u64>,
    reveal_cursor: bool,
    source_scroll: UniformListScrollHandle,
    viewport: Option<ByteRange>,
    panel: panel::Panel,
    prompt: Option<Prompt>,
    prompt_scroll: UniformListScrollHandle,
    status: Option<String>,
}

impl FileView {
    fn apply(&mut self, message: InstanceMessage) -> Result<bool, loro::LoroError> {
        match message {
            InstanceMessage::CrdtOp { buffer_id, op } if buffer_id == self.id => {
                self.document.import(&op.bytes)?;
            }
            InstanceMessage::StatusFacts {
                buffer_id,
                name,
                modified,
                message,
                ..
            } if buffer_id == self.id => {
                if self.name.as_ref() == name && self.modified == modified && self.status == message
                {
                    return Ok(false);
                }
                self.status = message;
                self.name = name.into();
                self.modified = modified;
            }
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => {
                self.document = Document::from_snapshot(&crdt_snapshot)?;
                if buffer_id != self.id {
                    self.id = buffer_id;
                    self.name = "".into();
                    self.status = None;
                    self.modified = false;
                    self.source_scroll = UniformListScrollHandle::new();
                }
                self.viewport = None;
                self.cursor = None;
            }
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } if self.id == buffer_id => {
                if self.cursor == Some(byte_pos) {
                    return Ok(false);
                }
                self.cursor = Some(byte_pos);
                self.reveal_cursor = true;
            }
            InstanceMessage::MinibufferPromptRows {
                prompt,
                input,
                cursor,
                rows,
                selected,
                total,
            } => {
                self.set_prompt(prompt.map(|label| Prompt {
                    label: label.into(),
                    input: input.into(),
                    cursor,
                    rows,
                    selected,
                    total,
                }));
            }
            InstanceMessage::MinibufferPrompt {
                prompt,
                input,
                cursor,
                candidates,
                selected,
                total,
            } => {
                self.set_prompt(prompt.map(|label| {
                    Prompt {
                        label: label.into(),
                        input: input.into(),
                        cursor,
                        selected,
                        total,
                        rows: candidates
                            .into_iter()
                            .map(|label| MinibufferRow {
                                label,
                                detail: None,
                            })
                            .collect(),
                    }
                }));
            }
            InstanceMessage::PanelFrame(payload) => {
                return Ok(self.panel.apply(payload));
            }
            InstanceMessage::Goodbye(reason) => {
                self.notice = Some(format!("Daemon disconnected · {reason:?}"));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn set_prompt(&mut self, prompt: Option<Prompt>) {
        if self.prompt.is_none() {
            self.prompt_scroll = UniformListScrollHandle::new();
        }
        if let Some(p) = &prompt {
            p.reveal_selected(&self.prompt_scroll);
        }
        self.prompt = prompt;
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.notice.is_some() {
            return;
        }
        let Some(input) = input::translate(&event.keystroke) else {
            return;
        };
        window.prevent_default();
        cx.stop_propagation();
        if let Some(client) = &self.client
            && let Err(error) = input.send(client)
        {
            self.notice = Some(format!("Could not send input · {error}"));
            cx.notify();
        }
    }

    fn listen(&self, events: Receiver<AttachEvent>, cx: &Context<Self>) {
        cx.spawn(async move |view, cx| {
            while let Ok(event) = events.recv().await {
                let result = view.update(cx, |view, cx| {
                    match event {
                        AttachEvent::Message(message) => match view.apply(*message) {
                            Ok(true) => cx.notify(),
                            Ok(false) => {}
                            Err(error) => {
                                view.notice = Some(format!("Buffer update failed · {error}"));
                                cx.notify();
                                return false;
                            }
                        },
                        AttachEvent::Disconnected(reason) => {
                            view.notice
                                .get_or_insert_with(|| format!("Connection lost · {reason}"));
                            cx.notify();
                            return false;
                        }
                    }
                    true
                });
                if !matches!(result, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }
}

/// The list reports the rows actually laid out, including scroll and resize.
/// The daemon needs a document viewport before it emits minibuffer and status facts.
struct SourceViewport(gpui::Entity<FileView>);

impl gpui::UniformListDecoration for SourceViewport {
    fn compute(
        &self,
        rows: std::ops::Range<usize>,
        _: Bounds<gpui::Pixels>,
        _: gpui::Point<gpui::Pixels>,
        _: gpui::Pixels,
        _: usize,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> gpui::AnyElement {
        self.0.update(cx, |view, cx| {
            let visible = view.document.with_lines(|r| ByteRange {
                start: r.line_to_byte(rows.start.min(r.len_lines())) as u64,
                end: r.line_to_byte(rows.end.min(r.len_lines())) as u64,
            });
            if view.viewport != Some(visible)
                && view.notice.is_none()
                && let Some(client) = &view.client
            {
                match client.send_viewport(view.id, visible, 0) {
                    Ok(()) => view.viewport = Some(visible),
                    Err(error) => {
                        view.notice =
                            Some(format!("Could not declare the document viewport · {error}"));
                        cx.notify();
                    }
                }
            }
        });
        div().into_any_element()
    }
}

impl Drop for FileView {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            client.detach();
        }
    }
}

impl Render for FileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.notice.is_none()
            && let Some(client) = &self.client
            && let Err(error) = self.panel.declare(window, client)
        {
            self.notice = Some(format!("Could not declare panel geometry · {error}"));
        }
        let (count, caret) = self.document.with_lines(|rope| {
            let caret = self.cursor.and_then(|byte| {
                let byte = usize::try_from(byte).ok()?;
                let row = rope.try_byte_to_line(byte).ok()?;
                Some((row, byte - rope.line_to_byte(row)))
            });
            (rope.len_lines(), caret)
        });
        if let Some((row, _)) = caret
            && self.reveal_cursor
        {
            self.source_scroll
                .scroll_to_item(row, ScrollStrategy::Center);
            self.reveal_cursor = false;
        }
        let caret = caret.filter(|_| {
            self.prompt.is_none() && !self.panel.focused() && window.is_window_active()
        });
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .h(px(40.))
            .flex_shrink_0()
            .bg(rgb(0x1b1b1b))
            .border_b_1()
            .border_color(rgb(0x333332))
            .text_sm()
            .font_family(".SystemUIFont")
            .child(self.name.clone())
            .when(self.modified, |row| row.child("•"));
        let rows = uniform_list(
            "source",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, _| {
                this.document.with_lines(|rope| {
                    range
                        .map(|index| {
                            let cursor =
                                caret.filter(|(row, _)| *row == index).map(|(_, byte)| byte);
                            let (line, cursor) =
                                document::display_line(rope.line(index).to_string(), cursor);
                            div()
                                .w_full()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(
                                    div()
                                        .w(px(64.))
                                        .pr_3()
                                        .text_right()
                                        .flex_shrink_0()
                                        .text_color(rgb(0x827f79))
                                        .child((index + 1).to_string()),
                                )
                                .child(text_with_caret(line.into(), cursor).flex_1().min_w_0())
                        })
                        .collect::<Vec<_>>()
                })
            }),
        )
        .with_decoration(SourceViewport(cx.entity()))
        .track_scroll(self.source_scroll.clone())
        .flex_1()
        .min_h_0();
        let status = self.notice.clone().or_else(|| self.status.clone());
        div()
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x121212))
            .text_color(rgb(0xd4d0c8))
            .text_size(px(16.))
            .font_family("JetBrains Mono")
            .child(header)
            .child(rows)
            .children(
                self.panel
                    .show(self.prompt.is_none() && window.is_window_active(), cx),
            )
            .when_some(
                status.or_else(|| self.panel.is_visible().then(String::new)),
                |root, text| {
                    root.child(
                        div()
                            .px_4()
                            .h(px(24.))
                            .line_height(px(24.))
                            .flex_shrink_0()
                            .truncate()
                            .text_sm()
                            .text_color(rgb(0xffc48a))
                            .child(text),
                    )
                },
            )
            .when_some(self.prompt.as_ref(), |root, prompt| {
                root.child(prompt.show(window, &self.prompt_scroll, cx))
            })
    }
}

/// GPUI shapes the whole active line once so ligatures, combining characters and
/// the caret use the same advances. An offscreen caret brings its suffix into view.
fn text_with_caret(text: SharedString, cursor: Option<usize>) -> Div {
    let row = div()
        .h(px(22.))
        .line_height(px(22.))
        .overflow_hidden()
        .whitespace_nowrap();
    let Some(cursor) = cursor else {
        return row.child(text);
    };
    row.child(
        canvas(
            move |bounds, window, _| {
                let style = window.text_style();
                let font_size = style.font_size.to_pixels(window.rem_size());
                let run = style.to_run(text.len());
                let line = window
                    .text_system()
                    .shape_line(text, font_size, &[run], None);
                let x = line.x_for_index(cursor);
                let shift = (x + px(2.) - bounds.size.width).max(px(0.));
                (line, x, shift)
            },
            |bounds, (line, x, shift), window, cx| {
                let origin = point(bounds.left() - shift, bounds.top());
                let _ = line.paint(origin, bounds.size.height, window, cx);
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + x, origin.y + px(2.)),
                        size(px(1.5), bounds.size.height - px(4.)),
                    ),
                    rgb(0xf6dba2),
                ));
            },
        )
        .size_full(),
    )
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let [flag, socket, file] = args.as_slice() else {
        return Err(USAGE.into());
    };
    if flag != "--attach" {
        return Err(USAGE.into());
    }
    let target = InitialTargetPaths {
        cwd: std::env::current_dir()?,
        path: PathBuf::from(file),
    };
    let name = target
        .path
        .file_name()
        .unwrap_or(file)
        .to_string_lossy()
        .into_owned();
    let (tx, rx) = async_channel::bounded(128);
    let mut client =
        connect_with_target_and_sink(socket.as_ref(), "pmacs-gpui", target, move |event| {
            tx.send_blocking(event).is_ok()
        })?;
    let Some(InstanceMessage::BufferSnapshot {
        buffer_id,
        crdt_snapshot,
    }) = client.take_initial_message()
    else {
        return Err("daemon did not supply a target snapshot".into());
    };
    let document = Document::from_snapshot(&crdt_snapshot)?;
    eprintln!(
        "pmacs-gpui: displaying {name} over protocol v{}",
        client.session_protocol_version()
    );
    // GPUI owns the event loop and may return after the startup closure.
    let launch_result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let launch = launch_result.clone();
    Application::new().run(move |cx| {
        if let Err(error) = cx.text_system().add_fonts(vec![SOURCE_FONT.into()]) {
            *launch.borrow_mut() = Some(format!("could not load JetBrains Mono: {error}"));
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let result = cx.open_window(
            WindowOptions {
                app_id: Some("pmacs-gpui".into()),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("pmacs — GPUI");
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    focus.focus(window);
                    cx.observe_window_activation(window, |view: &mut FileView, w, cx| {
                        if let Some(client) = &view.client
                            && let Err(error) = client.send_focus(w.is_window_active())
                        {
                            view.notice = Some(format!("Could not send focus · {error}"));
                        }
                        cx.notify();
                    })
                    .detach();
                    let client_version = client.session_protocol_version();
                    let view = FileView {
                        id: buffer_id,
                        name: name.into(),
                        modified: false,
                        notice: None,
                        document,
                        client: Some(client),
                        focus,
                        cursor: None,
                        reveal_cursor: false,
                        source_scroll: UniformListScrollHandle::new(),
                        viewport: None,
                        panel: panel::Panel::new(client_version),
                        prompt: None,
                        prompt_scroll: UniformListScrollHandle::new(),
                        status: None,
                    };
                    view.listen(rx, cx);
                    view
                })
            },
        );
        if let Err(err) = result {
            *launch.borrow_mut() = Some(format!("could not open GPUI window: {err:#}"));
            cx.quit();
        }
    });
    if let Some(error) = launch_result.take() {
        return Err(error.into());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pmacs-gpui: {error}");
            ExitCode::FAILURE
        }
    }
}
