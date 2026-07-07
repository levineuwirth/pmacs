# In-buffer completion popup — framing (Arc 1a)

pmacs has a complete completion system that nothing drives. M4.7 shipped
the LSP completion store + `CompletionView` popup (`src/completion.rs`);
M4.11 shipped the provider framework — registry, scoring, dedup, and
four providers (lsp=100, snippets=80, project_symbols=60, dabbrev=20 by
priority; `src/completion_framework.rs`); the async LSP request path
(`LspManager::request_completion`, `src/lsp.rs:1665`) works and is
exercised by m4 acceptance tests. But there is no keybinding, no
typed-char trigger, `CompletionView` is never instantiated, and no GPU
wire message exists. This arc wires it end-to-end in both frontends:
type, see candidates, TAB/RET accept.

Roadmap context: `docs/roadmap-2026-07.md` Arc 1a. Closest wire
precedent: `docs/gpu-minibuffer-framing.md` (the
`SearchPrompt`/`MenuPrompt`/`MinibufferPrompt` family). Tightened
2026-07-07 after a code review pass against the current tree
(findings folded into Q#C1/C3/C6/C7/C8/C9 below).

## What already exists (verified)

- **Framework**: `CompletionRegistry::collect(ctx)` is synchronous —
  filters enabled providers by priority, scores against `ctx.prefix`
  (exact=1000 / prefix=600 / word-boundary=300 / substring=100), dedups
  by `(label, insert_text)`, sorts (`completion_framework.rs:292-333`).
  `Rc<RefCell<…>>` — main-thread only. Instantiated at startup
  (`editor.rs:243`) and exposed as `pmacs.completion.{register,collect,
  context_for,snippets.*}` — never called by builtin Lua.
- **The LSP provider does not request, and is globally scoped** — it
  drains **every** cached `(server_id, uri)` entry in the store,
  ignoring context (`completion_framework.rs:617-645`), so unmodified
  it can surface stale candidates from other buffers/servers. Fresh
  requests are the editor's job: `pmacs.lsp._request_completion_raw` →
  awaitable handle (`builtin/runtime/lsp.lua:585-597`), response
  absorbed into the store with positions already normalized to bytes
  (`lsp.rs:2630-2647`).
- **`collect` does not filter non-matches** — `score_match` returns −1
  for a candidate that misses the prefix, but `collect` keeps it and
  merely sorts it last (`completion_framework.rs:292-333`); a no-hit
  prefix would still yield a popup full of unrelated rows.
- **The public attachment accessor does not flush** — interactive LSP
  commands resolve via the *local* `attached_for_active()`, which runs
  `flush_did_change` first so the server answers against current text
  (`builtin/runtime/lsp.lua:520-533`); the public
  `pmacs.lsp.active_attachment()` is deliberately side-effect-free and
  skips the flush (`lsp.lua:541-545`).
- **`buffer.after-edit` carries no payload** — both edit paths fire it
  with empty args (`editor.rs:575`, `daemon.rs:1923`); a driver cannot
  see what changed, only that something did.
- **Trigger characters**: `CompletionTriggers::from_capabilities` +
  `should_fire(ch)` per server (`completion.rs:406-454`), Lua-exposed.
  No prefix-extraction helper exists anywhere — the driver must derive
  the word before the cursor itself.
- **`CompletionView`** is a cell-owning popup (kind glyph + label +
  detail, reverse-video selection, width ≤ 40, CJK-aware) — but it
  reads the raw LSP store, not framework candidates, and the TUI
  overlay loop hands every overlay the full window viewport
  (`editor.rs:1667-1669`); nothing positions a sub-rect popup today.
  `MenuView` is the precedent for a self-positioning cell popup.
- **Dispatcher shadows** already exist for minibuffer
  (`editor.rs:520`), menu (`:500`), isearch (`:511`).

## Decisions

### Q#C1 — The driver is builtin Lua, core stays a library

A new `builtin/runtime/completion.lua`: derives the prefix (word chars
`[A-Za-z0-9_]` before the cursor), decides *when* to open (Q#C9),
fires the LSP request through a flushing accessor (Q#C8), runs
`pmacs.completion.collect`, **filters to `score >= 0`** (collect
returns non-matches sorted last; Lua receives `score`, so the driver
drops them — a Rust-side filter inside `collect` would change M4.11
semantics/tests and is left as a follow-up), and publishes the session
(Q#C2) only when at least one row survives. Mirrors how `lsp.lua`
drives every other LSP surface. Refresh-on-typing rides the same
after-edit hook while the session is open; the session closes when the
prefix dies (Q#C3's validation, `C-g`, or an edit that kills the
word). Nit rolled into phase 1: `pmacs.completion.context_for` cannot
express a `Char` trigger (`lua_bindings/mod.rs:9964` maps only
incomplete-vs-invoked) — either extend it or have the driver pass raw
ctx tables (already supported).

### Q#C2 — Core-owned session state, the search-store pattern

A frontend-agnostic `CompletionPopup` session on `EditorCore` (like
`menu: SharedMenu`): `{buffer_id, anchor_byte, prefix_len, candidates,
selected, total}`. Lua publishes into it; both frontends render from
it; keys resolve against it daemon-side. Control is daemon-owned even
though rendering is frontend-local — the Q#UX1 lesson, applied from
day one this time. Candidates carry `{label, kind, detail,
insert_text}` (framework `CompletionCandidate` projected down).

### Q#C3 — Key capture: a partial dispatcher shadow, not buffer-local binds

While the session is open, `dispatch_key` routes **only**
`TAB / RET / C-n / C-p / Up / Down / ESC / C-g` to
`dispatch_completion_key` (accept / accept / next / prev / next / prev
/ close / close); *everything else falls through* to normal dispatch,
so printable keys keep self-inserting and motion keys work. This is
the fourth member of the existing shadow family and avoids the
bind/unbind lifecycle discipline that transient buffer-local keymaps
would need. Considered and rejected: minibuffer-style full shadow
(typing must insert); `add_intercept` (sees edits only, never nav
chords); buffer-local binds (workable — the `*buffer-list*` idiom —
but the teardown hazard buys nothing here).

**Session validity is enforced in core, post-dispatch — not by hooks.**
Cursor motion fires no hook (`buffer.after-edit` is edits-only,
`editor.rs:573`), so "close when the cursor leaves the word" cannot be
Lua-driven. After any dispatched action while the session is open, the
dispatcher runs a validation step: the session survives only if the
active buffer still matches `buffer_id`, the cursor sits inside
`[anchor .. anchor + current-word-end]`, and the prefix re-derived
from the buffer still starts at `anchor`. Anything else — motion off
the word, buffer switch, window change, undo that rewrote the region —
closes the session. This also covers remote/CRDT edits landing under
the popup: the next dispatch (or the producer tick) revalidates
against the shifted text.

### Q#C4 — TUI popup: rework `CompletionView` on the MenuView pattern

`CompletionView` switches its data source to the Q#C2 session and
becomes self-positioning like `MenuView`: compute its own sub-rect
anchored to the row below the cursor (above when near the window
bottom), clamped to the window and offset past the gutter
(`viewport.gutter_w`), painting ≤ `POPUP_VISIBLE` rows. Attached once
per window like `MenuView`, self-suppressing when the session is
closed or belongs to another buffer.

### Q#C5 — Wire: `InstanceMessage::CompletionPopup`, protocol v15

Additive over v14; `SUPPORTED = [6..15]`, daemon-gated `>= 15`,
produced by `semantic_render::completion_popup_msg` with the family's
cached-compare suppression:

```
InstanceMessage::CompletionPopup {
    buffer_id: BufferId,
    anchor: Option<u64>,        // byte offset of the prefix start; None = closed
    prefix_len: u32,            // bytes of typed prefix (frontend may embolden)
    rows: Vec<CompletionRow>,   // windowed slice (≤ POPUP_VISIBLE = 10)
    selected: Option<u32>,      // within `rows`
    total: u32,
}
CompletionRow { label: String, kind: u8, detail: Option<String> }
```

Byte anchor, not pixels — the GPU already maps byte→glyph rect for the
caret and presence washes; first byte-anchored popup on the wire
(menu anchors at the click pixel locally). Rows are display-only;
accept is a daemon round-trip, so `insert_text` never ships.

### Q#C6 — GPU input while the popup is open: an explicit key predicate

The existing gates are all-or-nothing (`daemon_intercepts_keys`,
`pmacs-gpu/src/main.rs:1913`; `dispatch_idle`) — reusing them would
round-trip *typing* too, killing optimistic latency exactly when the
user is mid-word. Instead the GPU gets an explicit
`is_completion_control_key(key, mods)` predicate, checked **before**
the optimistic-insert path and before local key handling, active only
while `CompletionLocal.is_some()`: exactly
`TAB / RET / ESC / C-g / C-n / C-p / Up / Down` forward as
`FrontendEvent::Key` round-trips into `dispatch_completion_key`;
every other key stays on its normal path (plain chars optimistic,
chords per the usual forwarding rules). Three keys need the gate
specifically because their default handling is wrong under a popup:
RET and TAB are optimistic-eligible today (they reduce to plain
inserts, `main.rs:5300`-area), and **ESC is a hardcoded local
exception** (`main.rs:1423`) that would be swallowed and never reach
the daemon — it must be conditionally forwarded while the popup is
open. Rendering: a `CompletionLocal` mirror + a dropdown layer cloned
from the `mb_dropdown_*` functions, anchored at the anchor byte's
glyph rect instead of the status band, clamped to the window (the
F-007 fit logic).

### Q#C7 — Accept semantics: validate, then replace

Accept **re-validates before touching the buffer**: the active buffer,
cursor, anchor, and the prefix currently in the text must all still
match the session (the Q#C3 invariant, re-checked at the moment of
accept — a remote edit or race can invalidate between frames). On
mismatch, accept is a no-op that closes the session. On match, it
replaces `[anchor .. cursor]` with the candidate's
`effective_insert_text()` through the normal command/edit layer (one
undo entry; fires `buffer.after-edit`, so LSP `didChange` and styling
refresh ride existing machinery), then closes the session. Snippet
bodies insert literally in v1 — no tabstop engine (deferred).

### Q#C8 — LSP scoping + freshness: flush first, scope to the attachment

Two correctness rules, then the UX:

- **Flush before requesting.** `didChange` is debounced; a request
  issued without flushing answers against stale text. The driver must
  not use the non-flushing `pmacs.lsp.active_attachment()` — `lsp.lua`
  grows a public flushing accessor (wrapping the local
  `attached_for_active()`, `lsp.lua:520-533`) that the driver calls
  before `request_completion`, exactly as every interactive LSP
  command already does internally.
- **Scope candidates to the attachment.** The built-in LSP provider
  drains the whole store across all `(server, uri)` keys. Fix in the
  framework: `CompletionContext` gains `uri: Option<String>` (the
  driver fills it from the attachment record) and the provider reads
  only that URI's entries. Fallback if the context change proves
  awkward: disable the built-in LSP provider for the driver path and
  have Lua merge `pmacs.completion.items(rec.server, rec.uri)` itself
  — either way, no cross-buffer candidates.

UX: publish immediately from the synchronous providers
(dabbrev/snippets/project-symbols feel instant), fire the LSP request,
and re-collect + re-publish when the response lands
(`handle:on_complete`). `isIncomplete` responses re-request on further
typing. No spinner; the popup just gets better a beat later.

### Q#C9 — Trigger policy without edit metadata

`buffer.after-edit` fires with no payload from both edit paths
(`editor.rs:575`, `daemon.rs:1923`), so the driver cannot see *what*
changed — it must reconstruct intent from state. Policy (v1):

- The driver keeps a per-invocation snapshot `{buffer, revision,
  cursor}`.
- **Open** on after-edit only when: the active buffer is unchanged,
  the cursor sits at the end of a word with prefix ≥ 2, **and** the
  cursor advanced by exactly one codepoint since the last snapshot
  (the single-char-typing signature) — or the char before the cursor
  is a server trigger character (`should_fire`).
- **Paste, undo/redo, kill, and remote edits never auto-open** (their
  cursor delta ≠ 1 or the region signature doesn't match); `C-M-i`
  covers the deliberate cases.
- While **open**, any after-edit re-derives the prefix from the buffer
  and refreshes or closes — no metadata needed, the text is the truth.

Named alternative, deliberately not taken now: giving
`buffer.after-edit` an edit payload (buffer id, kind, range). Variadic
Lua handlers would tolerate the new args, but the PR #52 revert
history says hook-signature changes must not ride along in a feature
bundle — if the heuristic proves flaky, the payload becomes its own
small, separately-validated PR.

## Phasing (each phase independently green + user-validated)

1. **Core + TUI, no wire change.** Session store + dispatcher shadow
   with post-dispatch validation (Q#C3) + `CompletionView` rework +
   `builtin/runtime/completion.lua` driver (Q#C1/C9 policy, score ≥ 0
   filter) + validated accept (Q#C7) + `completion.at-point` — plus
   the small Rust/Lua seams the review surfaced:
   `CompletionContext.uri` + LSP-provider scoping, the flushing
   attachment accessor, and the `context_for` char-trigger fix
   (Q#C8/C1). Validate in the standalone TUI and a TUI-attached
   daemon.
2. **Wire + GPU.** Protocol v15, producer + daemon gate, GPU
   `CompletionLocal` + dropdown layer + the RET/TAB optimistic gate.
3. **Polish + docs.** `isIncomplete` re-query, per-server
   trigger-chars, as-built notes folded into this doc.

Arc 2 interleave points: after phase 1 and after phase 2 (query-replace
and kill-ring are the named next table-stakes items).

## Categorical bets (score at close)

1. **The prompt-family pattern generalizes a fourth time** — producer /
   cached-compare / gate / local-mirror drop in without core surgery.
   Risk concentrates in the one new element: byte-anchored positioning.
2. **Synchronous `collect` is fast enough per keystroke.** dabbrev
   scans the whole buffer text per call — O(buffer) on every refresh.
   Bet: fine at typical file sizes; a cap/debounce is the fallback,
   not a redesign.
3. **RET/TAB/ESC routing** — some path will both insert and accept
   (or neither), or ESC will die at the GPU's local handler, until the
   Q#C6 predicate is exactly right. Predicted highest-likelihood
   finding.
4. **Popup placement edge cases** — bottom-of-window flip, gutter
   offset, narrow windows. Clamping bugs, MenuView-class.
5. **The Q#C9 single-char heuristic holds.** Paste/undo/kill/remote
   edits stay quiet and ordinary typing always opens. If validation
   shows misfires, the fix is the named hook-payload PR, not
   heuristic patching.

## Deferred (named, not silently dropped)

- Snippet tabstops/placeholders (v1 inserts bodies literally).
- `completionItem/resolve` (lazy documentation/detail) and
  `additionalTextEdits` (auto-import) — needs a resolve round-trip on
  selection change.
- A documentation panel beside the popup (company-style doc buffer).
- Fuzzy matching beyond the current prefix/word-boundary/substring
  scorer.
- TUI/GPU visual unification (GPU leads, as with the minibuffer).
- Minibuffer-style persisted ranking / frequency weighting.
