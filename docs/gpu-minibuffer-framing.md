# GPU minibuffer — framing + as-built

pmacs-gpu couldn't render the minibuffer, so `M-x`, find-file,
switch-buffer, and the LSP rename prompt were invisible in the GUI —
both prior arcs (search, context menu) had to route *around* this gap.
This arc closed it. The build landed close to the framing (the third
surface in the `SearchPrompt` / `MenuPrompt` family); the "As-built"
notes record where it differed.

Decided (with the user):

- **Render-only.** The minibuffer logic already lived entirely in the
  core; the GPU *already round-tripped keys* while one was open
  (`dispatch_idle` goes false). So this was a wire-message + GPU
  render-surface arc — no new input logic.
- **Vertical dropdown** for candidates (Vertico / Telescope style), not
  the TUI's inline `[selected]`. The GPU leads the TUI here; the
  cross-frontend divergence is accepted (discussed against how Emacs and
  Neovim converged on a vertical list).

## What the core already exposed

A single **global** `EditorCore::minibuffer` (not per-buffer). While a
prompt is open, `MinibufferSession` carries everything a renderer needs:
`prompt` (e.g. `"M-x "`), the typed `input` (`minibuffer.contents()`),
the `cursor` (byte offset), `candidates: Vec<String>` (already
filtered + fuzzy-sorted best-first, plain strings), and
`selected: Option<usize>`. Input is fully handled by
`MinibufferAction::from_chord` (RET accept, TAB complete, `C-g`/Esc
cancel, Up/Down history, `M-n`/`M-p` cycle candidates, motion,
self-insert). These fields are all public, so the producer reads them
directly — **the core was untouched**, exactly the family bet.

## Architecture

### Q#MB1 — A `MinibufferPrompt` semantic message, mirroring the family

`InstanceMessage::MinibufferPrompt` (protocol v12), produced by
`semantic_render::minibuffer_prompt_msg` with the same cached-compare
suppression as `search_prompt_msg` / `menu_prompt_msg`, daemon-gated
`>= 12`. The GPU mirrors it into a `MinibufferLocal` and renders. The
minibuffer is global, so the message is **bufferless** — the producer
caches a single value (not a per-buffer `HashMap`) and emits only from
the active-buffer viewport so the bufferless message ships once per
frame.

### Q#MB2 — The prompt line lives in the bottom band

When the minibuffer is open, `compose_status_left` returns exactly
`prompt + input`, taking over the band ahead of the search prompt and
status. The buffer caret is hidden; a **band caret** quad draws at the
input cursor. **As-built:** the caret x uses the band font's monospace
advance — the shaped status-left width ÷ its char count, times
`prompt_chars + cursor` — rather than a per-glyph measurement (Q#MB4).
Exact for the ASCII command names / filenames that dominate; a
multibyte-exact caret is deferred.

### Q#MB3 — The candidate dropdown floats above the band

A vertical popup anchored just **above** the band at the input's left
edge, **growing upward, best match at the top**, the `selected` row
highlighted. **As-built:** it's a *third* `TextRenderer`
(`mb_text_renderer`) over bg/selection quads — the menu's popup pattern,
reusing the menu's colors — not "a second" renderer (the menu already
owns one). It only appears when there are candidates, so free-form
prompts (`project.search`, rename) stay a single line. The list is a
**scrolled window**: the producer sends a bounded slice
(≤ `MB_VISIBLE` = 10) around `selected` plus `total`, so a
1000-command `M-x` ships ~10 strings per keystroke, not 1000, and the
selected row stays in view as you cycle.

### Q#MB4 — Cursor as a codepoint offset

The message carries `cursor` as the count of codepoints before the
cursor in `input` (computed in the producer via `char_indices`). See
Q#MB2 for how the GPU turns it into the band caret x.

### Q#MB5 — Forwarding the chords that *open* the minibuffer

The GPU withholds command chords by default, so `M-x` and the `C-x`
prefix never reached the daemon — the minibuffer couldn't even be
opened. `is_minibuffer_open_chord` now forwards them: `M-x`
(→ execute-command, from which any command incl. find-file /
switch-buffer is reachable by name) and the **`C-x` prefix** (so the
bound `C-x b` / `C-x C-f` work). Both flip the daemon into a state
(`minibuffer active` / `pending prefix`) that makes `dispatch_idle` go
false, after which the intercept gate round-trips every key. Mirrors
`is_search_entry_chord` / `is_clipboard_chord`; no optimistic local flip
(the search precedent). General Emacs-chord forwarding stays a separate
thread; this arc forwarded only what opens a prompt.

## Wire (protocol v12)

Additive over v11; `SUPPORTED = [6..12]`:

```
InstanceMessage::MinibufferPrompt {
    prompt: Option<String>,   // None = minibuffer closed (clears the GUI)
    input: String,
    cursor: u32,              // codepoints before the cursor in `input`
    candidates: Vec<String>,  // windowed slice (≤ MB_VISIBLE)
    selected: Option<u32>,    // highlighted row *within* `candidates`
    total: u32,               // full candidate count
}
```

Cached-compare suppressed like `SearchPrompt`; first sight while closed
stays silent. Daemon-gated `>= 12`. The TUI ignores the variant (it
paints the minibuffer via its own bottom row). The whole shape (incl.
candidates) landed at v12, so the dropdown phase needed no second bump.

## Phasing (delivered; each commit binary-build-green)

1. **Wire + prompt line** — protocol v12 + producer + daemon gate + GPU
   handler + the band prompt line (prompt + input + caret) + the
   opening-chord forwarding (`M-x`, `C-x`). `M-x` opens, typing works,
   RET runs. User-validated.
2. **Candidate dropdown** — GPU-render-only: the vertical popup above the
   band, consuming the candidates already on the wire. User-validated.
3. **Docs** — this consolidation.

Phase 1 made the minibuffer *work* in the GUI; phase 2 made it
*pleasant*. The v12 bump means the daemon + pmacs-gpu must both be
rebuilt to negotiate it.

## Categorical bets (all held)

- **The family pattern generalized a third time.** `MinibufferPrompt` +
  the band/popup surfaces dropped in as the same shape as search and the
  menu; the producer/cached-compare/gate machinery reused cleanly and
  the core stayed untouched.
- **A windowed candidate slice was enough.** ~10 around the selection
  keeps the wire cheap and the dropdown correct as you cycle.
- **Forwarding only the opening chords was the right cut.** `M-x` + `C-x`
  make the minibuffer reachable without the general chord-forwarding can
  of worms.

## As-built divergences from the framing

1. **Caret via monospace advance** (Q#MB2/Q#MB4), not a per-glyph width
   measurement — simpler, exact for the ASCII case that dominates.
2. **A third `TextRenderer`** for the dropdown, not "a second" — the menu
   already owns its own popup renderer; the dropdown got a dedicated one
   and reuses the menu's colors.
3. **Dropdown ordering pinned to best-at-top, growing upward** — the
   framing left it unspecified; top-to-bottom reading with `M-n` moving
   the highlight down was the least surprising.
4. **`total` ships but isn't rendered** — the "i/total" hint is deferred
   (below).

## Deferred (named, not silently dropped)

- General Emacs-chord forwarding in the GPU (every binding, not just the
  prompt-openers) — the separate thread this arc brushes against.
- The "i/total" count hint and a Telescope-style preview pane beside the
  list.
- Candidate annotations (kind / docstring) — the core's candidates are
  bare strings today.
- Bringing the TUI's inline `[selected]` up to the same dropdown (unify
  the frontends); for now the GPU leads.
- Multibyte-exact caret positioning in the band.
