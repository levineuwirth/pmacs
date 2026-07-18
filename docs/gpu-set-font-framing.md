# GPU font preference — framing (Arc 4 stage 2, `pmacs.gpu.set_font`)

**Revision 5 — 2026-07-18. Status: implemented on branch
`gpu-set-font` (protocol v17); awaiting PR review.**

Revision 5 (PR review, findings 1–5): source bytes that fall inside a
shaped cluster are normalized to an explicit representable cosmic-text
cursor before geometry or following; combining-mark and real-ligature
fixtures pin that they can no longer fall back to the source-line start
(finding 1). Code-buffer reflow is now one transaction for every
dynamic horizontal input, not only font application: line-number mode,
gutter digit-count transitions, minimap appearance/disappearance, full
text replacement, incremental CRDT edits, and byte-identical
`BufferSnapshot` summary clearing all synchronize the shaping width
before the final reshape and re-follow only a previously painted caret
(finding 2). The monospace advance probe divides the total shaped-run
width by the probe's cell count, rather than sampling its first glyph;
the embedded alternate monospace carries a real multi-cell ligature so
the old result is observably wrong (finding 3). All four fixture fonts
now enter the explicit database before `FontSystem` construction and
their retained IDs are checked against cosmic-text's actual
`is_monospace` classification, removing the post-construction test
approximation (finding 4). Acceptance 10, 11, 17, and 18 now exercise
their complete claims at both size bounds, including rendered band
containment, selection/hit row identity, wrapped completion anchors,
gutter continuation alignment, reverse caret-free reflow, and both
directions of caret-free resize (finding 5).

Revision 4 (framing round 3, findings 1–5): caret preservation is now
VISUAL-RUN aware rather than source-line-only. The code buffer keeps a
normalized cosmic-text `Scroll`, full reshapes reapply it, and the
shared caret-follow helper uses `Buffer::shape_until_cursor` (which
handles wrapped runs vertically) before normalizing a
slice-local `scroll.line` into the frontend's whole-file `scroll_top`;
the font path and `CursorByte` path share the helper. A long line that
fits at size 6 and wraps below the viewport at size 72 is a load-bearing
acceptance case (finding 1). Context-menu vertical clipping joins its
already-accepted horizontal clipping at extreme sizes: minibuffer and
completion popups retain their existing surface-windowing guarantee,
while viewport-aware context-menu flip/window/ellipsis is one named
deferral; acceptance 10 no longer promises geometry the menu substrate
cannot provide (finding 2). Metrics and dimensions update together via
`set_metrics_and_size`; the code buffer uses the actual drawable code
width/height, the status buffers use the derived band height, all seven
buffers are brought current after a resize, and a resize→font-change
test pins the dimensions before pixels are inspected (finding 3). The
default family becomes a SANITIZED, CURRENT-ORDER `"JetBrains Mono"`
query: the explicit database preserves today's order (system fonts,
then the bundle), removes only non-monospace faces colliding with that
family, and is only then wrapped in `FontSystem`. A valid installed
JetBrains Mono keeps today's pixels, while a proportional same-family
face cannot win the default query or make the monospace fallback recurse
(finding 4). The last stale "bundled font" statement is aligned with
that now-structural guarantee (finding 5).
The final feasibility audit also pins three substrate details exposed by
the larger metric range: popup buffers use `Wrap::None` so one wire row
cannot become several interactive rows; caret and completion-anchor
mapping select the VISUAL run that actually contains the byte; and
font-dependent fallback/menu advances are measured relative to the
sanitized per-process default even when the code buffer is empty.
Glyphon 0.11 ignores cosmic-text's horizontal scroll component, so
stage 2 explicitly keeps
that component zero and defers the wider horizontal-offset substrate.
Every final shape normalizes a cosmic source-line advance back into the
frontend's whole-file origin (including caret-free shrink), optimistic
edits use the same visual follower, the Lua table is strict raw data,
and font resolution validates all four style queries rather than only
the normal face.

Revision 3 (framing round 2, findings 1–7): the GPU validates the
WIRE size before mutating any state — `FontFacts` is deserialized
protocol input, `u32` does not enforce the range, and
`Buffer::set_metrics` panics on a zero font size (cosmic-text
`buffer.rs:563`) — an out-of-range `size_centi_px` fails closed:
the whole message is ignored, logged, and current state kept;
direct GPU-arm tests cover 0, 599, 7201, and `u32::MAX`
(finding 1). The caret repair is re-ordered — record the
painted-visibility predicate (EXCLUDING the two-line overscan that
`view_range` carries, `main.rs:4290`), set metrics, conditionally
adjust `scroll_top`, THEN `reshape()` once at the final scroll,
then declare the viewport — matching the existing cursor path that
rebuilds lines before declaring (`main.rs:2854`); the
scrolled-away pin in acceptance 11 targets exactly the
one-line-past-the-painted-window overscan case (finding 2).
Acceptance 10 is narrowed to VERTICAL containment (code stops at
the band, status glyphs fit the band, popups stay inside the
surface): popups occlude the text area by layer contract, and at
size 72 the 380 px menu cap clips long labels today — accepted and
documented; viewport-aware menu width + ellipsis moves to Deferred
(finding 3). The default family is REDEFINED as the locally
resolved "JetBrains Mono" query, with the bundled asset
guaranteeing the query is never empty rather than promising face
identity — `FontSystem::new()` loads system fonts before the
bundle, and fontdb returns the first surviving candidate in
insertion order, so a system-installed JetBrains Mono may win;
never-set and fallback resolve through the same query in the same
process, so the byte-identical acceptance claims stay valid
per-machine (finding 4). Quantization is pinned: range-check the
ORIGINAL finite value first (6.0 ≤ size ≤ 72.0), then
nearest-hundredth via round — 5.999 errors rather than rounding
into range; values on both sides of a hundredth are pinned
(finding 5). Acceptance 2's field name corrected to
`size_centi_px` (finding 6). Atlas wording corrected: `trim()`
clears `glyphs_in_use`; old glyphs become ELIGIBLE for later
LRU-style eviction under allocation pressure, they do not age out
on their own (finding 7).

Revision 2 (framing round 1, findings 1–6): the wire size is an
**integer in hundredths of a logical pixel** (`Option<u32>`) —
`InstanceMessage` derives `Eq` (`message.rs:490`), so `Option<f32>`
cannot compile, and cosmic-text metrics are logical pixels, not
typographic points; units corrected throughout (finding 1).
`STATUS_BAND_HEIGHT` joins the derived-geometry inventory and is
threaded through buffer sizing, band geometry, `text_area_bottom`,
minimap height, and visible-line math; acceptance renders open
status/minibuffer/menu surfaces at both size bounds and asserts no
overlap or clipping (finding 2). `apply_font_facts` records caret
visibility before re-metricing and restores it afterward — without
snapping a viewport that was intentionally scrolled away — because
`reshape()` shrinks the shaped slice and `CursorByte` only
scroll-to-cursors on byte *change*, so an enlarged font could
otherwise hide a stationary caret indefinitely (finding 3). Family
resolution additionally requires the fontdb face to be
**monospaced** (`FaceInfo.monospaced`, fontdb 0.23); proportional
families take the deterministic fallback, pinned by an embedded
proportional test font — measured-advance support for proportional
faces is deferred, not designed around (finding 4). The `pmacs.gpu`
module and the preference handle install before
`load_user_config` runs, and acceptance proves `set_font` works
from init.lua and survives into the first attachment (finding 5).
Ground truth corrected: seven family-literal sites, five
`TextRenderer`s, `atlas.trim()` already runs after every submitted
frame and only clears the glyphs-in-use set (no immediate orphan
eviction — the per-frame trim cycle ages old-font glyphs out, no
explicit step needed), and menu/mb/completion buffers are rebuilt
unconditionally per frame, so explicit shaping-cache invalidation
applies only to the status buffers that cache composed strings
(finding 6).

Revision 1: initial framing.

## Ground truth (as of `de50a51`, protocol v16)

### Font handling in pmacs-gpu today — everything hardcoded

- One bundled font: `const JETBRAINS_MONO: &[u8] =
  include_bytes!("../fonts/JetBrainsMono-Regular.ttf")`
  (`pmacs-gpu/src/main.rs:54`), loaded via
  `font_system.db_mut().load_font_data(...)` in `assemble()`
  (`:1845-1846`) — the only font-DB mutation site. `FontSystem::new()`
  also loads system fonts (cosmic-text default), so system families
  are already resolvable; nothing selects them.
- The family literal `Family::Name("JetBrains Mono")` appears at
  seven call sites (`:1946, :3113, :3681, :3751, :3808, :3906,
  :6590`); sizes and line heights are compile-time consts in
  **logical pixels** — cosmic-text metrics, not typographic points —
  (`:90-158`): `CODE_FONT_SIZE 16.0` / `CODE_LINE_HEIGHT 22.0`,
  `STATUS_FONT_SIZE 13.0` / `STATUS_LINE_HEIGHT 18.0` /
  `STATUS_BAND_HEIGHT 26.0` (`:129`),
  `MENU_FONT_SIZE 14.0` / `MENU_LINE_HEIGHT 22.0` /
  `MENU_ROW_HEIGHT 22.0` / `MENU_CHAR_W 8.4`,
  `MB_DROP_FONT_SIZE 13.0` / `MB_DROP_LINE_HEIGHT 20.0` /
  `MB_DROP_ROW_HEIGHT 20.0`, `GUTTER_MONO_ADVANCE_FALLBACK 9.6`. The
  main code buffer additionally uses a literal
  `Metrics::new(16.0, 22.0)` (`:1879`) rather than the consts. No env
  var, no CLI flag (`parse_args`, `:301`).
- Seven glyphon `Buffer`s carry `Metrics` frozen at construction
  (`buffer`, `status_buffer`, `status_left_buffer`, `menu_buffer`,
  `mb_buffer`, `completion_buffer`, `gutter_buffer`; built
  `:1879-1942`). One shared `TextAtlas` (`:390`, built `:1857`) feeds
  five `TextRenderer`s; `self.atlas.trim()` already runs after every
  submitted frame (`:4952`) — glyphon's `trim` only clears the
  glyphs-in-use set (`text_atlas.rs:219`), which makes unused glyphs
  ELIGIBLE for later LRU-style eviction under allocation pressure;
  they do not age out on their own, and stale entries are wasted
  atlas space, never wrong rendering. Menu/minibuffer/completion
  buffers are rebuilt unconditionally each frame (`:3740`); only the
  two status buffers cache composed strings behind string-equality
  gates.
- Metric-derived layout: `mono_advance()` reads the first shaped
  glyph's width (`:3034-3040`); `gutter_width_px()` (`:3045-3063`);
  `text_left()` (`:3068`); `estimated_visible_lines()` divides by
  `CODE_LINE_HEIGHT` (`:5463`); scroll-wheel line math (`:1299`);
  caret pixels from shaped-run geometry (`caret_rect` `:5251-5294`);
  hit-testing (`:3142`) and popup row math (`:3219`, `:3974`).
- Scale factor is unhandled: `scale: 1.0` hardcoded in every
  `TextArea`; no `ScaleFactorChanged` arm. Pre-existing gap,
  orthogonal to this stage (Deferred).
- Reshape/resize paths that exist: `reshape()` (`:4305-4334`)
  rebuilds the code buffer's lines from `line_chunk_cache` and
  re-shapes; `resize()` (`:4344-4381`) reconfigures the surface and
  calls `set_size` on four buffers (width/height only — never
  `Metrics`) then `reshape()`. Neither touches family or size.
- **Library capabilities at the pinned versions** (glyphon 0.11.0 /
  cosmic-text 0.18.2, verified in the registry sources):
  `Buffer::set_metrics(&mut FontSystem, Metrics)` exists and
  re-shapes (`cosmic-text buffer.rs:566`; borrowed-with-font-system
  variant `:1419`), and `TextAtlas::trim()` exists (`glyphon
  text_atlas.rs:334`). Runtime reload therefore needs **no renderer
  or atlas rebuild** — the field types are owned values with no
  lifetime coupling (`font_system: FontSystem` `:387`; atlas and
  renderers meet only inside per-frame `prepare(...)` calls).
- Cosmic-text's `Buffer::shape_until_cursor` updates all three `Scroll`
  components (`buffer.rs:320-413`), but glyphon 0.11's `TextRenderer`
  consumes the vertical layout-run positions and passes each glyph's
  unshifted `x` to `physical(...)` (`text_render.rs:237-257`): it does
  NOT apply `Scroll.horizontal`. Stage 2 can therefore use the library's
  wrapped-run/vertical following, but cannot promise horizontal reveal
  from that field. With `Wrap::WordOrGlyph` and a buffer width equal to
  the paint clip, normal code wraps; visibility for one indivisible
  glyph wider than the code viewport remains a named Deferred case.

### The design-doc claim this stage corrects

`docs/pmacs-gpu-design.md:278` ("Font at v0.1", stance γ `:280`)
sketches `pmacs.gpu.set_font(path)` "or similar" (`:292`) and claims
(`:298-299`): "The bundled-default-plus-override shape means v0.1
works without configuration; **future customization needs no
wire-protocol changes**." That claim predates the Q#UX1 lesson
(rendering is frontend-local; *control* is daemon-owned Lua, so a
preference must cross the wire as a versioned fact — the LineNumbers
v13/v14 and ThemeFacts v16 shape). This framing supersedes it, and
the design doc is corrected in this stage's diff.

### The facts-channel template post-v16

- `PROTOCOL_VERSION = 16` (`pmacs-protocol/src/message.rs:1376`);
  `SUPPORTED_PROTOCOL_VERSIONS = [6..=16]` (`:1439`). Pin tests:
  version pin (`src/protocol.rs:1712`), resume ladder accepting
  `6..=16` and rejecting `17` (`:1776-1800`), postcard round-trips,
  and the placement byte-pin `completion_popup_encoding_is_
  unchanged_by_the_v16_build` (`:1835-1862`).
- `ThemeFacts` is the **final** `InstanceMessage` variant
  (`message.rs:991-997`); postcard discriminants are ordinal, so new
  variants append after it, and the final-pre-bump variant gets a
  byte-level encoding pin.
- Producer pattern (`src/semantic_render.rs`): bufferless global
  facts follow `theme_facts_msg` (`:1123-1153`) — an `Option`-seeded
  epoch gate plus an `Option`-seeded last-payload baseline, both
  advancing on computation, yielding exactly one authoritative send
  per attachment (the default state included) and cached-compare
  suppression thereafter. `on_buffer_snapshot_sent` (`:419-429`)
  deliberately does NOT reset bufferless baselines.
- Daemon write-loop: per-peer `>= version` gates + skip arms
  (`src/daemon.rs:1102-1141`, `:1177-1180` for ThemeFacts) as
  belt-and-braces over the producer-side `for_peer` gate.
- The grid TUI folds all facts variants into one silent-drop arm
  (`src/frontend.rs:395-425`) with a per-variant regression test.
- No `pmacs.gpu` Lua namespace exists anywhere; top-level modules
  install via `pmacs.set("<name>", install_<name>_module(...)?)` in
  `install()` (`src/lua_bindings/mod.rs:2029`; recent examples
  `:2085-2087`). `pmacs.theme.set` is deliberately NOT init-gated
  (mid-session live, `mod.rs:6983-7053`); `require_init_phase`
  (`:659`) gates only lifecycle APIs (`pmacs.attach`,
  `pmacs.packages.*`).

## Decisions

### Q#F1 — Scope: one global font preference, family + size, live

Stage 2 ships `pmacs.gpu.set_font { family?, size? }`: a single
GLOBAL daemon-side preference (family name and/or size in logical
pixels),
carried to GPU-capable peers as a new bufferless fact at protocol
v17, applied mid-session with full visual rebuild. The grid TUI
silently drops it (terminal fonts belong to the terminal). Out of
scope, named in Deferred: font-file paths/bytes, per-frontend
overrides, per-surface size knobs, weight/style variants, DPI/scale
work.

### Q#F2 — Lua surface: `pmacs.gpu.set_font`, un-gated, kwargs table

- `pmacs.gpu.set_font { family = "Iosevka", size = 18 }` — both
  fields optional; an absent field means "frontend default for that
  axis" (the sanitized current-order "JetBrains Mono" family query,
  Q#F6, or 16.0 logical px).
  `pmacs.gpu.set_font {}` therefore resets to defaults. Replacement
  semantics, not merge: the table IS the preference (the
  `pmacs.theme.set` wholesale lesson, Q#TH10). `size` is in
  **logical pixels** (what today's constants are). Quantization rule
  (round 2 finding 5): the ORIGINAL finite value is range-checked
  first (`6.0 <= size <= 72.0` — so `5.999` errors instead of
  rounding into range), then converted to the nearest hundredth via
  `round` (`(size * 100.0).round() as u32`); the getter returns the
  quantized value.
- Validation is daemon-local and throws (house conventions):
  `family` must be a non-empty string; `size` must be a finite
  number in `[6.0, 72.0]` logical px. **Family existence is NOT
  validated daemon-side** — fonts are frontend-local resources and
  the daemon never learns what is installed (no-pixels corollary);
  resolution failure is a deterministic frontend fallback (Q#F6).
  The kwargs table is strict plain data: read `family`/`size` with
  `raw_get`, reject every other raw key with that key named, and never
  consult `__index`/`__pairs`. Parse, validate, and quantize the complete
  table before locking or changing the preference. The getter returns a
  fresh plain table, not the stored table or a mutable handle.
- The module and the preference handle install before
  `load_user_config` runs (`src/editor.rs:455`), so `set_font` is
  init.lua-reachable: font selection is primarily configuration,
  and the preference set at init must survive into the first
  attachment (finding 5).
- Getter `pmacs.gpu.font()` returns the current preference table
  (getter, not a stored handle). No init gating: the setter follows
  the live `pmacs.theme.set` pattern, not `require_init_phase`.
- New module: `pmacs.set("gpu", install_gpu_module(...)?)` — the
  namespace is greenfield; `set_font`/`font` are its first members.

### Q#F3 — Daemon state: a shared handle with a monotonic epoch

`FontPref { family: Option<String>, size_centi_px: Option<u32>,
epoch: u64 }` behind an `Arc<Mutex<...>>` handle on `EditorState`,
mirroring the theme handle's shape (`syntax_registry.theme()`); the
Lua setter validates and quantizes fully before locking, writes the
whole preference, and bumps `epoch` from its prior value (the Q#TH6
transactional-mutator lesson — trivial here since the payload is two
scalars, but the increment-only invariant is kept so producer gates
stay monotonic). The handle exists before `load_user_config` so
init.lua writes land in the same state the first attachment's
producer reads (finding 5).

### Q#F4 — Wire fact: `FontFacts`, protocol v16→17, appended final

```rust
/// Arc 4 stage 2 (protocol v17). The daemon-relayed GPU font
/// preference. One global instance ⇒ bufferless (the ThemeFacts
/// shape). Complete replacement each send; `None` means the
/// frontend's built-in default for that axis. The daemon relays a
/// PREFERENCE — it never learns metrics, advances, or what
/// resolves; the frontend owns resolution and every pixel
/// consequence (no-pixels invariant).
FontFacts {
    family: Option<String>,
    /// Font size in HUNDREDTHS of a logical pixel (1600 = today's
    /// 16.0) — an integer because `InstanceMessage` derives `Eq`
    /// (`message.rs:490`), which `f32` cannot satisfy, and because
    /// cosmic-text metrics are logical pixels, not typographic
    /// points. Validated range 600..=7200.
    size_centi_px: Option<u32>,
},
```

- Appended after `ThemeFacts`, the current final variant; the
  placement guard is a byte-level encoding pin of `ThemeFacts` (the
  final pre-v17 variant), the same discipline as the v16
  `CompletionPopup` pin.
- `PROTOCOL_VERSION` 16→17 with a ladder paragraph;
  `SUPPORTED_PROTOCOL_VERSIONS` grows 17; pin tests updated
  (version pin 17; ladder accepts `6..=17`, rejects 18; `FontFacts`
  postcard round-trip incl. the all-`None` shape). The integer wire
  size keeps `Eq`/`Hash` derivability and makes the cached-compare
  exact by construction; the GPU converts once
  (`size_centi_px as f32 / 100.0`) at application.
- Daemon write-loop gains `peer_knows_font_facts = negotiated >= 17`
  + skip arm; the producer is peer-version-aware via the existing
  `for_peer` (the PR #120 round-1 lesson — but note there is no
  pre-v17 side channel that could leak font state, so the gate has
  no summary-style companion filter).
- TUI: `FontFacts` joins the silent-drop family arm with the
  family's regression-test pattern.
- `docs/semantic-frontend-protocol.md` gains the v17 bufferless
  `FontFacts` schema, authoritative-default/late-join behavior,
  `< 17` exclusion, and the fact that buffer snapshots reset neither
  producer nor frontend font preference. This is a wire contract, not
  only a GPU implementation detail.

### Q#F5 — Producer: `font_facts_msg`, the ThemeFacts discipline

`SemanticRenderState` gains `last_font_epoch: Option<u64>` and
`last_font_facts: Option<(Option<String>, Option<u32>)>`, both
seeded `None`: every attachment receives exactly one authoritative
`FontFacts` — the all-default `(None, None)` included — with its
first frame after viewport declaration; unchanged ticks compare one
`u64`; both records advance on computation (an identical re-set
emits nothing but records the inspected epoch). Bufferless ⇒
`on_buffer_snapshot_sent` does not touch either field. Emission
rides `render_frame`'s existing list after `theme_facts_msg`.

### Q#F6 — GPU application: resolve-or-fallback, then full re-metric

On a `FontFacts` arrival the GPU replaces its font state wholesale:

- **Wire validation, fail closed** (round 2 finding 1): `FontFacts`
  is deserialized protocol input — the daemon-local Lua validation
  is a UX courtesy, not a trust boundary. Before mutating ANY state,
  `apply_font_facts` checks `size_centi_px ∈ 600..=7200`; an
  out-of-range value (0 would panic `Buffer::set_metrics`,
  cosmic-text `buffer.rs:563`; huge values produce pathological
  metrics/allocations) rejects the whole message: logged to stderr,
  current state kept, nothing re-shaped.
- **Resolution (frontend-local)**: `family: Some(name)` resolves
  against the existing fontdb (bundled JetBrains Mono + system
  fonts) AND every face the shipped attribute set can select must be
  **monospaced**
  (`FaceInfo.monospaced`, fontdb 0.23) — `mono_advance()` treats the
  first shaped glyph's width as universal (`:3030-3040`) and menu
  width multiplies a fixed advance by character count (`:3194`), so
  a proportional family would silently under-size gutters and menu
  hitboxes (finding 4). Unresolvable OR non-monospaced families fall
  back to the default — deterministic, logged to stderr, never
  round-tripped back (the daemon never learns resolution outcomes;
  no-pixels). **The default is a sanitized, current-order "JetBrains
  Mono" FAMILY query** (round 3 finding 4). `assemble()` replaces the
  current `FontSystem::new(); db_mut().load_font_data(...)` order with
  an explicit `fontdb::Database`: call `load_system_fonts()` FIRST as
  `FontSystem::new()` does today, load the bundled bytes second with
  `load_font_source(Source::Binary(...))`, and retain the returned
  bundled `fontdb::ID`. Then remove any NON-monospace face that
  advertises the exact `"JetBrains
  Mono"` family (collect IDs before `remove_face`; the bundled face is
  monospaced and survives). This prevents a closer-weight proportional
  system face from winning bold/italic text even when the normal query
  selected a valid monospaced system face. Restore cosmic-text's current
  generic-family defaults (`Noto Sans Mono`, `Open Sans`, and `DejaVu
  Serif`), then construct
  `FontSystem::new_with_locale_and_db` using `sys_locale::get_locale`
  with cosmic-text's current `"en-US"` fallback when it returns `None`
  (new direct `pmacs-gpu` dependency; fontdb continues through
  glyphon's cosmic-text re-export). Fontdb returns the first surviving
  equally-good candidate in insertion order (`lib.rs:661`), so a valid
  monospaced system JetBrains face retains today's precedence while the
  bundled face guarantees a survivor after invalid collisions are
  removed. Constructing `FontSystem` only after every load/filter also
  includes every surviving monospaced ID (the bundle included) in
  cosmic-text's internal monospace-ID set.
  Resolution checks the four queries reachable through the shipped
  attributes — normal, bold, italic, and bold-italic, all at normal
  stretch — and rejects a requested family if any selected face is not
  monospaced. The normal query and assertion are the same
  `fontdb::Query` implied by the base `Attrs` installed on all seven
  buffers. The retained DEFAULT query ID and the bundled ID are both
  asserted present and monospaced at assembly. `family: None`
  and every rejected requested family use that same known-monospace
  query: the fallback is total, cannot recurse into a proportional
  collision, and never-set/reset/fallback resolve identically.
  Measured-advance support for proportional faces is Deferred, not
  designed around.
- **Metrics**: the hardcoded consts become `State` fields derived
  from the preference size (default 16.0) by fixed ratios — code
  line height 22/16, status 13/16 + 18/16 + **band height 26/16**
  (`STATUS_BAND_HEIGHT`, finding 2), menu 14/16 + 22/16, mb dropdown
  13/16 + 20/16, `MENU_CHAR_W` 8.4/16,
  `GUTTER_MONO_ADVANCE_FALLBACK` 9.6/16 — so one knob scales every
  surface coherently and an unset size reproduces today's constants
  bit-for-bit. Never-set/reset are byte-identical within the resulting
  process; pre-stage pixels are also preserved when the old winning
  family is monospaced across the four shipped style queries. A
  non-monospace collision is the deliberate safety exception (the
  Q#TH5 default-preservation lesson applied within the viable domain).
  The derived band height threads through buffer sizing, band quad
  geometry, `text_area_bottom`, minimap height, and the visible-line
  math — at size 72 the status line is 81 logical px and today's
  fixed 26 px band would clip it. The stray literal
  `Metrics::new(16.0, 22.0)` (`:1879`) is unified into the same
  fields. **Dimensions change atomically with metrics** (round 3
  finding 3): use `set_metrics_and_size`, not `set_metrics` (which
  deliberately preserves the old dimensions). Code and gutter height
  are the nonnegative drawable code height
  (`(text_area_bottom - TEXT_TOP).max(0.0)`), status
  height is the derived band height, and menu/mb/completion height is
  the current surface height. The code buffer's width is its actual
  nonnegative drawable width
  (`(text_bounds_right - text_left).max(0)`), not the whole
  surface: wrapping and `shape_until_cursor` must use the same clip the
  painter uses. Because `text_left` depends on the newly-shaped
  `mono_advance`, the rebuild may make one internal measure pass and
  one final size/shape pass; no frame is submitted between them.
  `resize()` goes through the same dimension helper for ALL seven
  buffers, closing the existing four-of-seven resize skew.
  The same helper and settle transaction also own every runtime input
  that changes the code clip without changing the font: line-number
  mode, a gutter digit-count transition after full or incremental text,
  minimap presence after `FileStyleSummary`, and summary removal during
  even a byte-identical `BufferSnapshot`. Each path captures the old
  painted-caret predicate before changing geometry, synchronizes the
  final buffer dimensions before shaping, and follows only when that
  predicate was true.
- **Rows stay rows**: menu, minibuffer-candidate, and completion buffers
  explicitly use `Wrap::None`. Their protocols, row-window calculations,
  selection quads, and hit tests all assign exactly one row-height to one
  source line; allowing a long label to wrap after a size/family change
  would paint glyphs on a second visual row that still hit-tests as the
  following item. The existing pixel bounds remain responsible for
  horizontal clipping. Code and gutter retain wrapping; status strings
  may clip within their single derived band.
- **Font-dependent advances without default drift**: the internal
  measure pass shapes a fixed ASCII probe in the resolved family at the
  relevant metrics, independent of document contents. It sums every
  shaped run's width and divides by the probe's logical cell count;
  sampling one glyph is invalid because even a monospaced face may
  shape several probe characters into one multi-cell ligature. It
  records the selected/default advance ratio. The empty-code gutter
  fallback and `menu_char_w` use today's exact constants multiplied by
  that ratio;
  the sanitized per-process default is therefore ratio 1 and remains
  byte-identical, while an alternate monospace family cannot leave stale
  JetBrains-only gutter/menu geometry. The measured NORMAL-face advance
  becomes authoritative for the normal-style gutter even when the first
  code glyph is bold/italic (different monospaced faces need not share an
  advance); `mono_advance()` no longer samples an arbitrary code glyph.
  The measure result is committed with the other derived geometry before
  hit maps are dirtied.
- **Extreme-size context-menu policy** (round 3 finding 2): the
  context menu keeps its current raw pointer anchor, full row set, and
  fixed 380 px width cap. Wgpu clips geometry at the surface, so it
  remains safe, but long labels and/or lower rows may be clipped at
  large configured sizes; unlike the minibuffer and completion
  dropdowns it does NOT claim surface containment. Adding viewport-
  aware horizontal ellipsis plus vertical flip/window/scroll is one
  named Deferred item, not smuggled into the font reload. Acceptance
  exercises the clipped route for no panic and coherent hit geometry,
  while containment assertions cover only surfaces that own it.
- **Caret visibility across the re-metric — visual runs, not source
  lines** (round 3 finding 1): `Buffer` defaults to
  `Wrap::WordOrGlyph`; a source line that fits at size 6 can occupy
  several visual runs at size 72, so source-line-only
  `scroll_to_cursor` cannot uphold a painted-caret guarantee. `State`
  therefore retains a normalized code-buffer `Scroll` (slice-local
  `line == 0`, the `vertical` residual, and `horizontal == 0`), and full
  reshapes reapply it instead of blindly installing `Scroll::default`.
  This residual is buffer-scoped view state: `BufferSnapshot` resets it
  to default even though the global font preference/metrics survive.
  A shared `normalize_code_scroll` runs after EVERY final code shape,
  not only caret following: if cosmic-text advances slice-local
  `scroll.line` because new wrapping/metrics make the retained vertical
  offset cross source lines, add that delta to whole-file `scroll_top`,
  retain the residual, rebuild from the new source origin, and repeat
  until `line == 0`. Each iteration must strictly advance the clamped
  source origin; at EOF, a non-advancing residual is clamped to the last
  source line with `Scroll::default` rather than looping. This preserves
  an intentionally caret-free viewport through a size decrease without
  leaving a stale/blank slice. Any
  incidental scroll changes made by intermediate measure/metric calls
  are discarded; normalization starts from the pre-transaction retained
  scroll against the FINAL family, metrics, dimensions, and attrs.
  A shared byte-to-layout helper selects the visual run whose glyph byte
  interval contains the target (or the final run at source-line end); it
  replaces the current first-run-of-source-line scans in both
  `caret_rect` and `completion_anchor_px`. The helper first inverts that
  line's `line_chunk_cache` projection — source bytes are not projected
  bytes when inline adornments are present — using the earliest
  projected boundary for an adornment anchor (the current left-gravity
  caret placement), then uses cosmic-text's `layout_cursor`/affinity so
  a wrap boundary selects the same run as `shape_until_cursor`. A source
  byte inside a combining or ligature cluster is snapped explicitly to
  the cluster's logical end with `Before` affinity; it is never handed
  to cosmic-text as an unrepresentable interior cursor, whose fallback
  is the source-line start. A shared
  `ensure_caret_painted` helper first performs the existing coarse
  source-line `scroll_to_cursor` and rebuild when the byte is outside the
  shaped slice, then maps the byte to a cosmic-text `Cursor` and calls
  the library's `Buffer::shape_until_cursor` (`buffer.rs:320-413`), which already
  follows wrapped layout runs vertically. The helper deliberately
  discards the call's `Scroll.horizontal` result: glyphon 0.11 does not
  apply that component when placing glyphs, so retaining it would make
  state claim a scroll the painter never displays. The helper then calls
  `normalize_code_scroll`; only after its final source origin is stable
  may it declare the viewport. Explicit wheel/minimap/source-line jumps
  clear that residual; ordinary full reshapes preserve it. The existing
  `CursorByte` arm uses this helper under its existing `moved` gate
  too, fixing its identical pre-existing wrapped-line hole without
  snapping a stationary cursor after a wheel/minimap scroll. The
  optimistic edit completion path also replaces its current
  source-line-only `scroll_to_cursor` call with the helper: it has already
  installed the predicted `own_cursor`, and the confirming identical
  `CursorByte` will have `moved == false`, so deferring visual-run repair
  would leave a newly wrapped caret off-screen indefinitely. The gutter
  projection mirrors the code layout at the same time: emit the source
  line number on its first visual run and blank gutter rows for wrapped
  continuation runs, then apply the same normalized vertical scroll to
  the gutter buffer. That keeps line numbers aligned when the font
  change creates wraps instead of exposing the existing one-row-per-
  source-line skew.
- The pre-change follow decision remains conservative: compute it
  before any mutation from the ACTUAL code-caret rectangle intersected
  with the drawable code clip (and only when the minibuffer is closed),
  not from `view_range`; this excludes both the two-line source
  overscan and wrapped runs clipped below the band. Painted before ⇒
  run `ensure_caret_painted` after the new family/metrics are shaped.
  Not painted before ⇒ preserve the user's scroll and do not call the
  helper. Thus the font change never turns an overscan-only caret into
  a snap-back, while a formerly painted caret survives new wrapping.
  `resize()` uses this same painted-before policy after its final
  dimensions are installed: narrowing a window cannot strand a
  stationary caret in a new wrap, and widening an intentionally
  caret-free viewport only normalizes its retained scroll.
- **Rebuild sequence** (one transaction, `apply_font_facts`): validate
  the wire size (fail closed, above); record actual painted-caret
  visibility; resolve the known-safe family; store the derived metrics;
  set the three row-oriented popup buffers to no-wrap; measure the
  selected/default advance ratio; update metrics + current dimensions
  on all seven buffers; clear the two status shaping caches
  (`status_text`, `status_left_text` — the
  only string-equality gates; menu/mb/completion rebuild
  unconditionally per frame); attrs-bearing reshape/measure; settle
  the final drawable code width and reshape if it changed; if the old
  caret was painted, run the visual-run helper; otherwise run the same
  scroll normalizer without caret following; recompute dependent layout,
  set `hit_map_dirty`, drop
  the minimap vertex cache, `request_redraw()`, and finally call
  `viewport_send_if_changed`. No intermediate state renders and the
  viewport is derived from the final normalized source origin. No
  atlas action: the per-frame `atlas.trim()` (`:4952`) clears
  `glyphs_in_use`, making old-font glyphs eligible for later LRU-style
  eviction under allocation pressure (they do not age out on their own
  — glyphon `text_atlas.rs:219`).
- The seven `Family::Name` literals collapse into one accessor on
  `State` so family application is a single site.

### Q#F7 — Default semantics and late join

`(None, None)` is a real, always-shipped state meaning "frontend
built-ins" — an attachment never infers defaults from silence (the
Q#TH7 authoritative-per-attachment lesson). A late-joining GPU peer
receives the current preference among its first frames; a running
peer receiving `(None, None)` after a themed session resets to the
sanitized current-order JetBrains Mono query and today's constants
exactly.

## Bets

- `Buffer::set_metrics_and_size` + attrs-bearing re-set is a sufficient
  reload path at glyphon 0.11 / cosmic-text 0.18 — no atlas or
  renderer rebuild, no `FontSystem` swap (the db only ever grows;
  system fonts load at startup); the per-frame `atlas.trim()`
  cycle makes old-font glyphs eligible for later eviction under
  allocation pressure, which is sufficient because stale entries
  are only wasted atlas space, never wrong rendering. The headless
  harness runs the identical `assemble()` path, so this bet is
  testable end to end under `PMACS_REQUIRE_GPU=1`.
- Proportional scaling of the chrome constants from one size knob is
  acceptable at stage 2; per-surface knobs are deferred, not
  designed around.
- Hermetic family testing: CI GPU runners may have zero system
  fonts, so acceptance embeds FOUR test-local faces (test bytes, not
  shipped assets): a second **monospaced** family to prove
  resolution-and-switch, a **proportional** family to pin the monospace
  gate's fallback, a monospaced TEST-DEFAULT face to model today's
  system-order winner, and a BOLD proportional face carrying that same
  test-only family name to pin the sanitized-default collision and
  styled-query rule. The database sanitizer takes the default family
  name and bundled-equivalent ID as internal parameters; production
  passes `"JetBrains Mono"` and the real bundled ID, while tests use an
  unreserved fixture name. A test-only assembly input loads all four into the explicit
  pre-`FontSystem` database so cache/monospace-ID construction and the
  collision filter are the production path, not a post-construction
  approximation. The missing-family path is exercised with a name
  guaranteed absent.
  Test-font licenses/notices live beside the fixtures.
- The scale-factor gap stays orthogonal: this stage neither fixes
  nor worsens DPI handling (`scale: 1.0` everywhere, unchanged).

## Deferred (named)

Font file paths/bytes over the wire (the design doc's original
`set_font(path)` sketch — needs a resource channel, plausibly the
dormant `ResourceOffer`, and a frontend-trust story);
**proportional-family support** (per-glyph layout widths replacing the
single measured monospace ratio and chars×constant hit geometry — the
monospace gate is the stage-2 stance, finding 4); per-frontend font
overrides (the LineNumbers `frontend_id`-routed shape is available
if wanted); per-surface size knobs (status/menu/dropdown independent
of code); weight/style variants (bold/italic family selection —
interacts with the chrome-attribute mask widening already deferred
by stage 1); fallback-chain configuration; ligature/feature toggles;
DPI/scale-factor handling (pre-existing gap: no `ScaleFactorChanged`
arm, `scale: 1.0` hardcoded); cursor-blink and other GPU chrome
config the roadmap groups nearby; TUI font anything (terminal-owned
by definition); **viewport-aware context-menu layout** (horizontal
ellipsis plus vertical flip/window/scroll — extreme sizes deliberately
surface-clip in stage 2); wrap-exact minimap-thumb/status-percentage
accounting (the current source-line estimate remains conservative;
visual-run exactness is load-bearing for caret following and gutter
alignment here, not promoted into the whole-file overview model);
horizontal reveal for a single indivisible glyph wider than the code
viewport (cosmic-text computes `Scroll.horizontal`, but glyphon 0.11's
renderer does not apply it; a frontend-local x-offset would have to
thread through painting, caret/decorations, popup anchors, and hit
testing).

## Acceptance

Suites: `tests/gpu_font_acceptance.rs` (wire + Lua + producer),
existing pin/round-trip homes in `src/protocol.rs`, and the GPU
route tests in pmacs-gpu's headless suite (`PMACS_REQUIRE_GPU=1`).

1. **Version pins**: `PROTOCOL_VERSION == 17`; ladder accepts
   `6..=17` and rejects 18; `FontFacts` postcard round-trip (both
   populated and all-`None`); byte-level encoding pin of
   `ThemeFacts` (the final pre-v17 variant) proving the appended
   placement shifted no existing discriminant.
2. **Authoritative default per attachment**: a fresh session's first
   frame after viewport declaration carries `FontFacts { family:
   None, size_centi_px: None }`; unchanged ticks are silent; a
   late-joining second session receives the current preference
   without any mutation post-attach.
3. **Live re-ship**: `pmacs.gpu.set_font { size = 18 }` mid-session
   emits exactly one `FontFacts` on the next frame; an identical
   re-set advances the inspected epoch without emitting (asserted on
   internal state, the caches-advance-on-computation pin).
4. **Snapshot survival, both sides**: `on_buffer_snapshot_sent` leaves
   the producer's font baselines untouched — an A → B → A round trip
   re-ships buffer facts but NOT `FontFacts`. The GPU `BufferSnapshot`
   arm retains the resolved family, metrics, and derived geometry; it
   resets the normalized code scroll with the other BUFFER-scoped view
   state so B cannot inherit A's visual residual. The new buffer shapes
   under the same preference without waiting for a redundant global
   fact. A byte-identical snapshot also removes the prior buffer's
   minimap reservation before reshaping, so identical text cannot retain
   the old buffer's narrower code clip.
5. **Version gate**: a v16 peer session never receives `FontFacts`
   (producer `for_peer` + daemon skip arm, the real-daemon probe
   shape from stage 1).
6. **Lua contract**: bad size (non-finite, out of range — including
   `5.999`, which must error rather than round into range) and bad
   family (empty, non-string) throw with the offending field named;
   nothing lands and nothing emits on a failed set; quantization
   pins values on both sides of a hundredth (e.g. `15.994` → 1599,
   `15.996` → 1600); an unknown key is rejected with its name; a hostile
   or value-providing metatable is never invoked; `pmacs.gpu.font()`
   returns a fresh quantized plain table; `set_font {}` resets both
   axes. Every rejected shape leaves state and emissions untouched.
7. **Init.lua reachability** (finding 5): a `load_user_config_at`
   fixture whose init.lua calls `pmacs.gpu.set_font` succeeds, and
   the first attachment's first frame ships that preference (the
   handle installs before user config runs, `src/editor.rs:455`).
8. **TUI drop arm**: the grid frontend consumes `FontFacts` without
   error (family test pattern).
9. **GPU size route**: applying `FontFacts { size_centi_px:
   Some(2000) }` to a headless state changes the rendered frame,
   widens `mono_advance`/gutter, and reduces
   `estimated_visible_lines`; re-applying `(None, None)` restores
   the original frame byte-identically (unset = today's constants).
10. **GPU band geometry at the bounds — owned containment only**
    (round 1 finding 2, narrowed in rounds 2/3): with status band,
    minibuffer, and completion surfaces open, applying the minimum
    (600) and maximum (7200) sizes yields VERTICAL containment — code
    glyphs stop at the band edge, status glyphs fit inside the derived
    band height, and the two dropdowns' existing row windows remain
    inside the surface. Popups may occlude code by layer contract. A
    separate context-menu route uses the shipped multi-row menu near
    the lower edge at size 7200 and asserts safe surface clipping, no
    panic, and hit-testing for pixels inside the surface that agrees
    with the same clipped geometry; it does NOT assert containment or
    complete labels/rows. The derived
    band height tracks the status line, and `text_area_bottom` /
    minimap height / source-line visible estimate follow.
11. **GPU caret survival — source + wrapped visual runs** (round 1
    finding 3 + round 2 finding 2 + round 3 finding 1):
    with the caret on the OLD last visible line, applying a larger
    size keeps the caret rendered and the re-declared viewport's
    origin corrected after scroll normalization; with the caret
    deliberately scrolled to exactly ONE source line past the painted
    window — inside `view_range`'s two-line overscan — the same size
    change does NOT snap back. The load-bearing wrap bite is one long
    source line whose end caret is painted at size 600 but wraps below
    the code clip at size 7200: after the change the caret is painted,
    `Buffer::scroll` carries the needed VERTICAL visual-run offset (and
    keeps `horizontal == 0`), any nonzero slice-local line is normalized
    into `scroll_top`; `view_range` agrees with that source origin, and
    gutter continuation blanks keep
    the next source-line number aligned. The caret rect and completion
    popup anchor both resolve to the run containing the byte rather than
    the first run of that source line; an inline adornment before the
    byte proves the source→projected conversion is not an identity map.
    A moved `CursorByte` repeats the guarantee; an optimistic insertion
    that creates a new bottom-edge
    wrap follows immediately and its identical confirming `CursorByte`
    needs no second repair; a stationary cursor after an explicit wheel
    scroll does not snap. A reverse 7200→600 change while the caret is
    off-screen collapses wraps, translates any cosmic `scroll.line`
    advance into whole-file `scroll_top`, and preserves a nonblank
    viewport without following the caret. A width-only narrow/widen
    resize repeats both painted and scrolled-away halves through the
    shared helper.
12. **GPU family routes** (rounds 1/3 finding 4): a test-embedded second
    monospaced font resolves and changes the frame; a test-embedded
    PROPORTIONAL font is rejected by the monospace gate and falls
    back to the default family query; an unresolvable family name does
    the same. A monospaced system-order default fixture remains the
    default query winner in the parameterized sanitizer unit (baseline
    preservation); a same-family BOLD proportional collision is removed
    during database assembly, styled
    code still selects only monospaced faces, and the bundled ID is
    present in cosmic-text's monospace-ID set. All four fixture IDs are
    retained from the pre-`FontSystem` assembly and checked against
    cosmic-text's real `is_monospace` classification. Direct resolver
    units cover normal, bold, italic, and bold-italic queries. Both
    rejected-request routes and the collision-safe default reset render
    byte-identically to never-set.
13. **GPU shaping-cache invalidation**: with composed band strings
    constant, a size change re-shapes the status band (the Q#TH8
    counter lesson applied to metrics; only the status buffers cache
    composed strings, finding 6).
14. **GPU viewport re-declaration**: a size change that alters
    `estimated_visible_lines` produces a `Viewport` re-declaration
    (`viewport_send_if_changed` returns `Some`).
15. **Protocol/design docs**: `docs/semantic-frontend-protocol.md`
    records `FontFacts`, its v17 gate, authoritative default, and
    snapshot survival; `docs/pmacs-gpu-design.md:298-299` no longer
    claims font customization needs no wire change and points here.
16. **GPU wire validation fails closed** (round 2 finding 1):
    applying `FontFacts` with `size_centi_px` of 0 (the
    `Buffer::set_metrics` panic value), 599, 7201, and `u32::MAX`
    directly to the GPU arm mutates nothing — the frame renders
    byte-identically to before, no panic, no partial application —
    while 600 and 7200 apply.
17. **Metric/dimension atomicity and resize symmetry** (round 3
    finding 3): resize the headless surface, then apply both size
    bounds before rendering. Immediately after each application,
    `Buffer::size()` reports the actual drawable code width/height for
    code, the derived band height for both status buffers, the current
    surface height for menu/mb/completion, and the current code height
    for gutter; no buffer retains construction-time or prior-size
    dimensions. A family whose advance changes the gutter width forces
    the final code-width reflow, and the resulting wrap/hit map and
    rendered clip use that same width. Line-number enable/disable,
    gutter digit transitions, and minimap appearance/removal exercise
    the same dynamic reflow transaction independently of a font change.
18. **Popup row invariance at both size bounds**: long menu,
    minibuffer-candidate, and completion labels remain one layout run per
    wire row at 600 and 7200 (`Wrap::None`). Their selection quads and
    in-surface hit tests select the same semantic row as the painted
    glyphs; overlong horizontal text clips rather than creating an
    untracked second row.
19. **Family-dependent geometry on an empty document**: with line
    numbers and a context menu open over an empty buffer, switching to
    the alternate embedded monospace family updates the measured
    gutter fallback and menu hit width by the selected/default advance
    ratio. Reset restores the exact original geometry and frame; the
    test does not depend on a code glyph already being shaped. A styled
    twin whose first code glyph is bold proves gutter measurement still
    uses the selected family's normal face rather than that glyph.
