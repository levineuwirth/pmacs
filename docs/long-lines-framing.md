# Long lines — QoL Stage 3

**Status: revision 16 — branched as `long-lines`; Q#LL1–LL7 settled;
**Q#LL8 not yet approved** (two corrections from review of `1c9ff6a`).
Not yet implemented.**

**Revision 2** corrected a load-bearing error in revision 1: it claimed
both frontends render from the same `CellGrid`. They do not — the GPU
ignores the grid variants and lays out locally, and it **already wraps
long lines**. See §1.2. The correction reverses the cost profile and
therefore the recommendation in §3.

**Revision 3** fixed two things review caught downstream of that
correction. The opening still called the defect "total ... in either
frontend," contradicting §1.2's own table — corrected below, along with
what the cross-frontend defect actually is. And §7's GPU witness
described the non-wrap mode as "scroll/truncate," conflating a deferred
Stage 4 capability with a Stage 3 mode; **§3.1 now states the Stage 3
surface explicitly** rather than letting a test mode imply a product
one.

Writing §3.1 surfaced a third thing neither review nor revision 2 had
named: agreeing on the mode is not agreeing on the wrap. **New Q#LL5
(§5a)** — the GPU wraps at `Wrap::WordOrGlyph` and a grid walk would
naturally wrap at the character, so both frontends could honor `wrap`
and still break lines in different places. It was briefly buried in
"not in scope"; it is the same class of defect this lane exists to
close, so it is a question now.

**Revision 4** — the verification sketch still asked
for `pos_to_display` / `display_to_pos` round trips "at non-zero
offset", and its first bullet for cell tests "at several offsets".
Both are `view_left` requirements, and `view_left` is Stage 4.
Deferring scope in §3.1 and §9 while test lines quietly assumed it is
exactly how deferred work creeps back in. Replaced with the wrapped
visual-row mapping witness `wrap` actually needs — including the
wrap-point boundary case and a `truncate` control — and with "at
several window widths" (§7).

**Revision 5** — revision 4's replacement then asked
for round-trip identity "for every position", which the existing
coordinate contract makes impossible: `pos_to_display` canonicalizes a
byte inside a multi-byte codepoint to that codepoint's column, and
`display_to_pos` returns the codepoint start. Restated as identity on
valid cursor boundaries plus projection elsewhere, with the
interior-byte canonicalization preserved under its own separate witness
(§7). A witness that cannot be satisfied gets weakened until it passes,
which would have cost exactly the discriminating power §7 exists for.

**Revision 6** — the wrap-point case still said the
last position on row *k* and the first on row *k+1* "must not
collide". They are **one** source position with two candidate
coordinates, so that assertion has no content. §7 now *decides* the
ownership — the wrap position is column 0 of the next row, because the
alternative coordinate is off-grid on a row that is full by
construction — states affinity as a deliberate non-goal, and separately
names the requirement revision 5 was actually reaching for: the two
*distinct* adjacent codepoint starts across the break must map
distinctly.

**Revision 7** — revision 6's justification was wrong
twice. A hard line ending at exactly `max_cols` also has column
`max_cols` — `pos_to_display` clamps nothing — so "off-grid" never
distinguished the soft-wrap case; and `pos_to_display` takes no
viewport at all, so it has no grid to be off. The decision stands, on a
rule that subsumes both cases: **a position maps to the cell of the
glyph that follows it when one exists, otherwise just past the last
glyph** — which resolves the soft wrap and leaves hard ends, including
full-row ones, exactly as they are today (now a control in §7).

Chasing that also surfaced a structural cost no earlier revision had:
under `wrap`, `pos_to_display` **cannot compute a visual row from its
current arguments**, so the wrap width has to reach it — a trait
signature change across ~35 call sites, though only `TextView`
overrides the method.

**Revision 8** — that cost was still framed too
narrowly: a signature is not a model. `display_to_pos` has the same
missing inputs and treats `coord.row` as a raw source-line index, so it
fails *silently* into the wrong line; and `view_top`, vertical motion,
paging, wheel scroll, gutters and overlays all operate in source-line
space today. `move_down` alone treats a display row as a source line.
**New Q#LL6 (§5b)** makes the
authoritative logical-to-visual row map a design item — both
directions, its width/mode inputs, how it *composes with* the existing
fold map rather than bypassing it, what becomes of `view_top` (a
persisted value, via `saveplace`), and `truncate` as the identity case.
§5b.7 restates the cost as an audit of both mapping APIs and every
source-row assumption.

**Revision 9** — revision 8's `view_top` question was
a **false binary**: "source line" and "visual row" are both
unworkable. A source line cannot name a viewport starting partway down
a wrapped line, so a line taller than the viewport could never scroll
to its second visual row — the exact buffers this lane is for. The
representation must be composite (anchor line + row-within-line),
composed with folds.

The persistence consequence is also sharper than "a format change".
`saveplace` has **no version marker** and stores a bare integer, so
redefining `view_top` silently reinterprets every existing record; and
its path field is the whitespace-split remainder, so appending a field
is not backward-compatible either. A visual row is additionally
**width-dependent** — saved at 120 columns, restored at 80, it denotes
a different place. §5b.6 is new, recommends persisting only the
width-independent anchor line (no migration needed, by construction),
and requires Q#LL6 to settle a **resize-restore policy** for the
row-within-line offset, which live resizes need regardless of what is
stored.

Also: revisions 4 through 8 each ended up labelled "the current one".
Only the Status line above is authoritative; the stale markers are
removed.

**Revision 10** — revision 9 described
`VisibleLineMap` as mapping source lines to a renumbered *visible-line*
space. It does not: `next_visible`, `prev_visible`, `visible_head_of`
and `clamp_view_top` all take and return **source-line indices**,
constrained to visible heads (`src/fold_view.rs:223`). Folds project
onto visible anchors; they do not renumber. The error mattered in the one place
this section exists to protect — a reader who believed folds renumber
would add a *second* renumbering for wrap and misindex every fold
consumer. §5b.2, §5b.3 and §5b.4 are corrected, and the same loose
wording is fixed in §2.1, §5 and §7, where it had also mislabelled
`pos_to_display`'s current return (a **source** line index, from
`line_at_offset`).

§5b.6 additionally turns the anchor-persistence recommendation into an
explicit **public API contract**: `pmacs.editor.view_top()` keeps
returning the source anchor and `set_view_top(n)` sets it with
`row_within_line = 0`, so `saveplace` needs no change and existing
records keep working by contract rather than by luck.

**Revision 11** — revision 10 replaced "renumbers"
with "restricts the source-line domain to visible heads". The second
half is also wrong, and in a way that inverts a contract:
`clamp_view_top` **deliberately accepts a hidden line** and projects it
to its visible head — that is why it exists (`src/fold_view.rs:215`),
and `text_view::render` depends on it. "Restricted domain" would make
the supported case read as a caller error. §5b.3 now describes the
first step as a **source-index-preserving projection onto a visible
source-line anchor**: total, idempotent, same index space — which is
the same shape as §7's coordinate rule one level up. The load-bearing
conclusion is unchanged: no dense middle coordinate space exists.

**Revision 12** — revision 11 stated that shared rule
as "projection to the **nearest** canonical value". It is not
proximity-based: a hidden line maps to its fold head even when the next
visible line is closer, and an interior UTF-8 byte maps to its
codepoint start rather than the nearer boundary. Nor is it uniformly
backward — `pos_to_display` projects an interior byte back to the
codepoint start while `display_to_pos` rounds forward to the next
boundary. The accurate rule is **identity on canonical inputs;
otherwise projection to the contract's designated canonical
representative**, with total-and-idempotent as the genuinely shared
algebra (§5b.3).

**Revision 13** — records decisions from the
2026-08-06 design discussion rather than correcting an error.

**Q#LL1 is answered** (§3): `wrap` + `truncate`, **default `wrap`**,
scroll deferred to Stage 4. The default is a knowing behavior change to
the TUI — no default can preserve both frontends, because they
currently disagree.

**Q#LL6 item 3 is answered** (§5b.4): `view_top`'s sub-line component
is a **byte**, not a row index — width-independent, exactly reversible
across resizes, and it **dissolves** the resize-restore policy §5b.6
demanded rather than answering it. Two further structural decisions are
recorded in the new §5b.5: **no global map is needed** (every vertical
consumer is local, so layout is per-line — which removes the
`open_100mb_under_200ms` risk §5b.7's framing invited), and
**`DisplayCoord` gains a `sub_row` rather than redefining `row`**, so
untouched consumers stay correct instead of merely findable.

Found while answering: **the GPU already carries this composite
anchor** — `scroll_top` plus `code_scroll_residual`, renormalized by
`normalize_code_scroll` (framing Q#F6) when reflow pushes the residual
across source lines. The shape is precedent, not invention.

**Revision 14** — answers the remaining four questions
and moves the document to APPROVED.

**Q#LL2** (§4): buffer-local, with `Viewport` carrying the *resolved*
mode as it already carries `folds`, so `TextView` stays
config-agnostic. **Q#LL4** (§6): do not adopt `editing.fill-column` —
it is orphaned because its consumer (`M-q` / auto-fill) does not exist,
which is a different defect from `full_grid`'s and §1.1 should not be
read as equating them; sharpen its description and name ours
`ui.line-wrap`. **Q#LL5** (§5a): character wrap in **both** frontends,
accepting that GUI users lose word wrap — the analysis changed on
discovering that a whitespace-based grid wrap would give only
*approximate* parity against cosmic-text's UAX #14 line breaking, which
is worse than honest divergence. **Q#LL6 items 1-2** (§5b.4):
`TextView` methods, no cache initially, one `Copy` context parameter —
breaking on the input side so the compiler enumerates the audit,
additive on the output side so untouched consumers stay correct.

**Revision 15** — review of `bd752f2` found two holes
and one notation hazard.

**Q#LL7 (§5c) — the GPU had no wire.** §4 resolves the mode into
`Viewport`, which reaches the *grid*. The GPU is not a grid consumer:
it lays out locally, `BufferSnapshot` carries only CRDT bytes, and no
message expresses a wrap mode. So `truncate` would have changed the TUI
and left the GPU wrapping — the exact disagreement this lane closes.
Specified as an additive variant at v22 (advertised baseline unmoved),
carrying `buffer_id` because the mode is buffer-local, resent on
attach, on config change, **and on buffer switch** — the third being
the one a `FontFacts`-shaped design misses, since font size is global
while wrap mode is per buffer.

**Q#LL8 (§5d) — "every vertical consumer is local" was false.** The
scroll indicator needs a total: a one-line buffer wrapping to fifty
rows has `total_lines == 1`, so `format_scroll_indicator` returns
`All` while forty-nine rows sit off-screen. §5b.5 is narrowed
accordingly, keeping the distinction that bounds the cost — a *total*
is one lazily-computed number, an *index* is `O(N)` resident. `All`,
`Top` and `Bot` need no aggregate at all; only `NN%` does.

**Notation (§7).** Revision 14 said `pos_to_display` returns "the
visual row", which reads as redefining `row` — the thing §5b.5
forbids. Every wrap-point example is now the explicit triple
`{row, sub_row, col}`, and both coordinates at a soft break share the
same `row`, which is the information a redefinition would have
destroyed.

**Revision 16 — the current one.** Review of `1c9ff6a` found two more,
both in Q#LL8, and both the same shape: a fix that looked complete
because it was correct in one of two places.

**The GPU has its own indicator** (§5d.3). `format_scroll_indicator` is
**duplicated, not shared** — `src/editor.rs:5509` and
`pmacs-gpu/src/main.rs:10114` — and the GPU passes
`current_line_starts.len()`, a source-line count. So revision 15 would
have fixed the indicator in the TUI and left the GPU reporting `All`
for a one-line wrapped buffer: **this lane's own defect, reproduced by
the section meant to close it.** Both copies keep their signature; what
changes is what the callers pass, so every existing formatter test
stays valid.

**The lazy total's cache key omitted fold state** (§5d.2). Folds are
per rendered window and can change with no edit, no resize and no mode
change, so all three of revision 15's key components stay put while the
projection moves. Corrected to **(buffer generation, content width,
mode, fold projection)** — keyed on the projection's own `components`,
which is `O(folds)` to compare and **cannot be forgotten**, rather than
a maintained revision counter that can. Same principle as byte-anchoring
and additive `sub_row`: self-validating over maintained. And *content*
width, not window width, because the gutter changes at the line-count
digit boundary.

Drafted while GitHub Actions was in a major outage and #220 could not
merge. Nothing here depends on #220 landing; the two lanes touch no
common code.

The user's report, from daily-driver use:

> long lines need to either wrap somehow or be scrollable. Haven't
> tried this in GUI, but in TUI, a line that extends off screen cannot
> be read in full in any way. This should also be something that the
> user can configure, whether to wrap or scrollable.

**The report is accurate where it makes a claim, and it is careful to
limit that claim to the TUI.** In the TUI a line wider than the window
is unreadable past the edge by any means. The GPU is not in that state:
it already wraps, so the text is readable there --- see §1.2, which
also records that revision 1 of this document asserted "in either
frontend" and was wrong to.

**The cross-frontend defect is not unreadability. It is that neither
behavior was chosen.** The TUI truncates because a cell walk breaks at
`max_cols`; the GPU wraps because a library default was never
overridden. Two accidents that disagree, and no way for the user to
express a preference in either --- which is the part of the report that
applies to both frontends: *"should also be something that the user can
configure."*

---

## 1. What is already built

Almost nothing, and that is the honest headline. Stage 1 and Stage 2
both drove machinery that already existed --- Stage 1 honored a flag
with a documented contract, Stage 2 drove a font preference with a
whole wire message behind it. **Stage 3 has no such seam.** It builds a
capability the codebase does not have.

What exists and helps:

- **`text_view::render` is the single place a source line becomes
  cells** (`src/text_view.rs:211`). One walk, one truncation site ---
  **for the grid path only.** See §1.2: that is one of two renderers,
  not the renderer.
- **The fold precedent.** Arc 6 already broke the row-to-source-line
  identity: `VisibleLineMap` + `Viewport.folds` let row `r` show the
  `r`-th *visible* line. The framing for it recorded why folding is not
  an overlay --- *"overlays repaint cells, they cannot delete rows"*
  (`src/view.rs`). Wrapping is the exact dual: it **adds** rows for one
  source line. The same argument forbids it being an overlay, for the
  same reason.
- **The config registry already supports buffer-local overrides.**
  `Registry::get(name, Option<BufferId>)` consults a per-buffer layer
  before global (`src/config_registry.rs:843`), with an explicit note
  that there is no ambient current buffer --- a caller wanting
  buffer-aware behavior must pass the `BufferId`. So "wrap in prose,
  scroll in logs" needs no new machinery.

What does **not** exist:

- **No horizontal offset anywhere.** `Viewport` (`src/view.rs:131`) has
  `buffer_start`, `buffer_end`, `cell_origin`, `cell_size`, `gutter_w`,
  `folds` --- and no column offset. `Window` has `view_top`
  (`src/desktop.rs:92`) and no `view_left`.
- **The truncation is one line of code with no alternative path.**
  `src/text_view.rs:251`:
  ```rust
  if col >= max_cols {
      break;
  }
  ```
  The walk always starts at the line's first character. There is no
  mode, no flag, and no caller that can ask for anything else.

### 1.1 An orphaned setting, of the exact shape Stage 1 just fixed

`editing.fill-column` is defined in the registry --- *"Preferred wrap
column."*, `ConfigKind::Number`, min 1, max 1000
(`src/config_registry.rs:1191`) --- and is **read by nothing**. It
appears only in its own definition and in tests of the registry and the
Lua binding.

This is the same shape as the `full_grid` defect Stage 1 closed: a
declared contract with full definition-side coverage and zero
consumers. **Stage 3 must either give it a consumer or explicitly say
why it does not deserve one.** Leaving it orphaned a second time, in
the very lane about line width, would be the worse outcome.

My reading: `fill-column` is a *fill* concept (where `M-q` reflows
text, editing the buffer), not a *display wrap* concept (where a long
line is shown across rows, buffer unchanged). Conflating them is a
known Emacs papercut. That argues for a separate display setting and a
note here --- but it is Q#LL4 below, not my call.

### 1.2 The two frontends already disagree, and that is the defect

**Revision 1 of this document got this wrong and the error inverted the
lane's cost.** It claimed both frontends consume the same `CellGrid`,
so one change in `text_view::render` would reach both for free. That is
false, and `pmacs-gpu` says so in its own words
(`pmacs-gpu/src/main.rs:4502`):

> The grid variants (`CellDelta`, `Cursor`, `CursorByte`) are
> **ignored** --- pmacs-gpu **lays out locally** and tracks the cursor
> via `PresenceUpdate`.

The `terminal.rs:772` comment revision 1 cited --- "one run per row,
never wrapped together" --- is the **vterm** path, where a run must
occupy exactly the cells the child gave it. It says nothing about
document text.

What the GPU actually does with a long line: **it wraps it.** Every
explicit `set_wrap` in `pmacs-gpu` is `Wrap::None` and every one is
chrome --- status, status-left, menu, minibuffer, completion --- or the
terminal run path (`:3930`, `:3940`, `:3956`, `:3966`, `:3978`,
`:5313`, `:5618`, `:8215-8221`). **The document buffer never sets a
wrap mode at all**, so it keeps the one `cosmic-text`'s `Buffer`
constructor installs --- `Wrap::WordOrGlyph` (`buffer.rs:262` in
0.18.2) --- which is what `sync_buffer_dimensions`'s comment assumes:
*"so cosmic-text wraps at the final clip"* (`:4338`).

It is tested, if incidentally: `wrapped_caret_survives_size_changes`
(`:15853`) puts a 180-character line in a 320px window with the caret
at byte 180 and asserts the caret paints inside the code clip. A
truncating renderer would have that byte off-screen.

**So the real state of the product is:**

| | long line | horizontal scroll |
|---|---|---|
| TUI (grid) | **truncated, unreadable** | none |
| GPU (local layout) | **wrapped** --- readable | none |

This reframes the lane. The user wrote *"Haven't tried this in GUI, but
in TUI, a line that extends off screen cannot be read in full"* --- and
that instinct was exactly right. The GUI half most likely already
works. What is broken there is different: **the wrap is implicit,
unconfigurable, and was never a decision** --- it is cosmic-text's
default leaking through as product behavior.

**Stage 3 is therefore not "build wrap and scroll." It is "make the two
frontends agree on a mode the user chose."** That is a §16 Semantic
Frontend Architecture concern, and it is a larger lane than revision 1
implied: the work lands in `text_view::render` *and* in the GPU's local
layout, with a shared setting deciding both.


---

## 2. The central question: wrap and scroll are not one mechanism

The user's phrasing --- "either wrap somehow or be scrollable ...
whether to wrap or scrollable" --- reads as one setting with two
values. **Internally they are not two settings on one mechanism; they
are two different mechanisms**, and the framing has to say so before
anything is built.

- **Horizontal scroll** is a *viewport offset*. Row `r` still shows
  exactly one source line. The walk starts at display column
  `view_left` instead of 0. The row-to-line relation is untouched.
- **Wrap** is a *row-multiplying line map*. One source line occupies
  ceil(width / cols) rows. The row-to-line relation **breaks**, in the
  same way folding broke it --- and in the opposite direction.

They cost very different amounts. Scroll is close to the cheap change
it looks like. Wrap is not.

### 2.1 What wrap breaks that scroll does not

`TextView::pos_to_display` (`src/text_view.rs:156`) returns
`DisplayCoord::new(row_idx, col)` where `row_idx` is the **source**
line index (`line_at_offset(pos)`) and `col` is the display width of
the line's prefix. Under wrap
neither half survives: one source line has many rows, and `col` is
width-modulo-cols rather than the prefix width.

That function is not a detail. Per `src/view.rs`, **cursor placement
and scrolling use the base text view's `pos_to_display` only.** And
`overlay_paint.rs` maps every overlay's display row through `view_top`
plus the fold map (`src/overlay_paint.rs:178`, `:318`). So a new row
mapping that does not go through the same place puts **every overlay
--- diagnostics, highlights, inlay hints --- on the wrong row** for any
buffer containing a wrapped line.

This is the real cost of wrap, and it is why I am not proposing to
build both at once.

---

## 3. Q#LL1 --- scope: one mechanism or two? **ANSWERED**

> **Answered 2026-08-06.** Stage 3 ships `wrap` and `truncate`, with
> **`wrap` as the default**; horizontal scroll is Stage 4. The default
> was decided knowingly: no default preserves both frontends, because
> they currently disagree (§1.2), so this one **changes the TUI's
> current behavior** and leaves the GPU's alone. `wrap` wins because it
> is the readable value and the one the reported defect asks for.
>
> Corroborating, though not the reason: **Emacs also wraps by
> default** --- `truncate-lines` is `nil` --- so the choice matches
> what an Emacs-shaped editor's users expect. See §5a for the part of
> that comparison which does *not* transfer.


**(a) Horizontal scroll only.** Closes the reported defect --- the line
becomes readable --- at the lowest risk. `view_left` on the window, an
offset in the walk, commands to move it, and cursor-follow. Does not
touch `pos_to_display`'s contract beyond a column shift.

**(b) Wrap only.** Matches what many users reach for first, but pays
the whole row-mapping cost immediately and puts every overlay's
correctness in the blast radius.

**(c) Both, in one lane.** Two mechanisms, one review. Against the
project's one-feature-one-branch rule in spirit even if it is one
"feature" in the user's words.

**(d) Scroll now (Stage 3), wrap as Stage 4.** Ships readability
quickly; leaves the mode setting's second value unimplemented for a
while, which is a discoverability wart --- a setting that names a value
it does not honor is its own coherence defect.

**Revision 2 changes this recommendation.** Revision 1 recommended (a)
or (d) --- scroll first, wrap later --- on the belief that one renderer
served both frontends. §1.2 shows that is false, and it undermines the
recommendation: with the GPU **already wrapping**, shipping
scroll-only would leave the TUI scrolling while the GUI wraps. The
frontends would still disagree, and the user would still have no say
--- which is the actual complaint.

**Revised recommendation: (b) wrap first**, as the mode both frontends
can already almost honor, then scroll as Stage 4.

The reasoning inverts cleanly. Wrap is the expensive one in the grid
renderer and **free in the GPU, where it already happens**; picking it
first means Stage 3 ends with both frontends doing the same declared
thing. Scroll is cheap in the grid renderer and **entirely new in the
GPU** --- which has `scroll_top` but no horizontal counterpart
anywhere (a search for `scroll_left|hscroll|x_offset` in
`pmacs-gpu` returns nothing).

I still do not recommend (c). But note the cost profile is now the
mirror image of what revision 1 claimed, so please read that
recommendation as withdrawn rather than merely amended.

### 3.1 The Stage 3 surface, stated

Wrap-first leaves an obvious hole: a setting with one legal value is
not a setting, and the user asked for a choice. Revision 2 left this
implicit and let a *test* mode stand in for a *product* mode, which is
how §7's witness ended up describing "scroll/truncate" as if they were
one thing. They are not, and scroll is deferred.

**Decision: Stage 3 ships two values, `wrap` (default) and
`truncate`.** Scroll is Stage 4.

- **`wrap`** --- the default, because it is the only value that leaves
  all text reachable with Stage 3's machinery alone. It is also what
  the GPU does today, so the default is not a behavior change there.
- **`truncate`** --- one source line per row, clipped at the edge. This
  is exactly what the TUI does today, named and made deliberate, and
  made available in the GPU where it currently is not.

**`truncate` is not a placeholder and not a test-only mode, but it is
incomplete until Stage 4.** The honest description: truncate is the
*mode*, horizontal scroll is the *navigation* that makes the clipped
remainder reachable. "Scrollable" in the user's request decomposes into
exactly those two, and Stage 3 ships the first. Until Stage 4 lands,
selecting `truncate` means accepting that text past the edge cannot be
read --- which is why it must not be the default, and why its
description string has to say so rather than implying a complete
feature.

The alternative --- ship `wrap` alone with no setting and defer the
whole config surface to Stage 4 --- is defensible and cheaper, and I
rejected it for one reason: it would leave the TUI's *current*
truncation reachable only by not-yet-existing configuration, so the
TUI's existing behavior would become unavailable the moment wrap
landed. Users reading logs want one line per row. Removing that,
even temporarily, is a regression dressed as a fix.

**If you would rather ship wrap-only and take that regression as
acceptable for one stage, say so and I will cut `truncate` --- but the
framing should not pretend the choice is free either way.**

---

## 4. Q#LL2 --- per-buffer or per-window? **ANSWERED**

> **Answered 2026-08-06: buffer-local.** Free in the existing registry,
> and it matches Emacs, where `truncate-lines` is a buffer-local
> variable shared by every window on the buffer. A window-local layer
> would be a **third** config layer built for a need nobody has
> reported.
>
> **How the mode reaches the renderer**, which is the part that needed
> deciding: `Viewport` carries the **resolved** mode, exactly as it
> already carries `folds`. The render driver in `editor.rs` holds both
> the registry and the `BufferId`, resolves once per window per frame,
> and `TextView` stays config-agnostic --- respecting the registry's
> "no ambient current buffer" rule (`src/config_registry.rs:837`)
> rather than working around it.
>
> This does put the **mode** per-buffer and the **byte anchor**
> per-window. That split is correct rather than merely tolerable: the
> mode is a property of the content, the anchor is a property of the
> viewport looking at it.


The registry supports **buffer-local** overrides today, for free.

But line display is arguably a **window** property: the same buffer in
a split could reasonably wrap in one pane and truncate in the other.

**Scoped to Stage 3, this question is only about the mode**, and
buffer-local answers it for free. The follow-on --- that `view_left`
would be unambiguously per-window, since two panes on one buffer must
scroll independently exactly as they already hold independent
`view_top`s (`src/desktop.rs:92`) --- **is Stage 4's, not this
lane's.**

Recording it here anyway, because the choice made now constrains it: if
Stage 3 makes the mode buffer-local and Stage 4 then needs a
per-window offset, the two halves of one user-facing concept end up
living at different scopes. That is what Emacs effectively does and it
is survivable, but it should be a decision rather than a discovery.
**Q#LL2 asks whether to accept that split now.**

---

## 5. Q#LL3 --- what does the cursor do?

**Under `wrap`, this is a Stage 3 question and it is not optional.**
`pos_to_display` returns `(source line index, prefix width)`, and
§2.1 shows both halves stop being true once one source line owns
several rows. Cursor placement uses that function exclusively, so a
wrapped buffer with an unrepaired mapping puts the caret on the wrong
row --- in the grid renderer, which is the half that has to be built.
Whatever answers it must also serve `overlay_paint`, or diagnostics
land on the wrong row too.

**Under `truncate`, the cursor question is trivial** --- one row per
line, the existing mapping holds --- **but only until Stage 4.** Moving
the cursor past the right edge in a truncating view is precisely what
horizontal scroll exists to handle, and Stage 3 has no answer for it:
the caret goes off-screen. That is a real, if minor, sharp edge of
shipping `truncate` without scroll, and §3.1's description string
should own it.

Deferred to Stage 4, recorded here so it is not rediscovered: whether
explicit horizontal scroll drags the cursor with it (as the wheel does
vertically) or leaves it for the next motion to snap back. The existing
"auto-scroll to keep cursor visible" pass that `scroll_window`
deliberately works around (`src/editor.rs:3624-3628`) is where a
horizontal analog would live, and its comment already records the
hazard --- an unconditional snap-back makes explicit scrolling feel
stuck.

---

## 5a. Q#LL5 --- agreeing on the mode is not agreeing on the wrap **ANSWERED**

> **Answered 2026-08-06: character wrap in BOTH frontends.** The GPU
> document buffer gets an explicit `Wrap::Glyph` --- its first explicit
> `set_wrap` --- and the grid walk wraps at the character.
>
> **The option analysis changed while deciding, and the change is the
> reason.** "Teach the grid word wrap" looked like the high-effort,
> high-parity option. It is not: cosmic-text performs **Unicode line
> breaking (UAX #14)**, so a grid walk breaking on whitespace would
> diverge on hyphens, CJK and non-breaking spaces. That buys
> *approximate* parity, which is worse than honest divergence because
> it looks unified until it is not. True parity that way requires a
> UAX #14 dependency.
>
> Character wrap in both is the only option that is **true parity,
> cheap, and Emacs-consistent** (Emacs's default wrap is a character
> wrap; word wrap is opt-in `visual-line-mode` / `word-wrap`). It also
> completes the lane's thesis: one declared mode, one declared wrap
> style, two frontends that agree.
>
> **Word wrap becomes a declared third value later** ---
> `ui.line-wrap = "word"` honored by both frontends --- rather than an
> inherited library default that only one frontend has. `ConfigKind::Enum`
> makes adding a choice a clean additive change.
>
> **The regression is real and must be stated where users see it, not
> only here.** GUI users have had word wrap since the GPU frontend
> existed and never opted into losing it. This belongs in the PR
> description and the release notes.


Raised by writing §9 and noticing I had put it in "not in scope" as if
it were a detail. It is not.

The lane's thesis is that two frontends should stop disagreeing. But
choosing `wrap` in both only makes them agree on *whether* to wrap, not
*how*.

The GPU's value is exact and worth naming: `cosmic-text` 0.18.2
constructs every `Buffer` with **`Wrap::WordOrGlyph`**
(`buffer.rs:262`) --- word wrap, falling back to glyph wrap for a word
that cannot fit a line by itself. It is not a trait `Default`; the
constructor sets it, which is why the GPU document gets it without ever
asking.

The natural grid implementation --- keep walking the cell row and
continue on the next --- is a plain **character** wrap. Ship both and
the same buffer at the same width breaks lines in different places in
the two frontends.

Worth noting for whoever writes the tests: the existing
`wrapped_caret_survives_size_changes` uses `"x".repeat(180)`, a line
with **no word boundary at all**, so it exercises only
`WordOrGlyph`'s glyph fallback. It would pass identically under
`Wrap::Glyph`, and therefore cannot detect the divergence this question
is about. A prose line is needed to see it.

That is a smaller defect than today's truncate-vs-wrap split, and it
may be an acceptable one. But it is the same *kind* of defect this lane
exists to close, so it should be decided rather than inherited ---
which is precisely the mistake §1.2 documents the GPU already making
once.

Options: match `WordOrGlyph` in the grid walk (most work, genuine
parity); accept character wrap in the grid and document the divergence;
or set `Wrap::Glyph` on the GPU document buffer so both are character
wraps (cheapest parity --- one line, and it would be the document
buffer's *first* explicit `set_wrap` --- but a visible downgrade for
GUI users who have had word wrap all along without anyone deciding they
should).

No recommendation yet --- I would rather know Q#LL1's answer first,
since this question only exists if `wrap` ships.

---

## 5b. Q#LL6 --- the visual-row map is the actual design problem

Revision 7 recorded that `pos_to_display` cannot compute a visual row
from its arguments, and framed that as a signature change. **Review was
right that this understates it: a signature is not a model.**

### 5b.1 The inverse has the same hole, and it is worse

`display_to_pos` takes `(&self, buf, coord)` (`src/view.rs:278`) and
opens by treating the row as a **source line index**
(`src/text_view.rs:188`):

```rust
let row = coord.row as usize;
if row >= self.line_count() { return None; }
```

Without width and mode it cannot invert a wrapped visual row at all ---
and unlike the forward direction, it will not fail loudly. It will
return a position from the wrong source line.

### 5b.2 The vertical stack is in source-line space, end to end

- **`Window::view_top` is documented as "First buffer line shown at the
  top of this window's viewport"** (`src/window.rs:374`). Not a visual
  row.
- **Rendering converts it as a source line:**
  `window.text_view.line_offset(window.view_top)` (`src/editor.rs:4334`).
- **`move_down` conflates display rows with source lines**
  (`src/editor_core.rs:2145`). It reads `coord.row` from
  `pos_to_display` --- a **display** row --- then treats it as a source
  line: bounding `next_row >= aw.text_view.line_count()` against the
  source line count and passing it through `map.next_visible()`. This
  is correct today only because display row and source line coincide.
  Under `wrap` they diverge, and a one-source-line buffer wrapping to
  two visual rows would refuse to move to the second row.

  **Revision 9 called `next_visible`'s argument a "visible-line index",
  as though folds introduced a third renumbered space. They do not**
  --- see §5b.3. That mistake matters here specifically: if a reader
  believed folds renumber, the natural fix to `move_down` would be to
  renumber again for wrap, which is precisely the fold-composition
  error §5b.3 exists to prevent.

The same source-row assumption runs through `goal_col`
(`src/window.rs:376`, "sticky display column for vertical motion"),
`visible_rows` and therefore `cursor.page-down` (`src/window.rs:377`),
`scroll_window` (`src/editor.rs:3629`), the gutter's line numbers, and
`overlay_paint`'s row arithmetic (`:178`, `:318`).

### 5b.3 It must compose with folds, not replace them

This is the constraint that makes it a design item rather than a
utility --- and getting the existing model right is the first half of
it, because **revision 9 got it wrong in the direction that would cause
the very bug this section prevents.**

Revision 9 said `VisibleLineMap` "mediates source line to **visible**
line" and drew:

> ~~source line --(folds)--> visible line --(wrap)--> visual row~~

**There is no renumbered visible-line space.** `next_visible(line)`
computes `line + 1` and then jumps past a collapsed component,
returning a **source-line index** (`src/fold_view.rs:223`); so do
`prev_visible`, `visible_head_of` and `clamp_view_top`.

Folds do not renumber lines. Nor --- a second-order correction, from
review of revision 10 --- do they **restrict the domain**, which is how
this paragraph first put it. `clamp_view_top` *deliberately accepts a
hidden line* and projects it to its visible head; that is the reason it
exists (`src/fold_view.rs:215`), and `text_view::render` relies on it,
noting that "a caller that hands us a hidden start still gets its
head". Calling the domain restricted would imply passing a hidden line
is a caller error. It is the supported case.

**The first step is a source-index-preserving projection onto a
visible source-line anchor.** Total --- every source line is a legal
input --- idempotent, and staying inside the same index space
throughout. (`visible_rows_between` does return a dense count, but that
is a *distance*, not a coordinate, and nothing indexes with it.)

That is the same shape as the coordinate rule in §7, one level up:
**identity on canonical inputs; otherwise projection to the contract's
designated canonical representative.**

"Designated", not "nearest" --- the distinction is the whole content of
the rule. A hidden line maps to its **fold head** even when the next
visible line is closer (`visible_head_of`, not "whichever visible line
is fewest lines away"), and an interior UTF-8 byte maps to its
**codepoint start**, which is not necessarily the nearer boundary. Each
contract names its representative; proximity never selects it.

The direction is not shared either, which is why the rule has to be
stated in terms of designation rather than of going backward.
`pos_to_display` projects an interior byte **back** to its codepoint
start, while `display_to_pos` rounds a column landing inside a wide
character **forward** to the next codepoint boundary
(`display_to_pos_jumps_over_wide_chars`,
`display_to_pos_inside_tab_rounds_to_next_codepoint`). Two directions,
one rule --- each names its own representative.

What the two levels genuinely share is the algebra: both are **total**
(every input is legal) and **idempotent** (projecting twice equals
projecting once). That is the property the wrap map must also hold, in
both directions.

The accurate model, and the one the wrap map must be built against:

> source line *(projected by folds onto a visible source-line anchor)*
> --(wrap)--> visual row

So there are exactly **two** coordinate spaces after this lane, not
three: source lines, and visual rows. Wrap is the only renumbering
step, and it consumes a source line the fold map has already projected
onto a visible anchor.

Why the distinction is load-bearing rather than pedantic: a reader who
believes folds renumber will reach for a second renumbering to layer
wrap on top, ending with a source-to-visible-to-visual chain in which
the middle space has no definition and every fold consumer is
subtly misindexed. A wrap map that instead goes straight from source
line to visual row *without consulting the fold map* bypasses folding
and silently breaks it; one that replaces `VisibleLineMap`
re-implements a merged and reviewed arc. **The map takes a
fold-vouched source line and returns a visual row**, and both
directions have to survive that composition.

### 5b.4 What Q#LL6 asks --- **ANSWERED**

> **Answered 2026-08-06**, in discussion. Item 3 (byte-anchored
> `view_top`) is above; two structural decisions are in §5b.5 --- **no
> global map**, and **`DisplayCoord` gains a sub-row rather than
> redefining `row`**. The resize-restore policy §5b.6 demanded is
> **dissolved** rather than answered: a byte anchor makes it
> unnecessary.
>
> **Item 1 --- where it lives: `TextView`, no new type, no cache.** It
> already owns the line offsets and the character walk. Two methods
> taking width and mode. No cache initially: a viewport is ~50 lines,
> so that is at most ~50 single-line layouts per frame, the same order
> as rendering, which already walks every visible line. A per-window
> `(line, width)` cache is a profiling response, not a design premise.
>
> **Item 2 --- signatures.** One `Copy` context value rather than two
> loose parameters:
>
> ```text
> pos_to_display(buf, pos, ctx) -> Option<DisplayCoord>   // ctx: { width, mode }
> display_to_pos(buf, coord, ctx) -> Option<Position>
> ```
>
> The asymmetry with the `DisplayCoord` decision (§5b.5) is
> deliberate, and it is the whole audit strategy:
>
> - **The input change is breaking on purpose.** A required parameter
>   makes the compiler enumerate every call site, so the §5b.7 audit is
>   mechanical rather than a grep.
> - **The output change is additive on purpose.** `sub_row` defaults to
>   0, so a consumer that does not know about wrap stays *correct*, not
>   merely *findable*.
>
> Compiler-enforced where enforcement is possible; correct-by-default
> where it is not.


1. **Where the map lives and who owns it.** Width is a *window*
   property, so a per-window map is the obvious home --- which then
   interacts with Q#LL2's buffer-local mode. A per-window map keyed by
   a buffer-local mode is coherent but should be stated.
2. **Both directions, explicitly.** Fold-vouched source line to first
   visual row, and visual row back to (source line, row-within-line). §7's
   witnesses test the second; nothing currently tests the first
   because it does not exist.
3. **What `view_top` becomes --- and revision 8 posed this as a false
   binary.** It offered "stays a source line" or "becomes a visual
   row". **Neither works.**

   A source line **cannot represent a viewport that begins partway
   down a wrapped line.** A line taller than the viewport must be
   scrollable from its visual row 0 to its visual row 1, and
   "convert the source line" always yields row 0 --- so the second
   half of a tall line would be unreachable by scrolling. That is not
   an edge case; it is the exact situation this lane exists for, since
   the motivating buffers are the ones with very long lines.

   A bare visual row fails differently --- see §5b.6.

   **The representation has to be composite: an anchor line plus a
   sub-line offset**, composed with folds (§5b.3). This is load-bearing
   for cursor visibility, wheel scrolling and paging, not a storage
   detail.

   **ANSWERED 2026-08-06: the sub-line component is a BYTE, not a row
   index.** `view_top` is the byte offset of the first visible
   character (equivalently, anchor line plus byte-within-line).

   A row index is width-dependent and lossy: narrowing then widening
   then narrowing again does not return the viewport where it started,
   and every width change needs a clamp-or-reset policy. **A byte is
   width-independent**, so resize needs no policy at all --- recompute
   which row that byte falls on at the new width, exactly and
   reversibly. It is the same property that made anchor-line
   persistence safe in §5b.6.

   It also composes with the established algebra rather than adding a
   rule: an arbitrary byte is not necessarily a row start, so it
   projects onto the row start containing it --- total, idempotent,
   designated representative, exactly as folds and `pos_to_display` do
   (§5b.3).

   And it satisfies the pinned API contract by construction:
   `view_top()` returns the line containing that byte;
   `set_view_top(n)` sets the byte to line `n`'s start, which *is*
   sub-row zero.

   **Precedent, found while answering this:** the GPU already carries a
   composite anchor --- `scroll_top` (a source line index into
   `current_line_starts`) plus `code_scroll_residual` (a sub-line
   offset) --- and `normalize_code_scroll` (`pmacs-gpu/src/main.rs:7955`,
   framing Q#F6) already handles reflow pushing that residual across
   source lines, by **renormalizing rather than clamping**. So the
   composite shape is not novel here. The grid can do better than the
   GPU on the sub-component only because it owns its own layout: the
   GPU's residual is in pixels because cosmic-text owns layout there,
   which is why it needs a renormalization loop that byte-anchoring
   does not.
4. **Whether `truncate` is the identity case.** It should be: under
   `truncate` the map is the identity and every current behavior holds
   unchanged, which is what makes the whole change additive and
   testable against today's suite.

### 5b.5 Two structural decisions taken with Q#LL6 --- **ANSWERED**

Both from the 2026-08-06 discussion, both load-bearing for cost.

**1. There is no global logical-to-visual map, and none is needed.**

A materialized "visual row of every line" prefix sum would be `O(N)`
memory and an `O(N)` rebuild on every width change --- against an M1
gate that includes `open_100mb_under_200ms`. That is a real perf risk
and §5b.7's "authoritative map" framing invited it.

No **index** is required, because every *positioning* consumer is
local: rendering walks forward from `view_top` bounded by viewport
height; `move_down`/`move_up` need one step; paging needs
viewport-height rows; the wheel needs *n* rows from `view_top`. Nothing
asks for the absolute visual row of line 40,000, and nothing **indexes**
by one.

**Revision 14 overstated this as "every vertical consumer is local",
and that is false.** Review of `bd752f2` found the counterexample: the
scroll indicator needs a **total**. See §5d --- and note the
distinction that survives, because it is what keeps the cost bounded: a
*total* is one number, computable lazily and cacheable; a *prefix-sum
index* is `O(N)` resident storage. Stage 3 needs the former and still
does not need the latter.

So "the map" is two per-line functions --- how many rows this line
occupies at this width, and which row a given byte falls on --- plus
incremental walks. **Layout is needed one line at a time.**

**2. `DisplayCoord` gains a sub-row; `row` keeps its meaning.**

`DisplayCoord { row, col }` (`src/view.rs:110`) is core-internal, 53
references, 39 inside `text_view.rs`'s own tests --- so roughly eight
real external uses. Either approach is tractable in size; they are not
equivalent in risk.

**Redefining `row` from source line to absolute visual row would
silently break every existing consumer** --- `overlay_paint`'s
`disp.row - view_top` (`:189`, `:321`), `move_down`'s bounds check
(`src/editor_core.rs:2145`) --- with no compile error, which is the
exact failure class this framing has been catching all along.

Adding a `sub_row` makes it **additive**: `sub_row == 0` under
`truncate` and for every unwrapped line, so existing consumers stay
correct by default and wrap-aware ones opt in explicitly. That is what
bounds the §5b.7 audit: the compiler cannot find these call sites for
us, so the design has to make the untouched ones *right* rather than
merely *findable*.

### 5b.6 `saveplace` persistence, and why a bare visual row is unsafe

`view_top` is written to disk. `saveplace` stores one
`<cursor> <view_top> <path>` line per file and parses it with
`^(%d+)%s+(%d+)%s+(.+)$` (`builtin/runtime/saveplace.lua:5`, `:37`),
restoring via `pmacs.editor.set_view_top` (`:76`). Two properties make
this sharper than "a format change":

- **There is no version marker.** Nothing distinguishes a record
  written before this lane from one written after. Redefine what the
  second integer means and **every existing record is silently
  reinterpreted** --- an old source line 500 becomes visual row 500,
  which in any wrapped buffer is a different place entirely. No error,
  no migration prompt, just a wrong viewport.
- **The path is the whitespace-split remainder.** So simply appending a
  fourth field is not backward-compatible either: an older pmacs
  reading a newer file parses the new sub-index as the head of the
  path and loses the entry.

There is a second, independent problem: **a visual row is
width-dependent.** Saved at 120 columns and restored at 80, the same
number denotes a different source location --- and windows legitimately
change width between sessions, which is precisely what QoL Stage 1 was
about.

**My recommendation, offered as the cheapest correct option rather than
a decision:** persist only the **anchor line**, never the
row-within-line offset. Then the stored value keeps its current
meaning, every existing record stays valid **by construction**, no
migration or version marker is needed, and the persisted number is
width-independent again. The cost is bounded and small: reopening a
file restores to the top of the anchor line rather than partway down
it --- at most one line's height of drift, and only for files closed
mid-wrapped-line.

**That recommendation is only real if it is an API contract, so state
it as one.** `saveplace` does not touch a field; it calls public Lua
(`builtin/runtime/saveplace.lua:60`, `:76`), and those bindings are
documented today as source lines --- *"view_top(): the active window's
first visible source line"* and *"set_view_top(line): set the first
visible source line"* (`src/lua_bindings/mod.rs:13714`). So the
contract Stage 3 must preserve is:

- **`pmacs.editor.view_top()` continues to return the source anchor
  line**, not a visual row, whatever the internal representation
  becomes.
- **`pmacs.editor.set_view_top(n)` sets that anchor with
  `row_within_line = 0`.**

With both held, `saveplace` needs **no change at all** and existing
records keep working --- the compatibility comes from the API contract,
not from `saveplace` being careful. Any future call that needs the
sub-row is a **new** binding, additive, and not what `saveplace`
writes.

This also decides a question Q#LL6 would otherwise leave open: the
composite `view_top` is an *internal* window representation, and the
Lua surface exposes only its anchor component. Widening the public
getter to return a pair would be the change that breaks records
silently, and it is exactly what "just make `view_top` composite"
invites if the API is not pinned here.

**Q#LL6 must also settle the resize-restore policy**, which the
composite representation does not escape: when the width changes, a
row-within-line offset may exceed the line's row count at the new
width. Clamp to the last row, or reset to 0? This applies to live
resizes as well as restores, so it is needed regardless of what is
persisted.

### 5b.7 The cost, restated honestly

Revision 7 costed this as "~35 `pos_to_display` call sites". **That was
the wrong unit.** The real work is an audit of *both* mapping APIs plus
every place that assumes a display row is a source line --- vertical
motion, paging, wheel scroll, `view_top` handling, gutter numbering,
overlay placement, and the fold interaction above.

Sizing that audit is itself part of Q#LL6, and it is a strong argument
for `wrap` and `truncate` shipping as one lane with `truncate` as the
identity case: it gives every one of those consumers a mode in which
its current behavior is provably unchanged.

---

## 5c. Q#LL7 --- the GPU needs a wire message, and revision 14 had none

**Raised in review of `bd752f2`, and it is a hole in the lane's central
claim.** §4 resolves `ui.line-wrap` into `Viewport`, which reaches the
**grid** renderer. The GPU is not a grid consumer (§1.2): it lays out
locally and ignores `CellDelta`. `BufferSnapshot` carries only CRDT
bytes (`pmacs-protocol/src/message.rs:777`), and no `InstanceMessage`
variant expresses a wrap mode.

So as framed through revision 14, `ui.line-wrap = "truncate"` would
change the TUI and **leave the GPU wrapping** --- the two frontends
still disagreeing, which is the exact defect this lane exists to close.
Q#LL5's "character wrap in both" is likewise unreachable without a
wire: setting `Wrap::Glyph` at GPU startup is not the same as honoring
a mode that can change.

### 5c.1 The message

**Additive variant, appended after the current final `InstanceMessage`
variant; `PROTOCOL_VERSION` 21 -> 22; `ADVERTISED_PROTOCOL_VERSION`
stays 20.** This is the path `FontFacts` took at v17 and the panel
shapes took at v21, and the constant's own doc reserves moving the
advertised baseline for changes "that cannot be expressed additively"
--- this one can.

It carries `buffer_id` alongside the mode. **Not optional: the mode is
buffer-local (§4)**, so "the current mode" is meaningless without
naming the buffer it belongs to, and the GPU tracks
`current_buffer_id` already.

### 5c.2 Resend semantics --- the part most likely to be got wrong

The mode must reach the GPU on **all three** of:

1. **Attach**, for the initially-shown buffer, as part of the same
   initial-state burst that establishes font facts. A frontend that
   attaches to an existing session must not have to wait for a change
   to learn the current mode.
2. **Config change**, via the registry's `on_change` --- for every
   attached frontend showing that buffer.
3. **Buffer switch.** This is the one a `FontFacts`-shaped design
   misses. Font size is global; **wrap mode is per buffer**, so
   switching from a buffer set to `truncate` to one left at `wrap`
   changes the effective mode with **no config event at all**. A
   design that only listens to `on_change` is silently wrong here, and
   would look correct in every single-buffer test.

### 5c.3 GPU behavior on receipt

Set `Wrap::Glyph` (mode `wrap`) or `Wrap::None` (mode `truncate`) on
the **document** buffer --- its first explicit `set_wrap` either way
(§1.2) --- then reshape and **renormalize the scroll anchor** through
`normalize_code_scroll` (`pmacs-gpu/src/main.rs:7955`). That path
already exists for exactly this situation: reflow moving the retained
residual across source lines. Changing wrap mode reflows the whole
document, so it is the same event class as a font-size change, and must
reuse that repair rather than reimplement it.

An out-of-range or unknown mode value is **rejected as a whole
message**, matching `apply_font_facts` rather than clamping --- the
convention Stage 2 followed (`docs/gui-zoom-framing.md`).

### 5c.4 Older frontends

A v21-or-older frontend never receives the variant and keeps wrapping.
That is a **documented divergence**, not a silent one: the guarantee
"both frontends agree" holds for peers that negotiated v22, and the
release notes must say so alongside the word-wrap regression (§5a).

---

## 5d. Q#LL8 --- the scroll indicator, which falsifies "everything is local"

**Raised in review of `bd752f2`.** `format_scroll_indicator`
(`src/editor.rs:5509`) reckons `All`/`Top`/`Bot`/`NN%` from
`total_lines`, fed in visible-line space (`src/editor.rs:4336`, Arc 6
Q#FD18) so a collapsed remainder correctly reads `All`.

Under `wrap` that is wrong in a way a user sees immediately. **A
one-line buffer wrapping to fifty screen rows has `total_lines == 1`,
so the very first branch --- `if total_lines <= 1 { return "All" }` ---
reports `All` while forty-nine rows sit below the viewport.** The
indicator claims the whole buffer is on screen when almost none of it
is.

### 5d.1 The contract

The indicator is reckoned in **visual rows** whenever the mode is
`wrap`, and in visible lines under `truncate` --- where the two
coincide, so `truncate` remains exactly today's behavior, consistent
with §5b.5's identity-case strategy.

- `All` --- every visual row of the buffer is on screen.
- `Top` --- the first visual row is on screen and `All` does not hold.
- `Bot` --- the last visual row is on screen and `All` does not hold.
- `NN%` --- the cursor's **visual row ordinal** as a percent of the
  buffer's total visual rows.

### 5d.2 What must be computed, and what must not

**`All` / `Top` / `Bot` need no aggregate.** Each is a local predicate:
is the first visual row on screen (`view_top` byte == first visible
byte), and is the last one (does the forward walk from `view_top` reach
the buffer end within the viewport)? Both fall out of the render walk
that already happens. **Only `NN%` needs a total**, which matters
because `All`/`Top`/`Bot` are the states a user reads most and the
common cases stay `O(viewport)`.

The total may be computed **lazily and cached** --- but revision 15's
key was wrong, and review of `1c9ff6a` caught it. It said "buffer
generation, width and mode". Two corrections:

**Fold state must be in the key.** `Viewport.folds` is built **per
rendered window** (`src/view.rs:146`) and a fold can be collapsed or
expanded with **no edit, no width change and no mode change** --- so
all three key components are unchanged while the projection underneath
them is not. Compute `NN%`, collapse a fold, and the stale total is
served for the new projection.

**Prefer a content-derived key over a maintained one.**
`VisibleLineMap` is `{ components: Vec<HiddenComponent> }`
(`src/fold_view.rs:104`) with no revision field, and
`fold_map_for_window` rebuilds it per call. Two ways to key on it:

- A revision counter on the fold registry, bumped by every mutation.
  Cheap to compare, and it carries a *did-you-remember-to-bump* hazard
  on every present and future mutation path --- the same failure shape
  as Q#LL7's buffer-switch trigger.
- **The projection's own contents.** `components` holds one entry per
  collapsed region, so hashing or comparing it is `O(folds)`, not
  `O(N)` --- negligible per frame, and it **cannot be forgotten**,
  because the key *is* the thing it guards.

**Take the second**, for the same reason byte-anchoring beat a row
index (§5b.4) and an additive `sub_row` beat redefining `row`
(§5b.5): a key that derives from the state is self-validating, while
one maintained alongside it is a standing invitation to drift.

**And it is the CONTENT width, not the window width.** Wrapping happens
in the text area, so the gutter is already subtracted --- and the
gutter's width changes with the line-count digit boundary (9 -> 10,
99 -> 100), which the GPU's `sync_buffer_dimensions` comment already
records for its own shaping (`pmacs-gpu/src/main.rs:4338`). Keying on
window width would serve a stale total across a digit boundary.

So: **(buffer generation, content width, mode, fold projection)**.

It composes with folds by counting rows only for lines the fold map
vouches as visible (§5b.3).

**It must not become a resident prefix-sum index** --- that is the
`O(N)` storage §5b.5 rules out, and the distinction is exactly one
number versus one number per line.

**The `open_100mb_under_200ms` gate (M1) constrains this.** Computing
total visual rows means laying out every line, so it must not happen on
open, on every frame, or on any path the gate measures --- only on
first `NN%` paint after an invalidation. If that proves too slow on
large buffers, the fallback is to report a **byte-based** percentage
under `wrap` and say so; what is not acceptable is today's silent
`All`.

### 5d.3 The GPU has its own indicator, and revision 15 missed it

**Raised in review of `1c9ff6a`.** §5d as written specified only the
TUI path. `format_scroll_indicator` is **duplicated, not shared** ---
`src/editor.rs:5509` and `pmacs-gpu/src/main.rs:10114`, each with its
own tests --- and the GPU calls its copy with
`self.current_line_starts.len()`, a **source-line** count
(`pmacs-gpu/src/main.rs:7199`).

So a one-line wrapped buffer reports `All` in the GPU too, by an
entirely independent path. **Stage 3 as framed through revision 15
would have fixed the indicator in one frontend and left it wrong in the
other** --- which is this lane's own defect, reproduced by the lane
meant to close it.

The GPU's `visible` argument is wrong under `wrap` for the same reason:
`estimated_visible_lines(...)` counts **lines**, and visible *rows* is
what the indicator needs once one line owns several.

**Both copies keep their signature and semantics.** The formatter is a
pure function over counts and is correct as written; what changes is
**what the callers pass** --- visual rows rather than source lines, and
visible rows rather than visible lines. That keeps every existing
formatter test valid, including the GPU's
`format_scroll_indicator(0, 10, 1, 0) == "All"` (`:13091`), which
correctly pins line-space behavior and must **not** silently change
meaning.

The GPU derives its total from its own layout, not from a wire message:
cosmic-text already knows each line's visual height, so this is a local
query there --- and Q#LL7's message tells it *which mode* to be in, not
how many rows there are.

**The duplication is itself the hazard worth naming.** Two copies means
two call sites must change, and nothing in the type system connects
them. That is the same shape as Q#LL7's three resend triggers: a
correct fix in one place that looks complete.

### 5d.4 Verification

- **The reported case, as a direct witness, IN BOTH FRONTENDS:** one
  source line, viewport shorter than its wrapped height, mode `wrap`
  --- the indicator must **not** be `All`. This fails against revision
  14's design in the TUI and revision 15's in the GPU, which is what
  makes it worth writing first, twice.
- **A "cache, then toggle a fold" witness.** Paint `NN%` so the total
  is cached, collapse (or expand) a fold **without any edit, resize or
  mode change**, and assert the indicator changes. This fails against
  revision 15's cache key, which is the point.
- **A digit-boundary witness**, since content width is in the key: a
  buffer crossing 9 -> 10 or 99 -> 100 lines changes the gutter and
  therefore the wrap width, and the total must not be served stale
  across it.
- `Top` at the buffer start, `Bot` at the end, `All` only when every
  visual row fits --- each with a wrapped line present.
- **A `truncate` control** asserting the indicator is byte-identical to
  today's output for the same buffer and viewport.
- A **folded + wrapped** case, since §5b.3's composition applies here
  too and the Q#FD18 contract must survive.

---

## 6. Q#LL4 --- `editing.fill-column` **ANSWERED**

> **Answered 2026-08-06: do not adopt it** --- but the reason is
> sharper than "a different concept", and §1.1 overstated the finding.
>
> `editing.fill-column` is orphaned because **its consumer does not
> exist yet**: there is no `M-q`, no auto-fill, and no reflow command
> anywhere in this codebase. It is a setting ahead of its feature.
> That is a *different* defect from `full_grid`'s, which was a flag
> with a live consumer that ignored it, and §1.1 should not be read as
> equating them.
>
> Two things Stage 3 does owe it:
>
> - **Sharpen its description.** Once a wrap setting ships,
>   `"Preferred wrap column."` actively invites the wrong conclusion.
>   It must say it governs reflow commands, not display.
> - **Name ours so confusion is impossible: `ui.line-wrap`**,
>   `ConfigKind::Enum { choices: ["wrap", "truncate"] }`, default
>   `"wrap"`. `editing.*` is buffer-editing behavior, `ui.*` is
>   display; both existing `ui.*` settings carry a `gpu-` prefix to
>   mark frontend-specificity, so its **absence** here is what signals
>   "both frontends".


Give it a consumer, or state why display wrap is a separate concept and
leave it orphaned with that reasoning recorded. See §1.1.

---

## 7. Verification sketch

Not final --- it depends on Q#LL1.

- Unit tests on `text_view::render` at the cell level, **at several
  window widths** --- not "at several offsets", which was the same
  `view_left` assumption in the very first bullet. A line longer than
  `max_cols`; a wide character straddling the wrap/clip column; a tab
  expanded across it (the walk's tab path at `src/text_view.rs:254` has
  its own `col >= max_cols` break, so wrap has to be taught there too,
  not only in the main character path).
- **Wrapped visual-row mapping**, which is Stage 3's version of this.
  Revision 3 asked for round trips "at non-zero offset" --- that is a
  `view_left` requirement and `view_left` is Stage 4, so the sketch was
  quietly re-importing deferred scope. What `wrap` actually needs
  witnessed:
  - For a source line occupying N visual rows, `pos_to_display` returns
    `{ row: source_line, sub_row, col }` --- the **same `row` it
    returns today**, plus which visual row *within* that line and the
    column within *that* row, rather than the whole prefix width
    (`src/text_view.rs:184`).

    **Notation matters here and revision 14 got it wrong.** It said
    `pos_to_display` returns "the visual row", which reads as a
    redefinition of `row` --- exactly what §5b.5 forbids. Every example
    below is therefore written as the explicit triple
    `{row, sub_row, col}`; a bare pair anywhere in this section is a
    bug in the document, not a shorthand.
  - `display_to_pos` inverts it: a click on visual row *k* of a wrapped
    line lands in that row's byte range, not the source line's head.
  - Round trip is identity for **every valid cursor boundary** in a
    wrapped line, walked exhaustively rather than sampled --- the line
    is short enough to make that cheap, and sampling is what would miss
    the next case.

    **"Every position" would be an impossible invariant, and revision 4
    asked for it.** `pos_to_display` accepts a byte offset *inside* a
    multi-byte codepoint and deliberately canonicalizes it: continuation
    bytes outside a complete codepoint are trimmed and the codepoint's
    own column is the answer (`src/text_view.rs:163-167`), while
    `display_to_pos` returns the codepoint **start**
    (`src/text_view.rs:185`, and the existing
    `display_to_pos_jumps_over_wide_chars` /
    `display_to_pos_inside_tab_rounds_to_next_codepoint` pin it). An
    interior byte therefore cannot round-trip to itself today, and
    demanding it would have made the witness unsatisfiable rather than
    discriminating --- the test would have been "fixed" by weakening it,
    which is the failure mode this whole sketch is trying to avoid.

    The accurate contract is: **identity on boundaries, projection
    elsewhere.** For an interior byte the round trip must land on the
    containing codepoint's start, and applying it twice must equal
    applying it once.
  - **That canonicalization is pre-existing behavior, and this lane
    preserves it unless it says otherwise.** It gets its own witness,
    separate from the wrap tests, so that "wrap changed the interior-byte
    rule" cannot hide inside a wrap failure — or vice versa. If wrap
    turns out to need a different rule at a wrap point that also splits a
    codepoint, that is a deliberate change with its own Q#, not a quiet
    consequence.
  - **The wrap point itself**, which is the case worth designing the
    test around --- and which revision 5 described in a way that
    collapsed two different requirements into one incoherent sentence.

    Take `abcdef` soft-wrapping after `abc`. Buffer positions are
    `0=a 1=b 2=c 3=d 4=e 5=f`. **Position 3 is a single source position
    with two defensible display coordinates**:
    `{row: L, sub_row: k, col: 3}` --- just past the last glyph of
    visual row *k* of line *L* --- and `{row: L, sub_row: k+1, col: 0}`
    --- just before the first glyph of visual row *k+1* of the **same
    source line**. Note both share `row: L`: the wrap point does not
    cross a source line, which is precisely why redefining `row` would
    have destroyed the information this case turns on. Revision 5 called these "the last
    position on row *k* and the first on row *k+1*" and demanded they
    "not collide". They are the same position. Nothing can be asserted
    about their collision.

    **Decision: the wrap position belongs to column 0 of row *k+1*.**

    Revision 6 justified this by calling the alternative "off-grid",
    and parenthetically claimed a hard line end is "within the row".
    **Both halves are wrong.** A hard line ending at exactly `max_cols`
    gets column `max_cols` --- `pos_to_display` sums the prefix width
    and clamps nothing (`src/text_view.rs:184`) --- so it is *also*
    just past the last cell, and that is existing, accepted behavior.
    Off-gridness therefore does not distinguish the two cases at all.

    Worse, the argument was incoherent on its own terms:
    **`pos_to_display` does not know the grid.** Its signature is
    `(&self, buf, pos)` (`src/view.rs:271`, `src/text_view.rs:156`) ---
    no viewport, no `max_cols`. A function with no notion of the grid
    cannot be reasoned about as producing coordinates "off" it.

    The rule that actually decides it, and subsumes both cases:

    > **A position maps to the cell of the glyph that follows it when
    > one exists on some row; otherwise to the column just past the
    > last glyph.**

    - Soft wrap: position 3 is followed by `d` at
      `{row: L, sub_row: k+1, col: 0}`. A
      following glyph exists, so that is the answer. There is a genuine
      choice here, and this resolves it.
    - Hard line end: no glyph follows on any row, so the coordinate is
      the column just past the last glyph --- `(k, width)`, **including
      `{row: L, sub_row: last, col: max_cols}` when the line fills its
      final visual row exactly.** No choice
      exists, and **this preserves current behavior unchanged**, which
      is the point: the wrap work must not quietly move hard-end
      coordinates.

    So the two cases differ because one has an alternative and the other
    does not --- not because one is off-grid.

    **A consequence the earlier revisions missed entirely:** under
    `wrap`, `pos_to_display` **cannot compute a visual row from its
    current arguments.** The wrap width has to reach it. Revision 7
    treated that as a trait signature change and costed it at ~35 call
    sites; **that framing was too narrow, and §5b (Q#LL6) replaces it**
    --- the inverse has the same hole, and `view_top`, vertical motion,
    paging, wheel scroll, gutters and overlays all currently work in
    source-line space. Read §5b before costing this.

    Consequences to witness, and they are the discriminating ones:
    - `pos_to_display(3)` is `{row: L, sub_row: k+1, col: 0}`, never
      `{row: L, sub_row: k, col: 3}`.
    - **A hard line end that exactly fills the row still maps to
      `{row: L, sub_row: 0, col: max_cols}` with `sub_row` still 0** ---
      a control asserting the wrap work left the
      existing hard-end coordinate alone, since the soft-wrap rule
      superficially resembles a rule that would have moved it.
    - `display_to_pos` on the trailing cells of row *k* --- which exist
      when a wide character forced an early break and left the row's
      last cell blank --- must land on the wrap position, not on the
      last glyph's start. This is the case a naive "clamp to row width"
      gets wrong.
    - **Affinity is explicitly not implemented.** Editors that let
      `End` on row *k* and `Home` on row *k+1* sit visually apart at
      one buffer position carry an upstream/downstream bit to do it.
      Stage 3 carries a single canonical coordinate instead. If that
      distinction is wanted later it is a feature with its own state,
      not a bug in this mapping --- named here so it is a decision
      rather than a discovery.
  - **Distinct adjacent codepoints across the break must map
    distinctly**, which is the requirement revision 5 was reaching for.
    The start of the last codepoint on row *k* (position 2, `c`) and
    the start of the first on row *k+1* (position 3, `d`) are two
    different positions; they must give `{row: L, sub_row: k, col: 2}`
    and `{row: L, sub_row: k+1, col: 0}`, and
    the round trip must return each unchanged.
  - A `truncate` **control** asserting the mapping is unchanged from
    today, so the wrap work cannot silently alter the non-wrapped path.
- If wrap is in scope: an overlay-placement test with a wrapped line
  above the overlay's row, which is the regression §2.1 predicts.
- A PTY acceptance test for the user-visible report: with `wrap`, a
  line longer than the terminal is readable in full --- following
  `full_grid_resync_acceptance.rs`'s content-anchored pattern rather
  than any time-based settle. (Not "scrolled past the edge" --- there
  is no scrolling in Stage 3. Revision 2 wrote it that way and was
  describing Stage 4.)
- **A GPU-side witness that the mode is honored rather than inherited.**
  `wrapped_caret_survives_size_changes` (`pmacs-gpu/src/main.rs:15853`)
  passes today against a wrap nobody configured, so it cannot
  distinguish "honors the setting" from "cosmic-text's default happens
  to match." The discriminating case is the other value: **with the
  mode set to `truncate`, an overlong line must NOT occupy a second
  row.** Without it this lane ships the defect Stage 1 just fixed --- a
  declared setting nothing enforces.

  This test is a **negative control for mode enforcement**, and it is
  worth being exact about what it does *not* stand for: it is not a
  witness for horizontal scroll, and `truncate` is not the user's
  "scrollable." Scroll is Stage 4 (§3.1). A reader who takes this test
  as evidence that the scroll alternative works would be reading it
  backwards --- it proves only that an explicit non-wrap mode reaches
  the GPU's layout.

---

## 8. Coherence impact (§20 requirement)

- **Scorecard row 11, "Config layering + provenance --- Partial
  (foundation only), 5 settings live in it."** This lane adds registry
  settings against that row, and either adopts or explicitly declines
  the orphaned `editing.fill-column`.
- **Journey step 4, "Understand interface --- Partial."** A line that
  cannot be read in full is a direct hit on this step; the scorecard
  does not currently name it, and should.
- **§16 Semantic Frontend Architecture --- this is the lane's primary
  coherence citation, per §1.2.** Two frontends currently render the
  same buffer's long lines differently, and neither behavior was
  chosen: the TUI truncates because the cell walk breaks at `max_cols`,
  the GPU wraps because cosmic-text's default was never overridden.
  Stage 3 replaces two accidents with one declared mode.
- **No new interaction island.** The mode command goes in the ordinary
  command registry and the global keymap. Per Q#Z3's finding in Stage 2,
  `keymap_stack::Scope` carries no frontend identity --- and unlike
  zoom, that is not a constraint here, because the setting is
  frontend-independent by design: both renderers read the same value
  and each honors it in its own layout.
- **No background-work attribution.** Nothing async.

---

## 9. Not in scope

**Horizontal scroll, in full: `view_left` on the window, the commands
that move it, and the cursor-follow pass. That is Stage 4** (§3.1,
§5). Stage 3 ships the `truncate` mode that Stage 4 makes navigable,
and ships it knowing text past the edge is unreachable in the
meantime.

Reflow/fill commands that *edit* the buffer (`M-q`). Bidi or RTL. A
minimap. Soft-wrap indicators in the gutter --- worth doing, but they
are a gutter-arc concern and would need their own Q#.
