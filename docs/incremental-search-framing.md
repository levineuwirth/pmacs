# Incremental in-buffer search — framing pass

Date: 2026-06-15. The last deferred GUI item: `SearchMatch` /
`SearchMatchActive` decorations exist on the wire (message.rs) and are
gated to `None` in both frontends "waiting on a search feature." This
builds that feature. Decided up front (user): **incremental isearch**
(highlight live as you type, same key steps to next, Enter accepts,
Esc/C-g restores origin) with **smart-case substring** matching
(case-insensitive unless the query has an uppercase letter).

## Survey facts (anchors)

- Greenfield in-buffer search; only `project.search` (cross-file grep)
  exists. No `Buffer::find` / rope search.
- `DiagnosticStore` (diag.rs) is a near-exact template: keyed
  `Arc<Mutex>` store, sorted entries, `next_after`/`previous_before`,
  stale tracking, Lua nav bindings, overlay `_attach_view`.
- Producer `scoped_decorations` (semantic_render.rs) + TUI
  `DiagnosticView::render` are the emit/paint templates (viewport clip,
  line-start cache, stale-skip).
- GPU `decoration_kind_to_bg_color` already draws bg decorations
  through the quad pipeline; SearchMatch/Active just need their arms.
- The minibuffer (minibuffer.rs) is keystroke-driven and pseudo-modal
  (dispatch_minibuffer_key intercepts all keys while active) but has
  **no live-preview/on_changed hook** — the one missing piece for
  incremental highlight.

## Q#SR1 — store shape & ownership

**Stance: a per-buffer `SearchStore` mirroring `DiagnosticStore`.**
`by_buffer: HashMap<BufferId, SearchState>` where `SearchState` holds
the resolved query, the sorted `Vec<ByteRange>` matches, and the
active index. Shared `Arc<Mutex>`. The active index lives on the store
(navigation state), not per-window — v1 accepts that two windows on
the same buffer share the active highlight (note it; selection is the
per-window concept, search mirrors diagnostics). Edits mark the
buffer's entry stale (M11.8 model) so matches at pre-edit byte
positions aren't painted until re-search.

## Q#SR2 — search primitive

**Stance: smart-case substring over a rope snapshot, regex deferred.**
`find_all(haystack, query) -> Vec<ByteRange>`: case-insensitive unless
`query` contains an uppercase char (then exact). Built on
`snapshot_rope().slice` bytes (the diagnostics path's cheap snapshot).
Recomputed on query change; invalidated on edit. No `regex` crate in
v1 (literal-text is the 95% case; regex is a later toggle). Overlapping
matches: advance past each match's start+1 (standard non-overlapping).

## Q#SR3 — decoration emission

**Stance: mirror the diagnostics producer.** In `scoped_decorations`,
read the search store for the viewport buffer, emit `SearchMatch` for
every visible match and `SearchMatchActive` for the active one
(emitted last / higher z so it wins the overlap). Reuse the line cache
+ `clip_to_viewport`. Stale-skip exactly like diagnostics. TUI gets a
`SearchView` overlay (mirrors `DiagnosticView`) painting bg, attached
via `pmacs.search._attach_view`.

## Q#SR4 — colors

**Stance: a single search palette.** SearchMatch = translucent yellow
wash; SearchMatchActive = stronger amber/orange. GPU: the two
`decoration_kind_to_bg_color` arms. TUI: reverse-ish colored bg in the
`SearchView`. Distinct from selection (blue) and diagnostics
(severity).

## Q#SR5 — input & modality (incremental)

**Stance: host the query in the minibuffer + a small `on_changed`
hook + targeted next/prev interception.** Entry opens a minibuffer
search session (prompt `I-search: `); `on_changed` (new optional
session callback, fired after each content mutation in
dispatch_minibuffer_key) recomputes matches → updates the store →
re-decorate. While that session is active, the entry chord again =
`search.next`, its shift/`C-r` variant = `search.prev` (control keys,
not self-insert, so safe to intercept in the search branch of
dispatch_minibuffer_key). `Enter` accepts (close, leave cursor at the
active match); `Esc`/`C-g` cancels (close, restore the origin cursor
saved at entry, clear the store). Reusing the minibuffer's input
editing + prompt avoids reimplementing a modal query line.

## Q#SR6 — navigation

**Stance: `search.next`/`search.prev` mirror `diag.next/prev`.**
Advance the active index with wrap, move the active window's cursor to
the active match start, scroll it into view. The active index drives
which match is `SearchMatchActive`. Usable both during the live
session and afterward (matches persist until cleared / next search).

## Binding (proposal, flagged for veto)

`C-f` → search (CUA "Find"), rebound from `cursor.right`. Consistent
with the editor's CUA direction (arrows move; Ctrl+F finds); the
Emacs-holdover `C-f = forward-char` is the inconsistent one. Easy to
change — call out in validation.

## Predicted findings (categorical bets)

1. **Stale-after-edit linger** (the squiggle lesson again): matches at
   pre-edit byte positions paint over shifted text until re-search —
   the store's stale gate + re-search-on-change must be right, or
   highlights drift during typing.
2. **Minibuffer `on_changed` × completion**: the hook interacts with
   the existing per-keystroke candidate recompute; the search session
   must opt out of completion cleanly (a session "kind" seam).
3. **Per-buffer active match across windows** surfaces as navigating
   in one window moving the active highlight in another — accepted for
   v1, but worth eyeballing.
4. **Empty / all-match queries**: empty query → no matches (not all);
   a 1-char common letter → many matches → viewport-clipped emission
   must stay cheap (line cache + only-visible).

## Session plan

Three green commits:
1. Core `SearchStore` + `find_all` smart-case primitive + unit tests.
2. Producer emission + GPU bg colors + TUI `SearchView` + attach.
3. Incremental UX: minibuffer `on_changed`, search session,
   next/prev interception + commands, cancel-restores-origin, binding.

Manual validation gate as usual (type to highlight live, step matches,
edit mid-search, Esc restores).
