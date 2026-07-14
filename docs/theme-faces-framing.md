# Theme faces — framing (Arc 4 stage 1, themes)

**Revision 4 — 2026-07-14. Status: awaiting approval; folds framing
rounds 1–3.**

Revision 4 (framing round 3, findings 1–7):
`ui.minibuffer.candidate` now has a real GPU site: its `fg` colors the
existing minibuffer-dropdown candidate glyph layer, while popup
background/selection theming stays deferred; the mask is therefore a
true intersection of both render paths (finding 1). The v15
discriminant guard pins the byte encoding of `CompletionPopup`, the
actual final pre-v16 variant, rather than the earlier `LineNumbers`
variant (finding 2). The deterministic transactional bite now targets
the production ordered-result helper: it consumes an iterator of
`Result<(String, Style)>`, collects every result before taking the
theme lock, and commits only the completed vector, so an `Ok` followed
by an `Err` is both representable and guaranteed to leave the theme
untouched (finding 3). An empty diagnostic child is named as an
explicit reset to the built-in severity color, not as "unset" -- it
blocks parent inheritance -- and acceptance covers a themed
`ui.diag` parent plus an empty `ui.diag.error` child (finding 4). GPU
acceptance now covers every distinct stage-1 route: modeline reverse,
transient status text, minibuffer and isearch text, candidate-dropdown
glyphs, normal/active search washes, and the existing gutter,
selection, and diagnostic paths (finding 5). The prose and unset
fixture use the exact `ui`-or-`ui.*` predicate (finding 6). Finally,
the peer-selection acceptance drives the actual decoration collector
with simultaneous local and peer selections and inspects their
emitted colors; it is not a helper-only assertion (finding 7).

Revision 3 (framing round 2, findings 1–9): the cross-frontend
contract is structural now — every face carries a stage-1
**component mask**, identical on both frontends and sized to what
both can render today (the GPU gutter has no background layer; wash
quads cannot recolor or reverse glyphs); out-of-mask components are
ignored *everywhere*, so `ui.selection = { fg = 1 }` and
`ui.gutter = { bg = 1 }` are component no-ops on the TUI too, and
mask widening is a named, additive deferral (finding 1). Diag faces
get a representable `Default` policy: `Default` fg means the
built-in severity color, not "plain" — the minimap summary encodes
diagnostic *presence* by copying that color into `underline_color`,
where `Default` reads as "no mark", so an all-default diag face
uses the built-in severity color rather than an unrepresentable plain mark
(finding 2). The face predicate is explicit —
`name == "ui" || name.starts_with("ui.")` — shared by the
reservation, the counter-bump classification, and a merge-bare-`ui`
acceptance case (finding 3). The consecutive-`set` bite interleaves
render/observe between the mutations; two `set`s inside one dispatch
legitimately coalesce into one emission (finding 4). Producer and
gate caches advance on computation, not on emission:
`last_face_epoch` is `Option<u64>` (the gate cannot short-circuit
while the payload baseline is absent, guaranteeing the first
authoritative send against an epoch-0 daemon), an identical rebuild
records the epoch it inspected, and the summary inserts its new key
even when payload equality withholds the message; Rust units pin
single-recompute, which traffic assertions cannot see (finding 5).
`SearchView::new` takes `Option<ThemeHandle>` — a bare core
constructs an unthemed view, no contradiction (finding 6).
`ThemeFacts` is appended after `CompletionPopup`, and a byte-level
encoding pin of the final existing variant guards the placement — postcard
discriminants are ordinal, and the new variant's own round-trip
cannot detect a shift that would corrupt v15 peers (finding 7). The
malformed-merge Lua acceptance pins only the order-independent user
contract; the deterministic bite is a Rust unit on an ordered result
stream, since Lua table iteration order is unspecified
(finding 8). `ui.selection` themes the local selection only: the
GPU splits its resolution path so peer selection rects keep the
constant, and peer theming stays deferred with the peer-cursor
palette (finding 9).

Revision 2 (framing round 1, findings 1–8): the theme mutation
contract is transactional and monotonic — all four mutators parse
and validate their full input before taking the lock, commit
all-or-nothing, and bump from the prior counter value after every
successful commit; `set` replaces the `by_capture` field, never the
`Theme` value, so counters survive wholesale replacement (finding 1).
The counter is split into `syntax_epoch` (keys the `StyleGate`) and
`face_epoch` (keys the `ThemeFacts` producer), so a face-only edit
never re-runs the tree-sitter span query; the minimap summary keys on
both — diag faces feed its marks — and gains payload-equality
suppression, so a non-diag face edit recomputes once per mutation and
emits nothing (finding 8). Stage 2 takes protocol v17: once a v16
binary ships, a variant added under v16 would let two incompatible
v16 schemas negotiate successfully (finding 2). Face application is
unified as "a set face owns its surface" — both frontends replace
the site default wholesale, with `Default` components meaning that
frontend's *plain* rendering (GPU mapping table in Q#TH5); the
finding-5 divergences (`{ fg }` keeping the GPU band bg, empty
`ui.selection` diverging) are gone, and the residual divergences
(`Indexed` palette meaning; GPU chrome honoring fg/bg/reverse only in
stage 1) are named and tested. `ThemeFacts` is authoritative per
attachment: the baseline seeds to `None`, every attachment receives
exactly one table — empty included — after viewport declaration, so
a frontend retaining face state across attachments is corrected even
by an unthemed daemon (finding 4; the current GPU has no reconnect
path, so this is contract, not accident). GPU ThemeFacts application
invalidates the status-band shaping cache — `E:`/`W:` colors are
baked into shaped rich text and re-shaping is gated on string
equality, so a diag-face change with constant counts kept stale
colors (finding 3). The grid views get a real theme path:
`EditorCore` gains an injected `ThemeHandle` for
`ensure_search_overlay`, and `install_diag` threads the handle to
`DiagnosticView`; acceptance drives the dispatched-search and Lua
diagnostic-attachment paths, not bare constructors (finding 6). The
reserved namespace covers the bare root `ui` as well as `ui.*`,
matching the inheritance walk's terminal (finding 7). Acceptance
adds consecutive-`set` monotonicity, malformed-merge atomicity, GPU
diag-recolor-with-constant-counts, owns-surface cross-frontend
semantics, and absence assertions for face-only mutations.

Revision 1: initial framing.

Arc 4 ("Themes + extensibility surface", `docs/roadmap-2026-07.md:83`)
names three deliverables: named UI faces wired to both frontends, a
`pmacs.gpu.set_font` runtime font swap, and a Lua statusline-segment
API. This framing covers **stage 1 only — named UI faces plus the
`ThemeFacts` wire channel (protocol v15→16)**. Font swap and
statusline segments are stages 2 and 3, each with its own framing
(Q#TH1). One branch (`theme-faces`), one PR.

## Ground truth (as of `13dbadd`)

### The theme substrate today

- `Theme { by_capture: HashMap<String, Style>, default_style: Style }`
  — `src/highlight.rs:59-67`. One global instance behind
  `ThemeHandle = Arc<Mutex<Theme>>` (`highlight.rs:190`), owned by the
  `SyntaxRegistry` (`src/syntax.rs:496`, seeded `Theme::default_dark()`
  at `:518`, exposed via `.theme()` at `:527`; line refs re-verified
  after the #114/#115/#116 highlight-stack merges, whose bundled CUDA
  and shell grammars introduce no `ui`-named capture).
- `Theme::lookup` (`highlight.rs:161-172`) tries the full dotted name,
  strips one `.`-segment at a time, and **falls back to
  `default_style`** — correct for syntax captures, wrong for faces
  (an unset face must mean "keep today's hardcoded look", which is
  not expressible as a `Style` on the semantic path).
- Lua surface is complete and mounted: `pmacs.theme.{set, merge, get,
  clear, default, current}` — `src/lua_bindings/mod.rs:6898-6993`.
  `set` replaces `by_capture` wholesale but preserves `default_style`
  (`:6913-6915`). Style/color marshalling: `lua_to_style` `:6865`,
  `lua_to_color` `:6780` (nil/"default" → `Color::Default`, 0..=255 →
  `Indexed`, `{r,g,b}` → `Rgb`).
- Mutator shapes today: `set` parses the whole table into a scratch
  `Theme` *before* locking, then wholesale-replaces the locked value
  (`:6906-6915`) — already transactional, but the replacement would
  reset any naive counter carried on `Theme`. `merge` locks first and
  inserts while iterating (`:6925-6931`) — a malformed later entry
  errors out with earlier entries already applied.
- Wire vocabulary already suffices: `Color { Default, Rgb, Indexed }`
  and `Style { fg, bg, bold, italic, underline, reverse,
  underline_color }` — `pmacs-protocol/src/cell.rs:84-133`. No alpha
  channel (relevant for GPU washes, Q#TH8).
- Theme colors already cross the wire *indirectly*: `scoped_style_spans`
  (`src/semantic_render.rs:1501-1566`) clones the theme, resolves
  capture → `Style`, and ships `StyleSpans`. Nothing chrome-related
  crosses at all.

### Chrome colors today — every one a hardcoded literal

Grid path (daemon paints cells for the TUI; `paint_frame`,
`src/editor.rs:2086`, has `&EditorState` and therefore theme access):

| Site | Where | Literal |
|---|---|---|
| Per-window mode line | `editor.rs:2474` (`paint_mode_line`) | `reverse: true` (`:2499-2502`) |
| Status/echo row | `editor.rs:2260` (`paint_status_line`) | `reverse: true` (`:2275-2286`) |
| Line-number gutter | `editor.rs:2320` | `fg: Indexed(8)` (`:2332-2335`) |
| Selection | `editor.rs:2375` | `cell.style.reverse = true` (`:2431`) |
| Minibuffer prompt/input/fill | `editor.rs:2541` | `Style::default()` (`:2571`, `:2588`, `:2613-2617`) |
| Minibuffer candidate suffix | `editor.rs:2605-2608` | `reverse: true` |
| Search prompt row | `editor.rs:2629` | `Style::default()` (`:2659`, `:2677`) |
| Search matches | `src/search.rs:362-378` | bg `Indexed(3)`/fg `Indexed(0)`; active bg `Indexed(11)` |
| Diagnostic severity color | `src/diag.rs:90-97` | `Indexed(1)/(3)/(6)/(8)` — shared by squiggle `underline_color`, column-0 marker, minimap mark |

GPU path (frontend-local constants, `pmacs-gpu/src/main.rs`):

| Site | Where | Literal |
|---|---|---|
| Status band bg | `:130` | `STATUS_BAND_BG [0.105, 0.105, 0.145, 1.0]` |
| Band left text (name/prompt) | `:4511` | `rgb(200,200,210)` |
| Band right readout | `:4496` | `rgb(168,168,180)` |
| Band `E:`/`W:` counters | `:3401`, `:3407` | `rgb(241,76,76)`, `rgb(245,245,67)` |
| Gutter digits | `:4534` | `rgb(120,120,135)` |
| Selection wash | `:6661` | `[0.31, 0.42, 0.82, 0.30]` |
| Search washes | `:6673-6674` | `[0.85,0.78,0.20,0.30]`, `[0.95,0.55,0.12,0.48]` |
| Diag squiggles/signs | `:6606-6623` | Error `[0.945,0.298,0.298]`, Warning `[0.961,0.961,0.263]`, Info `[0.231,0.557,0.918]`, Hint `[0.4,0.4,0.4]` |
| Surface clear / caret / menu / minimap | `:61-66`, `:73-74`, `:145-147`, `:111-114` | out of stage-1 scope (Deferred) |

`Indexed` colors mean different things per frontend: the TUI defers
to the user's terminal palette; the GPU has its own hardcoded xterm
table (`indexed_to_glyphon`, `main.rs:6552-6587`). A face set with
`Indexed` colors will render per-frontend, same as syntax styles do
today — documented, not fixed here (Deferred).

### The facts-channel template (LineNumbers, Q#UX1)

- `PROTOCOL_VERSION = 15` (`pmacs-protocol/src/message.rs:1333`);
  `SUPPORTED_PROTOCOL_VERSIONS = [6..=15]` (`:1393`). Standard bump
  shape: additive variant + daemon write-loop gate on
  `negotiated_protocol_version` (`src/daemon.rs:1119-1121`,
  `:1153-1157`); old peers receive nothing on the channel.
- Producer pattern: cached-compare field on `SemanticRenderState`,
  emit-on-change from `render_frame`'s emission list
  (`src/semantic_render.rs:462-478`; `line_numbers_msg` `:810-825`).
  Nothing emits before the frontend declares a viewport (`:307-310`).
- Late join is free: per-session state is built fresh at attach
  (`daemon.rs:1377-1381`), and a baseline seeded to the *frontend's
  default belief* re-emits exactly the non-default state
  (`semantic_render.rs:265-269`). `InlineAdornments` additionally
  suppresses "nothing to say, nothing ever sent" (`:297-304`).
- TUI drops the whole family silently (`src/frontend.rs:380-422`) —
  its chrome arrives pre-painted in the cell grid.
- Pin tests updated every bump: `assert_eq!(PROTOCOL_VERSION, …)` at
  `src/protocol.rs:1710`, the resume-ladder test at `:1774-1797`
  (accepted range and the rejected next version), and a postcard
  round-trip pin per new variant.
- The dormant `InstanceMessage::ModeLine(Vec<Cell>)` (`message.rs:506`)
  is cell-shaped, unproduced, and not what this arc needs; it stays
  dormant.

### Pre-existing staleness bug (this arc fixes it)

`StyleGate` (`semantic_render.rs:226-246`) keys the `StyleSpans`
recompute on (parse-bundle `Arc`, CRDT generation, viewport) — never
the theme. `file_style_summary_msg` similarly keys the minimap
summary on `(generation, diag_epoch)` (`:914`). A mid-session
`pmacs.theme.set` therefore re-ships **nothing** to a semantic
frontend until the next buffer edit: the GPU keeps stale colors. The
grid path is immune (`SyntaxHighlightView` clones the theme every
render, `highlight.rs:316`), as is the ungated LSP-token span path.
There is no theme mutation counter anywhere in `src/`. Q#TH6
introduces two (syntax/face) and folds them into the gates.

## Decisions

### Q#TH1 — Scope: stage 1 of Arc 4; faces + wire only

This PR delivers named UI faces resolved daemon-side, applied by both
frontends, controlled through the existing `pmacs.theme` surface, and
shipped to semantic frontends over a new v16 `ThemeFacts` channel.
`pmacs.gpu.set_font` (stage 2) and the statusline-segment API
(stage 3) are deferred with named hooks: **stage 2 takes protocol
v17** — once a v16 binary exists, adding another variant under v16
would let old and new v16 peers negotiate successfully despite
incompatible schemas (postcard is not self-describing); reusing 16
is possible only if stage 2's wire shape lands in this same PR,
which it does not. The segment API lands after faces so segments can
carry face names instead of raw colors.

### Q#TH2 — Faces are theme entries under the reserved `ui` / `ui.*` namespace

A face is an entry in the existing `by_capture` map whose name is
exactly `ui` or starts with `ui.` -- no new type, no second map, no new
Lua API.
`pmacs.theme.merge { ["ui.modeline"] = { bg = 236, fg = 252 } }`
works today, unmodified. The face predicate is one explicit
function, used by every consumer — the docs reservation, Q#TH6's
counter-bump classification, and the producer's key filter:

```rust
fn is_face_name(name: &str) -> bool {
    name == "ui" || name.starts_with("ui.")
}
```

The bare root `ui` is included because Q#TH4 makes it the walk's
deliberate catch-all — a `merge { ui = … }` must bump `face_epoch`
and emit `ThemeFacts`, not disappear into the syntax path. No
tree-sitter capture or LSP token type is named `ui` or `ui.*`
(verified against `default_dark` and the grammar queries).
`pmacs.theme.get("ui.x")` keeps its
existing `lookup` semantics (resolves through to `default_style`) —
introspection tolerates that; rendering does not (Q#TH4).

### Q#TH3 — Stage-1 face inventory: twelve faces

The roadmap's five surfaces, plus the two chrome families that
already have daemon-side palettes (search, diagnostics), plus the
minibuffer's one sub-face. Effects per frontend (unset = exactly
today's literal, from the Ground-truth tables):

The **mask** column is the stage-1 component contract of Q#TH5:
identical on both frontends, sized to what both can render today.

| Face | Mask | Grid/TUI effect | GPU effect |
|---|---|---|---|
| `ui.modeline` | fg bg reverse | per-window mode-line row | status-band bg quad; band name + readout text |
| `ui.statusline` | fg | bottom status/echo row | echo-message segment text |
| `ui.minibuffer` | fg | prompt + input + fill; search prompt row | minibuffer/isearch band text |
| `ui.minibuffer.candidate` | fg | inline candidate suffix | minibuffer-dropdown candidate glyphs |
| `ui.gutter` | fg | line-number strip | gutter digit text |
| `ui.selection` | bg | local-region cells (wash) | local-selection wash RGB |
| `ui.search.match` | bg | lazy-match cells (wash) | match wash RGB |
| `ui.search.match.active` | bg | active-match cells (wash) | active wash RGB |
| `ui.diag.error` | fg† | severity color | squiggle + gutter sign + band `E:` counter |
| `ui.diag.warning` | fg† | severity color | squiggle + sign + `W:` counter |
| `ui.diag.info` | fg† | severity color | squiggle + sign |
| `ui.diag.hint` | fg† | severity color | squiggle + sign |

† diag faces carry the special `Default` policy of Q#TH5: `Default`
fg means the built-in severity color, not "plain".

The GPU status band is the mode-line analogue (one band, no separate
echo row — a structural divergence this arc does not resolve), so it
takes `ui.modeline` for its surface and `ui.statusline`/`ui.minibuffer`
only for the text of the corresponding *content* it happens to be
showing. `ui.minibuffer.candidate` colors the existing dropdown's
candidate glyphs only; its background and selected-row quad remain
popup chrome. Deliberately excluded from stage 1 (all named in Deferred):
popup/menu/completion-dropdown backgrounds and selection quads,
completion/menu glyph faces, `ui.background`, `ui.caret`,
`ui.modeline.inactive`, minimap chrome, peer-cursor palette,
inlay-hint face.

### Q#TH4 — Resolution: `Theme::face()`, `None` when unset, never `default_style`

New method on `Theme`:

```rust
pub fn face(&self, name: &str) -> Option<Style>
```

Same dotted-prefix walk as `lookup`, but the walk returns `None` when
nothing matches instead of falling back to `default_style` — an
unset face must leave the paint site's hardcoded default untouched,
and a user's `pmacs.theme.default` (a *syntax* fallback) must never
bleed into chrome. The walk gives inheritance for free:
`ui.search.match.active` falls back to `ui.search.match`, the four
`ui.diag.*` to `ui.diag`, and everything to a deliberate catch-all
`ui` (a theme can tint all chrome in one entry; a consequence of
uniform walk semantics, not a special case). Callers pass full face
names only.

### Q#TH5 — A set face owns its surface, within its component mask

Wire `Style` cannot distinguish an omitted component from an
explicitly-requested default, so per-component fallback to the
site's old literal would diverge across frontends (`ui.modeline =
{ fg = 252 }` would drop the TUI's reverse video while the GPU kept
its band background; an empty `ui.selection` would disable the TUI
wash but leave the GPU wash intact). And an unmasked wholesale rule
is not implementable either: the GPU gutter has only a foreground
text layer (`main.rs:4522`), and the selection/search washes are
background quads that can neither recolor nor reverse glyphs
(`main.rs:4801`) — an unrestricted contract would make
`ui.selection = { fg = 1 }` recolor TUI glyphs with no GPU
counterpart. The unified rule:

- **Unset face** (resolution `None`): the site's hardcoded default,
  both frontends, byte-identical to today.
- **Set face**: the surface resets to plain, and the face's
  components **within the face's mask** (the Q#TH3 table) apply.
  Components outside the mask are ignored on BOTH frontends — the
  mask is the cross-frontend contract, sized to the intersection of
  what the two render paths can express today, so equivalence is
  structural rather than aspirational. `ui.selection = { fg = 1 }`
  is a component no-op on the TUI too (an effectively-empty wash
  face: wash disabled on both); `ui.gutter = { bg = 1 }` likewise
  renders as an all-default gutter face on both. Widening a mask
  later (GPU gutter background layer, wash glyph recolor/reverse,
  chrome bold/italic/underline) is additive and named in Deferred.
- **`Default` within the mask** means *that frontend's plain
  rendering*, never "the old chrome look". On the TUI, plain =
  terminal defaults. On the GPU, a fixed mapping: `Default` fg ↦
  the buffer-text default `rgb(230,230,235)` (`main.rs:4482`);
  `Default` bg ↦ the window background (an untinted band quad; no
  wash quad); `reverse` (in-mask only for `ui.modeline`) swaps
  fg/bg after the mapping. `ui.modeline = { fg = 252 }` therefore
  yields "plain background, color-252 text" on both frontends, and
  `ui.selection = {}` disables the selection wash on both —
  consistent, if deliberately drab; the user asked for an
  all-default surface.
- **Wash faces** (`ui.selection`, `ui.search.match`,
  `ui.search.match.active`) remain a *composition* class: the
  masked face style (bg only, stage 1) replaces the default
  **overlay** argument of the existing `merge_styles(cell, overlay)`
  (`src/overlay.rs:136` — non-`Default` components win, booleans
  OR), then merges over syntax-styled cells as today, so a wash
  never touches glyph fg in stage 1. The unset defaults remain the
  full overlay literals of today (`Style { reverse: true, .. }` for
  selection; the search literals as-is — the mask constrains *face*
  application, not the unset defaults). On the GPU the wash quad
  takes the face `bg` RGB with the site's current alpha — wire
  `Style` has no alpha; translucency is a rendering choice and
  stays frontend-local (Q#UX1 division).
- **`ui.selection` themes the local selection only.** The GPU
  currently colors peer selection rects through the same
  `DecorationKind::Selection` helper (`main.rs:4946`); the
  implementation splits that resolution path — the local path
  resolves the face, the peer path keeps the constant — so peer
  rendering stays outside the theme system until the deferred
  peer-cursor palette arc. (The TUI peer path, `overlay_paint.rs`,
  is already separate.)
- **`ui.minibuffer.candidate` has one GPU effect in stage 1:** its
  masked `fg` is the `default_color` of the existing minibuffer
  dropdown candidate `TextArea` (`main.rs:4594-4610`). This does not
  theme the popup background or selected-row quad; those remain on
  their current constants and in Deferred. On the grid path the same
  face colors the inline selected-candidate suffix.
- **Diag faces**: the `fg` component is the canonical severity color
  (the `diag.rs:84-97` contract: squiggle `underline_color`, marker,
  minimap mark all share it), with a face-family-specific `Default`
  policy: **`Default` fg means the built-in severity color**, not
  plain. The severity color doubles as the *presence* encoding in
  the minimap summary — `scoped_file_summary` copies it into the
  line's `underline_color` (`semantic_render.rs:1779`), and the GPU
  reads `underline_color == Default` as "no diagnostic mark"
  (`main.rs:5551`) — so a plain severity color is unrepresentable
  and `ui.diag.error = {}` explicitly resets that child to the
  built-in error color on every surface (squiggle, sign, minimap, band
  counter). It is not absent: an exact empty child stops Q#TH4's walk,
  so it overrides an inherited `ui.diag` color with the built-in.

Residual divergence, accepted and tested: `Indexed` colors mean the
user's terminal palette on the TUI and the `indexed_to_glyphon`
table on the GPU (pre-existing; identical to syntax styles today).

### Q#TH6 — Two monotonic mutation counters; transactional mutators

`Theme` gains `syntax_epoch: u64` and `face_epoch: u64` (both 0 at
construction), owned by the same mutex. The mutation contract for
the four Lua bindings — the only mid-session mutation paths:

1. **Parse before locking.** The production `set`/`merge` transaction
   helper accepts a commit mode plus an ordered iterator of
   `mlua::Result<(String, Style)>`, collects the whole iterator into
   `mlua::Result<Vec<(String, Style)>>`, and only after `Ok(Vec)`
   acquires the theme mutex and commits in the requested mode. The Lua
   table iterator maps each raw `(name, style_table)` entry through
   `lua_to_style` into that result stream. `default` parses its one
   `Style` before locking; `clear` has no input to parse. `set`
   already parses into a scratch value pre-lock; `merge` today
   inserts while iterating under the lock (`mod.rs:6925-6931`), so a
   malformed later entry currently leaves earlier entries applied —
   that shape is disallowed.
2. **Commit all-or-nothing.** Every entry applies, or the binding
   errors with the theme untouched (and therefore no counter bump
   and no downstream emission).
3. **Bump from the prior value after every successful commit.**
   `set` replaces the `by_capture` *field* — never `*theme =
   new_theme`, whose zeroed counters would let consecutive `set`
   calls share an epoch and stay invisible to every gate — then
   bumps; counters only ever increment. Per mutator: `set` and
   `clear` bump both counters; `merge` classifies every committed
   key through `is_face_name` (Q#TH2 — bare `ui` is a face key) and
   bumps `syntax_epoch` iff any non-face key committed, `face_epoch`
   iff any face key did; `default` bumps `syntax_epoch` only.

Consumers:

- `StyleGate` gains a `syntax_epoch` field, compared in `matches()`
  — a capture recolor forces the tree-sitter span recompute and the
  changed spans ship (the fix for the pre-existing GPU staleness
  bug, bite-able on its own: mid-session `pmacs.theme.set` with no
  buffer edit must produce a `StyleSpans` re-emission). A face-only
  edit leaves `syntax_epoch` untouched and never re-runs the query.
- `file_style_summary_msg`'s cache key becomes `(generation,
  diag_epoch, syntax_epoch, face_epoch)` — `face_epoch` belongs in
  the key because `ui.diag.*` feeds the minimap marks — **plus a
  payload-equality suppression**: a face edit that leaves the
  summary unchanged (e.g. `ui.modeline`) recomputes it once per
  mutation, not per tick, and emits nothing. The cache key advances
  on *computation*, not on emission — the suppressed send still
  inserts the new key, or the recompute repeats every tick and the
  performance contract is a fiction. Rust units pin the
  single-recompute (by asserting the advanced cache state); traffic
  assertions cannot see repeated computation.
- The `ThemeFacts` producer gates on `face_epoch` (Q#TH7).

The LSP-token span path and the whole grid path already re-resolve
per tick/frame and need nothing.

### Q#TH7 — `ThemeFacts` channel: bufferless resolved-face table, protocol v15→16

```rust
/// Themes arc (Q#TH7, protocol v16). The daemon-resolved UI faces.
/// The theme is one global instance, so this is bufferless (the
/// `MinibufferPrompt` shape). Complete replacement each send: a
/// face absent from `faces` is unset, and the frontend uses its
/// own default for that surface. Cached-compare suppressed;
/// daemon-gated `>= 16`.
ThemeFacts {
    /// Every stage-1 face that resolves to a style (Q#TH4 walk),
    /// full names, sorted by name for deterministic comparison.
    faces: Vec<ThemeFace>,
},

pub struct ThemeFace {
    pub name: String,
    pub style: crate::cell::Style,
}
```

- **Resolution is daemon-side**: the producer walks the known face
  list (the Q#TH3 twelve) through `Theme::face()` and ships concrete
  entries — frontends do exact-name lookup, no walk, no inheritance
  logic. The daemon owns face semantics; frontends own pixels
  (Q#UX1). No epoch on the wire — the payload is self-contained and
  the house facts style carries no counters.
- **Producer**: `theme_facts_msg` on `SemanticRenderState`, called
  from the `render_frame` emission list. Per-tick cheapness comes
  from a `last_face_epoch: Option<u64>` gate **seeded `None`** —
  `Option`, not a bare zero, because an unthemed daemon sits at
  `face_epoch == 0` and a `0 == 0` short-circuit would starve the
  first authoritative send; the gate can only short-circuit once it
  has recorded an inspected epoch. Emission truth comes from
  comparing the rebuilt table against `last_theme_faces`, also
  seeded `None`. Both records advance on *computation*: an
  identical rebuild (e.g. after a payload-equal re-merge) records
  the epoch it inspected even though nothing ships, so subsequent
  ticks skip — a Rust unit pins the no-rebuild second tick. Every
  attachment receives exactly one authoritative table — the empty
  table included — with its first emission after viewport
  declaration, cached-compare suppressed thereafter. One small
  message per unthemed session buys a reconnect-safe contract: a
  frontend that retains face state across attachments is corrected
  even by an unthemed daemon, instead of depending on every frontend
  starting empty. (The current GPU has no reconnect path — a
  disconnect only swaps the displayed text, `main.rs:1435` — so this
  is contract, not a load-bearing assumption about today's client.)
- **Variant placement**: `ThemeFacts` is **appended after
  `CompletionPopup`** (`message.rs:961`), the current final
  `InstanceMessage` variant. Postcard discriminants are ordinal — a
  variant inserted earlier shifts every later variant's tag and
  silently corrupts v15 peers on channels that are *not* gated, and
  the new variant's own round-trip test cannot detect the shift. A
  byte-level encoding pin of `CompletionPopup` -- the final existing
  variant, so any insertion before any v15 variant moves its ordinal --
  asserts its serialized bytes, discriminant included, are unchanged
  by the v16 build.
- **Gating**: `PROTOCOL_VERSION` 15→16 with a ladder paragraph;
  `SUPPORTED_PROTOCOL_VERSIONS` grows 16; daemon write-loop
  `peer_knows_theme_facts = negotiated_protocol_version >= 16` +
  skip arm; TUI silent-drop arm (`frontend.rs:380-422`); GPU
  debug-name arm (`main.rs:5606-5628`); `ThemeFace` re-exported from
  `pmacs-protocol/src/lib.rs`; pin tests at `protocol.rs:1710` and
  the ladder test (accept 16, reject 17); postcard round-trip pin.
- `docs/semantic-frontend-protocol.md` gains the channel's contract
  section.

### Q#TH8 — GPU application: exact-name map, Q#TH5 mapping, shaping-cache invalidation

`State` gains `faces: HashMap<String, Style>`; the
`apply_attach_message` arm replaces the map, **invalidates the
status-band shaping cache**, and `request_redraw()`s (extending the
`LineNumbers` arm shape, `main.rs:2686-2692`). The invalidation is
load-bearing, not belt-and-braces: the `E:`/`W:` counter colors are
baked into glyphon rich-text attributes at compose time
(`compose_status_spans`, `main.rs:3390-3441`), and
`refresh_status_line` skips re-shaping whenever the composed strings
are unchanged (`main.rs:3502-3546`) — without clearing those cached
comparison strings, a diag-face change with constant counts keeps
stale counter colors indefinitely.

Each themed site resolves per draw with the Q#TH5 rule: face absent
→ today's site constant; face present → the face's **in-mask**
components, with `Default` fg ↦ the buffer-text default, `Default`
bg ↦ the window background (untinted quad / no wash quad),
`reverse` (`ui.modeline` only) as a post-map swap, and the diag
`Default`-fg ↦ built-in-severity-color policy. Out-of-mask
components are never read. `Rgb` maps directly; `Indexed` through
the existing `indexed_to_glyphon` table; quad colors convert u8 RGB
into the same color space the current float constants use;
band/wash alphas keep their current values. Peer selection rects
resolve through the split constant path, never the face (Q#TH5).
The minibuffer candidate face supplies the existing dropdown
`TextArea.default_color`; `refresh_mb_buffer` already runs before each
dropdown paint (`main.rs:4337-4341`), and the default color is resolved
again when preparing the `TextArea`, so no new shaping-cache contract is
needed for its fg-only mask.

### Q#TH9 — Grid application: one theme clone per frame; views gain the handle

`paint_frame` clones the `Theme` once per frame (single mutex lock,
same discipline as `SyntaxHighlightView::render`) and passes `&Theme`
down to `paint_mode_line` / `paint_status_line` / `paint_minibuffer` /
`paint_search_prompt` / `paint_line_number_gutter` /
`paint_local_selection` (signature additions; `paint_mode_line` takes
the resolved `Style` since it is a pure formatter). `SearchView` and
`DiagnosticView` gain a `ThemeHandle` constructor parameter and
resolve faces under the lock per render — the exact
`SyntaxHighlightView::new` precedent (`highlight.rs:246`).

Ownership, named: `SearchView` is constructed inside
`EditorCore::ensure_search_overlay` (`src/editor_core.rs:858`), and
`EditorCore` owns no syntax state — so `EditorCore` gains a
`theme: Option<ThemeHandle>` field, injected once at editor bring-up
right after `SyntaxRegistry` construction, and
`SearchView::new(store, theme: Option<ThemeHandle>)` takes the
`Option` directly: `ensure_search_overlay` passes the field through
unconditionally, so a bare core (unit-test construction, field
`None`) still constructs a working view that resolves no faces and
paints today's literals — no contradiction between "views take the
handle" and "bare cores have none". `DiagnosticView`
is attached by the Lua path — `install_diag`
(`src/lua_bindings/diag.rs:46`, construction at `:220`) — which gains
the handle as a parameter, threaded from the installer call site
(`mod.rs:8626`). Acceptance exercises the real attachment paths: a
dispatched `C-s` reaching `ensure_search_overlay`, and the Lua
diagnostic attachment — never bare constructors. The completion
popup and menu keep their literals (faces deferred).

### Q#TH10 — `pmacs.theme.set` wholesale semantics include faces

`set` replaces the whole `by_capture` map today and therefore wipes
faces along with captures; `merge` is the incremental path. This is
kept unchanged — a "theme" legitimately includes its chrome, and a
theme-switching user wants no stale faces from the previous theme.
Documented in the Lua API docs alongside the `ui.` namespace
reservation.

## Bets

- The reserved-namespace design (Q#TH2) means zero new Lua API and
  zero protocol vocabulary beyond one variant — the whole feature
  rides existing `Style` plumbing.
- The authoritative-per-attachment contract (seed `None`) makes
  late-join and stateful reconnect correct by construction for the
  price of one small message per session; unthemed steady-state
  traffic remains zero.
- Daemon-side resolution keeps both frontends walk-free and makes
  face inheritance testable in one place.
- The split-counter fix (Q#TH6) closes the mid-session staleness bug
  at one-u64-compare per-tick cost, while face-only edits stay off
  the tree-sitter path entirely; the one whole-file summary
  recompute per face mutation is user-action-rate, not tick-rate.
- The owns-surface-within-mask rule (Q#TH5) is the only semantic
  expressible with the existing `Style` (no omitted-vs-default
  distinction) that renders equivalently on both frontends, and the
  masks make that equivalence structural — the contract promises
  only what both render paths can draw today, and widening a mask
  later is additive. Its cost — partial faces reset the rest of
  their surface to plain, and some components are simply not
  themable yet — is documented and falls only on users who theme.
- Unset faces reproduce today's rendering byte-identically on both
  frontends — the acceptance baseline tests pin this, so shipping
  stage 1 changes nothing for users who theme nothing.
- Twelve faces cover the roadmap's named surfaces; the deferred
  chrome (popups, background, caret) reuses the same substrate
  without another protocol bump.

## Deferred (named)

Popup/menu/completion-dropdown background, selected-row, and general
glyph faces (`ui.popup`, `ui.menu`, …; the fg-only
`ui.minibuffer.candidate` exception is in scope);
`ui.background` and `ui.caret` (GPU-only effect; background interacts
with the TUI's terminal-default philosophy and deserves its own
decision); `ui.modeline.inactive` (active/inactive split);
minimap chrome faces; peer-cursor palette theming
(`overlay_color.rs` — deliberately not on the wire today);
inlay-hint face (`ui.inlay_hint` — `semantic_render.rs:1137` names
the gap; its adornment producer needs the same epoch treatment when
it lands); alpha on the wire (washes keep frontend-local alpha until
a real theme needs to change it); unifying `Indexed` palette meaning
across frontends; named-theme registry / light-theme builtin /
persistence (no unified config surface yet — handoff §6); the
grid-vs-wire `default_style` asymmetry (uncaptured text: wire ships
nothing where the grid paints the base style — orthogonal to faces);
mask widening (GPU gutter background layer; wash glyph
recolor/reverse; a distinct GPU echo surface so `ui.statusline`
could carry bg; chrome bold/italic/underline — attribute re-shaping
that compounds the Q#TH8 shaping-cache invalidation);
`ui.selection` for peer selections (rides the deferred peer-cursor
palette; the GPU resolution path is already split);
`pmacs.gpu.set_font` (stage 2,
**protocol v17** per finding 2 — and the `pmacs-gpu-design.md:299`
claim that font customization needs no wire change is superseded by
the Q#UX1 lesson, to be corrected in that framing); Lua
statusline-segment API (stage 3 — should ship segments carrying face
names, hence after this arc).

## Acceptance

Suites: `tests/theme_faces_acceptance.rs` (grid-path rendering +
Lua surface + wire facts) and the existing pin/round-trip homes
(`src/protocol.rs`); GPU behavior in `pmacs-gpu` behind
`PMACS_REQUIRE_GPU=1` (frame-difference pattern, `main.rs:7877+`).
Keybinding-driven tests dispatch keys, never `pmacs.command.invoke`.

1. **Unset = today, byte-identical**: with neither a bare `ui` entry
   nor any `ui.*` entry, a
   painted frame (mode line, status row, gutter, minibuffer,
   selection, search matches, diag squiggle) is cell-for-cell
   identical to the pre-arc rendering; additionally a loud
   `pmacs.theme.default` (syntax fallback) leaks into **no** chrome
   cell (pins Q#TH4's `None`-not-`default_style`).
2. **Each surface face applies** (grid): setting `ui.modeline`,
   `ui.statusline`, `ui.minibuffer`, `ui.minibuffer.candidate`,
   `ui.gutter` each changes exactly its rows'/strip's cells to the
   masked face style on the next frame, per-cell asserted.
3. **Owns-surface semantics** (Q#TH5): `ui.modeline = { fg = 252 }`
   paints the grid mode line with fg 252 and **no** reverse video
   (partial face resets the surface to plain); `ui.selection = {}`
   leaves selected cells identical to unselected ones.
4. **Mask enforcement** (round 2 finding 1): `ui.selection = { fg = 1,
   reverse = true, bg = B }` renders exactly as `{ bg = B }` on both
   frontends — grid per-cell (glyph fg untouched under the wash),
   GPU frame identical to the bg-only variant; `ui.gutter = { bg =
   1, reverse = true, fg = F }` renders exactly as `{ fg = F }` on
   both.
5. **Wash faces merge**: `ui.selection` and both search faces
   compose over syntax-styled cells with `merge_styles` semantics —
   a bg-only wash keeps the syntax `fg` under the wash (per-cell
   fg+bg assertions, not any-styled-cell). Search asserted through a
   dispatched `C-s` reaching `ensure_search_overlay` (the real
   construction path), never a bare constructor.
6. **Diag faces**: `ui.diag.warning` recolors the squiggle
   `underline_color`, the column-0 marker, and the minimap mark,
   attached via the real Lua diagnostic path (`install_diag`);
   unset severities keep `Indexed` defaults. **Empty diag face**
   (round 2 finding 2, clarified in round 3): with no parent,
   `ui.diag.error = {}` renders identically to the built-in error face
   on every surface -- squiggle, sign, minimap mark (present, not
   vanished), band counter. With `ui.diag = { fg = C }` also set, the
   exact empty `ui.diag.error` child blocks inheritance and resets
   errors to the built-in color while warning/info/hint remain `C`.
7. **Inheritance**: with only `ui.diag` set, all four severities
   resolve to it; an explicit `ui.diag.error` then wins for errors
   only. Same for `ui.search.match.active` ← `ui.search.match`.
8. **Bare `ui` is a face** (round 2 finding 3):
   `pmacs.theme.merge { ui = … }` bumps `face_epoch` and ships a
   `ThemeFacts` table with every stage-1 face resolved to the
   catch-all, with **no** `StyleSpans` re-emission (the key
   classifies as face, not syntax).
9. **Mid-session recolor, no edit (the staleness bite)**: a semantic
   session with a grammar-backed buffer receives fresh `StyleSpans`
   after `pmacs.theme.set` changes a capture color with zero buffer
   edits — fails against pre-arc `StyleGate`. Twin for the minimap:
   `FileStyleSummary` re-emits on a diag-face change.
10. **Consecutive `set` monotonicity** (round 1 finding 1,
    observation shape per round 2 finding 4): `set` →
    render/observe one `StyleSpans` emission → `set` (different
    capture colors) → render/observe a second emission — fails if
    wholesale replacement resets the counters and the second `set`
    shares the first's epoch. Two mutations inside one dispatch
    legitimately coalesce: a companion assertion pins that
    back-to-back `set`s followed by one render yield one emission.
11. **Malformed merge is atomic**: the Lua acceptance pins the
    order-independent user contract — one `pmacs.theme.merge`
    carrying a valid and a malformed entry errors, every entry of
    that merge is absent afterwards, and no `ThemeFacts` or
    `StyleSpans` re-emission occurs. Lua table iteration order is
    unspecified (round 2 finding 8), so the deterministic bite drives
    the production transaction helper with an ordered iterator
    `[Ok((valid_name, valid_style)), Err(malformed)]`. The helper must
    return the error before acquiring the theme lock; the unit asserts
    the pre-existing map, `syntax_epoch`, and `face_epoch` are all
    unchanged. An implementation that locks and inserts while
    consuming the iterator fails deterministically.
12. **ThemeFacts emission discipline**: every v16 semantic attachment
    receives exactly one authoritative table after viewport
    declaration — the **empty table** for an unthemed session
    (`face_epoch` still 0; pins the `Option` gate of Q#TH7) — then
    silence; after `pmacs.theme.merge` of a face, exactly one
    message with the resolved sorted table; an identical re-merge
    emits nothing; `pmacs.theme.clear` emits the empty table.
13. **Face-only mutations stay cheap** (round 1 finding 8): after a
    `ui.modeline` merge, no `StyleSpans` and no `FileStyleSummary`
    re-emission occurs (absence assertions) while `ThemeFacts`
    ships.
14. **Caches advance on computation** (round 2 finding 5, Rust
    units): after an identical re-merge, `theme_facts_msg` rebuilds
    once and the next tick skips via the recorded epoch; after a
    non-diag face mutation, the summary recomputes once and its
    cache key advances despite the suppressed emission — asserted
    on the internal cache state, since traffic cannot show repeated
    computation.
15. **Resolution is daemon-side**: with only `ui.diag` set, the
    shipped table contains the four concrete `ui.diag.*` entries
    (frontends never walk).
16. **Late join**: faces set via init.lua/first session; a second
    semantic frontend attaching later receives the face table among
    its first frames without any mutation occurring post-attach.
17. **Version gate and placement**: a v15 peer session never
    receives `ThemeFacts` (daemon skip arm); pin tests updated —
    `assert_eq!(PROTOCOL_VERSION, 16)`, ladder accepts 6..=16 and
    rejects 17, `ThemeFacts` postcard round-trip pin, **and a
    byte-level encoding pin of `CompletionPopup`**, the final v15
    variant, proving the appended placement shifted no existing
    discriminant (round 2 finding 7, corrected in round 3).
18. **TUI drop arm**: grid-session frontend consumes a `ThemeFacts`
    message without error (silent drop, family test pattern).
19. **`set` wipes faces** (Q#TH10): `pmacs.theme.set` with a
    captures-only table removes prior faces — next frame chrome is
    back to defaults and `ThemeFacts` ships the empty table.
20. **GPU surface/text routes** (`PMACS_REQUIRE_GPU=1`): an empty
    `ThemeFacts` table renders identically to never-themed. A
    `ui.modeline` bg changes the band, and a modeline face with
    `reverse = true` swaps the sampled band/text colors after Default
    mapping. A `ui.gutter` fg changes only the gutter digits. With the
    composed strings held constant, `ui.statusline` recolors a
    transient-message left segment, while `ui.minibuffer` recolors
    both a live minibuffer and an isearch band without recoloring the
    ordinary buffer-name state. `ui.minibuffer.candidate` recolors the
    actual dropdown candidate glyph region while leaving its popup
    background and selected-row quad at their constants.
21. **GPU wash routes** (`PMACS_REQUIRE_GPU=1`): a local selection
    uses `ui.selection` (including the mask-equivalence assertion in
    item 4); simultaneous normal and active search decorations use
    `ui.search.match` and `ui.search.match.active` respectively, with
    distinct sampled/decoded quad colors and unchanged glyph fg.
22. **GPU diagnostic route and shaping invalidation**
    (`PMACS_REQUIRE_GPU=1`): changing `ui.diag.error` while the
    composed band text and diagnostic counts stay constant changes
    the squiggle, gutter sign, minimap mark, and `E:` counter. The
    counter assertion fails without clearing the rich-text comparison
    cache; the empty-child reset case is item 6.
23. **Peer selections keep the constant** (round 2 finding 9,
    integration shape corrected in round 3): construct simultaneous,
    non-overlapping local and peer selections and drive the actual
    `decoration_background_vertex_bytes` / `collect_peer_rects` path.
    Decode the emitted vertex colors: the local rectangle uses
    `ui.selection`, while the peer rectangle remains the hardcoded
    constant. A helper-only resolver assertion is insufficient.
24. **Lua surface unchanged**: `pmacs.theme.current()` lists both bare
    `ui` and `ui.*` entries alongside captures; `get("ui.modeline")`
    returns the set style (existing lookup semantics).
