//! pmacs-gpu — GPU/GUI frontend for pmacs.
//!
//! Session 2 of the pmacs-gpu arc (`docs/pmacs-gpu-design.md`):
//! **hello-world binary**. Opens a window via `winit`, initializes
//! `wgpu` against its surface, sets up `glyphon` text rendering with
//! the bundled `JetBrains` Mono font, and renders "hello, pmacs" once
//! per frame. No protocol consumption yet — that arrives in session
//! 3 (the attach loop). No editor state, no input handling beyond
//! close + Escape.
//!
//! The bundled font is `JetBrains` Mono Regular, distributed under
//! the SIL Open Font License 1.1 (see `fonts/OFL.txt`). The design
//! note incorrectly recorded the license as Apache 2.0; the actual
//! license has been OFL since the family's open-source release. The
//! design-doc note will be corrected as part of this session.

use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use wgpu::MultisampleState;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// Bundled font (SIL Open Font License 1.1 — see `fonts/OFL.txt`).
const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

/// Initial window size in logical pixels. Session 2 is fixed-size for
/// simplicity; resizes still work, this is just the boot dimension.
const INITIAL_WIDTH: u32 = 800;
const INITIAL_HEIGHT: u32 = 200;

/// Color the surface clears to before text renders.
const BG: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.05,
    b: 0.07,
    a: 1.0,
};

/// Hello-world payload. Stays inert here — session 3 wires this to
/// the daemon's `BufferSnapshot` instead.
const HELLO_TEXT: &str = "hello, pmacs";

fn main() {
    // wgpu emits useful trace output on adapter selection / surface
    // configuration. The default `RUST_LOG=info` is fine for
    // development.
    env_logger::init();

    let event_loop = EventLoop::new().expect("create winit event loop");
    let mut app = App { state: None };
    event_loop
        .run_app(&mut app)
        .expect("winit event loop run_app");
}

/// Top-level application handler. Holds an `Option<State>` because
/// winit 0.30 requires the window + GPU resources to be created
/// *after* `resumed()` fires, not at `main()` start.
struct App {
    state: Option<State>,
}

/// All resources owned by a single running pmacs-gpu instance: the
/// window, the wgpu device/queue/surface, the glyphon stack, and the
/// shaped text buffer.
struct State {
    window: Arc<Window>,

    // wgpu plumbing.
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    // glyphon plumbing.
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    buffer: Buffer,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            // `resumed` can fire more than once on platforms that
            // suspend/restore (e.g. mobile). The hello-world doesn't
            // reinitialize on resume; first call wins.
            return;
        }
        self.state = Some(State::new(event_loop));
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
}

impl State {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("pmacs-gpu hello-world")
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            f64::from(INITIAL_WIDTH),
                            f64::from(INITIAL_HEIGHT),
                        )),
                )
                .expect("create window"),
        );

        // wgpu instance + surface.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        // Pick an adapter that supports our surface. Power preference
        // = LowPower because the hello-world has no GPU appetite;
        // saves laptop battery during development.
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

        // Configure the surface. Pick the first format the surface
        // and adapter both like; glyphon handles colorspace conversion
        // internally, so sRGB vs UNORM is the renderer's concern, not
        // ours at this layer.
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

        // glyphon plumbing — `FontSystem` owns the font database and
        // shaper state; we register the bundled JetBrains Mono before
        // anything tries to shape with it.
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

        // Shape "hello, pmacs" with JetBrains Mono at a comfortable
        // hello-world size. cosmic-text 0.18 (glyphon's pinned
        // version) threads `&mut FontSystem` through every Buffer
        // mutator that needs to re-shape; the 5th `None` on `set_text`
        // is the optional `Align` (we let cosmic-text default it).
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(48.0, 56.0));
        buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(config.height as f32),
        );
        buffer.set_text(
            &mut font_system,
            HELLO_TEXT,
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
        }
    }

    /// Reconfigure surface + glyphon viewport on window-size change.
    /// The shaped text buffer also gets a new max-size so wrap and
    /// scroll align with the new viewport.
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

    /// One frame: clear the surface to `BG`, render the text buffer,
    /// present. Acquisition failures cause a re-configure and skip
    /// the frame (a typical recovery for transient surface losses).
    fn render(&mut self) {
        // wgpu 29 collapses success/error into a single enum
        // (`CurrentSurfaceTexture`), not `Result<SurfaceTexture, SurfaceError>`
        // as earlier versions did. Lost / Outdated trigger a
        // re-configure and skip the frame; Suboptimal is rendered
        // through but flagged for the next configure cycle (we don't
        // act on it in the hello-world).
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                // `Timeout`: transient acquisition stall — drop this
                // frame, try again next redraw.
                // `Occluded`: window minimized / behind another
                // window — skip the frame, save the GPU work.
                return;
            }
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
                    left: 24.0,
                    top: 60.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        // Surface dimensions are u32 but `TextBounds`
                        // is i32; `cast_signed` keeps the bit pattern
                        // and is correct for typical window sizes well
                        // below 2^31.
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
                label: Some("pmacs-gpu hello-world pass"),
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
