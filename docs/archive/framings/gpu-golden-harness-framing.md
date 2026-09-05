# GPU headless render harness — framing + as-built

pmacs-gpu has 40 unit tests, but every one is helper/logic or
vertex-math — **none exercises the wgpu composition path**. Layout and
rendering regressions (hit-testing, dropdown clipping, minimap
projection, squiggles, selection/current-line backgrounds, status/
minibuffer composition) pass the suite silently. This is why every GPU
arc this session ended with "looks good in the GUI" — a human eyeball
was the only render gate. Audit F-014.

This arc adds the **first test that actually renders a frame** — headless
(no window), to an offscreen texture, read back to CPU for pixel
assertions.

## What the recon established

- `State` (`main.rs:321`) holds device, queue, surface, window, the
  renderers, and ~70 render-input fields. Device/queue creation is
  **separable** from window/surface (`request_device` takes no surface;
  `request_adapter` can pass `compatible_surface: None`).
- `render()` (`main.rs:3526`) renders to the surface's current texture
  view — its **only** target. Everything after the acquire uses `view` +
  `config.width/height`, so swapping in an offscreen view is localized.
- Render inputs are **plain settable fields**; `apply_attach_message` is
  the wire path, not a gate — a test can set `current_text`, spans,
  decorations, menu/minibuffer, etc. directly, no daemon.
- The one blocker: **`State` can't be built without a device + window**
  (`window`/`surface` are non-optional).

## The rule

**Q#GH1 — make the render core headless-constructible, least-invasively.**
`window: Option<Arc<Window>>` and `surface: Option<Surface>`; a
`State::new_headless(width, height)` that builds device/queue/renderers/
atlas/viewport/config with `compatible_surface: None` and no window;
promote the surface format to a `format` field on `State` (pick a fixed
renderable `Rgba8UnormSrgb` headless, shared by the offscreen texture and
all three pipelines + atlas — they must match). `request_redraw` and
`surface.configure` become `if let Some(...)`. *Not* the full
`Renderer`-sub-struct extraction the recon floated as the "clean" option
— that's hundreds of `self.device → self.renderer.device` edits; deferred
until a second consumer needs it.

**Q#GH2 — split `render()` into acquire + `render_to_view`.** Move the
body (`3543-3853`) into `render_to_view(&mut self, view: &TextureView)`;
`render()` keeps the surface acquire and calls it; a new
`render_offscreen() -> Vec<u8>` creates an `RENDER_ATTACHMENT | COPY_SRC`
texture, calls `render_to_view`, then `copy_texture_to_buffer` into a
`MAP_READ` buffer (256-byte `bytes_per_row` alignment), `map_async`, and
returns RGBA. The composition path is exercised **as-is** — the test
renders through the real `render_to_view`, not a reimplementation.

**Q#GH3 — start narrow (the audit's own guidance).** Smoke assertions,
not pixel-exact golden PNGs:
- an **empty** buffer renders the clear color (`BG`) everywhere — the
  baseline;
- with `current_text` set, **ink appears** — the frame differs from the
  empty baseline in the text region (glyphs rasterized);
- a colored decoration background paints **its** color where placed.

These catch the real regression class ("nothing renders" / "text stopped
drawing" / "a layer broke") without brittle exact-pixel goldens.

## Categorical bets

- **Exercise the real path, don't mock it.** The value is in rendering
  through the actual `render_to_view` + real renderers; a reimplemented
  mini-pipeline would test itself, not the code.
- **`Rgba8UnormSrgb` headless is representative enough.** The surface
  picks an srgb format anyway; a fixed one keeps readback deterministic.

## CI gating (decided: lavapipe now)

Headless wgpu needs an adapter and CI runners have no GPU, so the CI job
installs the Vulkan software rasterizer **lavapipe** (`mesa-vulkan-
drivers`) and points wgpu at its ICD. A dedicated **GPU Render
(headless)** job then runs `cargo test -p pmacs-gpu` — which *also*
closes a gap the recon surfaced: the workspace default member is only the
root `pmacs` package, so pmacs-gpu's tests weren't being executed in CI
at all (the clippy job builds them but never runs them). This job runs
them, render tests included.

The test still **skips gracefully** when `request_adapter` returns `None`
(log + early return) so it doesn't fail on a dev box without a working
adapter — but with lavapipe present in CI it runs for real and gates
every PR.

## Validation implication

Making `surface`/`window` optional touches `render()` and `resize()` —
the live windowed path. The headless test passing does **not** prove the
windowed frontend still renders; **that needs a human eyeball** after
this lands (attach the GUI, confirm it still draws/resizes).

## As-built

Landed as framed, via the least-invasive refactor (not the `Renderer`
extraction):

- `State.window`/`State.surface` are now `Option`; a shared
  `State::assemble(window, surface, device, queue, config, initial_text)`
  builds the window-agnostic half, called by both the windowed `new` and
  a `#[cfg(test)] new_headless(w, h, text)` that passes
  `compatible_surface: None` and returns `None` when no adapter exists.
  The stored `format` field turned out redundant with `config.format`, so
  it was dropped — `config.format` is the single source the pipelines,
  atlas, and offscreen texture share.
- `render()` split into a surface-acquire wrapper + `render_to_view(&
  TextureView)`; a `#[cfg(test)] render_offscreen()` renders through the
  same `render_to_view` into a `RENDER_ATTACHMENT | COPY_SRC` texture and
  reads it back (256-byte row alignment, `PollType::wait_indefinitely`).
  `request_redraw` became an `Option`-guarded helper across its 15 call
  sites.
- Two smoke tests through the **real** composition path: a full frame is
  non-uniform (something composited), and setting text changes the frame
  vs an empty buffer. Both pass locally on this box's AMD Vulkan adapter.
- CI: a **GPU Render (headless)** job installs `mesa-vulkan-drivers`
  (lavapipe) and runs `cargo test -p pmacs-gpu` — also the first time
  pmacs-gpu's tests run in CI at all. `PMACS_REQUIRE_GPU=1` there turns a
  missing adapter into a hard failure, so a broken lavapipe setup can't
  masquerade as a silently-skipped green.

Divergence from framing: the fixed-format `format` field was dropped as
redundant (above). No other divergence.

## Deferred (named)

- Golden-PNG comparison + a case gallery (dropdown clipped above a short
  window, minimap projection, squiggle under a glyph, resize reflow).
- The `Renderer`-sub-struct extraction, if a second headless consumer
  appears.
