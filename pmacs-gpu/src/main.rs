//! pmacs-gpu — GPU/GUI frontend for pmacs.
//!
//! Two run modes:
//!
//! - **Hello-world** (no `--attach` argument; session 2 default).
//!   Opens a window and renders "hello, pmacs" in the bundled
//!   `JetBrains` Mono. Used to confirm the wgpu/winit/glyphon stack
//!   without depending on a daemon.
//! - **Attach** (`--attach <unix-socket-path>`; session 3). Connects
//!   to a running pmacs daemon, negotiates `semantic_render +
//!   crdt_replica`, imports the daemon's `BufferSnapshot` into a
//!   local loro replica, and renders the resulting rope text. Live
//!   `CrdtOp` updates from the daemon are applied as they arrive.
//!
//! See `docs/pmacs-gpu-design.md` for the arc framing. Session 3's
//! gate at close is "Daemon ↔ frontend handshake; rope reconstruction
//! matches" — handled by the attach mode below.
//!
//! The bundled font is `JetBrains` Mono Regular, distributed under
//! the SIL Open Font License 1.1 (see `fonts/OFL.txt`).

mod attach;

use std::path::PathBuf;
use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use pmacs_protocol::InstanceMessage;
use wgpu::MultisampleState;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::attach::{AttachClient, AttachEvent};

/// Bundled font (SIL Open Font License 1.1 — see `fonts/OFL.txt`).
const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

/// Initial window size in logical pixels.
const INITIAL_WIDTH: u32 = 800;
const INITIAL_HEIGHT: u32 = 200;

/// Color the surface clears to before text renders.
const BG: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.05,
    b: 0.07,
    a: 1.0,
};

/// Text the hello-world (and attach-pre-snapshot / attach-failed)
/// modes render. Once the daemon's `BufferSnapshot` arrives the
/// rendered text becomes the rope contents instead.
const HELLO_TEXT: &str = "hello, pmacs";

/// Container id the daemon uses on its loro `LoroDoc` for the
/// buffer's text. Must match `pmacs::crdt::CrdtState`'s container
/// name (`"body"`).
const LORO_TEXT_CONTAINER: &str = "body";

/// Custom events delivered to the winit event loop. The reader thread
/// in `attach.rs` forwards each decoded `InstanceMessage` through the
/// `EventLoopProxy<AppEvent>` it was handed by `connect()`; the main
/// thread dispatches them in `user_event` below.
#[derive(Debug)]
pub enum AppEvent {
    /// A message or disconnect notification from the attach reader
    /// thread.
    Attach(AttachEvent),
}

/// CLI mode derived from argv.
#[derive(Debug, Clone)]
enum Mode {
    /// `pmacs-gpu` (no args): inert hello-world.
    HelloWorld,
    /// `pmacs-gpu --attach <socket>`: connect + render the daemon's
    /// rope.
    Attach { socket: PathBuf },
}

fn main() {
    env_logger::init();
    let mode = parse_args(std::env::args().skip(1).collect());
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("create winit event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App {
        mode,
        proxy: Some(proxy),
        state: None,
        attach_client: None,
    };
    event_loop
        .run_app(&mut app)
        .expect("winit event loop run_app");
}

/// Tiny argv parser. No `clap` because the surface is genuinely two
/// shapes; full CLI parsing arrives when there's more to parse. The
/// `for` ranges over a small set: at most one `--attach <socket>` or
/// `--help` arrives, plus any stray unrecognized flag.
fn parse_args(args: Vec<String>) -> Mode {
    let mut iter = args.into_iter();
    let Some(first) = iter.next() else {
        return Mode::HelloWorld;
    };
    match first.as_str() {
        "--attach" => {
            let socket = iter.next().unwrap_or_else(|| {
                eprintln!("pmacs-gpu: --attach requires a socket path");
                std::process::exit(2);
            });
            Mode::Attach {
                socket: PathBuf::from(socket),
            }
        }
        "--help" | "-h" => {
            eprintln!(
                "pmacs-gpu — GPU/GUI frontend for pmacs\n\nUSAGE:\n  pmacs-gpu                       \
                 hello-world (renders \"hello, pmacs\")\n  pmacs-gpu --attach <socket>     \
                 connect to a daemon's Unix socket and render its rope\n"
            );
            std::process::exit(0);
        }
        other => {
            eprintln!("pmacs-gpu: unrecognized argument: {other}");
            std::process::exit(2);
        }
    }
}

/// Top-level application handler. `state` is `Option` because winit
/// 0.30 builds the window in `resumed()`, not at `main()` start;
/// `attach_client` is held so the write half of the Unix stream
/// stays alive for as long as the window does.
struct App {
    mode: Mode,
    /// The event-loop proxy is taken in `resumed()` and handed to the
    /// reader thread. `Option` only because it can't be cloned out of
    /// a non-Option in a borrow.
    proxy: Option<winit::event_loop::EventLoopProxy<AppEvent>>,
    state: Option<State>,
    /// Held for lifetime; session 3 doesn't write back yet.
    #[allow(dead_code)]
    attach_client: Option<AttachClient>,
}

/// All resources owned by one running pmacs-gpu instance.
struct State {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    buffer: Buffer,
    /// What the buffer is currently shaped to. Held so we can detect
    /// no-op updates and skip the re-shape.
    current_text: String,
    /// Local CRDT replica seeded by `BufferSnapshot`. `None` in
    /// hello-world mode or before the first snapshot arrives in
    /// attach mode.
    loro_doc: Option<loro::LoroDoc>,
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let initial_text = match &self.mode {
            Mode::HelloWorld => HELLO_TEXT,
            Mode::Attach { .. } => "(connecting...)",
        };
        self.state = Some(State::new(event_loop, initial_text));

        // In attach mode, kick off the connection now that the event
        // loop is running and a proxy is available. Failure logs and
        // leaves the window showing its `(connecting...)` placeholder
        // — better UX than killing the window during dev.
        if let Mode::Attach { socket } = self.mode.clone() {
            let proxy = self.proxy.take().expect("proxy taken twice");
            match attach::connect(&socket, proxy) {
                Ok(client) => {
                    self.attach_client = Some(client);
                }
                Err(e) => {
                    eprintln!("pmacs-gpu: attach failed: {e}");
                    if let Some(state) = self.state.as_mut() {
                        state.set_text("(attach failed; see stderr)");
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width.max(1), size.height.max(1)),
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            AppEvent::Attach(AttachEvent::Message(msg)) => state.apply_attach_message(*msg),
            AppEvent::Attach(AttachEvent::Disconnected(reason)) => {
                eprintln!("pmacs-gpu: daemon disconnected ({reason})");
                state.set_text("(daemon disconnected)");
            }
        }
    }
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_text: &str) -> Self {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("pmacs-gpu")
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            f64::from(INITIAL_WIDTH),
                            f64::from(INITIAL_HEIGHT),
                        )),
                )
                .expect("create window"),
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("request_adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pmacs-gpu device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .expect("request_device");

        let inner_size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: inner_size.width.max(1),
            height: inner_size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let mut font_system = FontSystem::new();
        font_system.db_mut().load_font_data(JETBRAINS_MONO.to_vec());
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let mut viewport = Viewport::new(&device, &cache);
        viewport.update(
            &queue,
            Resolution {
                width: config.width,
                height: config.height,
            },
        );
        let mut atlas = TextAtlas::new(&device, &queue, &cache, surface_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);

        // Smaller font in attach mode (file contents tend to be more
        // than one line); larger only fits "hello, pmacs"-shaped
        // strings. Picked metrics that look reasonable for code at
        // 800px wide.
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(config.height as f32),
        );
        buffer.set_text(
            &mut font_system,
            initial_text,
            &Attrs::new().family(Family::Name("JetBrains Mono")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        Self {
            window,
            device,
            queue,
            surface,
            config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            buffer,
            current_text: initial_text.to_owned(),
            loro_doc: None,
        }
    }

    /// Replace the rendered text with `text` and request a redraw.
    /// No-op when `text` is byte-identical to the current rendering
    /// (avoids the re-shape cost when an unchanged buffer ticks).
    fn set_text(&mut self, text: &str) {
        if self.current_text == text {
            return;
        }
        self.current_text.clear();
        self.current_text.push_str(text);
        self.buffer.set_text(
            &mut self.font_system,
            &self.current_text,
            &Attrs::new().family(Family::Name("JetBrains Mono")),
            Shaping::Advanced,
            None,
        );
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.window.request_redraw();
    }

    /// Apply one `InstanceMessage` to the local replica. Session 3
    /// handles the two variants that matter for rope reconstruction:
    /// `BufferSnapshot` (bootstrap) and `CrdtOp` (live updates). Every
    /// other variant is ignored (logged at debug) — the `SemanticFrame`
    /// family will be consumed in later sessions.
    fn apply_attach_message(&mut self, msg: InstanceMessage) {
        match msg {
            InstanceMessage::BufferSnapshot { crdt_snapshot, .. } => {
                let doc = loro::LoroDoc::new();
                if let Err(e) = doc.import(&crdt_snapshot) {
                    eprintln!("pmacs-gpu: BufferSnapshot import failed: {e:?}");
                    return;
                }
                let text = doc.get_text(LORO_TEXT_CONTAINER).to_string();
                self.loro_doc = Some(doc);
                self.set_text(&text);
            }
            InstanceMessage::CrdtOp { op, .. } => {
                let Some(doc) = self.loro_doc.as_ref() else {
                    // No snapshot yet — drop. The daemon's send order
                    // is snapshot-then-ops, so this case is rare
                    // (mid-attach race only). When the snapshot lands
                    // it'll have the ops baked in anyway.
                    return;
                };
                if let Err(e) = doc.import(&op.bytes) {
                    eprintln!("pmacs-gpu: CrdtOp import failed: {e:?}");
                    return;
                }
                let text = doc.get_text(LORO_TEXT_CONTAINER).to_string();
                self.set_text(&text);
            }
            _ => {
                // Other variants (Cursor, CellDelta, semantic frame
                // family, presence, goodbye, etc.) are ignored in
                // session 3. Goodbye in particular surfaces via the
                // reader thread's clean-EOF path as a Disconnected
                // event — not handled here.
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.viewport
            .update(&self.queue, Resolution { width, height });
        self.buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(height as f32),
        );
        self.window.request_redraw();
    }

    fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("surface acquisition raised a validation error");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [TextArea {
                    buffer: &self.buffer,
                    left: 16.0,
                    top: 16.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: self.config.width.cast_signed(),
                        bottom: self.config.height.cast_signed(),
                    },
                    default_color: Color::rgb(230, 230, 235),
                    custom_glyphs: &[],
                }],
                &mut self.swash_cache,
            )
            .expect("text_renderer prepare");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pmacs-gpu frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pmacs-gpu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BG),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("text_renderer render");
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.atlas.trim();
    }
}
