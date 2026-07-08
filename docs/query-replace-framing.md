# Query-replace — framing (Arc 2 interleave)

pmacs has incremental search, substring and regex, in both frontends —
and no replace at all. `search.rs` finds matches and never substitutes;
there is no `M-%`. This is the highest-value missing editing table-stake
(you reach for it hourly), and it sits right on top of isearch.

The happy discovery from scouting: **the whole feature is
zero-protocol-change.** It reuses three things already on the wire —
the `StatusFacts.message` band (added v15 for exactly this class of
transient prompt), the `SearchMatch`/`SearchMatchActive` decorations
(store-driven, both frontends), and the `dispatch_idle`-false gate that
already makes semantic frontends round-trip keys during a search. The
entire arc is core + Lua; no v16.

Roadmap context: `docs/roadmap-2026-07.md` Arc 2 (the editing
table-stakes interleave, promised after Arc 1's panels).

## What already exists (verified)

- **Match store** (`src/search.rs`): `SearchStore` is per-buffer,
  keyed `BufferId → SearchState { query, matches: Vec<ByteRange>,
  active }`; `SharedSearchStore = Arc<Mutex<…>>` on the core. `set`
  replaces query+matches, `focus_from(byte)` points `active` at the
  first match `≥ byte`, `step` advances with wrap. Matchers are free
  functions over `&[u8]`: `find_all` (smart-case substring,
  non-overlapping) and `find_all_regex` (`None` iff the pattern fails
  to compile). **No replace API — greenfield.**
- **Highlights are store-driven, not session-driven**
  (`semantic_render.rs`): the producer emits `SearchMatchActive` for
  the match equal to `active_match()`, `SearchMatch` for the rest,
  reading only the store. So writing a match into `search_store` and
  pointing `active` at it renders in both frontends with **zero new
  rendering code**.
- **The isearch shadow is the template** (`src/editor.rs`):
  `dispatch_key` routes to `dispatch_search_key` while
  `search_active()`; `SearchKey::from_chord` maps a fixed key
  vocabulary; an active search eats every key. `dispatch_idle()`
  returns false while `search_active()`, so the GPU round-trips keys
  instead of optimistically self-inserting (test
  `isearch_flips_dispatch_idle_so_gpu_round_trips`).
- **The transient prompt band** (`StatusFacts.message`, v15): setting
  `core.status` shows an echo-area string in both frontends (TUI
  bottom row, GPU band). `dispatch_key` clears `core.status` at entry,
  so a handler that re-sets it at the end owns the band cleanly.
- **Region replace** is `EditOp::Replace { range, bytes }` via
  `apply_active_edit` — one undo step (the `insert_char_over_region`
  precedent). Every edit marks the search store stale; nothing
  auto-recomputes matches (Q#QR2 owns this).

## Decisions

### Q#QR1 — A distinct `QueryReplaceSession` + a 5th dispatcher shadow

Not an overload of the isearch session — the lifecycles differ (isearch
is one string, cancel-restores-origin; query-replace is from+to plus an
interactive y/n phase). Mirror the structure instead:
`QueryReplaceSession` on `EditorCore`, `query_replace_active()`, and a
`QueryReplaceKey::from_chord` + `dispatch_query_replace_key` in
`editor.rs` — the fifth member of the shadow family (minibuffer,
search, menu, completion, **query-replace**). Add
`query_replace_active()` to the `dispatch_idle()` disjunction and the
modal-close guard, exactly as the others.

**`buffer.after-edit` must fire from inside the shadow (P1).** A modal
shadow `return`s before `dispatch_key`'s normal post-command edit
check, so `apply_active_edit` from `dispatch_query_replace_key` would
not notify LSP `didChange` / syntax reparse / anything on the edit
chain — the replaced text would silently keep stale styling and
diagnostics. Mirror `dispatch_completion_key` (`editor.rs:797`
precedent): snapshot `active_buffer_revision()` before handling the
key, and `run_hook("buffer.after-edit", …)` if it changed. `!`
(replace-all) applies many edits in one keypress — fire the hook
**once** after the batch (revision compared across the whole handler),
not per replacement, so the debounced `didChange` coalesces naturally.

### Q#QR2 — Search-forward-after-each-replace (Emacs's algorithm), not precompute-all

The load-bearing correctness decision. Do **not** precompute the whole
match list and walk it — replacing `a`→`aa` (or `foo`→`foobar`) would
re-match the replacement text and loop, and precomputed offsets go
stale after the first edit. Instead, hold a `next_from` byte cursor;
each step finds the *next* match at/after `next_from` in the **current**
buffer:

- **replace** (`y`/`SPC`): `Replace` the match with the to-bytes; set
  `next_from = match.start + to.len()` (past the replacement, so it's
  never re-matched); advance.
- **skip** (`n`/`DEL`): set `next_from = match.end`; advance.
- **advance**: find the next match from `next_from`; none ⇒ finish.

Offset shift is handled by construction (every search is on the live
buffer from a byte past the last edit), and replacements are never
re-matched. Add `find_first_from(haystack, query, start)` (literal +
regex) to `search.rs` so each step is one bounded forward scan, not an
`O(buffer)` `find_all` filtered — keeps `!` (replace-all) linear.

**Cursor reveal (P2).** Highlighting a match that's off-screen is
useless — like isearch's `search_place_cursor` after each step, move
point so the frontend scrolls it into view: **advance** sets the cursor
to the current match's `start`; **replace** sets it to the *end of the
inserted replacement* (`match.start + to.len()`), which is also
`next_from`; **natural finish** (ran out of matches) leaves point
there. Quit semantics are Q#QR10.

**Regex specifics (P2).** Compile the pattern **once** at session start
and store the `regex::bytes::Regex` in the session; the regex
`find_first_from` scans from `next_from` using that cached engine
(`Regex::find_at`) — recompiling per step would make `!` quadratic and
defeat the "linear" claim. **Invalid pattern at start**: if the regex
fails to compile, don't begin the session — set a status
(`"Invalid regex: …"`) and return, the same clean refusal isearch's
`invalid` flag gives (there's no mid-session recompile since the
pattern is fixed once entered). **Zero-width matches**: the regex
first-match path filters them exactly as `find_all_regex` does (a
zero-width match would never advance `next_from` and would loop) — skip
forward past a zero-width hit.

### Q#QR3 — Two entry strings via chained `minibuffer.read`

The Lua command collects the from-string, then the to-string in its
`on_accept`, then calls `ed.query_replace_start(from, to, regex)` which
begins the core session. Both prompts ride `minibuffer.read`
(dual-frontend since v12); the minibuffer is closed by the time the
second `on_accept` starts the session, so the handoff into the
query-replace shadow is clean. Emacs's "Query replace: X  Query replace
X with: Y" flow, faithfully.

**Separate history buckets (P3):** `history = "query-replace-from"` and
`history = "query-replace-to"` — one shared bucket would mix search
patterns and replacement text in both dropdowns.

**Empty-string rules (P3):** an empty *from* string is rejected (the
from-prompt's `on_accept` returns early with a status, like other
minibuffer flows) — there's nothing to search for. An empty *to* string
is **valid** and means deletion (replace each match with nothing); the
to-prompt must not copy the reject-empty pattern.

### Q#QR4 — Per-match prompt via `core.status` (StatusFacts.message)

No new wire message. Each prompt sets
`core.status = "Query replacing FROM with TO (SPC/y, n, !, ., q)"` —
shown in both frontends via the v15 band. This is exactly the v15
rider's purpose (transient echo-area content), and it's the Emacs
behavior (query-replace prompts live in the echo area = the status
line). A running count (`… — 3 replaced`) can ride the same string.

### Q#QR5 — Current-match highlight reuses `SearchMatchActive`

Write just the current match into `search_store`
(`set(buffer, from, [current])`, active = 0); the producer renders it
as `SearchMatchActive` (amber) in both frontends. Clear the store on
finish (isearch's cancel discipline). Store contention with a lingering
isearch is moot: shadows are modal and mutually exclusive, and the
first write overwrites whatever isearch left. v1 highlights only the
current match (Emacs's default prompt highlight); lazy-highlighting all
remaining matches is deferred.

### Q#QR6 — Key vocabulary (v1)

`y` / `SPC` replace-and-advance; `n` / `DEL` skip-and-advance; `!`
replace this and all remaining without prompting; `.` replace this then
quit; `q` / `RET` / `Esc` / `C-g` quit. Unrecognized keys are eaten
(the isearch precedent). Deferred: `,` (replace-but-stay), `^` (back
up), `?` (help). Finish/quit semantics are Q#QR10.

### Q#QR7 — Undo granularity

Each replacement is one `EditOp::Replace` = one undo step, so an
N-match query-replace is N undo steps. Simple and correct; a single
undo-group for the whole run is deferred (it's the same
`begin/end_undo_group` mechanism the CUA type-over framing Q#U1 parked
— induct it when a second caller wants it).

### Q#QR8 — Scope

Forward, from point to buffer end (Emacs's default). Matches before the
cursor are not touched. Cursor placement is Q#QR2 (per-step) and Q#QR10
(on finish). Whole-buffer and backward query-replace are deferred.

### Q#QR9 — Regex replacement is literal in v1

`C-M-%` (`query-replace-regexp`) matches via `find_all_regex`/the regex
`find_first_from`, but the replacement string is inserted literally —
no `\1` capture-group references. Capture-group substitution is
deferred (it needs the regex engine's capture API threaded through the
replace step).

### Q#QR10 — Finish / quit semantics (NOT isearch's)

The load-bearing difference from isearch: query-replace has usually
**already mutated the buffer** by the time it ends, so "cancel" cannot
mean "restore." Precisely:

- **Quit** (`q`/`RET`/`Esc`/`C-g`, or `.` after its replace, or running
  out of matches): does **not** roll back any replacement already made;
  clears the highlight (`search_store.clear`); leaves point at the
  current/last-inspected match (Q#QR2's cursor rule already put it
  there); sets a status count (`"Replaced N occurrence(s)"`). `C-g`
  behaves the same as `q` — Emacs's query-replace `C-g` exits and keeps
  the replacements; it is *not* an undo.
- **Nothing matched** (the session never found a first match): a
  distinct case — restore the origin cursor (isearch's cancel
  discipline, since nothing was touched) and status `"No matches for
  'FROM'"`. This is the only path that restores point.

So the session records `origin` (for the nothing-matched restore only)
and whether any replacement happened; every other exit leaves point at
the inspected match.

## Phasing

One implementation pass (the feature is small and cohesive), validated
in both frontends:

1. **Core + Lua + literal & regex.** `QueryReplaceSession` + methods +
   the shadow (with the Q#QR1 after-edit hook) + `find_first_from`
   (literal & regex, cached engine) + the status prompt + the
   highlight-and-reveal; Lua `query-replace` / `query-replace-regexp`
   commands (chained `minibuffer.read`, Q#QR3 empty-string + bucket
   rules) bound `M-%` / `C-M-%`. Acceptance tests through `dispatch_key`
   (hermetic, like `completion_popup_acceptance`): replace/skip/`!`/`.`,
   the three quit paths + nothing-matched-restores-origin (Q#QR10),
   empty-to deletion, offset-shift correctness (`a`→`aa` doesn't loop),
   regex incl. invalid-at-start and zero-width, buffer.after-edit fires
   (an LSP/syntax observer sees the replaced text), and the
   `dispatch_idle`-false gate. **Explicit binding tests for both
   `M-%` and `C-M-%`** — control-meta-shifted punctuation is exactly
   the chord that can parse differently across the TUI and GPU key
   paths (the `C-c H` lesson from Arc 1b), so assert `from_chord`
   resolves each and that it fires through `dispatch_key`. GPU
   validation scores bets #1/#3.

If the interactive-phase key handling or the offset bookkeeping proves
fiddlier than expected, phase 2 splits out regex + `!`/`.`; but the
plan is a single PR.

## Categorical bets (score at close)

1. **Zero protocol change holds.** Status band + `SearchMatchActive` +
   `dispatch_idle` gate carry the whole feature to the GPU with no v16
   and no GPU code. (The panels arc's bet-#3 lesson makes me watch the
   GPU path specifically — "the mechanism exists" bit us there.)
2. **Search-forward-after-replace is correct where precompute-all
   loops.** `a`→`aa`, `foo`→`foobar`, and a replacement that would form
   a new downstream match all terminate correctly because each search
   starts past the replacement on the live buffer.
3. **The 5th shadow drops in cleanly** — `dispatch_idle`, modal-close
   guard, and GPU round-trip all generalize like the completion popup
   did (Arc 1a).
4. **A store-contention or status-clear edge** — some interaction where
   a lingering isearch highlight, or `dispatch_key`'s entry
   status-clear, briefly shows the wrong band/highlight during the
   interactive phase.

## Deferred (named, not silently dropped)

- Capture-group references (`\1`) in regex replacements (Q#QR9).
- `,` (replace-but-stay), `^` (back up a match), `?` (help) keys.
- Backward and whole-buffer query-replace (Q#QR8).
- Single undo-group for a whole run (Q#QR7; shares CUA Q#U1's
  `begin/end_undo_group`).
- Smart default from-string (word/region at point) + last-used default.
- Lazy-highlight of all remaining matches during the prompt (v1 shows
  only the current match).
- `replace-string` / `replace-regexp` (non-interactive replace-all) —
  trivial once the replace core exists; a thin non-prompting entry.
