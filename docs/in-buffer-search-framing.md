# In-buffer search — consolidated framing + as-built

Consolidates the two framing passes for in-buffer search:
**incremental substring isearch** (PR #70) and **regex search** (this
arc). Supersedes the separate `incremental-search-framing.md` and
`regex-search-framing.md`. Where the implementation diverged from a
framing stance, the "As-built" notes record what actually shipped and
why.

User-decided up front:

- **Incremental isearch** — highlight live as you type, the same key
  steps to the next match, `RET` accepts, `Esc`/`C-g` restores origin.
- **Smart-case** matching (case-insensitive unless the query has an
  uppercase letter), for both substring and regex.
- **Regex** with **both** dedicated entry keys (`C-M-s` / `C-M-r`)
  **and** a mid-search toggle (`M-r`), matching **multi-line**.

## Architecture (as-built)

- **`search::SearchStore`** — per-buffer `HashMap<BufferId,
  SearchState>` (query + sorted `Vec<ByteRange>` matches + active
  index), shared `Arc<Mutex>`, mirroring `diag::DiagnosticStore`. The
  active index is navigation state on the store (two windows on one
  buffer share the active highlight — the diagnostics tradeoff). Edits
  mark the entry stale (M11.8) so matches at pre-edit byte positions
  never paint until a re-search.
- **`EditorCore::SearchSession { query, origin, forward, regex,
  invalid }`** — the live input state. `search_begin/input_char/
  backspace/step/finish/toggle_regex/recompute` drive it. `recompute`
  runs the matcher over an `O(1)` rope snapshot, writes the store,
  refocuses from the origin cursor, moves the cursor to the active
  match.
- **Shared dispatch.** Keys are intercepted in `EditorState::
  dispatch_key` → `dispatch_search_key` (`SearchKey::from_chord`).
  This is the *same* path the daemon runs for round-tripped GUI
  keystrokes, so isearch behaves identically in both frontends; only
  the prompt *surface* differs.

## Matching

- **`find_all(haystack, query)`** — smart-case ASCII substring,
  non-overlapping. Case-insensitive unless `query` has an uppercase
  char.
- **`find_all_regex(haystack, pattern) -> Option<Vec<ByteRange>>`** —
  `regex::bytes::Regex` over the whole buffer. `Some` for a valid
  pattern (possibly empty), `None` when it won't compile, so the caller
  distinguishes *invalid* (show `[invalid]`) from *zero matches*.
  Smart-case via a `(?i)` prefix unless the pattern carries an
  uppercase letter. Multi-line is free — the regex runs over the whole
  byte slice, so an explicit `\n` (or `(?s).`) spans lines while `.`
  stays line-bound. Zero-width matches (`a*`, `^`, `$`) are filtered.
  The `regex` crate's linear-time engine makes a pathological pattern
  slow at worst, never catastrophic. An uppercase letter inside an
  escape/class (`\D`, `[A-Z]`) trips case-sensitivity — accepted, the
  same coarse rule as the literal path.

## Input & bindings

- `C-s` / `C-r` — start a literal isearch forward / backward; once
  running, the same keys step next / previous (intercepted in Rust, no
  binding). `RET` accepts (keeps cursor + highlights until the next
  edit); `C-g` / `Esc` cancels (restores the origin cursor, clears the
  store); `BS` shortens the query.
- `C-M-s` / `C-M-r` — start a **regex** isearch (`search.forward-regex`
  / `search.backward-regex` → `ed.search_start(forward, regex)`).
- `M-r` — toggle literal ↔ regex mid-search (a `SearchKey` decoded in
  `dispatch_search_key`, so it works the same in both frontends).
- Both keys were free in the default map (save is `C-x C-s`, redo is
  `C-x r`), so isearch landed without disturbing the CUA / Emacs
  editing keys.

## Frontend surfaces

- **TUI** — a `SearchView` overlay attached to the active window on
  `search_begin` (deduped; self-suppresses with no matches / when
  stale) washes matches; a bottom-row prompt reads `[Regex] I-search:
  <query> (n/m)` (or `[no match]` / `[invalid]`). The terminal cursor
  stays in the buffer at the active match. Multi-line matches wash each
  spanned row (mirrors `paint_local_selection`'s per-row clip, newline
  excluded).
- **GUI (pmacs-gpu)** — matches wash via `SearchMatch` /
  `SearchMatchActive` decorations through `push_glyph_extent_rects`,
  which already fans a byte range across visual lines, so **multi-line
  needed no GUI rendering change**. The query reaches the band via the
  `SearchPrompt` wire message. Key routing reuses the M11.6
  `DispatchIdle` gate: `daemon_intercepts_keys` (a live `SearchPrompt`
  or `!dispatch_idle`) round-trips every key into the daemon's search
  while it runs, and `is_search_entry_chord` (`C-s`/`C-r`/`C-M-s`/
  `C-M-r`) forwards the entry chords that are otherwise withheld.
  Escape cancels an active search instead of quitting the window.

## Wire (`InstanceMessage::SearchPrompt`)

`{ buffer_id, query: Option<String>, active: Option<u32>, total: u32,
regex: bool, invalid: bool }`. Emitted by the semantic producer
(cached-compare suppressed like `StatusFacts`); `query: None` clears
the band. Protocol **v9** added the message (query/active/total);
**v10** added `regex` / `invalid` (an encoding change to the variant),
so the daemon's per-session filter gates it at `>= 10` — a v9 peer is
sent no `SearchPrompt` (decorations still highlight) rather than
mis-decoding the wider shape. `SUPPORTED = [6, 7, 8, 9, 10]`.

## As-built divergences from the framing passes

1. **Entry binding: `C-f` → `C-s` / `C-r`.** The incremental framing
   penciled `C-f` (CUA "Find", rebinding `cursor.right`) "for veto."
   `C-s` / `C-r` shipped instead: both were unbound, Emacs-faithful,
   and need no `cursor.right` rebind. User-validated.
2. **Input model: minibuffer-hosted → dedicated core search mode.**
   The framing (Q#SR5) proposed hosting the query in the minibuffer
   with a new `on_changed` hook. Shipped as a frontend-agnostic
   `SearchSession` on `EditorCore` driven by `dispatch_search_key`,
   because **pmacs-gpu has no minibuffer** — a shared core mode was the
   only way to make search work identically in both frontends.
3. **Regex: deferred → shipped.** Q#SR2 deferred regex ("literal-text
   is the 95% case"); this arc added it as `find_all_regex` + a mode
   flag, keeping the literal path the default.
4. **Multi-line: substring single-line → regex multi-line.** Substring
   matches never span lines; regex can. The TUI `SearchView` (which
   assumed single-line) gained per-row washing; the GUI was already
   multi-line-capable.

## Bets that held (validation gate)

- Stale-after-edit linger — closed by `apply_active_edit` marking the
  store stale.
- Invalid-regex incremental states — `foo(` shows `[invalid]`, never
  panics, recovers on completion.
- Multi-line TUI wash — per-row clip at line boundaries, no phantom
  trailing cell.
- GUI key routing — `dispatch_idle` flips during search; the optimistic
  path round-trips instead of editing the buffer.
