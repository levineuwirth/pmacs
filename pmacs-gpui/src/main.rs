#![forbid(unsafe_code)]

mod document;

use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use async_channel::Receiver;
use document::Document;
use gpui::{
    Application, Bounds, Context, Render, SharedString, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size, uniform_list,
};
use pmacs_attach_client::{
    AttachClient, AttachEvent, InitialTargetPaths, connect_with_target_and_sink,
};
use pmacs_protocol::{BufferId, InstanceMessage};

const SOURCE_FONT: &[u8] = include_bytes!("../../pmacs-gpu/fonts/JetBrainsMono-Regular.ttf");
const USAGE: &str = "Usage: pmacs-gpui --attach SOCKET FILE";

struct FileView {
    id: BufferId,
    name: SharedString,
    modified: bool,
    notice: Option<String>,
    document: Document,
    client: Option<AttachClient>,
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
                ..
            } if buffer_id == self.id => {
                self.name = name.into();
                self.modified = modified;
            }
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } if buffer_id == self.id => {
                self.document = Document::from_snapshot(&crdt_snapshot)?;
            }
            InstanceMessage::Goodbye(reason) => {
                self.notice = Some(format!("Daemon disconnected · {reason:?}"));
            }
            _ => return Ok(false),
        }
        Ok(true)
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

impl Drop for FileView {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            client.detach();
        }
    }
}

impl Render for FileView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.document.with_lines(|r| r.len_lines());
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
            cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                this.document.with_lines(|rope| {
                    range
                        .map(|index| {
                            let line = rope.line(index).to_string();
                            div()
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
                                .child(line.trim_end_matches(['\n', '\r']).to_owned())
                        })
                        .collect::<Vec<_>>()
                })
            }),
        )
        .flex_1()
        .min_h_0();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x121212))
            .text_color(rgb(0xd4d0c8))
            .text_size(px(16.))
            .font_family("JetBrains Mono")
            .child(header)
            .child(rows)
            .when_some(self.notice.clone(), |root, text| {
                root.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(rgb(0xffc48a))
                        .child(text),
                )
            })
    }
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
                window.set_window_title(&format!("{name} — pmacs"));
                cx.new(|cx| {
                    let view = FileView {
                        id: buffer_id,
                        name: name.into(),
                        modified: false,
                        notice: None,
                        document,
                        client: Some(client),
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
