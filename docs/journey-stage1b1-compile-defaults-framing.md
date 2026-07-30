# Journey Stage 1b-1 — make building discoverable

**Status: framing, rev 1 — awaiting approval.**
**Serves `COHERENCE.md` §2 (the golden product journey, step 9), §19
(coherence acceptance tests), §20 Priority 1.**

## 0. Revision history

- rev 1 (2026-07-30) — first framing. Scouted against `githubsucks/main`
  @ `22df6ab` (test-ambient isolation framing #201).

## 1. What this stage is, and what it is not

`COHERENCE.md` §20 Priority 1 names the remainder of the journey arc:

> Journey Stage 1b is the named remainder: the compile binding + Cargo
> defaults, LSP spawn guidance, and the welcome buffer.

Those three are unrelated in mechanism, in failure mode, and in cost.
This stage takes **only the first**: journey step 9, *build or test the
project*. LSP spawn guidance (step 6) and the welcome buffer (step 4)
get their own stages; §7 states the split and why.

Nothing here is a new Rust primitive. The stage is Lua, tests, and docs.

## 2. Ground truth

Every claim below was read in the tree at `22df6ab`, not inferred from a
name.

### 2.1 `compile.run` has no binding, and the docs say so

`compile.lua` binds four global sequences (`:1183-1188`): `M-g n`,
`M-g p`, `C-x \``, `M-!`. `bind_slot_keys` (`:220-240`) binds
`RET`/`n`/`p`/`q`/`C-c C-k`/`g` at **buffer** scope inside a generated
buffer. `builtin/keymaps/default.lua` never mentions compile. There is
no third keymap file — `builtin/keymaps/` contains `default.lua` alone.

`docs/keybindings.md:191-192` states it outright:

> `compile.run` and `compile.recompile` are available through `M-x`; no
> global key is assigned to them.

### 2.2 The first prompt is empty

```lua
-- builtin/runtime/compile.lua:1131-1144
name = "compile.run",
fn = function()
  local last = pmacs.compile._last
  pmacs.minibuffer.read {
    prompt = "Compile command: ",
    history = "compile",
    initial = last and last.cmdline or "",
```

`_last` is set only by a completed `pmacs.compile.run` (`:1094`), so on a
fresh session `initial` is `""`. The user is asked what to build and
given nothing to build with.

`initial` does reach the user: `Minibuffer::begin` calls
`replace_contents(&session.initial)` (`src/minibuffer.rs:106-107`), and
`contents()` (`:129`) reads it back. The prefill mechanism works; it is
handed an empty string.

### 2.3 `ProjectKind::Cargo` does not exist — the variant is `Rust`

**`COHERENCE.md` is wrong here, in two places.** Its §2 step-9 row and
its §20 Priority 1 paragraph both say "`ProjectKind::Cargo` existing
(`src/project.rs:77`)". Line 77 is a doc comment; the variant on line 78
is `Rust`:

```rust
// src/project.rs:77-78
/// A Cargo workspace (`Cargo.toml`).
Rust,
```

The marker that produces it is `{ name: "Cargo.toml", kind:
ProjectKind::Rust, is_directory: false }` (`:148-150`). The audit read
the comment and named the comment. This framing corrects both COHERENCE
sites (§8).

The correction is not cosmetic: it decides what the Lua side matches on.

### 2.4 Lua already sees the project kind — as a tag string

`pmacs.project.detect(path)` returns a table (`src/lua_bindings/mod.rs:11683-11694`):

```rust
t.set("root", root.display().to_string())?;
t.set("kind", kind.tag())?;
t.set("language_id", kind.default_language_id())?;
```

`ProjectKind::Rust.tag()` is `"rust"` (`src/project.rs:101`). So the Lua
key is the string `"rust"`, and **no Rust change is needed to learn the
project kind**.

`detect` accepts a directory as well as a file: `walk_for_marker` uses
`start` unchanged when it is not a file (`src/project.rs:225-229`).

### 2.5 The cwd is resolved *inside* `run`, after the prompt has closed

```lua
-- builtin/runtime/compile.lua:765
local cwd = opts.cwd or project_root_of_active() or daemon_working_directory()
```

The interactive `fn` that builds the prompt cannot see this. Any
suggestion computed in the `fn` today would be computed from a different
rule than the one the run obeys — which is the failure this stage must
not ship. §3.1 is about closing that gap before adding the suggestion,
not after.

### 2.6 The active buffer at the journey moment is **pathless**

`project_root_of_active()` (`:600-609`) needs `buf:path()`. Directly
after `pmacs .` the active buffer is dired's, created by
`pmacs.buffer.create(name)` (`dired.lua:506`) — a name, never a path.
`buf:path()` returns `file_path()` mapped to a string
(`src/lua_bindings/mod.rs:1242-1248`), so it is `nil`.

This is not a guess. dired itself compensates, and its own helper is the
evidence:

```lua
-- builtin/runtime/dired.lua:205-217
local function current_directory()
  local buf = pmacs.window.buffer()
  if buf ~= nil then
    local ok, path = pcall(function() return buf:path() end)
    if ok and path then ... end
    local h = handle_for_buffer(buf)     -- <-- the pathless case
    if h then return h.path end
  end
  return canonicalize(".")
end
```

`handle_for_buffer` is a **module-local** table. Compile cannot reach it,
and reaching for it would make compile depend on dired — a new
interaction island for one directory string. §5 decides against it and
§6 states the residual gap.

### 2.7 The last-resort cwd is the *process* cwd, evaluated at call time

`daemon_working_directory()` reads
`pmacs.instance.identity().working_directory`, which is:

```rust
// pmacs-protocol/src/message.rs:1703-1706
working_directory: std::env::current_dir()
    .ok()
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_default(),
```

Two consequences, both load-bearing:

- **In production**, `pmacs .` is launched from the project directory, so
  the fallback happens to be right. `pmacs ~/code/proj` launched from
  `~` resolves to `~`, and the fallback is wrong. That is a pre-existing
  step-9 defect this stage does not fix (§6.1).
- **In tests**, it is the *test runner's* cwd — the pmacs repo root,
  **which is a Cargo project**. `compile_mode_acceptance.rs:1721-1733`
  already pins exactly this (`r1f8_inherited_cwd_resolves_to_the_daemon_working_directory`),
  and `:1382` already warns that "fallback would search the test
  process's cwd". Any pin that asserts *no* Cargo suggestion is at the
  mercy of where `cargo test` was invoked unless it is designed to never
  reach the fallback. §5.2 designs for that; it is the single largest
  trap in this stage.

### 2.8 `C-c c` is free, and `C-c` is not CUA copy

Global `C-c` sequences in the tree: `C-c a`/`f`/`h`/`H`/`i`/`o`/`r`/`s`/`y`
(`lsp.lua`), `C-c t` (`terminal.lua:203`), `C-c @ …` (folding).
`C-c c` is unbound. Copy is `M-w` (`builtin/keymaps/default.lua:114`) —
the CUA trio was deliberately not taken — so binding under the `C-c`
prefix collides with nothing in the default map.

`terminal.lua` binds its own `C-c t`; `lsp.lua` binds its own `C-c o`.
A runtime module owning its global binding is the established pattern,
and this stage follows it rather than editing `default.lua`.

### 2.9 dired already decided the prompt shape

`C-x d` prefills its prompt and **deliberately refuses a completion
source** (`dired.lua:749-756`): a candidate list makes RET-on-empty open
whatever sorts first, and a selected candidate shadows typed text. The
same reasoning applies to a compile command, so this stage prefills and
adds no completion source.

## 3. Design

### 3.1 One resolution, consumed twice

Extract the `:765` expression into a helper and expose a read-only view
of it:

```lua
local function resolve_cwd(explicit)
  return explicit or project_root_of_active() or daemon_working_directory()
end

--- Where the next compile would run, and what kind of project is
--- detected *from that directory*. Public getter (API conventions):
--- `{ cwd = string|nil, kind = string|nil }`.
function pmacs.compile.context(explicit_cwd)
```

`run` calls `resolve_cwd(opts.cwd)`; the interactive `fn` calls
`pmacs.compile.context()`. **The suggestion is therefore a function of
the directory the command will execute in**, and the two cannot drift.

`kind` is `pmacs.project.detect(cwd).kind` — detection *from* the cwd,
which is not the same as "the cwd is the root": `opts.cwd = /proj/src`
in a Cargo workspace yields kind `rust` with root `/proj`. That is
correct (cargo works from a subdirectory) and is stated so a reader does
not read `kind` as "this directory is a project root".

`project_root_of_active()` already calls `detect` once to get a root;
`context` then calls it again on the result. The second call is
redundant in that branch and is kept anyway, because the alternative is
two different rules for where `kind` comes from depending on which
branch produced the cwd. One rule, stated as: **`kind` is always
`detect(cwd)`.**

### 3.2 The default table

```lua
--- Default compile command per detected project kind, keyed by the
--- tag `pmacs.project.detect` returns. Assign into this table from
--- `init.lua` to add or override one.
pmacs.compile.defaults = { rust = "cargo build" }
```

**Only `rust` is seeded, and that is a decision rather than an
omission.** Rust has one answer. Node does not (`npm` / `yarn` / `pnpm`
/ a `scripts.build` that may not exist); Python does not; Go's build and
test are different commands with equal claim. A wrong prefill is worse
than an empty one — the user must first delete it, then type. The table
exists so a user or a package can add the answer *they* know.

`cargo test` is reachable by editing the prefill. The prompt prefills
one string; offering both would need a candidate list, which §2.9
already ruled out for this prompt.

### 3.3 Precedence at the prompt

```
last.cmdline          -- unchanged; a session that has compiled keeps its command
  or defaults[kind]   -- new
  or ""               -- unchanged
```

`last` winning is deliberate: a user who ran `cargo test` once should get
`cargo test` back, not be reset to `cargo build`. This is a preservation
pin (§5.4, P1), not an accident of ordering.

### 3.4 The table is user-writable, so reading it is guarded

`pmacs.compile.defaults` is public and assignable, which means a
metatable with a throwing `__index`, a non-string value, or a
non-table replacement all have to be survivable. The module already
holds this discipline for a hostile rule container (`validated_rules`,
round-2 finding 3: shell-command "must neither surface compile-rule
warnings nor fail on a hostile rule container").

The lookup therefore runs under `pcall` and accepts a value only when it
is a non-empty string. Anything else yields `""` — the pre-stage
behavior. **A broken `defaults` degrades to today's prompt; it never
prevents compiling.**

### 3.5 The binding

```lua
pmacs.keymap.bind { scope = "global", sequence = "C-c c", command = "compile.run" }
```

In `compile.lua`, beside the four existing global binds (§2.8).

Two reachability limits it inherits, both pre-existing and both stated
rather than discovered later:

- Inside a **terminal** window `C-c` is consumed as the escape key, so
  `C-c c` does not arrive. `COHERENCE.md` §2 already records this for
  `C-c t`. `M-x compile.run` still works there.
- The repl package binds `C-c` at **buffer** scope
  (`builtin/packages/repl/init.lua:300`), which shadows the global
  prefix in a repl buffer. Same escape hatch.

`compile.recompile` gets no global binding: `g` in `*compilation*`
already covers rerun, and adding a second global chord for it is scope
this stage has no journey argument for.

## 4. What this changes for the journey

Walking `COHERENCE.md` §2 on a Rust project, unconfigured:

| | before | after |
|---|---|---|
| launch | `pmacs .` lists the directory (Stage 1a) | unchanged |
| open a file | `RET` visits it (Stage 1a) | unchanged |
| build | *no key exists*; `M-x compile.run` → empty prompt | `C-c c` → `Compile command: cargo build` |
| accept | — | RET runs it in the detected root |
| errors | `M-g n` walks them | unchanged |

Step 9's verdict row moves from **Partial** to **Works**; step 10 stops
being gated on the user already knowing `M-x compile.run`.

## 5. Acceptance

### 5.0 Two labels, as Stage 1a established

**N** — new behavior, must fail on full revert. **P** — preservation,
legitimately green on the pre-image, falsified only by a named targeted
mutation. Stage 1a's §6.0 is the reason the distinction is kept: an
equivalence assertion between two implementations that already agree
proves nothing.

Every pin below names the mutation that falsifies it. `scripts/bite` is
run against the suite before the PR opens.

### 5.1 Where the pins live

- **`tests/journey_acceptance.rs`** gains a step-9 section. This file is
  the ratchet — *stages add rows, none removes them*. Its pins go
  through the **real** entry points: `EditorState::open` on a directory,
  a dispatched `RET`, a dispatched `C-c c`. Nothing calls
  `pmacs.compile.context()` directly in this file.
- **`tests/compile_mode_acceptance.rs`** gains the module-contract pins:
  `context()`'s shape, and the hostile-table guards.

### 5.2 The fixture problem, and its only safe shape

Per §2.7, the last-resort cwd is the test runner's cwd, and the test
runner's cwd is a Cargo project. **A pin that asserts "no `cargo build`
suggestion" and reaches the fallback will report the pmacs repo's own
`Cargo.toml` as the fixture's answer.** It would pass or fail on where
`cargo test` was invoked from.

The negative pins are therefore built so the fallback is **never
consulted**: the fixture carries a *different* project marker, so
`project_root_of_active()` resolves inside the fixture and returns
before `daemon_working_directory()` is reached.

```
rust_fixture/            Cargo.toml, main.rs
mixed_fixture/           Cargo.toml, main.rs
  sub/                   package.json, index.js
```

`mixed_fixture` is what makes N4 discriminating: the file opened is
`sub/index.js`, the nearest marker is `package.json` (kind `node`, no
default), and the *outer* marker is Cargo. A suggestion computed from
anything other than the resolved cwd — the outermost marker, the launch
directory, the process cwd — produces `cargo build` here. The correct
implementation produces `""`.

`pmacs.project.set_search_boundary` is set to the fixture root in each
test that detects, so a stray marker above the tempdir (a developer's
`/tmp/.git`) cannot leak in. Note what it does **not** do: it clamps the
upward walk from a start *below* the boundary, so it is no protection at
all for the fallback path, whose start is the repo root. That is why the
fixture shape above, not the boundary call, is the actual defense.

### 5.3 New-behavior pins

**N1 — the chord reaches the command.**
Launch on `rust_fixture`, `RET` on `main.rs`, dispatch `C-c c`; assert
`pmacs.minibuffer.is_active()`.
*Falsifier:* remove the `keymap.bind` line — the chord is unbound, no
session opens.
*Why it is separate from N2:* a prefill assertion alone would stay green
if the binding were removed and the prompt were opened some other way.
The binding is the thing COHERENCE says is missing; it gets its own pin.

**N2 — the prompt is prefilled from the project kind.**
Same walk; assert `pmacs.minibuffer.contents() == "cargo build"`.
*Falsifier:* drop the `defaults[kind]` term from the precedence chain —
contents become `""`.

**N3 — the prefill is executable as offered.**
Same walk; assert the prompt contents are exactly
`pmacs.compile.defaults[pmacs.compile.context().kind]`, and that
`context().cwd` is the fixture root.
*Falsifier:* compute the prefill from `project_root_of_active()` instead
of from `context()`. In the plain fixture the two agree, so this pin
alone is **not** sufficient — which is exactly why N4 exists.

**N4 — the suggestion follows the directory the run will use.**
Launch on `mixed_fixture`, `RET` into `sub/index.js`, dispatch `C-c c`;
assert contents are `""` and `context().kind == "node"`.
*Falsifier:* any rule that is not "detect from the resolved cwd" — take
the outermost marker, or the launch directory, or the process cwd — all
yield `cargo build`.
This is the pin that carries §3.1's whole claim.

**N5 — `context()` is total in a launched session.**
Property, not a constant, because the value is environment-dependent
(§2.7): after a launch, `context().cwd` is non-nil, and `context().kind`
equals `pmacs.project.detect(context().cwd)`'s kind (both nil, or both
the same string). Asserted with a dired buffer active — the pathless
case — so the fallback branch is the one under test.
*Falsifier:* make `resolve_cwd` return `nil` when the active buffer has
no path, i.e. drop the `daemon_working_directory()` term.
*What it deliberately does not assert:* which directory. Pinning that
would pin the test runner's cwd.

### 5.4 Preservation pins

**P1 — `_last` still outranks the kind default.**
Run a compile with an explicit cmdline that is not `cargo build`, then
open the prompt in the Rust fixture; assert the contents are the last
cmdline.
*Targeted mutation:* reorder the precedence chain to put `defaults[kind]`
first. Green on the pre-image (there was no default), red under the
mutation.

**P2 — a hostile `defaults` cannot break compiling.**
Three cases in `compile_mode_acceptance`: `defaults` replaced by a
non-table; a `__index` metatable that raises; a non-string entry for
`rust`. In all three the prompt opens with `""` and `compile.run` still
executes a typed command.
*Targeted mutation:* remove the `pcall` / type guard — the raising case
propagates out of the command and no prompt opens.

**P3 — the existing compile bindings are unchanged.**
`M-g n`, `M-g p`, `C-x \``, `M-!` still dispatch to their commands, and
`compile.run` is still reachable through `M-x`.
*Targeted mutation:* the new `bind` call written as an `unbind`+`bind`
pair over the wrong sequence.

### 5.5 Gates

The full suite from `CLAUDE.md`, plus `compile_mode_acceptance`,
`journey_acceptance`, `dired_acceptance`, and `find_file_acceptance` as
the touched suites. Local runs must control all five bootstrap-storage
variables (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`,
`XDG_CACHE_HOME`, `PMACS_STATE_HOME`) — the ambient-root isolation lane
(#201) is **framing only**, so the workaround is still required and
`compile_mode_acceptance` is one of the suites that goes red without it.

## 6. Named limitations — stated, not discovered later

### 6.1 `pmacs <dir>` from elsewhere still resolves the wrong cwd

Launched as `pmacs ~/code/proj` from `~`, the active buffer is dired and
pathless (§2.6), so the cwd falls through to the process cwd `~`
(§2.7). Compile would then run in `~`, and — consistently, since §3.1
ties them — suggest nothing.

**This stage does not fix it.** The fix needs a notion of "the directory
this session is working in" that is not any one module's private table,
which is `COHERENCE.md` §8 (First-Class Execution Locations) — a model
gap, not wiring. Reaching into dired's `handle_for_buffer` would make
compile depend on dired for one string and add exactly the kind of
interaction island §6 of COHERENCE is about.

What this stage does guarantee is that the failure is **coherent**: the
suggestion describes the directory the command will run in, whatever
that directory turns out to be. The user is never offered `cargo build`
for a directory with no `Cargo.toml`.

N5 pins the property; it deliberately does not pin the value.

### 6.2 The default is per-kind, not configurable through the registry

`pmacs.compile.defaults` is a plain Lua table, not a registered setting.
It cannot be one: `ConfigValue` is four scalars, so a kind→command map
is not expressible. A scalar `compile.default-command` that overrides
the table *is* expressible and is **deferred, not skipped** — it is one
more precedence step and a registry entry, and it belongs with the
config-adoption work (§20 Priority 6) rather than bolted on here.

### 6.3 One binding does not retire the inversion

`COHERENCE.md` §2 keeps a standing observation verbatim: keybinding
coverage is inverted relative to frequency, with `C-c @ C-M-s` bound
while opening a file, opening a terminal, and running a build were not.
Two of the three have been answered (#162, #173); this stage answers the
third, and §8 updates the paragraph accordingly. The *quote* stays as
written, because it names a bias in how new work gets bound rather than
three omissions.

## 7. Staging — why 1b is split

`COHERENCE.md` §20 bundles three items under "Stage 1b". They share a
priority and nothing else:

| | subsystem | shape | risk |
|---|---|---|---|
| **1b-1** (this) | compile + project | wiring, Lua only | low |
| 1b-2 | LSP lifecycle | a failure that is currently *silent* (§1.2) must become visible without becoming noise | medium |
| 1b-3 | startup buffer | new content, plus §18's `C-h`-deletes-a-word problem | low, but touches the default keymap |

One feature, one branch, one PR. 1b-2 is the hard one — the silence
asymmetry is a design question about *when* to speak, not a wiring
question — and bundling it with a keybinding would hold the cheapest
journey fix in the tree behind the most contested one.

## 8. Coherence impact

Per `CLAUDE.md` and `COHERENCE.md` §20's standing process change.

- **Journey steps touched:** 9 directly (Partial → Works); 10 indirectly
  — it was "gated entirely on step 6 or 9 succeeding first".
- **Interaction islands:** none added. The prompt is the existing
  minibuffer; the binding joins the existing `C-c` prefix; the kind
  comes from the existing detector. `pmacs.compile.defaults` is an
  extensible table, not a new modal surface.
- **Config registry adoption:** none, deliberately — §6.2 gives the
  mechanism reason and names the deferred scalar.
- **Background-work attribution:** unchanged. Compile already spawns
  through the process-group machinery; this stage changes what is typed
  into the prompt, not what is spawned or how it is tracked.
- **Doc updates riding this PR** (§25 requires it):
  - `COHERENCE.md` §2 step-9 verdict row → Works, and the
    `ProjectKind::Cargo` → `ProjectKind::Rust` correction **in both
    places** (§2 row and §20 Priority 1). §24 gains the drift entry.
  - `COHERENCE.md` §2's post-table paragraph: "Running a build still has
    no binding" → answered, with the quote itself left intact (§6.3).
  - `docs/keybindings.md`: the `C-c c` row, and the removal of the
    "no global key is assigned to them" sentence at `:191`.
  - `docs/agent-handoff.md` §1: the journey arc bullet gains Stage 1b-1.

## 9. Open questions for review

- **Q#J1 — is `C-c c` the right chord?** It is free, it is under the
  established `C-c` prefix, and it matches what most Emacs distributions
  bind compile to. The alternative worth naming is `C-c C-c`, which is
  more finger-friendly but is the chord many major modes claim
  buffer-locally, so a global one would be shadowed unpredictably later.
- **Q#J2 — should `rust` be the only seeded default?** §3.2 argues yes
  on the grounds that a wrong prefill costs more than an empty one. The
  counter-argument is that `go build` and `make` are about as
  unambiguous as `cargo build`, and seeding them would make the table
  read as a real registry rather than a Rust special case.
- **Q#J3 — should the prefill be selected, so typing replaces it?**
  Emacs leaves the prefill unselected and the point at the end. dired's
  prefill does the same. Matching them means "accept" is RET and
  "replace" is a kill-line first. Changing it is a minibuffer-wide
  behavior change and out of scope, but it is the ergonomic difference a
  user will notice first.

## 10. Ledger

Branch `journey-stage1b1-compile-defaults`, worktree
`../pmacs-journey-1b1`, based on `githubsucks/main` @ `22df6ab`.
Framing only; no code, no PR yet.

Recovery from a clean checkout — the two-argument form of
`git worktree add` does not work for a remote-only branch (it fails with
`fatal: invalid reference`, because after a bare fetch no local branch
exists):

```sh
git fetch githubsucks
git worktree add ../pmacs-journey-1b1 \
  -b journey-stage1b1-compile-defaults \
  githubsucks/journey-stage1b1-compile-defaults
```
