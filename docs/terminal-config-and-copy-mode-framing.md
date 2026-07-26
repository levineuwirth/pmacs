# Terminal configuration and copy mode

**Revision 4 — scouted against canonical `main` @ `b889873` (protocol v20),
2026-07-25. APPROVED after four review rounds. Stage 1 MERGED as #173
(`main` @ `cf54270`, 2026-07-26). Stage 2 implemented on branch
`terminal-copy-mode` off `main` @ `cf54270`; no protocol change.**

**Stage 2 ships eight of its nine criteria, plus 18a and 18b added in review
round 1 and 16c-16e in rounds 2-3.** Rounds 2 and 3 changed the design, not
just the code: the snapshot is now genuinely `read_only` at the rope, so
**Q#TC6a's analysis below is superseded in part** — read the box at its head
before the analysis. Q#TC6a's conclusion survives; two of its premises do
not, and **criterion 17's bite was restated with them** — the daemon now
refuses the op, so the failure it must look for is mirror mutation plus
divergence, not silent agreement.

Criterion 17's semantic-frontend end-to-end pin is deliberately
absent — see the note under it — because a faithful version requires the real
`pmacs-gpu` optimistic path, and therefore the `a37` foundation, which CI never
compiles and which skips silently. Both halves of the *mechanism* it guards are
pinned ungated instead (16, 16b). No other criterion is partial.

**Review round 1 found four defects, and the pair of them rhymes.** Two were
implementation (18a's foreign-buffer clobber, 18b's name-keyed identity) and
two were vacuous pins (18/19's refresh, 20's tail-follow) — and all four trace
to the same root: **a name is not an identity, and a context-free readout is
not a state observation.** The name mistake produced both P1s; the readout
mistake produced both P2s.

Revision 4 gives the escape-key cache an owner and a lifecycle (Q#TC4c) —
revision 3 named the key but not the storage, and two implementations
satisfied its acceptance while behaving differently on A→B→A. It also corrects
the read-only deferral, which understated the substrate required: the bypass
path is `ensure_writable`-guarded too, so genuine immutability alone would
break every generated buffer that refreshes.

Revision 3 corrects two design errors and decides the chords. The
round-trip failure shape in revision 2 was **wrong in the reporter's favour**:
a Lua intercept does not set `Buffer::read_only`, and there is no Lua binding
that does, so an optimistic `CrdtOp` bypasses the intercept *and* passes
`ensure_writable()` — the daemon buffer mutates too, rather than the mirror
diverging alone (Q#TC6a). Revision 2 also had all three settings resolving
against the terminal identity buffer, which is impossible for the two read
*before* that buffer exists (Q#TC2b). Chords are now decided and
collision-scouted rather than deferred to implementation (Q#TC10, Q#TC8a).

Revision 2 answered seven review findings. Four were load-bearing: the settings
are `Live`, so the registry **accepts buffer-local overrides whether or not we
want them**, and `value_epoch()` does not move on a buffer switch — an
epoch-only cache can serve the wrong terminal's escape chord (Q#TC4); the
double-escape byte is a hardcoded `0x03`, so a configured escape would still
send Ctrl-C and make its own literal chord unreachable (Q#TC4b); the snapshot
buffer needs `set_round_trip_input`, not only a read-only intercept, or a
semantic frontend can optimistically edit it before daemon dispatch (Q#TC6);
and the two stages must be two branches and two PRs. Revision 1's
materialized-copy reframe is unchanged.

Two stages, one arc, no protocol change:

- **Stage 1 — configuration.** Terminal profiles, scrollback, and the escape
  key become configurable. Today the terminal has **zero** configuration
  surface: the `terminal` command hardcodes `os.getenv("SHELL") or "/bin/sh"`,
  `scrollback_rows` is a per-open argument only, and the escape chord is a
  literal in Rust.
- **Stage 2 — copy mode and search over scrollback.** A command that turns
  the retained terminal screen and scrollback into an ordinary buffer, where
  isearch, motion, selection, and the kill ring already work.

Explicitly **not** in this arc: the panel terminal (blocked on bottom-panel
Stage 2), and shell integration (cwd tracking, prompt marks, command zones) —
the keystone that unlocks the VS Code-style cluster, which needs its own
security framing because it decides what a child process may make the editor
do.

## Branch and PR plan

**Two branches, two PRs.** Configuration and copy mode are independently
releasable and have no dependency on each other; one framing covers the arc,
but the one-feature/one-branch/one-PR rule governs the implementation.

1. `terminal-config` — Stage 1. Also carries the **terminal opening
   keybinding** (Q#TC10).
2. `terminal-copy-mode` — Stage 2, branched off `main` after Stage 1 merges.

Sequencing is not a dependency but avoids a conflict: both stages edit
`builtin/runtime/terminal.lua`.

## Ground truth (measured, not recalled)

Three facts constrain the design, and two of them rule out the obvious plan.

### 1. Terminal profiles cannot be a config-registry setting

`ConfigValue` is **four scalars** — `Bool`, `Int`, `Num`, `Str`
(`src/config_registry.rs:312`) — and its own doc comment says they "are never
stored --- only these four scalars (Q#CR3)". `ConfigKind` adds `Enum`, which
is physically a string validated against choices fixed at `define` time
(`src/config_registry.rs:115-145`). There is no table, list, or map kind.

A terminal profile is inherently a table: `{ command, args, cwd, env }` per
name. **Table-valued settings are an existing named deferral of the config
registry arc** — the same gap that keeps `pmacs.lsp.config`,
`pmacs.pair.sets`, `pmacs.comment.strings`, and the `pmacs.parse.*` proxies as
raw Lua. Profiles join that list rather than forcing that deferral open here.

### 2. Search cannot reuse isearch in place over a terminal

`SearchStore::set(buffer_id, query, matches: Vec<ByteRange>)`
(`src/search.rs:99`) keys matches by buffer and addresses them as **byte
ranges into that buffer's rope**; the painting path materializes the source
with `buf.snapshot_rope().slice(0, buf.len(), ..)` (`src/search.rs:435`).

A terminal identity buffer is **empty and read-only** by construction. Its
content lives in `TerminalScreen` as cells addressed by `(row, col)` across
history plus visible rows — there are no rope bytes to range over. Searching a
terminal in place therefore means a second, parallel search facility with its
own match store and its own highlight path, because terminal painting consumes
owned cells and not document style spans.

### 3. An in-place copy mode would be the seventh dispatch shadow

`dispatch_key`'s terminal-transport arm intercepts **every** key before
ordinary keymap dispatch whenever `active_terminal_key` is `Some`, which keys
purely on `is_terminal(window.buffer_id)` (`src/editor.rs:1098-1107`,
`973-1016`). A mode that keeps the terminal buffer focused while rebinding
keys to motion/selection must therefore add a new precedence rung.

`COHERENCE.md` §6 grades that ladder **weak, "and growing by one island per
modal feature"**, records that **no transient-keymap mechanism exists to
migrate to** (`KeymapStack` has exactly three fixed scopes, no layer stack, no
push/pop, no lifetime), and notes that `describe-key` already lies while a
shadow is active. It also names the counter-example: the entire picker/panel
family uses ordinary **buffer-local keymaps** and is inspectable and
rebindable.

### 4. What already exists and is reusable

- `retained_rows(projection)` (`src/terminal/view.rs:539`) iterates history
  plus visible rows; `copy_selection_bytes(rows, selection)`
  (`src/terminal/view.rs:849`) serializes a range with the fidelity Stage 2
  criterion 21 already pins — soft wraps joined, hard rows separated, trailing
  default blanks trimmed, wide glyphs and combining clusters copied once.
- `ConfigRegistry::value_epoch()` (`src/config_registry.rs:1127`) is public and
  monotonic — cheap invalidation for a hot-path cache.
- The Lua surface is `define` / `get` / `set` / `set_local` / `on_change` with
  a disposable handle (`src/lua_bindings/config.rs`).
- `pmacs.terminal.open` already accepts
  `command, args, cwd, env, name, rows, cols, scrollback_rows, display,
  window`. **`display = "panel"` already works** (bottom-panel Stage 1) — the
  panel terminal is blocked on rendering, not on this surface.
- Terminal buffers already carry buffer-local bindings (`M-w`, `M-v`, `C-v`,
  `M-<`, `M->`) installed by `terminal.open` in `builtin/runtime/terminal.lua`.

## Stage 1 — configuration

**Q#TC1 — Profiles are a raw Lua table, not a setting.**
`pmacs.terminal.profiles` maps a name to a spec table, exactly following the
`pmacs.lsp.config` precedent. The registry holds only scalars. Rejected
alternative: widening `ConfigValue` with a table kind — that is the config
arc's own named deferral, it is cross-cutting (persistence, `describe-setting`
rendering, the `custom-file` question all key on the scalar assumption), and
smuggling it into a terminal PR would be the wrong place to decide it.

**Q#TC2 — `terminal.default-profile` is `String`, not `Enum`.** `Enum`
choices are frozen at `define` time; profiles are user-extensible from
`init.lua` and later. Validation happens at open time, and an unknown name
must produce a pointed error that **names the known profiles**, not a bare
"unknown profile".

**Q#TC2a — the exact settings, defaults, and bounds.** All three are `Live`
(see Q#TC2b), and every default reproduces today's behavior exactly, so a tree
with no settings written behaves identically (acceptance 12).

| name | kind | default | bounds |
|---|---|---|---|
| `terminal.default-profile` | `String { allow_empty: true }` | `""` | — |
| `terminal.scrollback-rows` | `Integer` | `10_000` (`DEFAULT_TERMINAL_SCROLLBACK_ROWS`) | `0 ..= 4_000_000` (`MAX_TERMINAL_HISTORY_CELLS`) |
| `terminal.escape-key` | `String { allow_empty: false }` | `"C-c"` | parsed as a chord |

**Zero is a legal scrollback value meaning "retain no history".** The core's
own validation rejects only values *above* `MAX_TERMINAL_HISTORY_CELLS`
(`src/terminal/session.rs:114`), so `scrollback_rows = 0` is accepted through
`terminal.open` today. A `1` minimum here would invent an asymmetry between the
setting and the per-open field for no reason.

`""` is the **"no default profile" sentinel**: an empty string means "fall
through to `$SHELL`", not "a profile named empty". `allow_empty: true` exists
precisely to express it, and the open path treats empty and unset identically.

**Q#TC2b — the settings are `Live`, and the registry therefore accepts
buffer-local overrides. That is specified rather than accidental.**
`ConfigRegistry::set_local` refuses only `StartupOnly` definitions
(`src/config_registry.rs:949`); a `Live` setting can be pinned per buffer by
anyone. Declaring these global-only is **not currently expressible** — a
`scope = "global"` define flag is one of the config registry's own named
deferrals, and `autosave.interval-ms` already has the same latent problem.

Making them `StartupOnly` instead would buy enforcement at the cost of the
feature: the escape key could never be changed mid-session, which kills Q#TC4's
whole point. So they stay `Live`, and resolution is defined **per setting,
because the three are not read at the same moment**:

| setting | read when | resolution |
|---|---|---|
| `terminal.escape-key` | every keystroke in a terminal (cached) | `get(name, terminal_buffer)` — **buffer-local → global → default** |
| `terminal.default-profile` | once, **before** the terminal exists | `get(name)` — **global chain only** |
| `terminal.scrollback-rows` | once, **before** the terminal exists | `get(name)` — **global chain only** |

The split is forced, not stylistic. The two open-time settings are consumed by
`_open` **before it creates the identity buffer**, so there is no terminal
buffer to resolve against — and no caller could have pinned a local override on
a buffer that does not yet exist. `pmacs.config.get(name)` with no buffer
argument already means exactly "the global chain, never an ambient buffer", so
this is the registry's existing semantic rather than a new rule.

Consequences, stated so they are not discovered later:

- a per-terminal escape key is a supported feature, not a bug;
- `set_local` on `terminal.default-profile` or `terminal.scrollback-rows` is
  **always inert**, for any buffer, because the open path never consults a
  buffer chain. This is deliberate; the alternative — resolving against
  whichever buffer happened to be current at open time — would make a
  terminal's scrollback depend on what the user was looking at when they
  pressed the key.

Rejected alternative: resolving the open-time settings against the *target
window's pre-open buffer*. It is expressible, but it makes an ambient buffer
load-bearing for a value the user set globally, which is the trap
`pmacs.config`'s two-argument/one-argument split exists to avoid.

**Q#TC3 — `terminal.scrollback-rows` is `Integer` with bounds, and an explicit
per-open `scrollback_rows` still wins.** The precedence is
**explicit argument over global setting** — there is no ambient buffer in this
chain at all (Q#TC2b resolves it through `get(name)`), so the rule is simply
that what a caller passes to `terminal.open` beats what the user configured
globally. The bounds above come from the existing validation, so the setting
cannot express a value the core will reject.

**Q#TC3a — profile resolution order, field by field.** `profile` is accepted
by **`pmacs.terminal.open` as well as the command**, so a Lua caller is not
forced through the command to use one. For each field, the first source that
supplies it wins:

1. an explicit `pmacs.terminal.open` field;
2. the named profile's field — `profile` argument, else
   `terminal.default-profile` when non-empty;
3. the scalar setting, where one exists (`scrollback_rows` only);
4. the built-in fallback (`command` = `$SHELL`, else `/bin/sh`).

`env` is the one field where "first wins" is ambiguous, so it is stated:
profile `env` and explicit `env` are **merged**, with explicit entries
overriding profile entries of the same name. Any other reading silently drops
half a user's environment.

An explicitly passed `profile` that does not exist is an error even when
`terminal.default-profile` is valid — a typo must not silently fall back to
the default.

**Q#TC4 — `terminal.escape-key` is a `String` chord spelling, parsed once and
cached by `(buffer_id, value_epoch)`.** `is_terminal_escape_chord`
(`src/editor.rs:4413`) currently compares against a literal `C-c`. Reading and
parsing a setting on **every keystroke in a terminal** is not acceptable in
that path.

**The cache key must include the buffer.** `value_epoch()` advances only on
`set` / `set_local` / removal (`src/config_registry.rs:918`, `970`, `1011`,
`1029`) — **it does not move when the focused terminal changes**. An
epoch-only cache therefore serves terminal A's escape chord to terminal B for
as long as no setting is written, which is exactly the case where nothing looks
wrong. Keying on `(buffer_id, value_epoch)` is the minimum correct identity.

**Q#TC4c — the cache lives on `TerminalSession`, so its lifecycle is the
terminal's.** Revision 3 named the key `(buffer_id, value_epoch)` but not the
storage, and the two obvious storages behave differently on A→B→A:

- a **single last-entry cache** reparses on every switch between two
  terminals, and re-reports an invalid value each time — a status line that
  scolds you for a setting you already know about, forever;
- an **editor-side map** preserves "parsed and reported once" but **leaks an
  entry per terminal** unless something purges it, and that purge is a second
  thing to get wrong.

`TerminalSession` (`src/terminal/session.rs:215`) is created in
`TerminalManager::open` and dropped on kill/prune, so putting the cache there
gets the lifecycle for free with no purge hook to forget. It carries the parsed
chord, the `value_epoch` it was parsed at, and whether the current invalid
value has already been reported.

**"Reports once" means once per terminal, per effective invalid value.**
A→B→A must not re-report. Changing the setting from one invalid value to a
*different* invalid value **does** re-report, because that is new information
about a new mistake.

**The reporting channel is `EditorCore::status`** — the same channel
`send_terminal_bytes` already uses for terminal failures
(`src/editor.rs:1122`). Explicitly **not** `pmacs.error`: it is not installed
as a module anywhere in `src/lua_bindings`, so its call sites across the
runtime are dead, and a report sent there would be a report nobody sees.

**Q#TC4a — an unparseable escape key must not brick terminal input.** A bad
value falls back to `C-c` and reports once. The failure mode this avoids is
severe: with no escape chord, every key goes to the child and the user cannot
reach any editor binding to fix the setting that broke it.

**Q#TC4b — repeating the configured escape sends THAT chord to the child, not
Ctrl-C.** The double-escape arm currently writes a hardcoded
`&[0x03]` (`src/editor.rs:988`). With `terminal.escape-key = "C-x"`, `C-x C-x`
would send Ctrl-C — and literal Ctrl-X would become unreachable, since the
first `C-x` is always consumed as the escape. The repeat arm must encode the
**configured** chord through the existing `crate::terminal::input::encode_key`
path, which is also how it inherits application-cursor and modifier handling
rather than growing a second encoder.

Corollary worth pinning: after changing the escape away from `C-c`, an ordinary
`C-c` must reach the child as `0x03` like any other unescaped key.

**Q#TC5 — the `terminal` command gains an optional profile argument** and
otherwise keeps its current behavior; `$SHELL` remains the fallback when no
profile is configured. No existing invocation changes meaning.

**Q#TC10 — the terminal opening keybinding is pulled forward into Stage 1.**
`COHERENCE.md` Priority 1 names "a terminal keybinding" as part of protecting
the golden journey, §2 step 8 grades the terminal "works but undiscoverable",
and this stage already edits `terminal.lua`. Panel rendering imposes no
dependency on binding a command that already exists. Close/kill semantics stay
with the panel work, where the entry and exit points get designed together.

The chord is **decided and scouted, not deferred**: `C-c t`, global. See
Q#TC8a for the collision evidence and for why binding under the existing `C-c`
prefix is a new leaf rather than a shadow.

## Stage 2 — copy mode and search

**Q#TC6 — copy mode MATERIALIZES into an ordinary buffer. It does not add a
dispatch shadow.**

`M-x terminal.copy-mode` snapshots the retained rows into a read-only,
path-less buffer (`*terminal-copy: NAME*`) and displays it. That buffer is an
ordinary document buffer, so:

- **isearch works, with no new search substrate** — it is a rope, so
  `SearchStore` and the existing match-painting path apply unchanged. Ground
  truth 2 is answered by not fighting it.
- **motion, selection, `M-w`, the kill ring, even `M-x occur`-style consumers
  work** — everything that operates on a buffer.
- **The "keys must not reach the child" problem dissolves structurally.**
  `active_terminal_key` keys on `is_terminal(window.buffer_id)`; the snapshot
  buffer is not a terminal, so the transport arm never fires. No new guard, no
  new precedence rung, and ground truth 3's coherence cost is avoided rather
  than paid.
- **`describe-key` stays truthful**, because the bindings are buffer-local and
  inspectable — the idiom `COHERENCE.md` §6 identifies as the right side of
  the line.

**Q#TC6a — the snapshot is read-only at the rope AND round-trip-marked, and
each guard covers a copy the other cannot reach: `read_only` refuses the op
at the daemon, `set_round_trip_input` is the ONLY thing standing between a
replica frontend and unauthorized mutation of its own mirror.**

> **SUPERSEDED IN PART BY IMPLEMENTATION (review rounds 2-3). Read this
> box before the analysis below it.** The reasoning is still the correct
> account of the substrate *as it stood when this was written*, and its
> conclusion about round-trip input still holds. Two of its premises no
> longer do:
>
> - "**No Lua binding sets `read_only` at all**" — one does now.
>   `pmacs.buffer.set_generated_contents` leaves it asserted, so on the
>   daemon side undo, redo, ordinary edits and imported CRDT ops are all
>   refused by `ensure_writable()`. That closed a real defect: undo
>   bypasses the intercept chain, so `M-x buffer.undo` emptied the
>   snapshot.
> - "**`set_round_trip_input` is the ONLY thing**" — it is now the only
>   thing standing between a replica and *mirror* mutation, which is the
>   half `read_only` cannot reach. A semantic frontend applies
>   optimistically in its own mirror before the daemon sees the op; a
>   daemon-side refusal cannot prevent that, it can only make the two
>   copies disagree.
>
> The protection is therefore **layered, not singular**: rope-level
> read-only protects the daemon copy, round-trip input protects the
> replica copy, and neither substitutes for the other. The intercept
> survives only to give a dispatching edit a named error. The Deferred
> lane below records what this leaves open for `*compilation*` and
> listview, which have **not** adopted the primitive.

The established idiom is two calls: `listview.lua:106` and `compile.lua:272`
each pair `pmacs.buffer.add_intercept` with
`pmacs.buffer.set_round_trip_input(buf, true)`. Revision 2 described the
intercept as the guard and round-trip as defence in depth. **That was wrong,
and the correction matters:**

- A Lua intercept guards the **dispatch/edit** path only. It does **not** set
  `Buffer::read_only`, which is "deliberately independent of edit intercepts"
  (`src/buffer.rs:493-500`) — that flag is what makes terminal identity buffers
  reject rope, undo/redo, and remote-CRDT mutation alike.
- **No Lua binding sets `read_only` at all.** The whole `src/lua_bindings`
  tree only ever *reads* it (`fold.rs:313`). A Lua-created "read-only" buffer
  is therefore read-only against dispatch and nothing else.
- So an optimistic `CrdtOp` from a semantic frontend bypasses the intercept
  **and passes `ensure_writable()`**. It is applied. The daemon buffer mutates
  in lockstep with the mirror — the user silently edits a buffer the editor
  told them is read-only. There is no divergence to notice, which is worse
  than divergence.

`set_round_trip_input` prevents this at the only point it can be prevented: it
makes `dispatch_idle_for` report false while the buffer is focused, so the
frontend never applies optimistically and never emits the op. It is not
hardening — it is the guard.

Two things follow, and both are recorded rather than fixed here:

- **The same exposure exists today** for every Lua-created read-only buffer —
  listview panels and `*compilation*` included. They are correct only because
  they call `set_round_trip_input`. This arc must not be the place that
  unilaterally changes that substrate.
- **Exposing `Buffer::set_read_only` to Lua** would make these buffers
  genuinely immutable at the rope/CRDT boundary the way terminal identity
  buffers are, turning round-trip back into real defence in depth. That is a
  substrate change affecting listview and compile as much as this snapshot, so
  it is named in Deferred with its own lane. **Done for this snapshot only**,
  and not by exposing the setter — see the Deferred lane and the box above.

**Q#TC7 — the materializer reuses the existing serializer.** A whole-range
variant of `copy_selection_bytes` over `retained_rows` inherits the criterion
21 fidelity rather than re-deriving soft-wrap, wide-glyph, and trailing-blank
behavior. Writing a second serializer would guarantee the two drift.

**Q#TC8 — one snapshot buffer per terminal, reused on re-invoke.** Re-running
the command against the same terminal replaces the contents in place rather
than accumulating buffers. It is killed with its terminal; killing the
snapshot alone leaves the terminal untouched.

**Q#TC8a — the chords, decided and collision-scouted.**

Worth stating first because it is easy to get backwards: in a terminal window
every **unescaped** key goes to the child, so terminal-local bindings are
reached as `<escape> <key>`. The existing `M-w` copy is physically `C-c M-w`.
The escape consumes itself and the next key starts a fresh ordinary sequence,
which is also why `C-c`-leading bindings are structurally unreachable *inside*
a terminal.

| action | scope | binding | physically typed |
|---|---|---|---|
| open a terminal (Q#TC10) | global | `C-c t` | `C-c t` |
| enter copy mode | terminal buffer | `C-t` | `C-c C-t` |
| refresh snapshot | snapshot buffer | `g` | `g` |
| return to terminal | snapshot buffer | `q` | `q` |

Scouted against the real keymaps:

- **`C-c t` is free.** No bare global `C-c` binding exists; `C-c` is already a
  live global prefix from `fold.lua:48-52` (`C-c @ …`), and `C-c C-k` is
  buffer-scoped in compile/async. `C-c t` is a new leaf under an existing
  prefix, not a shadow.
- **`C-t` is globally `edit.transpose-chars`** (`editops.lua:909`), and binding
  it **buffer-locally is legitimate**: `keymap.bind`'s strictness rejects
  binding a *prefix* of an existing sequence within a scope
  (`keymap_bind_conflict_surfaces_at_bind_time` — "would shadow"), not
  cross-scope shadowing, which is what scopes are for. Listview already binds
  `n`/`p`/`g`/`q`/`RET`/`SPC` buffer-locally. Transpose-chars is meaningless in
  a read-only terminal buffer.
- `C-c C-t` matches emacs-libvterm's own `vterm-copy-mode` chord, so the muscle
  memory transfers.
- `g` / `q` in the snapshot follow listview's precedent exactly.

**Named limitation:** `C-c t` cannot open a terminal *from inside* a terminal,
because `C-c` is consumed as the escape there. `M-x terminal` still works. This
is the documented consequence of Stage 2 criterion 19, not a new defect.

These are what make acceptance 21's `describe-key` claim testable: named
bindings, in named buffers, that introspection must report truthfully.

**Q#TC9 — the live-terminal keys stay.** `M-w`, `M-v`, `C-v`, `M-<`, `M->` on
the terminal buffer are the live affordances and do not change. Copy mode is
additive, on its own binding, and does not replace scroll-and-select.

## Bets

- **B1.** Materializing gives search for free: no second match store, no
  second highlight path, no terminal-specific search UI. *Scored by Stage 2
  landing with zero changes under `src/search.rs`.*
- **B2.** Point-in-time is sufficient for read-back/search/copy. *Scored by
  use; if false, the live frozen mode in Deferred becomes the real feature and
  this becomes its snapshot fallback.*
- **B3.** No protocol change. The snapshot is an ordinary buffer, so both
  frontends render it with existing machinery. *Scored by the diff.*
- **B4.** The escape-key cache keyed by `(buffer_id, value_epoch)` never
  becomes stale in a way a user can observe. *Scored by two acceptances, not
  one: changing the setting mid-session (8) and two terminals with different
  buffer-local values and no write between them (7). Revision 1's epoch-only
  cache would pass the first and fail the second, which is why the bet now
  names both.*
- **B5.** Buffer-local escape keys are a feature rather than a hazard.
  *Unscored and honestly so: the registry cannot express global-only, so this
  is what we get either way. If per-terminal escapes turn out to confuse more
  than they help, the fix is the config registry's `scope = "global"` deferral,
  not a terminal change.*

## Deferred (named)

- **Live frozen copy mode** (true `vterm-copy-mode` semantics: freeze the
  terminal in place, navigate it, resume). Strictly larger; needs either the
  transient-keymap primitive `COHERENCE.md` §6 specifies or a deliberate
  seventh shadow.
- **Shell integration** — cwd tracking, prompt marks, command zones, and the
  VS Code cluster downstream of it (command decorations, exit-code markers,
  rerun, sticky scroll, terminal IntelliSense). Its own arc, with a security
  framing.
- **Table-valued settings** — the config registry's own deferral. This arc
  adds a **second** blocked adopter (after `pmacs.lsp.config` /
  `pmacs.pair.sets`); worth recording as evidence when that deferral is
  ranked.
- **A `scope = "global"` define flag** — also the config registry's own
  deferral, and this arc is its second live case after `autosave.interval-ms`.
  Until it exists, `set_local` on any `Live` setting is accepted whether or not
  the owner wants it, so Q#TC2b specifies the behavior instead of pretending
  it is prevented.
- **Panel terminal** — blocked on bottom-panel Stage 2 (semantic frontends are
  not `panel_capable`). `display = "panel"` already exists and works on the
  grid frontend.
- OSC 8 hyperlinks, images (sixel/kitty), `faint`/`blink`/`conceal`/
  `strikethrough` (needs a shared `Style` widening, so a protocol bump),
  cursor shape/blink, kitty keyboard protocol.
- Terminal session persistence/reconnect across editor restart.
- **A terminal close/kill command** — the remaining half of `COHERENCE.md`
  §2 step 8's discoverability gap. It belongs with the panel-terminal work,
  where entry and exit points get designed together. The *opening* keybinding
  is **no longer deferred**: Stage 1 carries it as Q#TC10.
- **Genuine immutability for generated buffers — and it is bigger than a Lua
  setter.** Today no Lua binding sets `read_only` (`src/lua_bindings` only
  reads it, `fold.rs:313`), so every Lua-created "read-only" buffer — listview
  panels, `*compilation*`, and this snapshot — is read-only against dispatch
  alone and relies entirely on `set_round_trip_input` (Q#TC6a).

  Merely **exposing `set_read_only` would break all three.** The
  intercept-bypass path is `ensure_writable`-guarded too:
  `apply_edit_skip_intercepts` calls it first (`src/buffer.rs:994`), and that
  is exactly the primitive an owner uses to rewrite its own generated buffer.
  Flipping the flag would stop listview refreshing, `*compilation*` streaming,
  and this snapshot refreshing — the very operations those buffers exist for.

  So the lane needs **two** things, not one: genuine immutability at the
  rope/CRDT boundary, *and* an owner-authorized update path that is not simply
  "skip the intercepts". Naming only the setter would have made it look like a
  one-line follow-up.

  **PARTIALLY RETIRED in Stage 2, because review round 2 turned it from a
  nice-to-have into a defect.** An intercept guards the dispatch path only,
  and `Buffer::undo` reaches the rope through `ensure_writable` without ever
  consulting the intercept chain — so a single `C-/` replaced a freshly
  rendered snapshot with an empty buffer. Rebinding the undo chords
  buffer-locally, which is `*compilation*`'s existing idiom, does **not**
  close it: `compile.lua` says so itself ("command/menu undo stays
  dispatchable"), and `M-x buffer.undo` needs no keymap.

  The fix ships the deferral's two halves together as **one** primitive
  rather than exposing the setter: `Buffer::set_generated_contents` (Lua:
  `pmacs.buffer.set_generated_contents`) lifts `read_only`, replaces the
  contents skipping intercepts, **discards the history**, and re-asserts
  `read_only`. Pairing the lock with the write is precisely what makes it
  safe — a bare `set_read_only` would let a caller lock a buffer it can no
  longer refresh, which is why the lane was deferred in the first place.
  Discarding history is load-bearing twice: it removes the entries undo
  would replay, and it stops a periodically refreshed buffer accumulating
  rope clones that `read_only` guarantees nothing can ever pop.

  **What remains of the lane:** `*compilation*` and listview panels still
  rely on intercept-plus-round-trip and are still emptiable by
  `M-x buffer.undo`. The primitive they need now exists and is proven, so
  the remaining work is adoption plus a streaming-friendly variant
  (`*compilation*` appends rather than replacing wholesale).

  **The CRDT half is closed too** (review round 3). Clearing the v0.1
  stacks proves nothing in CRDT mode, where they are bypassed entirely and
  the history lives in loro's `UndoManager`. `read_only` would stop that
  history being *replayed* but not *retained* — a panel refreshed on a
  timer still grows without bound, which is the condition the contract
  says it eliminates. `UndoManager` exposes no `clear`, but it needs none:
  a manager records only what happens after it is constructed, which
  `CrdtState::from_bytes` already relies on to keep the seed insert out of
  undo. `CrdtState::clear_undo_history` rebinds a fresh manager to the same
  doc, and `set_generated_contents` clears whichever history the buffer
  actually has.

## Acceptance

### Stage 1 — `terminal-config`

1. `pmacs.terminal.profiles` accepts a strict spec table per name and rejects
   unknown fields before anything is spawned, matching `terminal.open`'s
   existing transactional contract.
2. `terminal.default-profile` naming an unknown profile fails at open with an
   error that **lists the known profile names**, and creates no buffer,
   session, or process. An explicitly passed unknown `profile` fails the same
   way **even when `terminal.default-profile` is valid** (Q#TC3a).
2a. That diagnostic is **total over a malformed profiles table** (review round
   1). `pmacs.terminal.profiles` is a raw user table, so listing its names must
   not assume its keys are comparable and rendering a requested name must not
   assume it is a string: a table holding both a string and a numeric key made
   `table.sort` raise `attempt to compare number with string` *on the
   unknown-profile path*, replacing the exact error being asked for, and `%q`
   raises on a non-string `profile` argument. Both are partial functions
   applied to user input on a diagnostic path — the failure class is
   "the error reporter is the thing that fails".
3. Field-by-field resolution follows Q#TC3a: explicit open field beats profile
   field beats scalar setting beats `$SHELL`. `env` **merges**, with explicit
   entries overriding profile entries of the same name.
4. `""` in `terminal.default-profile` means "no profile" and is
   indistinguishable from unset (Q#TC2a).
5. `terminal.scrollback-rows` takes effect for a terminal opened without an
   explicit `scrollback_rows`; an explicit per-open value overrides it; values
   outside `0 ..= 4_000_000` are rejected by the registry rather than by the
   core, and `0` is accepted as "retain no history".
6. `terminal.escape-key` changes which chord escapes to the editor, observed
   through the **real dispatch path**, not by calling the predicate directly.
7. **Two terminals with different buffer-local escape keys each honor their
   own**, with no setting written in between (Q#TC4/Q#TC2b). Driven as
   **A→B→A**, asserting both directions. This is the pin an epoch-only cache
   fails.
8. Across that same **A→B→A** switch with no setting written, the parse count
   does **not** increase after each terminal's first keystroke (Q#TC4c) —
   pinned by counting parses, not by timing. This is the pin a single
   last-entry cache fails while still satisfying 7.
8a. A terminal's cache does not outlive it: killing a terminal and opening a
   new one does not serve the dead terminal's chord, and no per-terminal cache
   entry survives its session (Q#TC4c). This is the pin an unpurged
   editor-side map fails.
9. With `terminal.escape-key = "C-x"`: `C-x C-x` sends **Ctrl-X** to the child,
   and an ordinary `C-c` reaches the child as `0x03` like any other unescaped
   key (Q#TC4b). Bite: against the hardcoded `&[0x03]`, the first assertion
   fails.
10. An unparseable `terminal.escape-key` falls back to `C-c`, reports through
    `EditorCore::status`, and leaves the terminal usable (Q#TC4a). Bite: with
    the fallback removed, the terminal becomes unescapable.
10a. "Reports once" is once per terminal per effective invalid value
    (Q#TC4c): an **A→B→A** switch with the same invalid value reports **once**,
    while changing it to a *different* invalid value reports again. The report
    count is asserted, not the message text.
11. The terminal opening keybinding invokes the existing command, and is
    verified to have shadowed nothing (Q#TC10).
12. Existing `terminal` invocations and every existing terminal test behave
    identically with no settings defined and no profiles registered.

### Stage 2 — `terminal-copy-mode`

13. `terminal.copy-mode` produces a read-only buffer whose text is
    byte-identical to serializing the full retained range through the existing
    copy path (Q#TC7) — pinned against the serializer, so the two cannot drift.
14. Soft wraps, hard rows, wide glyphs, combining clusters, and trailing
    default blanks appear in the snapshot exactly as Stage 2 criterion 21 pins
    them for selection copy.
15. isearch over the snapshot finds content that is **only in scrollback**
    (scrolled off the visible screen), with no change to `src/search.rs` (B1).
16c. **Undo cannot empty the snapshot, by chord OR by command** (review
    round 2). `Buffer::undo` bypasses the intercept chain entirely, so the
    snapshot must be `read_only` at the rope. Pinning only the chords would
    be a false pass: `M-x buffer.undo` and the menu reach the command with
    no keymap involved, which is why `*compilation*`'s chord-rebinding idiom
    does not close this. Pinned through **`invoke_interactive`**, the real
    M-x path, plus the chord, plus redo — and paired with an assertion that
    the owner's own refresh still works, since that is what plain
    `read_only` would have broken.
16d. **A generated write reaches the window, not just the rope** (review
    round 3). `set_generated_contents` returns one whole-buffer `Replace`
    and its binding fans it out; swallowing it leaves a displaying
    window's `TextView` line index describing the *previous* contents.
    Pinned by **painting** — a shrinking write, so the stale offsets point
    past the buffer end and the next render trips
    `assertion failed: end <= self.len()` in `src/rope.rs`, which is the
    reported crash rather than merely stale pixels. Driven through the Lua
    binding copy mode itself calls, so it covers every future owner of the
    primitive.
16e. **The same write is queued for replica mirrors** (review round 3,
    CRDT half). The dropped fan-out also skipped
    `queue_daemon_origin_crdt_op`, so a replica's mirror never imports the
    owner's write and its optimistic edits are generated against content
    already replaced. Pinned through the real copy-mode refresh on an
    upgraded snapshot. `crdt`-gated, therefore dark in CI — 16d is the half
    that actually runs there.
16. **Ungated, runs in CI:** focusing the snapshot buffer makes
    `dispatch_idle_for` report **false**. This is the whole mechanism Q#TC6a
    depends on, it needs no CRDT, and it fails the moment
    `set_round_trip_input` is dropped — so the load-bearing regression is
    caught by the default configuration rather than only by a `crdt`-gated
    test that CI never compiles.
17. **Through a semantic frontend** (this one does need CRDT): keys typed in
    the snapshot buffer reach ordinary dispatch and never the child, and
    **neither the daemon buffer nor the frontend's mirror is mutated**
    (Q#TC6a). Bite: with `set_round_trip_input` removed, the frontend
    applies the edit **optimistically to its own mirror** and emits the op;
    the mirror now shows text the user was told is read-only. The daemon
    refuses the op at `ensure_writable()` — `set_generated_contents` leaves
    `read_only` asserted — so the two copies **diverge**, and the local
    mirror is the one the user is looking at.

    **This bite changed in review round 3, and the direction matters.**
    Rounds 1-2 specified it as "mutates *both sides*, silently, with no
    divergence to notice" — true when nothing set `read_only` from Lua,
    and false now. The eventual real-GPU test must assert **mirror
    mutation plus daemon refusal**, not silent agreement; written the old
    way it would look for a daemon-side edit that can no longer happen and
    pass for the wrong reason. That the daemon now holds is exactly why
    round-trip input is still load-bearing rather than redundant: a
    refusal protects the daemon's copy and does nothing for the replica's.

    **NOT PINNED as specified, deliberately, and this is the one gap in
    Stage 2.** A faithful test has to drive the *real* `pmacs-gpu` binary:
    the optimistic apply lives only in `pmacs-gpu/src/main.rs`
    (`optimistic_crdt_insert` / `optimistic_insert_text`), and the headless
    `SemanticClient` the other semantic tests use has no optimistic path at
    all, so it cannot produce the op whose absence is the claim. That means
    building on the `a37` foundation — which is `crdt`-gated so CI never
    compiles it, **returns `ok` without running** when `pmacs-gpu` is absent
    from the target directory, and is load-sensitive enough to pass and fail
    at the same commit twenty minutes apart. A second test on that footing
    would add the appearance of coverage without the substance.

    What IS pinned instead, ungated and in CI: acceptance 16 asserts the
    guard is armed (`dispatch_idle` false while the snapshot is focused, so
    no replica can apply optimistically or emit), and acceptance 16b asserts
    the buffer is `is_read_only()` **true** at the rope, so an op that did
    arrive at the daemon would be refused by `ensure_writable()` rather
    than applied. (Rounds 1-2 asserted **false** here, documenting the
    hazard; round 2 closed it, and the assertion was flipped with it.
    That does not make 17 redundant — a daemon-side refusal cannot stop a
    replica mutating its own mirror, which is precisely what
    `set_round_trip_input` is for.) Together those cover both halves of
    Q#TC6a's *mechanism*. What remains unproven is only the end-to-end wire
    behaviour of a real GPU frontend, and it stays an explicit obligation of
    the CI `crdt`-coverage lane rather than being quietly dropped.
18. Re-invoking against the same terminal refreshes in place; the buffer count
    does not grow (Q#TC8). Killing the snapshot leaves the terminal running;
    killing the terminal removes the snapshot.

    **The refresh half must be observed by CONTENT, not by buffer count**
    (review round 1). Counting buffers, or comparing a quiet terminal's
    snapshot against itself, passes with `render_snapshot` replaced by a
    no-op. The child is `exec cat`, so the test types a marker into the
    focused terminal, requires it **absent** from the existing snapshot, and
    only then re-invokes — the "advance the world" discipline.
18a. **A foreign buffer carrying the snapshot's name is never adopted.**
    `pmacs.buffer.create` accepts any caller-chosen name, and snapshot writes
    use `bypass_intercept`, so found-by-name adoption silently overwrites a
    user's data — reproduced in review round 1 as "do not clobber" becoming
    23 newlines. Ownership means **"in copy mode's own handle table"**, which
    is dired's F7 rule; a taken name yields a `<2>` variant.
18b. **Snapshot identity is the terminal BUFFER, not its name.**
    `TerminalManager::open` uniquifies only the *derived* name — an explicit
    `name = ...` is inserted verbatim — so two valid terminals can share one.
    A name-keyed table hands them a single snapshot: the second invocation
    retargets it, `q` returns to the wrong terminal, and killing either one
    removes the shared buffer. Keyed instead by comparing buffer handles in
    an array, because `BufferIdLua` implements `__eq` but each wrapper is a
    distinct table key — comparison works, hashing does not.
19. `C-t` in a terminal buffer (physically `C-c C-t`) enters copy mode; `g`
    refreshes the snapshot from the live terminal and `q` returns to the source
    terminal (Q#TC8a).
20. The live terminal's own keys are unchanged while a snapshot exists
    (Q#TC9), and the terminal keeps following its tail.

    **Tail-following must be read through the registered VIEW.** Review
    round 1: `TerminalManager::snapshot(buffer_id)` is context-free and
    always returns the live screen, so it reports "at the tail" even for a
    view forced to the oldest retained row — falsified by doing exactly
    that and watching the assertion still pass. `snapshot_for_view`'s
    `at_bottom` plus its projected cells are the only observables that can
    tell the two apart.
21. The dispatch-shadow count is **unchanged at six** — pinned by asserting
    `describe-key` reports the truth for the snapshot buffer's `g` and `q`,
    which is the observable difference between the buffer-local idiom and a
    shadow.

## Coherence impact (`COHERENCE.md` §20)

- **§6 Interaction islands — this arc deliberately adds none.** It is the
  first modal-feeling terminal feature that resolves to the buffer-local
  keymap idiom §6 identifies as correct, rather than a seventh rung on the
  precedence ladder. The shadow count stays at six and `describe-key` stays
  truthful (acceptance 21). Worth recording in §6 as a worked example that the
  idiom scales to a case that looks modal.
- **§11 Configuration as typed, layered data** — the terminal gains its first
  settings, and produces a second blocked adopter for **two** distinct registry
  deferrals: the missing table-valued kind (profiles) and the missing
  `scope = "global"` flag (the **two open-time settings** —
  `terminal.escape-key` deliberately supports buffer-locals, so only
  `default-profile` and `scrollback-rows` want an enforcement the registry
  cannot express). §11's ground truth should
  record both, because the argument for prioritizing them is now cumulative
  rather than hypothetical.
- **§2 golden journey, step 8 — partially closed here.** Stage 1 carries the
  **terminal opening keybinding** that Priority 1 explicitly names (Q#TC10),
  which is the larger half of "works but undiscoverable". Close/kill stays with
  the panel work so the entry and exit points are designed together, and is
  named in Deferred rather than silently skipped.
- **§5 Unify discovery** — the new commands must carry real descriptions so
  M-x rows are useful; no new introspection surface is added.
- No background-work attribution change; no new activity view; no protocol
  change.

## Verification plan

Full gate suite per `CLAUDE.md` for each PR separately, plus:

- **The touched terminal suites in BOTH configurations** — default and
  `--features crdt` — not only the CRDT one. `vterm_stage1_acceptance`,
  `vterm_stage2_acceptance`, and `vterm_stage3_acceptance` all carry tests in
  each, and acceptance 12 is a claim about the default configuration too.
- `cargo test --test config_registry_acceptance` for the new settings.
- New suites: `tests/terminal_config_acceptance.rs` (Stage 1) and
  `tests/terminal_copy_mode_acceptance.rs` (Stage 2).
- Every behavioral claim bite-verified. The bites that matter most:
  **7/8/8a** — three pins that fail against three *different* wrong cache
  implementations (epoch-only key, single last-entry, unpurged map), which is
  why one pin was not enough; **9** (a hardcoded `0x03` makes the configured
  chord unreachable); **10** (its failure mode is a terminal nobody can
  escape); and **16** (a read-only buffer whose replica mirror accepts an
  edit the user is then looking at — 17's daemon half was closed in review
  round 2, and its bite restated in round 3).
- **The observation seams the cache pins need are `escape_parses` (how often)
  and `escape_caches` (how many are still held).** Neither is inferable from
  behavior: for a *valid* setting a correct per-session cache and a leaking
  editor-side map produce identical keystroke results, and both leave the
  session count draining normally. Review round 1 caught 8a asserting the
  session count instead — which the unpurged-map bite passes, since a map with
  no purge hook leaks *while* sessions drain. A lifecycle claim needs a
  lifecycle observable; the count of live sessions is not one.
- **Criterion 5 must open a real terminal and read back retained history.**
  Round 1 caught it asserting a registry round-trip instead, which is a test of
  the registry: it stays green with the setting's only consumer deleted. The
  same shape to watch for anywhere — *asserting that a value was stored is not
  asserting that anything reads it*.
- **Do not gate the new suites on `#[cfg(feature = "crdt")]` unless a test
  genuinely needs CRDT.** CI never enables that feature, so a suite gated that
  way is written and then never run — 264 tests are currently dark for exactly
  this reason. That measurement and its lane live on **PR #168**, which is open
  and unmerged; it is not yet in `docs/active-work.md` on `main`.
  Acceptance 17 does need a semantic frontend, so that one test is gated — but
  acceptance 16 pins the same mechanism ungated, so the regression is caught in
  CI regardless. That pairing is the pattern to reuse whenever a claim's
  end-to-end proof needs CRDT.
