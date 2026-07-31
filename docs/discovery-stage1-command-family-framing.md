# Discovery Stage 1 — the describe/list command family

**Status: framing, rev 3 — awaiting review round 3.**
**Serves `COHERENCE.md` §5 (unify discoverability), §1.1 (substrate
without surface), §20 Priority 4.**

## 0. Revision history

- rev 3 (2026-07-31) — review round 2. Two blocking, two major; all four
  accepted.
  - **The ledger lane still said revision 1 and kept refuted claims.**
    Rev 2's ledger edit **aborted on a failed assertion before writing**,
    so only one paragraph of it landed while the commit message reported
    all of it. The lane is rewritten from scratch and the result was
    verified by re-reading the file, not inferred from an exit code.
    *(Second occurrence of this failure mode in this project — an
    assert-then-write block discards every earlier edit in the block.)*
  - **`names_from` does not exist.** Rev 2's completion source called a
    helper nobody has written, over `config.list()`'s descriptor
    **tables** where `Custom` wants a sequence of **strings** — the
    prompt would have raised on an undefined global the first time it
    opened. §3.2 now specifies the mapper.
  - **`*help*` has no read-only intercept.** Rev 2's §3.4 claimed one.
    `show_help_text` writes with plain `delete`/`insert` and #205
    recorded that this mechanism has not adopted the generated-buffer
    write invariant. §3.4 now names the four policies that are actually
    shared.
  - **The naming was underspecified.** The nine-command table contains
    no `help.describe-command`, so calling the `editor.*` commands
    "aliases-by-retention" was wrong on both halves. They are now stated
    as explicit exceptions, and Q#D2 is sharpened to the two ways out.
- rev 2 (2026-07-31) — review round 1. Two blocking, two major; all four
  accepted, all four verified in the code first.
  - **Completion does not close the free-text hole**, and rev 1 said it
    did. `resolve_accepted_value` (`src/minibuffer.rs:564-575`) returns
    the **literal typed text** whenever `session.selected` is `None`, so
    a non-matching typo still reaches `on_accept` — and a fuzzy match
    can instead select a *different* setting silently. Completion here
    is **assistance**, not validation. §3.2 is reframed and acceptance 5
    now pins what actually happens; closed-set acceptance is named as
    Rust work in §5.
  - **`invoke_interactive` is not the M-x path** — the exact error #205
    corrected, repeated one PR later. It rotates the interactive-command
    boundary and calls the body (`mod.rs:6097-6110`); it does not open a
    palette. The path is **dispatch `M-x` → `editor.execute-command` →
    accept a command → `invoke_interactive`**. Acceptances 1, 7 and 9
    are rewritten around it, including the **second** prompt for
    commands that take an argument.
  - **`_show_help(text)` is an output sink, not a migration seam.**
    `src/help.rs` has semantic renderers for command / key / buffer /
    mode / hook / view and **none** for settings, lists or apropos, so
    once Lua has flattened those to text a later migration still has to
    change each command's subject-specific logic. §3.4's claim is
    narrowed to what is true — one owner for Lua `*help*` writes — and
    §3.4a states the structure that makes the future Rust work
    per-subject rather than per-call-site.
  - **Ground-truth and counting errors.** §2.2 listed eight missing
    commands and omitted `list-settings` while §3.1 listed nine; §2.5's
    "two sites into ten" should have been eleven; `pmacs.keymap.lookup`
    does **not** return `description` (it calls `key_info_table` with
    `cmd = None`, `mod.rs:6938-6940`); and the `has_predicate` /
    raw-predicate sites rev 1 cited are **`MenuItem` fields**, not
    `Command.predicate`. The predicate conclusion survives on correct
    evidence (§2.4).
- rev 1 (2026-07-31) — first framing. Scouted against `githubsucks/main`
  @ `54a092e` (Journey Stage 1b-3, #205).

## 1. Why this, and why now

`COHERENCE.md` §20 Priority 4 calls unified discovery **"almost pure
wiring — the best payoff-per-effort in this document"**, and §5 grades
it *"substrate without surface — the sharpest instance of §1.1"*.

Journey Stage 1b-3 (#205) just landed `M-x help` and documented it as
**the root of this family**: *"when the discovery arc adds `help.keys`
and friends, `help` stays the index they are reached from, so no rename
is owed."* This stage is that family. It also inherits 1b-3's deferred
question — the help prefix — with the constraint already recorded (§6).

Two journey steps are graded *works but undiscoverable* and move here
without any new machinery: step 7 (symbol search — `M-.`/`M-?`/`C-c o`
bound but "advertised nowhere") and step 11 (`*workers*` — "no
keybinding, no indicator").

## 2. Ground truth

Read in the tree at `54a092e`. **The substrate is already there**; this
stage adds no Rust.

### 2.1 What Lua can already ask

| Surface | Returns |
|---|---|
| `pmacs.command.list()` | every command name |
| `pmacs.describe.command(name)` | description, source, **`key_bindings`** (where-is, computed on demand) |
| `pmacs.describe.key(seq)` | resolved against the **active buffer + major mode** |
| `pmacs.describe.{buffer,view,mode,hook}` | structured tables |
| `pmacs.keymap.list()` | `{ sequence, command, scope }` for **every** binding, via `KeymapStack::iter_all` |
| `pmacs.keymap.lookup(seq)` | `{ sequence, command, scope, source }` — **not** `description`: it calls `key_info_table` with `cmd = None` (`mod.rs:6938-6940`), so the description arm never fires |
| `pmacs.config.list()` / `pmacs.config.describe(name, buf)` | full typed descriptors |

Every one of the commands in §3 is a rendering of data already
reachable, and `CompletionSource::Custom(Function)` means even the
prompts need no new Rust (§3.2). **This stage adds no Rust at all.**

That is what "pure wiring" means here, and it is worth stating precisely
so the stage is not oversold: **the work is surface, and the risk is in
what the surface leaves out.**

### 2.2 What exists as a command today

`editor.describe-command`, `editor.describe-setting`,
`editor.describe-instance[-buffer]`, `editor.list-buffers`,
`editor.list-workers`, and `help` (#205).

**Missing entirely — nine, matching §3.1 exactly:** describe-key,
describe-mode, describe-hook, describe-buffer, where-is, list-commands,
list-keybindings, **list-settings**, apropos. (Rev 1 listed eight here
and nine in §3.1.)

### 2.3 `describe-setting` prompts free-text, deliberately

```lua
-- builtin/commands/default.lua
pmacs.minibuffer.read {
  prompt = "Describe setting: ",
  history = "command",          -- note: no `source`
  on_accept = function(name) … end,
```

A typo yields a status-line error. `describe-command` **does** pass
`source = "commands"`. The asymmetry is real and this stage closes it
(§3.2) — but note *why* it was skipped: dired's `C-x d` records that a
completion source makes RET-on-empty accept whatever sorts first, and a
selected candidate shadows typed text. That is a genuine trade, not an
oversight, so §3.2 says what changes about it.

### 2.4 `Command.predicate` is stored, exposed, and never evaluated

`predicate: Option<Function>` (`src/command.rs:79`) is read in exactly
**two** places: `src/help.rs:76` — inside the orphaned renderer — and one
assertion past `#[cfg(test)]`. No production call site *evaluates* it:
not `invoke`, not `invoke_interactive`, not dispatch, not M-x filtering,
not the menu. Its doc comment describes palette gray-out that never
shipped (§24 already logs this).

*(Rev 1 cited `mod.rs:6204` / `:6257` as evidence. Those are
**`MenuItem`** fields — `item.label`, `item.group`, `item.order`,
`item.predicate` — a different type with its own predicate. The
conclusion held; the evidence did not.)*

**This stage does not evaluate it either** (§5), because doing so makes
commands stop being invocable — a behaviour change needing its own
decision about what "unavailable" means at each call site.

### 2.5 The help layer is duplicated, and this stage would deepen it

`src/help.rs` has `render_command` / `render_key` / `render_buffer` /
`render_mode` / `render_hook` / `render_view` plus link resolution, and
is **orphaned** — the reachable renderer is the Lua `show_help_text`,
which renders *less* (no source, no scope).

**Nine new commands each calling `show_help_text` would turn a two-site
migration into an eleven-site one.** §3.4 is the answer, and it is the
most consequential decision in this framing — but §3.4 is careful about
what it can actually promise, because `src/help.rs` has renderers for
command / key / buffer / mode / hook / view and **none for settings,
lists, or apropos**.

## 3. Design

### 3.1 The family

Nine commands, all rendering existing data:

| Command | Reads |
|---|---|
| `describe-key` | `pmacs.describe.key` — prompts for a chord, resolved against the active buffer + mode |
| `describe-mode` | `pmacs.describe.mode` for the active buffer |
| `describe-buffer` | `pmacs.describe.buffer` |
| `describe-hook` | `pmacs.describe.hook` |
| `where-is` | `describe.command(name).key_bindings` |
| `list-commands` | `command.list()` + each description |
| `list-keybindings` | `keymap.list()`, grouped by scope |
| `apropos` | substring match over **names and descriptions** |
| `list-settings` | `config.list()` |

**Naming, stated exactly.** The nine **new** commands are `help.*`
(`help.describe-key`, `help.where-is`, …). #205 established `help` as
the index, so the family it indexes shares its prefix.

**The two pre-existing commands are intentional exceptions, not
aliases.** `editor.describe-command` and `editor.describe-setting` keep
their names and are **not** duplicated under `help.*` — rev 1 called
them "aliases-by-retention", which was wrong twice over: nothing
forwards to them, and the nine-command table never contained a
`help.describe-command` for them to be aliases *of*.

So the shipped surface is nine `help.*` commands plus two `editor.*`
ones covering the same family. **That asymmetry is a wart**, and it is
Q#D2's whole subject: either the two get `help.*` names with the old
ones forwarding (eleven commands, two forwarders), or the family stays
`editor.*` throughout (nine commands, no new prefix). This framing does
not decide it, because a rename that this arc's later stages would
revisit is worse than an explicit exception recorded for one round.

### 3.2 `describe-setting` gains completion — which is assistance, not validation

Rev 1 claimed a completion source means "a typo cannot reach
`on_accept`". **It does not.**

```rust
// src/minibuffer.rs:564-575
fn resolve_accepted_value(session: &MinibufferSession, typed: &str) -> String {
    if matches!(session.source, CompletionSource::None) { return typed.to_owned(); }
    if let Some(idx) = session.selected
        && let Some(cand) = session.candidates.get(idx) { return cand.clone(); }
    typed.to_owned()          // <-- no selection: the literal typed text
}
```

So with a source attached there are **two** outcomes rev 1 conflated:

- **No candidate selected** (a typo matching nothing) → the literal text
  reaches `on_accept`, exactly as today, and the existing
  `no such setting: <name>` status path handles it.
- **A candidate selected** → that candidate wins over the typed text. On
  a fuzzy source a near-miss can therefore **silently describe a
  different setting** — a new failure mode, milder than the old one but
  not nothing.

What the source genuinely buys is *assistance*: the closed set is
visible and reachable by completion instead of having to be known. That
is worth doing and is what §3.1 promises. **Closed-set acceptance
semantics — "refuse a value that is not a candidate" — is Rust work**
(`resolve_accepted_value` and a per-session flag) and is deferred to
§5 rather than smuggled in as a side effect.

**Still no Rust in this stage**, but the source needs a mapper.
`parse_completion_source` (`mod.rs:14145-14165`) accepts `none` /
`commands` / `buffers` / `files` **and a Lua `Function`** →
`CompletionSource::Custom`, and `Custom` consumes a **sequence of
strings**. `pmacs.config.list()` returns descriptor **tables**, so
handing it over directly would not typecheck — and rev 1 wrote
`names_from(...)`, **a helper that does not exist**; the prompt would
have raised on an undefined global the first time it opened.

The mapper is three lines and belongs to this stage:

```lua
source = function()
  local names = {}
  for _, d in ipairs(pmacs.config.list()) do names[#names + 1] = d.name end
  table.sort(names)
  return names
end,
```

Sorted because `Custom` candidates are presented in the order returned,
and `config.list()`'s order is registration order — which is neither
stable across a config edit nor useful to a reader.

### 3.3 `M-x help` becomes the index

#205 shipped `help` as a static cheat sheet. It now lists the family
above, so the arc's own promise — *`help` stays the index they are
reached from* — is kept rather than merely restated.

### 3.4 One owner for Lua `*help*` writes — the honest version of the claim

Every new command renders through **`pmacs.editor._show_help`** — the
seam #205 added — and **not** by calling `show_help_text` directly or
building its own buffer.

**What that buys, precisely: one owner for `*help*` writes.** Four
shared policies get decided in one place instead of eleven:

- **reuse-by-name** — a single `*help*` buffer found by name and reused
  across invocations;
- **wholesale replacement** — `buf:delete(0, len)` then `buf:insert(0,
  text)`, never a diff, because `*help*` is reflowed per subject;
- the **`q` binding**, rebound per fresh buffer;
- the **foreign-`*help*` hazard** — found-by-name is not ownership, so a
  user's buffer of that name is cleared.

**`*help*` is ordinary editable content.** Rev 1 wrote "the read-only
intercept"; there isn't one. `show_help_text` writes with plain
`delete`/`insert`, the buffer keeps its undo history, and #205 already
recorded that this mechanism has **not** adopted the generated-buffer
write invariant. Saying otherwise would have had this stage claim a
guarantee it does not provide — and the fourth policy above is exactly
the hazard that a read-only intercept would have mitigated and does
not.

**What it does not buy, and rev 1 claimed it did:** a one-site migration
to `src/help.rs`. That layer has semantic renderers for **command, key,
buffer, mode, hook and view** — and **none for settings, lists, or
apropos**. `_show_help` takes *already-flattened text*, so by the time a
subject reaches it the structure a richer renderer would need is gone.
A later migration still has to change each command's subject-specific
logic; the seam saves the plumbing, not the semantics.

### 3.4a The structure that makes the future work per-subject

So the funnel is paired with a shape that keeps the semantics
addressable: each command's rendering is a **named per-subject
function** returning text — `render_key_help(info)`,
`render_settings_list(rows)` — and the command body does nothing but
call it and hand the result to `_show_help`.

Then the future help-unification stage is: replace each named renderer
whose subject `src/help.rs` already covers (key, mode, hook, buffer),
and **write new Rust renderers for the three subjects it does not
cover** (settings, lists, apropos). That work is enumerated here rather
than discovered later, which is the actual deliverable of this section.

**Corollary this stage must respect:** where the Lua renderer is poorer
than `src/help.rs` for a subject it *does* cover (no source, no scope),
the new commands render the poorer form rather than inventing a third
shape.

## 4. Acceptance

**N** = new behaviour, must fail on full revert. **P** = preservation,
falsified by a named mutation.

### 4.0 The M-x path, stated once

Rev 1 said "driven through `pmacs.command.invoke_interactive` (the M-x
path)". **That is not the M-x path**, and #205 established as much one
PR earlier. `invoke_interactive` rotates the interactive-command
boundary and calls the body (`mod.rs:6097-6110`); it opens no palette.

Every pin below that claims to exercise a command as a user does drives:

```
dispatch M-x
  → editor.execute-command opens the minibuffer (source = "commands")
  → type the command name
  → assert pmacs.minibuffer.selected() == "<name>"     -- BEFORE RET
  → dispatch RET                                        -- accept
  → editor.execute-command calls invoke_interactive
```

The pre-RET assertion is not decoration: `accept()` does
`session.take()`, so afterwards nothing about the accepted value
survives, and a selected candidate shadows typed text.

**Commands that take an argument open a SECOND prompt** (`where-is`,
`describe-key`, `describe-hook`, `describe-setting`, `apropos`). Those
pins drive that prompt too, and assert against it with the same
pre-accept discipline. A pin that stops after the first RET has tested
the palette, not the command.

### 4.1 Pins

1. **N — each of the nine commands runs from M-x and renders content.**
   Through §4.0's full path, including the second prompt where the
   command takes one. Asserts **content produced** in `*help*`.
2. **N — `where-is` agrees with the keymap.** Bind a command to a known
   chord, then assert `where-is` reports that chord. Falsified by
   rendering a static string.
3. **N — `list-keybindings` covers every binding `keymap.list()`
   reports.** A property over the data, with a non-empty precondition so
   the loop cannot be vacuous.
4. **N — `apropos` matches descriptions, not only names.** Search a word
   that appears in exactly one command's *description* and in no command
   *name*. This is what distinguishes apropos from a name filter.
5. **N — `describe-setting` completes, and a non-matching typo still
   reaches the existing error path.** Two assertions, because §3.2 has
   two outcomes: (a) typing a real setting's prefix makes it the
   selected candidate, and accepting describes it; (b) typing a string
   that matches **nothing** leaves `selected()` nil, and accepting
   produces the `no such setting` status — **not** a described setting.
   *Rev 1 asserted a typo "cannot reach `on_accept`", which
   `resolve_accepted_value` contradicts.*
6. **N — `M-x help` lists the family.** A property over the family list,
   so adding a tenth command without indexing it fails.
7. **P — every new command's `*help*` write goes through
   `_show_help`.** Replace that function with a counting stub, drive all
   nine through §4.0's path, and assert the count equals nine. Pins
   §3.4's *actual* claim — one owner for `*help*` writes — rather than
   the migration claim rev 1 overstated.
8. **P — the existing describe/list commands still work.**
   `editor.describe-command`, `editor.describe-setting`,
   `editor.list-buffers`, `editor.list-workers` unchanged. Targeted
   mutation: renaming rather than retaining them.
9. **P — no command's predicate is evaluated.** Register a command whose
   predicate **raises**, then run it through §4.0's full M-x path and
   assert it **runs**. Pins §2.4's deliberate non-change, so a stage
   that starts evaluating predicates must change this pin knowingly.
   Driven through the palette, not `invoke_interactive` directly —
   otherwise it would pass even if M-x grew predicate filtering.

## 5. Deferred, each with its reason

- **Richer M-x rows.** `MinibufferPrompt.candidates` is `Vec<String>` on
  the wire; `CompletionPopupRow` already carries `kind`/`detail`, so the
  pattern exists — but it is a **protocol change** and belongs to its
  own stage with a version bump.
- **`Command` gaining title/category/aliases/flags/arg-schema.** A Rust
  type change with ~147 definition sites; own stage.
- **Predicate evaluation** (§2.4) — a behaviour change.
- **Help-layer unification.** The reason §3.4 exists; own stage, made
  cheaper by this one rather than harder.
- **A help prefix key.** #205 recorded the constraint in §18: `C-h`
  deletes a word because non-kitty terminals cannot disambiguate
  Ctrl+Backspace from Ctrl+H. This stage adds **no keybindings at all**,
  so the prefix decision is taken once, for the whole family, by the
  stage that can weigh `F1` / `C-c ?` / a rebind together.
- **Settings value provenance** (§11) — `describe-setting` will still
  answer "who set this?" with the *definition* site.
- **Closed-set acceptance semantics** (§3.2). Making a prompt *refuse* a
  value that is not a candidate needs `resolve_accepted_value` and a
  per-session flag — Rust, and a change every existing prompt with a
  source would inherit. Named here because rev 1 claimed this stage
  delivered it as a side effect of adding completion.

## 6. Coherence impact

- **Journey steps touched:** 7 and 11, both graded *works but
  undiscoverable*; this makes the existing bindings and the `*workers*`
  view reachable by name. No step's grade flips on this stage alone —
  both also want keybindings, which §5 defers.
- **§5's grade moves** from "substrate without surface" toward Partial:
  the surface exists for commands, keys, modes, hooks and settings; it
  does not yet exist for packages or workers (§13, §9). **That is a
  §25 obligation on the landing PR.**
- **Interaction islands: none added.** Everything renders into the one
  `*help*` buffer through the one seam (§3.4) — this stage's main job
  is to *avoid* becoming the next island.
- **Config registry: not adopted.** Nothing here is a tunable.
- **Background-work attribution:** unchanged.

## 7. Questions

- **Q#D2 — `help.*` prefix, or keep `editor.*`?** §3.1 ships nine
  `help.*` commands and leaves `editor.describe-command` /
  `editor.describe-setting` as **exceptions**, so the family is split
  across two prefixes. Resolve it one of two ways: give those two
  `help.*` names with the `editor.*` ones **forwarding** (eleven
  commands, two forwarders, one canonical prefix), or drop `help.*` and
  keep the family `editor.*` throughout (nine commands, no new prefix).
  A split surface is the one outcome that should not survive review.
- **Q#D3 — should `apropos` fuzzy-match?** `fuzzy_score` exists
  (`src/minibuffer.rs:637-666`) and M-x already uses it. Substring is
  more predictable for a search command; fuzzy is more consistent with
  M-x. Recommended: substring, and say so in the command description.

## 8. Ledger

Branch `discovery-stage1-commands`, worktree `../pmacs-p4-discovery`,
based on `githubsucks/main` @ `54a092e`. Framing only; no code, no PR.

A sibling lane (`test-ambient-isolation-impl`) is in flight in another
worktree and touches `src/editor.rs` and `tests/`. This lane is
`builtin/` plus its own suite, so the surfaces are disjoint; integrate
`main` late regardless.

```sh
git fetch githubsucks
git worktree add ../pmacs-p4-discovery \
  -b discovery-stage1-commands \
  githubsucks/discovery-stage1-commands
```
