# Statusline segments - framing (Arc 4 stage 3)

**Revision 3 - 2026-07-21. Implemented on branch
`statusline-segments` against current `main` `bb17ec9` (#123 atop #124,
protocol v17). It advances the wire to v18, satisfies Acceptance 1-27,
and is fully gated; awaiting review, not merged.**

Revision 3: closes review findings on authoritative-empty baseline retention
and the TUI's protected-suffix clipping boundary.

Revision 2: closes review findings on invalidation, terminal-control-safe
grapheme painting, separator ownership, detached-frontend latches, and the
unknown-LSP label. Revision 1 was the initial post-#124 architecture scout.

Arc 4 names three deliverables: named UI faces, a live GPU font
preference, and a Lua statusline-segment API
(`docs/roadmap-2026-07.md:83-90`). Stages 1 and 2 landed as #120 and
#124. This framing covers **stage 3 only**. It adds composable Lua
providers to the per-window modeline, carries their text plus face
names to semantic frontends at protocol v18, and uses the existing LSP
status tracker as the first built-in provider. Completing this stage
completes Arc 4.

## Implementation record (2026-07-21)

The approved Q#SL1-Q#SL11 design is implemented without changing the
framed ownership boundary:

- `pmacs.statusline` owns a shared editor-global registry with strict
  registration, lifecycle/introspection, monotonic layout/face-set
  epochs, borrow-released three-phase evaluation, per-context failure
  latches, deterministic ordering, and bounded one-line results.
- TUI composition preserves the legacy modeline when providers are
  absent, owns separators by adjacent segment face, shapes terminal-safe
  grapheme runs, and protects the right diagnostic/cursor/scroll suffix.
- Protocol v18 appends complete `StatuslineSegments` replacements. The
  semantic producer distinguishes authoritative empty from no message,
  versions provider execution before callbacks, expands dynamic
  `ThemeFacts`, and resets buffer baselines symmetrically with
  `BufferSnapshot`.
- The GPU consumes v18 atomically, resolves exact dynamic faces, clips
  provider runs without wrapping or displacing the protected suffix,
  and preserves its prior valid state on malformed input.
- `builtin/runtime/lsp.lua` registers the first pure right-side provider
  from its private attachment map; the Rust tracker exposes bounded
  `init`/`ready`/`degraded`/`crashed`/`stopped`/unknown labels.

The final gate run was sequential and clean: `cargo fmt --check`;
workspace/all-target Clippy with `-D warnings`; 1,617 default and 1,791
CRDT library tests; 7 default and 8 CRDT stage-3 acceptance tests; 114
M4 acceptance tests (3 ignored, `basedpyright` filtered); 108 required
GPU tests; and the one-invocation workspace sweep (2,715 passed across
78 suites, 19 ignored, `basedpyright` filtered). `git diff --check` was
clean. No flaky rerun was needed.

## Ground truth (as of `main` at `bb17ec9`, protocol v17)

### There are two different bottom surfaces in the TUI

- `EditorCore.status: String` is one global, one-line transient message
  (`src/editor_core.rs:234-235`). Lua writes it through
  `pmacs.editor.set_status` (`src/lua_bindings/mod.rs:11414-11422`).
  `dispatch_key` clears it at entry (`src/editor.rs:677-685`), and the
  optimistic CRDT self-insert path clears it too
  (`src/daemon.rs:2159-2165`).
- Every TUI window reserves its own final row for a **modeline**.
  `paint_frame` renders all windows in the active frontend's layout,
  then calls `paint_mode_line` with buffer name, modified state,
  active-window marker, diagnostics, cursor L:C, and scroll state
  (`src/editor.rs:2102-2240`). The current formatter has a left string
  (`+/-`, modified marker, name) and a right string (diagnostics, L:C,
  scroll); the right side is right-aligned and dropped wholesale if it
  is wider than the window (`:2556-2617`).
- The terminal's last physical row is a separate **global echo row**.
  `build_status_line` contains only `core.status`, the last captured Lua
  error, and an in-flight key prefix (`src/editor.rs:2791-2827`).
  Isearch or the minibuffer paints over that row afterward
  (`:2244-2265`). Per-window buffer facts deliberately do not live
  there.
- `ui.modeline` owns the per-window row within its stage-1
  `{fg,bg,reverse}` mask. `ui.statusline` owns the global echo row's
  foreground only. Search/minibuffer text uses `ui.minibuffer`
  (`docs/theme-faces-framing.md` Q#TH3/Q#TH5). The two face names are
  not synonyms.
- Modeline width currently counts `char`s, not terminal display
  columns (`editor.rs:2594-2616`). A custom CJK or combining segment
  would therefore overlap its neighbor unless this stage moves the
  whole modeline through Unicode display-width discipline.
- The cell protocol already has `Glyph::Cluster` for a UTF-8 grapheme
  plus `Glyph::Continuation` for its trailing columns, and the terminal
  emitter writes clusters verbatim (`pmacs-protocol/src/cell.rs:65-77`;
  `src/frontend.rs:580-591`). `TextView` still skips combining marks,
  but that older limitation need not be copied into this new painter.
  `unicode-segmentation` is currently only transitive through
  cosmic-text; using it in the core requires one direct manifest entry.

### The GPU compresses those surfaces into one physical band

- `StatusFacts` (protocol v8, widened at v15) carries daemon-owned
  buffer name, modified flag, error/warning counts, and the transient
  `core.status` message. Cursor and scroll deliberately stay
  frontend-derived so they follow the optimistic caret
  (`pmacs-protocol/src/message.rs:764-792`;
  `docs/pmacs-gpu-status-band-framing.md` Q#S1).
- `SemanticRenderState::last_status` is a per-buffer peer-emission
  baseline. `status_facts_msg` frame-polls cheap Rust state and emits
  only on payload change (`src/semantic_render.rs:176-180`,
  `:909-976`). `on_buffer_snapshot_sent` removes that baseline because
  the frontend snapshot clears its buffer-scoped status mirror
  (`:412-450`).
- GPU composition has one left glyphon buffer and one right glyphon
  buffer. The left side's priority is minibuffer, isearch, transient
  message, then buffer name/modified (`pmacs-gpu/src/main.rs:4033-4087`).
  The right side is diagnostics followed by optimistic L:C and scroll
  (`:3971-4030`). Both use string-equality shaping caches
  (`:4089-4137`); `ThemeFacts` clears those caches because colors can
  change while strings do not (`:3035-3047`).
- The right buffer is measured and positioned flush right; the left
  buffer's clip ends before it (`main.rs:5318-5408`). Search,
  minibuffer, and transient messages replace only the left content.
  Diagnostics/cursor/scroll remain visible on the right.
- Unlike the three popup buffers, neither status glyphon buffer is
  currently set to `Wrap::None` (`main.rs:2171-2201`). Long custom text
  would otherwise wrap before its measured origin can enforce the
  single-band clipping policy. In pinned glyphon 0.11,
  `TextArea.left` is an independent `f32` origin and `TextBounds`
  performs clipping, so a negative origin is supported without
  reshaping away the protected right suffix.
- `BufferSnapshot` clears spans, decorations, adornments, summary,
  completion, search, menu, and `status_facts`; it deliberately keeps
  global minibuffer, theme, and font state (`main.rs:2736-2818`).
  A new buffer's first closed prompt state may be suppressed, so every
  new buffer-scoped status mirror must join this symmetric reset
  contract rather than wait for a later close message.

### The old `ModeLine` wire variant is not this feature's carrier

- `InstanceMessage::ModeLine(Vec<Cell>)` has existed since the first
  protocol and remains unused (`pmacs-protocol/src/message.rs:506-510`;
  the only consumers are silent-drop/debug-name arms). It contains
  daemon-painted grid cells, not structured text and face names.
- The status-band framing already rejected it: preformatted cells bake
  TUI layout into a frontend that owns font shaping and would make a
  daemon-formatted cursor visibly lag optimistic typing
  (`docs/pmacs-gpu-status-band-framing.md` Q#S1).
- Changing that existing variant's shape would be a wire break under an
  already-shipped discriminant. Reusing it unchanged would contradict
  both the frontend-local-rendering boundary and this arc's requirement
  that segments carry face names rather than raw colors.

### Lua has provider and error-isolation precedents, but no statusline registry

- `pmacs.completion.register { name, priority?, fn }` returns a stable
  userdata handle and supports unregister, priority, enable, and
  introspection (`src/lua_bindings/mod.rs:10624-10712`). The completion
  registry establishes the repository pattern for composable
  Lua-defined providers.
- Hooks snapshot callbacks before invocation so a callback can re-enter
  its registry without a `RefCell` double borrow (`src/hook.rs:250-259`).
  Hook callback errors are isolated and appended to `*errors*`
  (`src/lua.rs:278-306`).
- `paint_frame` takes a mutable `EditorCore` borrow before walking
  windows and holds it through both bottom surfaces
  (`src/editor.rs:2120-2250`). Calling arbitrary Lua inside
  `paint_mode_line` would let an ordinary provider call
  `pmacs.window.*` or `pmacs.buffer.*` and immediately double-borrow
  the core. Provider evaluation must therefore happen before that
  paint borrow, against owned context snapshots.
- The daemon stamps `core.active_frontend` before every frontend's
  projection (`src/daemon.rs:958-960`) and at session establishment
  (`:1426-1428`). `pmacs.frontend.id()` consequently has the correct
  per-session value during a pre-render provider fan-out.
- `EditorCore` already owns distinct layouts/windows per
  `FrontendId`; `active_window_for(fid)` has no cross-frontend fallback
  (`src/editor_core.rs:512-526`). A grid frontend may have several
  visible windows, while the current semantic GPU has one active
  buffer/view. Provider output must be evaluated and cached per
  frontend/window context, never as one global string.

### A real first consumer is already waiting

- `LspStatusTracker` exists specifically as the stable higher-level
  state a modeline can read (`src/lsp_status.rs:30-85`). Its tracker
  labels are the bounded set `init`, `ready`, `idx`, `degraded`,
  `crashed`, and `stopped`; `pmacs.lsp.modeline_label` additionally
  returns `"?"` for a forgotten/unknown server id (`src/lsp.rs:1190`).
- Lua already exposes `pmacs.lsp.modeline_label(server)` and a richer
  `status_summary` intended for one call per render frame
  (`src/lua_bindings/mod.rs:8700-8805`).
- `builtin/runtime/lsp.lua` owns the authoritative
  buffer-handle-to-attachment map. Its public `active_attachment`
  deliberately reads only the active window (`:721-731`), but a
  statusline provider in that same Lua chunk can safely index the
  private map by a passed `ctx.buffer`, including passive TUI windows.
- Despite comments saying LSP data feeds a modeline, no renderer
  currently consumes it. Stage 3 can prove the API on a shipped,
  useful segment instead of landing an unused extension point.

### ThemeFacts currently cannot represent arbitrary segment faces

- `Theme::face(name)` owns daemon-side dotted-prefix inheritance for
  `ui`/`ui.*` names and returns `None` when unset
  (`src/highlight.rs:207-226`).
- The namespace predicate itself currently lives only in the main
  crate as `highlight::is_face_name` (`src/highlight.rs:92-95`).
  `pmacs-gpu` cannot import that crate without reversing the dependency
  graph, so merely calling two copied expressions "shared" would leave
  registration and the untrusted wire boundary free to drift.
- The `ThemeFacts` producer resolves only the fixed twelve stage-1 face
  names in `UI_FACES` (`src/semantic_render.rs:281-299`,
  `:1137-1177`). Frontends perform exact-name lookup; they never walk
  parent names.
- Therefore a segment naming `ui.modeline.lsp` cannot inherit a
  configured `ui.modeline` on the GPU unless the producer learns that
  exact referenced name and ships its resolved style. Sending raw
  theme entries and reimplementing the walk frontend-side would
  contradict Q#TH7.

### Protocol placement

- `PROTOCOL_VERSION == 17`; supported versions are `6..=17`
  (`pmacs-protocol/src/message.rs:1414`, `:1472-1480`).
- `FontFacts` is the final variant. Postcard enum discriminants are
  ordinal; stage 2 pinned the byte encoding of the final pre-v17
  `ThemeFacts` variant. Stage 3 must append after `FontFacts` and pin
  `FontFacts` bytes before changing the enum.

## Decisions

### Q#SL1 - Scope: additive per-window modeline segments; Arc 4 ends here

Stage 3 extends the **per-window modeline/status band**, not the global
echo area:

- TUI: custom left/right segments render on each visible window's
  modeline.
- GPU: the same custom segments render in the existing status band,
  scoped to its current buffer.
- The TUI echo row remains owned by `core.status`, Lua errors, pending
  keys, isearch, and minibuffer. `pmacs.editor.set_status` is unchanged.
- GPU minibuffer/isearch/transient-message precedence remains
  unchanged. The physical single-band compromise is explicit in Q#SL5.
- Existing buffer identity, modified state, diagnostics, cursor L:C,
  and scroll facts remain built in. This API is additive; replacing,
  removing, or arbitrarily reordering those built-ins is Deferred.
- Cursor and scroll remain frontend-derived. A Lua provider receives no
  cursor/scroll value in its context; sending the daemon's cursor as a
  custom segment would regress optimistic freshness by design.

No popup, click action, second row, or new layout surface is in scope.
Protocol v17 -> v18 is reserved for one additive segment-facts variant.
When this stage lands, Arc 4 is complete.

### Q#SL2 - Lua surface: composable provider registry

The new module is `pmacs.statusline`:

```lua
local handle = pmacs.statusline.register {
  name = "my-project",
  side = "left",               -- required: "left" or "right"
  priority = 20,               -- optional signed 32-bit integer; default 0
  face = "ui.modeline.project",-- optional; default "ui.modeline"
  fn = function(ctx)
    if not ctx.buffer then return nil end
    return "project"
  end,
}

pmacs.statusline.set_priority(handle, 50) -- true iff handle is live
pmacs.statusline.set_enabled(handle, false)
pmacs.statusline.unregister(handle)
local providers = pmacs.statusline.providers()
```

Contract:

- A new `SharedStatuslineRegistry` is installed from `EditorState::new`
  before `builtin/runtime/lsp.lua`, stored on `EditorState`, and passed
  by reference to both grid and semantic renderers. User config still
  runs after all builtins, so it can discover and tune the built-in LSP
  provider. Bare test states construct an empty registry rather than an
  optional/absent surface.
- `register` returns a stable `StatuslineProviderId` userdata. Names are
  non-empty display/debug labels, not unique keys; handles own
  lifecycle, matching completion providers and package unload
  discipline. Registrations start enabled; ids are monotonic and are
  never reused, so registration-id tie breaks remain stable. The
  binding captures `caller_source(lua, 2)` at registration for later
  error attribution.
- The registration table is strict plain data. Raw keys are exactly
  `name`, `side`, `priority`, `face`, and `fn`; an unknown key is
  rejected with its name. Raw reads/traversal do not invoke
  `__index`/`__pairs`. `name`, `side`, `face`, integer range, and
  function type are completely validated before mutating the registry.
  Priority accepts a finite, mathematically integral Lua number in the
  signed-32-bit range on both LuaJIT and Lua 5.4; strings/fractional
  values do not coerce.
- The namespace tests move to dependency-neutral protocol helpers:
  `pmacs_protocol::is_ui_face_name` retains the exact stage-1
  `name == "ui" || name.starts_with("ui.")` reservation, while
  `is_modeline_face_name` accepts only `ui.modeline` or
  `ui.modeline.*`. The core's `highlight::is_face_name` delegates to
  the former; statusline registration, ThemeFacts expansion, and GPU
  wire validation delegate to the latter. A modeline segment cannot
  borrow another surface family's special mask/Default policy.
  Statusline registration additionally requires valid UTF-8, rejects
  control characters, and bounds `name` and `face` to
  `MAX_STATUSLINE_PROVIDER_NAME_BYTES` / `MAX_STATUSLINE_FACE_BYTES`
  (256 each).
- `face` is static for the registration. Dynamic face changes use two
  providers or unregister/register; this keeps the authoritative face
  inventory knowable without executing user code.
- The callback returns a valid UTF-8 string or `nil`. `nil` and the
  empty string omit the segment and contribute no separator. Invalid
  UTF-8 or any other return type is an isolated provider error.
- At most `MAX_STATUSLINE_PROVIDERS` (64) registrations may be live.
  Disabled registrations still count; unregistering releases the slot.
  This makes the producer's wire-size bound structural rather than a
  lossy "drop some providers after evaluation" policy.
- Returned text is flattened with the existing one-line policy: stop at
  the first `\n`, replace other control characters with spaces. A
  post-sanitization value above `MAX_STATUSLINE_SEGMENT_BYTES` (1024)
  is a provider error rather than an unbounded wire/shaping input.
- `providers()` returns fresh plain metadata tables in registration
  order: handle, name, side, priority, face, enabled. It never exposes
  the stored function.
- `set_priority` and `set_enabled` return `false` for a stale handle;
  an actual change advances registry state. Mutator arguments are also
  strict raw types (`set_enabled` accepts only a boolean, never Lua
  truthiness). `unregister` is idempotent and returns whether a live
  provider was removed.
- The module/registry installs before `builtin/runtime/lsp.lua` and
  before user config. Registration and all mutators are live
  mid-session, not init-gated.

The registry carries two monotonic counters:

- `layout_epoch`: register/unregister, actual priority changes, and
  enable changes. It guards evaluation snapshots and orders.
- `face_set_epoch`: register/unregister and enable changes that alter
  the enabled referenced-face set. It keys `ThemeFacts` expansion
  (Q#SL6). Priority-only changes do not make every semantic session
  re-resolve theme faces.

Both advance from their prior values and never reset.

### Q#SL3 - Callback context and evaluation lifecycle

Each enabled provider is called once per rendered window context:

```lua
ctx = {
  frontend = 7,       -- integer FrontendId
  window = 42,        -- integer WindowId
  buffer = buffer_id, -- normal pmacs buffer-handle userdata
  active = true,      -- focused window within that frontend
}
```

There is deliberately no terminal width, pixel width, cursor, scroll,
or frontend-kind field. Layout stays frontend-local; providers produce
semantic text, not presentation guesses. A provider that supports
passive split windows must read `ctx.buffer`, not
`pmacs.window.buffer()` (which names the focused window).

Evaluation is a three-phase, borrow-released transaction:

1. Borrow the core only long enough to capture the target frontend's
   visible `(window, buffer, active)` contexts. For the semantic path,
   capture only `active_window_for(frontend_id)` and require its buffer
   to match the declared viewport; during a snapshot -> new-viewport
   transition, emit nothing for the stale viewport.
2. Snapshot enabled provider definitions plus `layout_epoch`, release
   every core/registry borrow, then invoke Lua in the deterministic
   order from Q#SL4. Every call gets a fresh context table.
3. Re-read `layout_epoch` and the core contexts. Publish the owned
   results only if the registry epoch is unchanged and every
   `(frontend, window)` still exists on the same buffer with the same
   active flag. A callback that changes layout, switches/kills a buffer,
   or registers/unregisters/disables a provider makes this evaluation
   **invalid**. Invalid is not a silent dropped fan-out: for the
   declared matching v18 buffer, the producer emits an authoritative
   replacement `StatuslineSegments { left: [], right: [] }`, records
   that empty payload as the new emission baseline only after queuing
   the replacement, and discards every evaluated result. The next frame
   therefore stays silent if the surviving truth is also empty, or
   emits the newly evaluated non-empty truth as a change from empty. If
   a callback changed the initially matching window away from the
   declared buffer, the empty replacement clears that prior buffer's
   mirror before the next frame evaluates the new truth. A snapshot ->
   new-viewport transition that was already stale at phase 1 instead
   follows that phase's no-message rule: `BufferSnapshot` has already
   cleared the frontend mirror, and `on_buffer_snapshot_sent` owns the
   corresponding baseline removal. Thus no callback mutation can leave
   a prior non-empty GPU payload resident indefinitely, and no invalid
   evaluation creates a redundant second empty send.

The TUI calls the evaluator at the start of `paint_frame`, before the
long-lived mutable core borrow. `SemanticRenderState::render_frame`
calls it before producing `StatuslineSegments`, but only for a peer
that negotiated v18. A v17 semantic peer pays no Lua callback cost for
an unsupported surface. The daemon already stamps `active_frontend`
before both paths, so `pmacs.frontend.id()` agrees with `ctx.frontend`.

Provider failures are independent:

- One error or invalid return omits only that provider. Later providers
  still run and all built-in facts still render.
- The first failure in a consecutive failure run is appended to
  `*errors*` with provider name and registration source. Repeating the
  same failing callback every frame does not flood the buffer. Latches
  are keyed by the full `(provider_id, frontend_id, window_id,
  buffer_id, active)` context: success in one split must not re-arm a
  provider that keeps failing in another, and switching a window to a
  different buffer or focus role starts a truthful new failure run.
  A successful string-or-`nil` result clears only that context's latch,
  so a later failure there is reportable again. Unregister and stale
  context cleanup discard the corresponding latches; disabling a
  provider clears all of its latches so re-enable begins a new run.
  Frontend detach also discards every latch keyed by that `FrontendId`
  (with a live-context sweep as defense in depth), so a detached session
  cannot retain failure suppression into a later reconnect.
- Evaluation snapshots definitions before calls; a provider may
  unregister itself without a `RefCell` panic. The epoch guard drops the
  old fan-out's result and takes the authoritative-empty invalidation
  path above.
- Providers are documented as pure, fast render functions. The binding
  cannot prevent a callback from invoking editor mutators, but the
  context/epoch guard prevents wrong-window publication; recurring
  mutation loops are user-code bugs, not an implicit scheduling API.

No content epoch is assumed. LSP/process/async state can change without
touching the registry, so enabled callbacks are polled each render.
Owned output is payload-compared before wire emission; an empty registry
or no enabled providers is an O(1) fast path.

### Q#SL4 - Composition, order, separators, and narrow-window policy

Current built-ins remain protected:

- **Left:** the frontend's current active/modified/buffer-identity group,
  with its existing edge padding, then custom left segments.
- **Right:** custom right segments, then the frontend's current
  diagnostic/cursor/scroll group with its existing internal and edge
  spacing.

The compositor inserts exactly one ASCII space between adjacent custom
segments and at a custom/built-in boundary. Provider text does not need
to carry padding. No separator is emitted for `nil`/empty results.
Every compositor-inserted separator is a base `ui.modeline` run: it
never inherits an adjacent custom segment face. Legacy built-in internal
spacing retains its current base modeline styling too. This rule is
identical in TUI cells and GPU rich text, so a face colors only the
provider's visible text, not the gaps around it.
Each legacy built-in group stays atomic and byte-for-byte unchanged
inside: in particular, stage 3 does not normalize the GPU's existing
two-space diagnostic/readout separators to the TUI's one-space
formatting.

Priority means **survival priority when horizontal space is tight**:

- Left custom providers are ordered by `(priority descending,
  registration id ascending)`. Higher-priority items sit closest to the
  protected buffer identity. Overflow clips the low-priority tail.
- Right custom providers are displayed by `(priority ascending,
  registration id ascending)`, placing higher-priority items closest to
  the protected diagnostic/cursor/scroll suffix. The complete right run
  is right-aligned; overflow clips its low-priority left edge.
- The protected built-in suffix is never discarded merely because a
  custom provider is long. If the built-in suffix itself cannot fit,
  each frontend retains today's behavior: the TUI drops the right group
  wholesale, while the GPU keeps its right edge fixed and clips its left
  edge. Custom-prefix clipping preserves the complete built-in suffix
  only when that suffix fits by itself.
- The left group gets the space before the right group's measured
  origin and clips at the collision boundary. It never overwrites the
  right group.

This asymmetric visual ordering is intentional: priority determines
what survives, not a generic ascending sort that would protect opposite
ends on the two sides. Registration id makes ties deterministic across
TUI painting, payload comparison, and wire encoding.

### Q#SL5 - Echo/minibuffer precedence on the single GPU band

The TUI always keeps modelines visible while its separate global row
shows a message, search, or minibuffer. The GPU has one physical band,
so exact topology parity is impossible without adding a second GPU
surface (Deferred). Stage 3 follows the existing content priority:

- Ordinary buffer-name state: buffer identity followed by custom left
  segments.
- Minibuffer, isearch, or transient message state: that content owns
  the whole left group; custom left segments are suppressed.
- Custom right segments remain visible with the existing
  diagnostic/cursor/scroll right group, just as that group remains
  visible during minibuffer/search/message state today.

This makes custom segments modeline content, never echo content.
`ui.statusline` continues to color transient messages only.

### Q#SL6 - Segment faces and dynamic ThemeFacts inventory

Every segment carries a face **name**, never raw color. The registered
default is `ui.modeline`; a typical package uses a child such as
`ui.modeline.lsp`.

Segment faces have a stage-3 component mask of **visual `{fg}` only**
on both frontends:

- The modeline/status-band background remains wholly owned by
  `ui.modeline`; a text segment cannot create a per-run background on
  one frontend only.
- The default face name `ui.modeline` and an unresolved custom face keep
  the base modeline's EFFECTIVE text color after its own reverse
  mapping.
- A resolved custom face applies only its logical `fg` as the
  POST-modeline visible glyph color when that component is concrete.
  `Default` means "use the effective base modeline text color": an
  exact all-default child still blocks a colored intermediate parent,
  but returns the run to the base rather than trying to express a
  terminal-default foreground through a reversed background channel.
  The visible background remains the base modeline surface.
  Out-of-mask bg/bold/italic/underline/reverse fields are ignored by
  both frontends.
- The TUI's built-in modeline is normally `reverse = true`. To apply a
  visible glyph color without changing that surface, the cell painter
  writes the override into the base style's logical `bg` when reverse
  is set, and into logical `fg` otherwise. After the terminal performs
  reverse, the requested color is the glyph foreground in both cases.
  The GPU writes the same requested color into the glyphon run.
- A `ui.modeline.*` custom child uses **base-relative inheritance**:
  walk exact child/intermediate entries but stop before
  `ui.modeline`; reaching the base means "no override", so the segment
  inherits the modeline's already-mapped effective text color. This
  avoids taking `ui.modeline`'s pre-reverse logical `fg` and applying it
  as a post-reverse glyph color. One shared
  `Theme::modeline_segment_face` helper owns this rule for TUI
  resolution and ThemeFacts production. A concrete custom foreground
  returns a mask-normalized `Style { fg, ..Default::default() }`; a
  found Default foreground stops inheritance and returns `None` (base).
  Out-of-mask components never enter the dynamic wire table.
- GPU performs exact lookup in `ThemeFacts`; absence means base
  modeline text. Existing `Indexed` palette divergence remains the
  stage-1 accepted behavior.

For semantic peers at v18, the `ThemeFacts` inventory becomes:

```text
fixed stage-1 UI_FACES
UNION
distinct face names of enabled statusline providers
```

The union is sorted/deduplicated. Custom names resolve through
`Theme::modeline_segment_face`: exact/intermediate concrete foreground
overrides are shipped, while a name that reaches the base or finds a
Default foreground is omitted and therefore uses the frontend's
effective modeline text. Thus an unset `ui.modeline.lsp` correctly
follows a configured, possibly reversed `ui.modeline` without shipping
a pre-reverse component under a post-reverse mask. Frontend lookup
remains exact; the Q#TH7 ownership boundary does not move.

`theme_facts_msg` keys its computation on
`(theme.face_epoch, statusline.face_set_epoch)` for a v18 peer. Both
cache records advance on computation; payload equality can suppress a
send. Removing/disabling the last provider for a custom face removes
that entry from the next authoritative table. For v16/v17 peers the
inventory stays the fixed stage-1 list: they cannot render segments and
pay no irrelevant face traffic.

If a face-table change and segment payload occur in one frame,
`ThemeFacts` is ordered before `StatuslineSegments`. A theme-only
recolor sends `ThemeFacts` but not unchanged segment text; the GPU face
arm invalidates both status shaping caches, so existing runs reshape
under the new color. The invalid-evaluation authoritative-empty path
uses this same ordering: a provider removal may remove its dynamic face
from `ThemeFacts`, but its prior non-empty segment payload is replaced
by empty vectors in that frame rather than being retained beside the
reduced face inventory.

### Q#SL7 - Wire: `StatuslineSegments`, protocol v18, appended final

```rust
/// One daemon-produced custom modeline segment. Text has already been
/// sanitized to one line; `face` is ui.modeline or a child name. A
/// custom override, when set, is resolved in the authoritative
/// ThemeFacts table; absence means the base modeline text color.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatuslineSegment {
    pub text: String,
    pub face: String,
}

/// Arc 4 stage 3 (protocol v18): custom Lua modeline output for the
/// semantic frontend's current buffer. Complete replacement each
/// send; empty vectors authoritatively mean no custom segments.
StatuslineSegments {
    buffer_id: BufferId,
    left: Vec<StatuslineSegment>,
    right: Vec<StatuslineSegment>,
},
```

- Append after `FontFacts`, the final v17 variant. Before appending,
  add a byte-level encoding pin of representative `FontFacts` values;
  the new variant's own round-trip cannot detect an accidental ordinal
  shift of old channels.
- `PROTOCOL_VERSION` becomes 18; supported versions become `6..=18`;
  the ladder accepts 18 and rejects 19. Add populated and empty
  postcard round-trips.
- Daemon write-loop and producer both gate at negotiated `>=18`.
  A v17 GPU keeps today's built-in band. The grid TUI silently drops
  the semantic-only variant if one is delivered unexpectedly.
- The payload contains custom provider output only. Existing
  `StatusFacts` remains unchanged at v15; widening it would move its
  whole gate to v18 and unnecessarily darken buffer/diagnostic facts
  for v15-v17 peers.
- `docs/semantic-frontend-protocol.md` records the v18 schema,
  authoritative-empty rule, ordering after `ThemeFacts`, snapshot
  reset, and the division between custom daemon text and
  frontend-derived cursor/scroll.

Wire values are untrusted at the GPU boundary. Before replacing current
state, the GPU validates the whole message atomically. The provider,
segment-text, face, and total-text limits live as public constants in
`pmacs-protocol`; registration/production and consumption do not copy
numeric policy:

- no more than 64 segments total across both sides;
- total text bytes no more than 64 KiB;
- each text is non-empty, at most 1024 bytes, and contains no control
  character;
- each face is at most 256 bytes, contains no control character, and
  satisfies `pmacs_protocol::is_modeline_face_name`.

An invalid message is logged and ignored wholesale; the prior valid
state remains. These bounds protect shaping/layout even if a malformed
peer bypasses the trusted Lua producer.

### Q#SL8 - Producer, emission baselines, and snapshot symmetry

`SemanticRenderState` gains:

- `peer_knows_statusline_segments: bool`;
- `last_statusline: HashMap<BufferId, (Vec<StatuslineSegment>,
  Vec<StatuslineSegment>)>`.

After viewport declaration and only when the declared buffer matches
the frontend's active daemon window, the producer evaluates the active
context and compares the complete ordered payload:

- First sight of every buffer emits an authoritative message, including
  `(left=[], right=[])`.
- Changed output emits one complete replacement.
- Byte-identical output is silent even though callbacks were evaluated.
- Back-to-back state changes before one frame legitimately coalesce into
  the latest payload.
- `on_buffer_snapshot_sent(buffer_id)` removes that buffer's baseline.
  An unchanged A -> B -> A revisit must re-send A's segment payload.

The GPU `BufferSnapshot` arm clears its custom left/right segment
mirror alongside `status_facts`, search, and menu. `ThemeFacts` and the
provider registry remain global and survive. This is the #120
snapshot/baseline contract applied symmetrically, not a new special
case.

### Q#SL9 - TUI rendering: styled runs and display-column correctness

`paint_mode_line` stops flattening each side to an unstyled `String`.
It receives logical runs `(text, effective Style)` and uses one shared
single-row painter:

- Before grapheme segmentation, **every** logical run passes through a
  shared terminal-control sanitizer: provider text, buffer names, mode
  markers, diagnostics/readouts, and compositor separators alike have
  all control scalars (including CR, LF, and ESC) replaced with spaces.
  Provider-return sanitation remains an earlier validation boundary;
  this final run-level pass is defense in depth for core-owned text.
  Consequently `Glyph::Cluster`, whose frontend emitter writes bytes
  verbatim, can never carry a terminal control sequence.
- Runs are split with `UnicodeSegmentation::graphemes`; width and
  clipping use `UnicodeWidthStr` on each complete grapheme. This stage
  adds `unicode-segmentation` as a direct dependency rather than
  relying on cosmic-text's transitive copy.
- A one-scalar grapheme writes `Glyph::Char`; a multi-scalar printable
  grapheme writes `Glyph::Cluster`. Every extra display column writes a
  `Glyph::Continuation`; clipping never emits half a wide grapheme.
  A standalone zero-column grapheme is skipped, while a combining
  sequence such as `e` + U+0301 remains one visible cluster.
- Left/right collision uses display columns, not scalar count or UTF-8
  bytes.
- The row is still filled once with the `ui.modeline` base style.
  Built-in runs keep that style. A set segment face replaces only the
  run's visible foreground per Q#SL6, writing logical `bg` rather than
  `fg` when the base row is reversed.
- Every separator inserted by Q#SL4 is likewise painted with this base
  style, regardless of the faces on either side.
- When the protected built-in right suffix fits by itself, clipping the
  combined right group removes only the low-priority custom prefix and
  preserves that suffix in full. If the built-in suffix itself does not
  fit, the TUI retains today's wholesale drop instead of introducing a
  new partial-suffix policy. Left clipping keeps the prefix. No run can
  write outside its window rect or into another split's modeline.

With no visible provider output, the resulting cells are byte-for-byte
the current modeline for ordinary ASCII buffers.

### Q#SL10 - GPU application: rich runs, cache invalidation, clipping

GPU state stores the latest validated custom segment vectors plus their
`buffer_id`. Composition filters them against `current_buffer_id`, the
same belt as `StatusFacts`.

- Right custom segments are inserted before diagnostic/cursor/scroll
  spans. Each segment becomes a rich-text run. The color resolver
  special-cases `ui.modeline` and an absent child to the already-mapped
  base modeline text color; a present child maps only its concrete
  `fg`, with defensive Default handling also selecting the base. It
  never re-applies the base face's pre-reverse logical foreground.
- Ordinary left composition becomes rich text: buffer-name/modified
  base run followed by custom left runs; each Q#SL4 separator is its
  own base-color rich run. Modal/message states produce their existing
  single content run and no custom left runs. Right-side custom/built-in
  separators are likewise base-color runs, never extensions of an
  adjacent provider face.
- The two shaping caches become
  `Option<Vec<(text, explicit_color)>>`, seeded/invalidation-set to
  `None`, and retain the complete ordered rich-run vectors after shape.
  Concatenation is not a sufficient key once `"buffer" + custom` can
  equal a transient/minibuffer string byte-for-byte while requiring
  different attributes; an empty vector is legitimate content, not an
  invalidation sentinel. Cache state advances only after the matching
  rich text has been installed.
- Applying a changed `StatuslineSegments` payload clears both status
  shaping caches before redraw. This is required even when concatenated
  text is unchanged but a face name changed.
- Both status glyphon buffers use `Wrap::None`, set at construction and
  retained across the FontFacts metric transaction. They remain
  single-line surfaces even when a custom segment is wider than the
  viewport.
- Right placement uses the full shaped width without clamping its
  origin to `TEXT_LEFT`: the run's right edge stays at the right pad,
  while a negative/left-of-surface origin clips low-priority custom
  prefixes and preserves the built-in tail. The left TextArea clips at
  the right group's actual origin. Existing geometry bounds still keep
  all glyphs inside the band.
- `ThemeFacts` continues to invalidate both caches. FontFacts already
  re-metrics/re-shapes both status buffers; the new rich runs ride that
  path without a new font transaction.

The message does not request a viewport re-declaration: status text
changes no code geometry or visible-line count.

### Q#SL11 - Built-in LSP segment proves the extension point

After `pmacs.statusline` is installed, `builtin/runtime/lsp.lua`
registers one right provider:

```lua
pmacs.statusline.register {
  name = "lsp",
  side = "right",
  priority = 0,
  face = "ui.modeline.lsp",
  fn = function(ctx)
    local rec = attachments[tostring(ctx.buffer)]
    if not rec then return nil end
    return "LSP:" .. pmacs.lsp.modeline_label(rec.server)
  end,
}
```

It is pure: it never triggers attachment, flushes didChange, or mutates
the server. It indexes the private attachment map by `ctx.buffer`, so
passive split windows show their own buffer's state. No attachment means
`nil`, preserving today's modeline outside LSP-backed buffers.

The face name is intentionally a new child. Unset, it inherits
`ui.modeline`/the built-in segment color. A user can theme LSP state
without changing the whole band:

```lua
pmacs.theme.merge {
  ["ui.modeline.lsp"] = { fg = 6 },
}
```

The provider handle appears in `pmacs.statusline.providers()`, so user
config can disable or reprioritize it without a special LSP option.

## Bets

- Additive providers are sufficient for the first extensibility stage:
  they deliver real package/user value without turning optimistic
  cursor/scroll facts into stale daemon text or destabilizing the
  existing default layout.
- Static registration faces plus the dynamic ThemeFacts inventory keep
  inheritance daemon-owned and make face availability independent of
  callback output. No frontend walk or raw color enters the API.
- Per-render Lua polling is the honest freshness mechanism. Generic
  callbacks can depend on LSP/process/plugin state with no shared epoch;
  payload comparison keeps the wire quiet, and an empty registry takes
  the O(1) fast path. The existing `status_summary` API was already
  shaped for one call per render frame.
- Three-phase evaluation prevents the known core/registry `RefCell`
  hazards and fails closed across context-changing callbacks. It does
  not pretend arbitrary mutating render code is a supported scheduling
  model.
- One authoritative v18 message per buffer plus snapshot-symmetric
  reset makes first attach, late join, and unchanged A -> B -> A
  revisits correct without an epoch on the wire.
- The priority-at-the-protected-edge rule is deterministic and keeps
  today's essential built-ins readable under narrow layouts.
- The first built-in LSP provider validates passive-window context,
  live async updates, arbitrary child faces, and cross-frontend wire
  rendering in one useful feature.

## Deferred (named)

Wholesale replacement/removal/reordering of built-in buffer,
diagnostic, cursor, and scroll components; a frontend-local custom
cursor/scroll token vocabulary; customization of the global echo row;
a second GPU bottom surface that would keep modeline left segments
visible during minibuffer/search/messages exactly like the TUI;
segment click/hover actions and mouse hit maps; multi-row statuslines;
icons/images/resources; per-segment backgrounds, reverse,
bold/italic/underline, and wider chrome masks; `ui.modeline.inactive`;
borrowing face families outside `ui.modeline`; dynamic face names
returned by callbacks; async/yielding providers;
provider-specific separators; timed refresh scheduling below/above the
normal frame cadence; automatic package ownership/unregister (packages
retain handles and use unload hooks today); GPU splits/multi-buffer
status bands (Arc 8 structural work); horizontal scrolling/marquee and
ellipsis policies; repurposing or deleting the legacy
`ModeLine(Vec<Cell>)` variant.

## Acceptance

Primary suite: `tests/statusline_segments_acceptance.rs` for Lua,
TUI, producer, and daemon/wire behavior; protocol pins stay in
`src/protocol.rs`; GPU routes live in the headless
`PMACS_REQUIRE_GPU=1` suite. Dispatch/render tests use real
`RenderState`/semantic frame paths, not direct helper-only formatting.

1. **Default preservation:** with no visible provider output, scratch
   TUI cells and GPU pixels are byte-identical to the pre-stage
   modeline/status band. The global TUI echo row is unchanged.
2. **Lua strict contract:** valid registration returns a handle and
   appears in `providers`; bad/unknown side, empty name, non-integer or
   out-of-range priority, non-function `fn`, non-modeline face
   (including another valid `ui.*` family), control or over-limit
   name/face, provider 65, and unknown key all error with the field/key
   named and leave registry epochs and provider list untouched. A
   value-providing or raising metatable is never invoked. Protocol,
   core, producer, and GPU tests pin the same namespace predicate table
   through the shared helpers.
3. **Handle lifecycle:** priority and enable changes affect order/output
   and advance only their specified epochs; no-op setters do not;
   unregister is true then false; stale-handle setters return false;
   fractional/coerced priority and truthy non-boolean enable values
   error without mutation.
4. **Callback result contract:** string renders; `nil` and empty string
   omit without separators; newline/control output is sanitized;
   invalid UTF-8, non-string, and over-limit output omit that provider
   and report an error.
5. **Error isolation and latch:** a failing provider between two good
   providers does not suppress either neighbor or built-ins; one error
   lands in `*errors*`, repeated frames do not append duplicates, a
   successful evaluation clears the latch, and a later failure reports
   once again. In two splits, success in B does not re-arm a provider
   that remains failing in A; closing A or unregistering the provider
   releases that context's latch, and disable/re-enable starts a new
   failure run. Detaching a frontend releases every latch carrying its
   `FrontendId`; reconnecting and failing again reports once rather than
   inheriting suppression from the detached session.
6. **Re-entrant registry mutation:** a provider unregistering itself
   during evaluation causes no borrow panic and discards the old
   fan-out by epoch guard. A semantic producer test first establishes a
   non-empty payload (and its custom face) for the matching buffer, then
   triggers self-unregister/disable: the invalid evaluation emits one
   authoritative empty replacement, the reduced `ThemeFacts` precedes
   that replacement, the resulting GPU frame has no prior text, and the
   empty replacement becomes the emission baseline. The provider is
   absent and a still-empty next frame is wire-silent; a surviving good
   provider instead reappears on that next frame as a change from empty.
7. **Context-change guard:** callbacks that switch the window buffer,
   close a split, or kill the source buffer cannot publish text under
   the old context; the next frame evaluates the surviving truth.
8. **Per-window context:** two TUI splits on different buffers receive
   distinct `ctx.window`, `ctx.buffer`, and `ctx.active` values and
   render their own text. Focusing the other split flips only `active`;
   two frontends cannot consume each other's context/output.
9. **Ordering and separators:** mixed left/right providers with tied and
   distinct priorities produce the exact Q#SL4 order, stable id tie
   break, and one-space custom boundaries with nil providers absent;
   the built-in groups retain their legacy internal spacing. With two
   visibly different custom faces, every custom/custom and
   custom/built-in separator is pinned to the base `ui.modeline` style
   in TUI cells and GPU rich runs/pixels.
10. **TUI placement:** buffer identity remains first on the left;
    custom right segments precede diagnostics/L:C/scroll; the global
    echo row still shows `pmacs.editor.set_status` independently.
11. **TUI Unicode and clipping bite:** CJK, combining, and ASCII custom
    runs beside a right suffix occupy correct display columns with
    cluster/continuation cells and no overlap; the combining sequence is
    emitted rather than silently dropped. A narrow-split fixture whose
    built-in suffix fits by itself clips the low-priority custom edges
    while retaining that suffix in full and never writes outside its
    rect. A second fixture where the built-in suffix itself does not fit
    pins the current TUI wholesale-drop behavior. A buffer name
    containing CR, LF, and ESC is sanitized before segmentation: its
    resulting `Glyph::Char` / `Glyph::Cluster` cells and captured
    terminal bytes contain no raw control scalar or escape sequence.
12. **LSP built-in:** an unattached buffer adds nothing. Attached
    buffers show `LSP:init/ready/idx/degraded/crashed/stopped` as the
    tracker changes without a buffer edit, and `LSP:?` for a forgotten
    server id; a passive split uses its own attachment.
    Disabling/reprioritizing the discovered provider handle works.
13. **Version and placement pins:** protocol is 18; ladder accepts
    `6..=18` and rejects 19; empty/populated
    `StatuslineSegments` round-trip; a byte-level `FontFacts` encoding
    pin proves the append shifted no v17 discriminant.
14. **Authoritative first frame and live output:** a v18 session's first
    matching-viewport frame carries empty vectors when no provider is
    visible, then silence. A callback-state change with no edit/registry
    mutation emits exactly one updated payload; unchanged polling is
    wire-silent.
15. **Init and late join:** a provider/theme established from
    `init.lua` is present in the first attachment's first matching
    frame. The same established state is present in a later second
    session without a post-attach mutation.
16. **Version gate:** a real-daemon v17 semantic peer receives neither
    `StatuslineSegments` nor dynamic provider-only ThemeFacts entries
    and does not execute the provider; a v18 peer receives both. Daemon
    producer and write-loop gates are independently pinned.
17. **TUI drop arm:** the grid frontend consumes an unexpected
    `StatuslineSegments` message without error.
18. **Snapshot round trip:** after A's segment payload is established,
    A -> B -> A at unchanged generations re-sends A because the producer
    baseline reset; the GPU snapshot clears A's mirror immediately and
    restores the exact A pixels only after the authoritative re-send.
19. **Dynamic face inventory:** registering enabled
    `ui.modeline.lsp` adds its daemon-resolved exact name to v18
    `ThemeFacts` only when a custom override exists; a configured
    `ui.modeline` parent is inherited through base absence (no redundant
    child entry), while an intermediate custom parent is shipped under
    the exact referenced child name with only `fg` retained;
    disabling/removing the last reference removes any custom entry. A
    priority-only change does not recompute the face set.
20. **Message ordering and recolor:** when registration and theme
    change together, `ThemeFacts` precedes `StatuslineSegments`.
    Recoloring a segment face with constant text emits ThemeFacts only
    and changes both TUI cells and GPU pixels through cache
    invalidation.
21. **Face mask parity:** a segment face carrying
    `{fg=F,bg=B,reverse=true,bold=true}` renders exactly like `{fg=F}`
    on both frontends, including under the TUI's default reverse row;
    an exact empty child blocks a colored intermediate parent and
    returns to the effective base text while retaining the base
    modeline surface.
22. **GPU normal composition:** ordinary state renders buffer identity
    plus differently faced custom left runs, and custom right runs
    before colored diagnostics and optimistic cursor/scroll. A changed
    face name with identical concatenated text still reshapes.
23. **GPU precedence:** minibuffer, isearch, and transient status each
    suppress custom left segments while preserving custom right and the
    existing right facts; closing the modal/message restores the custom
    left payload without requiring a new segment message. A fixture
    makes the ordinary rich composition and transient message
    concatenate to identical bytes and proves both transitions reshape
    with the correct attributes.
24. **GPU narrow-band clipping:** an over-wide right provider is clipped
    at the left edge while diagnostic/L:C/scroll pixels remain at the
    right; left content stops before the right origin. Bounds contain
    all glyphs at both stage-2 font-size limits. A wrapping-sensitive
    fixture proves both status buffers remain one visual row.
25. **GPU wire validation:** direct messages with too many segments,
    excess bytes, control text, overlong/invalid face names, or a
    face outside `ui.modeline` are rejected atomically with the prior
    valid frame byte-identical and no panic; boundary-valid payloads
    apply. The predicate cases are the same table exercised by Lua/core
    tests, not a copied GPU interpretation.
26. **Unsupported-peer cost:** a semantic v17 render with an enabled
    side-effect-counting callback never invokes it. Grid TUI and v18
    semantic renders invoke exactly once per target window per frame.
27. **Docs/handoff:** semantic protocol documents v18 and ownership;
    `docs/package-author-guide.md` shows register/unregister lifecycle
    and passive `ctx.buffer` use; the roadmap/handoff record Arc 4
    complete once the implementation lands.
