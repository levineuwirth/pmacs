# Tab-width rendering parity - side quest

**Status:** Revision 2 implemented on `tab-width-parity`; all fifteen
acceptance criteria pass locally. PR #137 is open for review.

**Base:** `githubsucks/main` at `40111dc` (landed-state documentation for
locals-query processing #134); protocol v18.

## Problem

Pmacs has no single tab-rendering contract:

- `src/text_view.rs`, `src/highlight.rs`, `src/diag.rs`, and
  `src/completion.rs` independently hard-code an 8-column tab stop.
- `src/overlay.rs` repeats the same 8-column arithmetic as literals.
- `pmacs-gpu/src/main.rs::advance_minimap_col` uses 4 columns and counts every
  non-tab character as one column.
- The GPU code buffer sends raw `\t` bytes to cosmic-text. Its visible width is
  therefore whatever the selected font's tab glyph happens to provide, not a
  pmacs tab stop.

The disagreement is observable. The same buffer can place text, syntax faces,
diagnostic squiggles, completion popups, selections, carets, and minimap marks
at different columns between the TUI and GPU frontends. Merely defining an
`editor.tab-width` config key would not fix the GPU: tab expansion, source-byte
mapping, styling, and hit testing all happen inside the frontend after the raw
semantic frame arrives.

This is the remaining top-ranked item in `docs/side-quest-backlog.md:123-130`
and `:245-248`.

## Goal

Define one fixed 8-column tab-stop invariant, make every shipped buffer-text
renderer honor it, and preserve byte-addressed editor semantics while the GPU
shapes an expanded display projection.

For a tab beginning at logical display column `c`, its width is

```text
8 - (c mod 8)
```

so a tab at an already aligned column advances a full eight columns. Source
text remains byte-for-byte unchanged.

## Scope

### In

- One canonical tab-stop constant shared by the core and GPU crates.
- One core display-column utility used by plain text, syntax styling,
  diagnostics, completion placement, and generic buffer-style overlays.
- Tab expansion in the GPU code-buffer projection before cosmic-text shaping.
- Correct projected-to-source and source-to-projected mapping for clicks,
  carets, selections, diagnostic geometry, wrapping, and inline adornments.
- GPU minimap width/indent accounting using the same tab and Unicode-width
  rules as the code view.
- Focused regression coverage at tab-stop boundaries, after wide Unicode, and
  through styled and selected tab bytes.

### Out

- A user-configurable `editor.tab-width` setting. This change deliberately
  chooses the already-shipped TUI behavior, 8, and makes it universal.
- Per-buffer or per-language tab widths, indentation policy, soft-tab
  insertion, tab-to-spaces conversion, or retabbing existing files.
- Changing what the Tab key inserts. A literal tab remains one source byte.
- Expanding tabs in protocol payloads or mutating `SemanticFrame` byte ranges.
- Tabs in statusline, minibuffer, menu, hover panels, or other non-buffer UI
  strings.
- A wire schema or protocol-version change.

## Ground truth and contracts to preserve

### Core renderers are byte-addressed but paint in display columns

`TextView` already expands a tab to spaces at the next multiple of 8 and maps
the one source byte to that display interval. Syntax highlighting and
diagnostics independently translate byte ranges into display columns.
Completion computes its popup anchor from a byte offset. Generic
`BufferStyleSpan` overlays compute both ends from the line start. All five
paths require the same prefix-width operation; today they implement it
separately.

The current TUI inverse mapping rounds every display column inside an expanded
tab forward to the source boundary after the tab. That behavior is observable
and remains the cross-frontend rule.

### GPU code text is already a source-preserving projection

`projected_rich_chunks` interleaves source text, foreground style spans, and
inline adornments. `line_from_chunks` is the only content fed to cosmic-text.
`line_chunk_cache` drives source-byte-to-layout-cursor conversion, while
`current_hit_runs` and `projected_line_starts` convert cosmic-text hit results
back into source bytes. Incremental line reshaping and full slice rebuilds both
consume the same chunk construction path.

Tab expansion belongs at this projection boundary. Expanding the daemon's text
would invalidate every protocol byte range; asking cosmic-text to interpret raw
tabs would retain font-dependent behavior.

### GPU geometry currently assumes source bytes equal shaped bytes

Foreground colors are attached to chunks and therefore naturally survive a
projection when the chunk provenance is retained. Background selections,
current-line washes, and diagnostic squiggles are different:
`push_glyph_extent_rects` currently compares source-relative decoration bytes
directly with cosmic-text glyph byte offsets. That equality already needs
special handling for adornments and becomes definitively false once one tab
byte projects to multiple spaces. The geometry path must use the same
source/projection mapping as caret placement and hit testing.

### The protocol transports raw text and raw byte ranges

`pmacs-protocol` owns the types shared by the daemon and GPU. `SemanticFrame`
continues to carry unmodified text plus byte-addressed spans, decorations, and
adornments. A tab-stop constant is a rendering semantic for those existing
fields, not a serialized field. Adding it changes neither postcard encoding nor
version negotiation.

## Decisions

### Q#TW1 - The canonical tab stop is fixed at eight columns

Add a documented public constant named `TAB_STOP_COLUMNS: u32 = 8` to
`pmacs-protocol` and re-export it through the crate root. Both the pmacs core
and `pmacs-gpu` consume that constant.

The shared protocol crate is the narrow existing dependency common to both
frontends. A second rendering crate is unjustified, while two frontend-local
constants would preserve the drift this work is meant to remove. The constant
is normative metadata for interpreting raw text already carried by the
semantic protocol; it is not serialized.

Do not add a config-registry key. A future configurable width would need a
buffer-effective value in every semantic frame (or another versioned frontend
fact), cache invalidation when it changes, and tests across reconnects. That is
a separate feature, not hidden scope in this parity fix.

This work changes no `PROTOCOL_VERSION`: no message variant, field, encoding,
capability, or negotiation rule changes. It adds a compiled rendering invariant
for a previously unspecified raw-tab case to the protocol version present on
its implementation base.

### Q#TW2 - One core module owns display-column arithmetic

Add `src/display_width.rs` and export it from `src/lib.rs`. It owns:

- `TAB_STOP_COLUMNS` consumption from `pmacs_protocol`;
- advancing a logical column by one character, including tabs and
  `unicode-width` handling;
- the width of a valid UTF-8 string from a specified starting column;
- the display column at a byte boundary in a line; and
- the display-column pair for a half-open byte range.

Byte helpers clamp to the input length and use the longest valid UTF-8 prefix
when a stale/asynchronous range lands inside a code point. They do not allocate.
Tabs are always evaluated from the line's logical column zero, not from the
viewport edge or a range's start.

Migrate `text_view`, `highlight`, `diag`, `completion`, and `overlay` to this
module. Delete their constants and private copies rather than leaving aliases
or wrapper functions. `TextView` may still special-case tab painting, but its
pad count comes from the shared column advance.

### Q#TW3 - GPU expands tabs in the rich-chunk projection

After source/style/adornment boundaries have produced `RichChunk`s, run one
projection pass before either full-slice or per-line shaping. The pass walks
chunks and code points in display order while tracking a logical display
column:

- ordinary characters retain their text and provenance and advance by
  `unicode-width`;
- newline resets the logical column to zero;
- a source tab becomes `TAB_STOP_COLUMNS - (column % TAB_STOP_COLUMNS)` ASCII
  spaces carrying explicit provenance for that one source byte;
- a tab inside an adornment also becomes spaces but retains the adornment's
  anchor provenance; and
- zero-width characters do not advance the logical column.

A chunk with no tab is retained rather than copied again. Chunks containing
one or more tabs are split only at those tab boundaries. The existing visible
slice and per-line caches therefore bound both allocations and work; the
frontend never expands the whole file merely to draw one viewport.

`line_from_chunks`, `build_hit_runs`, incremental line replacement, and the
full rebuild all consume the expanded chunks. No alternate shaping path may
feed raw buffer tabs to cosmic-text.

### Q#TW4 - A projected tab run has first-class source provenance

Extend `ChunkSource` with a source-tab form containing the tab's
slice-relative byte offset. The derived `ProjectedRun` then represents three
semantics:

1. source text is byte-linear;
2. adornment text snaps to its anchor; and
3. all projected spaces for a tab correspond to one source byte.

Boundary rules are explicit:

- source offset at the tab byte maps to the first projected space;
- source offset immediately after the tab maps after the final projected
  space;
- a projected hit exactly at the tab's leading boundary maps before the tab;
- any hit inside its expanded interval maps after the tab, matching
  `TextView::display_to_pos`; and
- a hit at the following projected boundary maps to the following source
  boundary without crossing an adornment's established left-gravity rule.

Factor the per-line source-to-projected conversion out of
`State::code_byte_to_projected` so caret placement, decoration geometry, and
unit tests use the same boundary implementation. Keep projected-to-source in
the run map built from those exact chunks. Do not infer positions from counts
of spaces after shaping.

### Q#TW5 - Tab stops use the final visible logical column

The projection pass counts all visible content before a tab, including wide
Unicode and inline-adornment text. This makes the expanded tab end on a visible
8-column boundary instead of overlapping or drifting when an inlay hint occurs
before it.

Cosmic-text remains responsible for glyph shaping and pixel geometry. The tab
rule controls how many monospace spaces are supplied; it does not replace
shaping with manual pixel placement. Code font fallback may vary in pixels,
but logical columns remain deterministic.

Add `unicode-width = "0.2"` as a direct `pmacs-gpu` dependency. Do not reach
through another crate's transitive dependency.

### Q#TW6 - Styles and decorations cover the full projected tab

Foreground styling is preserved by assigning every expanded source-tab chunk
the color of the source chunk that contained the tab. A style span covering
`[tab, tab + 1)` therefore colors every projected space; a span ending at the
tab colors none of them.

For background selections and diagnostic squiggles, convert each source-range
intersection on a shaped line to projected byte boundaries before comparing it
with `LayoutGlyph::{start,end}`. The conversion uses that line's cached chunks
and the Q#TW4 boundary rules. Do not rewrite protocol ranges, and do not use
source line offsets as if they were projected byte offsets.

This same conversion covers own selections, peer selections, current-line
geometry where applicable, and diagnostic ranges. Gutter diagnostic signs are
line-presence indicators and remain source-line based; they need no horizontal
projection.

### Q#TW7 - The minimap uses the same logical-width rule

Replace the hard-coded 4-column `advance_minimap_col` branch with the shared
`TAB_STOP_COLUMNS` value. Ordinary characters advance by `unicode-width`
instead of unconditionally by one; zero-width characters advance by zero and
wide characters by two.

The minimap remains a density abstraction rather than shaped text, but its
indent and content extents now agree with the code view's logical columns.
Clipping and pixel compression are unchanged.

### Q#TW8 - Projection invalidation follows existing text/chunk invalidation

Tab width is fixed at compile time, so it introduces no runtime invalidation
source. Text edits, style/adornment updates, font changes, resizes, scrolling,
and buffer switches already rebuild or replace the affected chunk cache. Tab
projection runs inside those existing paths.

`try_reshape_line` must regenerate the expanded chunks for the edited line;
`rebuild_lines_reusing_scroll` may retain an unchanged line and its already
expanded cache. `hit_map_dirty` continues to mark when the whole-slice reverse
map must be rebuilt. No new generation counter or whole-file cache is needed.

## Data flow

```text
                         daemon / TUI core
source bytes ───────────────────────────────────────────────┐
   │                                                       │
   ├─ display_width helpers ──> TUI glyph/style columns    │
   │                                                       │
   └─ SemanticFrame { raw text, byte ranges } ─────────────┤
                                                           v
                                                     pmacs-gpu
                                                           │
                     style + adornment boundaries ─────────┤
                                                           v
                                              source-rich chunks
                                                           │
                                         expand tabs to spaces
                                         + preserve provenance
                                                           │
                          ┌────────────────────────────────┼─────────────┐
                          v                                v             v
                    cosmic-text                    hit/caret map   decoration map
                    shaping/render                   ↕ source       source → glyph
                          │                                │             │
                          └────────────────────────────────┴─────────────┘
```

The invariant is that only the display projection expands a tab. Every editor,
protocol, edit, selection, syntax, diagnostic, and adornment coordinate remains
a source-byte coordinate.

## Bets

1. **Eight is the correct parity target.** It is the established TUI behavior
   and existing tests already encode it. This work removes divergence rather
   than introducing a new preference.
2. **ASCII spaces are the stable shaping input.** The code font is measured as
   monospace and spaces participate in wrapping, hit testing, and glyph ranges
   that the existing GPU architecture already understands.
3. **Visible-slice expansion is cheap enough.** The GPU already allocates owned
   rich chunks for the shaped viewport. Scanning them once and allocating only
   around actual tabs is below shaping cost and avoids a whole-file projection.
4. **Forward rounding inside a tab is acceptable.** It matches the shipped TUI
   inverse mapping and avoids inventing fractional positions inside one source
   byte.
5. **The fixed semantic constant needs no protocol-version change.** This work
   changes no message representation or negotiation rule. Existing compatible
   clients remain decodable but must adopt the documented invariant to obtain
   visual parity.

## Acceptance criteria

1. **Canonical rule:** one exported `TAB_STOP_COLUMNS = 8` definition is shared
   by the pmacs core and GPU; no renderer-local tab-width literals remain in
   the touched buffer-rendering paths.
2. **Source preservation:** inserting/opening `"\t"` leaves one tab byte in the
   buffer, semantic frame, edits, undo history, and saved file. Rendering never
   replaces source text.
3. **TUI boundary behavior:** tabs beginning at columns 0, 7, and 8 end at
   columns 8, 8, and 16 respectively in plain rendering and position mapping.
4. **Core overlay parity:** syntax foreground spans, diagnostic underlines,
   completion popup anchors, and generic buffer-style spans all resolve the
   same byte boundary after tabs and wide Unicode to the same display column.
5. **GPU shaping input:** code-buffer chunks presented to cosmic-text contain
   no raw tab from source text or text adornments. The equivalent expanded
   spaces end at the next logical 8-column boundary.
6. **GPU visual geometry:** for `"\tx"`, `"1234567\tx"`, and
   `"12345678\tx"`, the GPU lays out `x` at logical columns 8, 8, and 16.
   A case with a width-2 Unicode character before the tab also lands on the
   mathematically correct stop.
7. **Caret mapping:** source carets immediately before and after a tab render at
   the leading and trailing edges of the expanded interval, including when the
   line wraps near that interval.
8. **Hit testing:** clicking the tab's leading boundary resolves before the tab;
   clicking within its expanded spaces resolves after it; clicking following
   text resolves to its original source byte. No click returns a synthetic
   space offset.
9. **Styled tab:** a source foreground span exactly covering a tab colors every
   expanded space and does not color the following source character.
10. **Selected/diagnostic tab:** own and peer selections and diagnostic
    squiggles whose source range covers a tab span the full projected interval
    and remain aligned with following text. The GPU layout case includes a soft
    wrap whose boundary falls inside the expanded tab, proving that one source
    byte produces correct geometry on both visual lines.
11. **Adornment interaction:** an inline text adornment before a source tab
    contributes to the visible logical column, the tab still ends at the next
    8-column stop, and source/adornment hit gravity remains deterministic.
12. **Minimap parity:** leading and interior tabs use 8-column stops; width-2
    and zero-width Unicode affect minimap logical columns by 2 and 0 rather
    than 1.
13. **Edit freshness:** inserting or deleting a tab on a visible line updates
    shaping, caret position, hit testing, styles/decorations, and minimap shape
    on the next normal refresh without switching buffers or forcing a full
    rebuild.
14. **No scope creep:** `pmacs.config` gains no tab-width key, and this work
    changes no `PROTOCOL_VERSION`, wire message shape, or negotiation rule.
15. **Quality gates:** focused default/Lua 5.4 tests, the touched acceptance
    suite, both GPU unit and required hardware-backed tests, the standard
    project gates, workspace sweep, and `git diff --check` pass.

## Verification plan

Focused checks should exercise the shared arithmetic and the two real render
paths rather than inspecting source text:

- core unit tests for valid-prefix handling, Unicode widths, and tab starts at
  0/7/8;
- existing and extended `TextView`, highlight, diagnostic, completion, and
  overlay tests using byte ranges that cross tabs;
- GPU projection-map tests for tab expansion and both mapping directions;
- GPU layout/offscreen tests for caret, selection/diagnostic geometry,
  a soft-wrap boundary inside an expanded tab, adornments, and edit freshness;
- minimap shape tests for tabs and Unicode; and
- `tests/tab_width_acceptance.rs` rendering one tabbed fixture through the
  core-facing path while the GPU suite proves the frontend projection.

Required commands before a PR:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --lib
cargo test --lib --features crdt
cargo test --no-default-features --features lua54 --lib
cargo test --test tab_width_acceptance
cargo test --test m4_acceptance -- --skip basedpyright
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
cargo test --workspace -- --skip basedpyright
git diff --check
```

Run strict Clippy as its own command. Any known timing-only failure must be
rerun isolated per `docs/agent-handoff.md`; a rerun is evidence only when the
failure matches a documented flaky test.

## Expected files

- `pmacs-protocol/src/lib.rs` - canonical tab-stop rendering constant and
  protocol-level documentation.
- `src/display_width.rs` and `src/lib.rs` - shared core display-column logic and
  module export.
- `src/text_view.rs`, `src/highlight.rs`, `src/diag.rs`, `src/completion.rs`,
  and `src/overlay.rs` - remove duplicated arithmetic and consume the helper.
- `pmacs-gpu/Cargo.toml`, `Cargo.lock`, and `pmacs-gpu/src/main.rs` - direct
  Unicode-width dependency, tab projection/provenance, geometry mapping,
  minimap parity, and focused tests.
- `tests/tab_width_acceptance.rs` - focused observable TUI/core parity across
  plain text and byte-addressed overlays.
- `docs/agent-handoff.md`, `docs/active-work.md`,
  `docs/side-quest-backlog.md`, and this framing document - updated only after
  implementation is proven and published according to their protocols.

No Lua runtime, config-registry, syntax-query, theme, or serialized protocol
file should need a behavior change.
