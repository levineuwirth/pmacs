# Discovery Stage 1 — the describe/list command family

**Status: framing, rev 1 — awaiting approval.**
**Serves `COHERENCE.md` §5 (unify discoverability), §1.1 (substrate
without surface), §20 Priority 4.**

## 0. Revision history

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
| `pmacs.keymap.lookup(seq)` | `{ sequence, command, scope, source, description }` |
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

**Missing entirely:** describe-key, describe-mode, describe-hook,
describe-buffer, where-is, list-commands, list-keybindings, apropos.

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

`predicate: Option<Function>` (`src/command.rs:79`) is surfaced as
`has_predicate` (`mod.rs:6204`) and handed out whole (`:6257`). No call
site *evaluates* it — not `invoke`, not `invoke_interactive`, not
dispatch, not M-x filtering, not the menu. Its doc comment describes
palette gray-out that never shipped (§24 already logs this).

**This stage does not evaluate it either** (§5), because doing so makes
commands stop being invocable — a behaviour change needing its own
decision about what "unavailable" means at each call site.

### 2.5 The help layer is duplicated, and this stage would deepen it

`src/help.rs` has `render_command` / `render_key` / `render_buffer` /
`render_mode` / `render_hook` / `render_view` plus link resolution, and
is **orphaned** — the reachable renderer is the Lua `show_help_text`,
which renders *less* (no source, no scope).

**A family of eight new commands each calling `show_help_text` turns a
two-site migration into a ten-site one.** §3.4 is the answer to that,
and it is the most consequential decision in this framing.

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

**Naming.** They are `help.*`-prefixed (`help.describe-key`,
`help.where-is`, …) with the existing `editor.describe-*` kept as
aliases-by-retention, not renamed. #205 established `help` as the index;
this makes the family's identity match. Existing names are not removed
— `editor.describe-command` is bound in muscle memory and in
`docs/keybindings.md`.

### 3.2 `describe-setting` gains a completion source

It gains a completion source so a typo cannot reach `on_accept`. The
dired trade (§2.3) applies but resolves differently here: dired's prompt
takes an arbitrary *path*, where a shadowing candidate silently opens
the wrong directory; this prompt takes a name **from a closed set**, so
a candidate is what the user wants and free text is the failure mode.

**No Rust is needed for it.** `parse_completion_source`
(`src/lua_bindings/mod.rs:14145-14165`) accepts the strings `none` /
`commands` / `buffers` / `files` **and a Lua `Function`**, which becomes
`CompletionSource::Custom` and is called for candidates. So the source
is `function() return names_from(pmacs.config.list()) end` — the stage
stays entirely Lua, and `Custom` is the general escape hatch every other
command in §3.1 can use if it needs one.

### 3.3 `M-x help` becomes the index

#205 shipped `help` as a static cheat sheet. It now lists the family
above, so the arc's own promise — *`help` stays the index they are
reached from* — is kept rather than merely restated.

### 3.4 One rendering seam, so the later unification is one site

Every new command renders through **`pmacs.editor._show_help`** — the
seam #205 added — and **not** by calling `show_help_text` or building
its own buffer.

That is the whole mitigation for §2.5: when the help-layer unification
stage arrives, migrating to `src/help.rs`'s richer renderer is a change
at **one** Lua function, not at ten call sites. A stage that adds
consumers to a duplicated layer without funnelling them is how the
duplication becomes permanent.

**Corollary this stage must respect:** no new command may render
anything `src/help.rs` cannot eventually produce. Where the Lua renderer
is poorer (no source, no scope), the new commands render the poorer form
rather than inventing a third shape.

## 4. Acceptance

**N** = new behaviour, must fail on full revert. **P** = preservation,
falsified by a named mutation.

1. **N — each of the nine commands exists and renders content.** Driven
   through `pmacs.command.invoke_interactive` (the M-x path), asserting
   **content produced** in `*help*` — not that a buffer exists.
2. **N — `where-is` agrees with the keymap.** Bind a command to a known
   chord, then assert `where-is` reports that chord. Falsified by
   rendering a static string.
3. **N — `list-keybindings` covers every binding `keymap.list()`
   reports.** A property over the data, not a fixed expected list, with
   a non-empty precondition so the loop cannot be vacuous.
4. **N — `apropos` matches on descriptions, not only names.** Search for
   a word that appears in exactly one command's *description* and in no
   command *name*, and assert that command is listed. This is the pin
   that distinguishes apropos from a name filter.
5. **N — `describe-setting` refuses a typo before `on_accept`.** With
   the completion source attached, assert `pmacs.minibuffer.selected()`
   resolves to a real setting — the pre-RET observable, since
   `accept()` does `session.take()` and nothing survives it.
6. **N — `M-x help` lists the family.** Every command in §3.1 appears in
   the index. A property over the family list, so adding a tenth command
   without indexing it fails.
7. **P — every new command renders through `_show_help`.** Replace that
   seam with a counting stub and assert the count equals the number of
   commands exercised. This is §3.4's guarantee, and without it the
   funnelling is a convention rather than a fact.
8. **P — the existing describe/list commands still work.**
   `editor.describe-command`, `editor.describe-setting`,
   `editor.list-buffers`, `editor.list-workers` unchanged. Targeted
   mutation: renaming rather than retaining them.
9. **P — no command's predicate is evaluated.** Register a command whose
   predicate raises, then invoke it through M-x and assert it **runs**.
   Pins §2.4's deliberate non-change, so a later stage that starts
   evaluating predicates has to change this pin knowingly.

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

- **Q#D2 — `help.*` prefix, or keep `editor.*`?** §3.1 proposes `help.*`
  with the old names retained. The counter-argument is that two names
  for one thing is exactly the duplication this arc exists to remove.
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
