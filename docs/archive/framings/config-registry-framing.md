# Config registry — framing (cross-cutting substrate)

pmacs has no unified configuration surface. Every setting that exists
today invented its own shape: a getter-when-nil function here, a raw
mutable Lua table there, a window field, a Rust preference struct with
its own epoch counter and wire channel. Nothing is discoverable, nothing
is validated centrally, and there is no way at all to say "this setting,
but only in this buffer."

That gap is the named blocker on five separate backlog items — the
per-buffer auto-pair toggle, language-aware indent, per-language comment
padding, per-project compile commands, and the tab-width duplication —
and `docs/side-quest-backlog.md` ranks it first on the north star for
exactly that reason.

This framing proposes a third registry alongside `CommandRegistry` and
`HookRegistry`, built to the rules those two already enforce, with two
scopes (global and buffer-local) and no wire surface.

Backlog: `docs/side-quest-backlog.md` — "Cross-cutting substrate",
north-star item 1. Not a numbered roadmap arc; it is the substrate the
roadmap keeps tripping over.

**Revision 1 — 2026-07-21.** Supersedes a withdrawn same-day draft.
Ideas carried forward from it, credited where they land: the
post-commit listener API with disposable handles (Q#CR6), `Live` vs
`StartupOnly` mutability (Q#CR10), owned-value/deep-copy storage
(Q#CR3), the two-epoch counters (Q#CR2), strict raw-table spec
validation (Q#CR3), and the LuaJIT-vs-lua54 integer-exactness
requirement (acceptance 6). Not carried forward, with reasons inline:
global-only scoping (Q#CR4), `editor.tab_width` as the proving adopter
(Q#CR13), snake_case names (Q#CR9), and Lua bindings inside
`lua_bindings/mod.rs` (Q#CR2).

**Revision 2 — 2026-07-21, review round 1.** Findings F1–F11.
- **F1 (the one that mattered):** "equal-value set is a true no-op"
  contradicted `is_set`, and under the naive reading an equal-valued
  buffer-local override stored nothing — so a later global set would
  flip the very buffer the user had pinned, silently breaking the
  flagship feature. Overrides are now **always stored**; only
  `value_epoch` and listener dispatch key on effective-value change
  (Q#CR2, Q#CR4, acceptance 11).
- **F2:** listener dispatch semantics pinned across scopes (Q#CR6).
- **F3:** GC-collected listeners dropped — no `MetaMethod::Gc` exists
  anywhere in `mod.rs` (verified: zero matches), the compile-mode
  precedent is explicit-dispose only, and GC timing differs across the
  two Lua backends (Q#CR6).
- **F4:** the migration wrappers keep their legacy coercion (Q#CR8).
- **F5:** `StartupOnly` × `set_local` resolved — the combination is
  rejected at define time (Q#CR10).
- **F6:** `string-list` dropped from the stage-1 vocabulary (Q#CR3).
- **F7:** `describe`'s `local` field renamed `buffer_local` — `local`
  is a Lua keyword (Q#CR11).
- **F8:** the direct-remove leak is permanent-but-bounded with no
  aliasing hazard, and now has an acceptance item (Q#CR5).
- **F9:** `get(name)` with no buffer resolves the global chain only
  (Q#CR4).
- **F10:** builtin defines moved to their owning modules (Q#CR14).
- **F11:** all stale line anchors corrected against `7bc0c61`.
- **Bet 6 confirmed and retired** into ground truth, with a correction
  to this doc's own "Init ordering" claim.

**Revision 3 — 2026-07-21, implementation round 1.** Two corrections
found while building the adopters, both against this document rather
than against the code.
- **Acceptance 30 vs 31 contradicted each other** — 31 forbade a
  "per-frame lookup" that 30 required. Q#CR15's prohibition is about
  render hot paths (per cell, per line, table construction), not one
  O(1) scalar `get` per tick. Item 31 reworded.
- **`builtin/runtime/config.lua` does not exist** (Q#CR14). `pmacs.config`
  installs from Rust before any runtime chunk, exactly as `pmacs.command`
  and `pmacs.hook` do, so the chunk had nothing to hold — and the one
  helper it might have held would have broken acceptance 9 by capturing
  every builtin define's `SourceLocation` at `config.lua`. Acceptance 26
  rewritten from a now-vacuous claim. Net effect: this arc touches
  `src/editor.rs` zero times.
- **F5's rejection point was unimplementable as written** (Q#CR10).
  Revision 2 had `define` rejecting `StartupOnly` + `set_local`, but
  `define` cannot know about a future call and `StartupOnly` is legal
  alone. Moved to `set_local`, where the combination actually manifests.
  Acceptance 24 rewritten.
- The registry's `frozen` flag is driven from the existing
  `InitCompleteFlag` at write time rather than by a new `editor.rs`
  call, which is what keeps the zero-touch claim above true.

---

## Ground truth (as of `7bc0c61`)

### `src/config.rs` is not a config registry

Despite the name, `src/config.rs` (274 lines) is the `init.lua` *loader*
and nothing else: XDG config-dir resolution (`user_config_dir`,
`resolve_config_dir`), a `package.path` prepend so a user's config can
span files (`install_package_path`), and a non-fatal eval
(`load_user_config_at` — missing file, unreadable file, parse error and
runtime error are all survivable by contract). It stores no settings and
knows no setting names. The name is taken; that is a naming problem for
the new module, not a design one.

### The settings zoo — nine shapes, none of them shared

Every one of these is a real user-facing configuration surface at
`7bc0c61`. They agree on nothing:

| Surface | Shape | Storage | Validated? |
| --- | --- | --- | --- |
| `pmacs.async_config.frame_target_ms(ms)` (`async.lua:458`) | getter-when-nil fn | module-local upvalue | ad-hoc |
| `pmacs.async_config.default_max_batch(n)` (`async.lua:468`) | getter-when-nil fn | module-local upvalue | ad-hoc |
| `pmacs.autosave.interval_ms(ms)` (`autosave.lua:42`) | getter-when-nil fn | module-local upvalue | hand-rolled floor + NaN check |
| `pmacs.killring.max(n)` (`killring.lua:65`) | getter-when-nil fn | module-local upvalue | ad-hoc |
| `pmacs.editops.trim_on_save(on)` (`editops.lua:844`) | getter-when-nil fn | module-local upvalue | none |
| `pmacs.autosave.enable` (`autosave.lua:34`), `pmacs.recentf.enable` (`recentf.lua:21`), `pmacs.saveplace.enable` (`saveplace.lua:21`), `pmacs.session.desktop_mode` (`desktop.lua:19`) | boolean fn | module-local upvalue | `on ~= false` |
| `pmacs.lsp.config`, `pmacs.lsp.filetypes`, `pmacs.comment.strings`, `pmacs.pair.sets` (`pair.lua:40`) | raw mutable table | plain Lua | at read time, per-consumer |
| `pmacs.parse.shebangs` / `.filenames` / `.injection_aliases` (`syntax.lua:55`) | write-through proxy | Rust registry behind a proxy | at write |
| `pmacs.theme.set/merge`, `pmacs.gpu.set_font` (`font_pref.rs`), `pmacs.window.set_line_numbers` (`mod.rs:11254`), `pmacs.statusline.register` (`statusline.rs`) | Rust binding | Rust struct + epoch + wire channel | at the binding |

Consequences worth stating plainly: no enumeration (you cannot ask what
is configurable), no `describe`, no type discipline, no change
notification, and **no scoping** — every entry above is global except
`line_numbers`, which is window-local because a window happened to be
the convenient place to hang it.

### Two registries already do this correctly

`src/command.rs` (320 lines) and `src/hook.rs` (627 lines) are the
precedent, and they agree with each other:

- `HashMap<String, T>` by name plus a `Vec<String>` insertion order for
  stable listing.
- **R42** — a non-empty `description` is mandatory at define time
  (`CommandError::MissingDescription`, `HookError::MissingDescription`).
- **R50** — spec-table keys are checked against a closed set, so a typo
  is an error, not a silent no-op (`UnknownField`, carrying the
  supported-key list in the message).
- Duplicate names are rejected, never silently overwritten — "silent
  overwrite makes refactoring bugs invisible" (`command.rs:12`).
- `SourceLocation { file, line }` captured from Lua debug info at
  registration, surfaced verbatim by `pmacs.describe.*`.
- Both live behind `Rc<RefCell<…>>` as Lua app data
  (`SharedCommandRegistry`, `SharedHookRegistry`, `mod.rs:2265-2271`).
- `HookRegistry::snapshot` exists specifically so the caller can drop
  the registry borrow before running user code that may re-enter.

A settings registry that does not look like these two would be the odd
one out for no reason.

### Scoping: what exists, what does not

- **Window-local** exists exactly once, ad-hoc:
  `core.active_window_mut().line_numbers` (`mod.rs:11254`).
- **Buffer-keyed side tables** are established:
  `BufferRemoveCallbacks` is a `HashMap<BufferId, Vec<…>>`
  (`mod.rs:133-176`); `KeymapStack` keys per-buffer maps the same way.
- **`Buffer` has no property bag.** The struct (`buffer.rs:158`) is
  rope, name, revision, views, marks, undo/redo, path, file meta, the
  in-flight-edit flag, and optional CRDT state. There is nowhere to put
  a setting, and it is the type carrying undo and CRDT invariants.
- **Buffer death has one choke point**, `after_buffer_removed`
  (`mod.rs:1458`), which already drops that buffer's keymaps before
  firing remove callbacks. Three call sites route through it
  (`mod.rs:3033`, `3123`, `5014`).
- **`BufferId`s are never reused** — allocated from a global counter and
  documented unique (`buffer_registry.rs:82-84`). Any leaked per-buffer
  state is permanent-but-bounded and can never alias onto a future
  buffer.
- **The mode system is unwired.** Of 33 `.resolve(` sites in `src/`,
  every `KeymapStack::resolve` in the editor passes `&[]` for
  `active_modes`; the only non-empty callers are two unit tests inside
  `keymap_stack.rs`. Mode-scoped anything is unavailable today.
- **Project detection exists**: `pmacs.project.detect(path)` →
  `{root, kind}` (`mod.rs:10360`).
- **Language resolution exists**: `buffer_language` (`lsp.lua:471`),
  exported as `pmacs.lsp.buffer_language` (`lsp.lua:494`), added by
  auto-pairing (#110).

### Init ordering, and the freeze point (corrected in revision 2)

Revision 1 said `init.lua` runs "from `load_user_config` on the real
entry points only." **That was wrong** — it described `install_state_dirs`
(`editor.rs:551`), the Arc 3 pattern, not user config. The truth:

Builtin runtime chunks are evaluated inside `EditorState::new()`
(`editor.rs:194-408`, the linear bootstrap). At the tail of the *same*
function, under a single `#[cfg(not(test))]` block (`editor.rs:465-476`):

```rust
#[cfg(not(test))]
{
    crate::config::load_user_config(&mut lua_host);
    lua_host.set_init_complete();
}
```

Three consequences the design depends on:

1. **Builtins define, the user sets, the flag freezes** — one ordered
   sequence, no new machinery needed.
2. Because it lives in `new()` itself, the freeze covers the daemon
   entry (`daemon.rs:468`) and the local one uniformly. This retires
   revision 1's bet 6: the `StartupOnly` freeze point exists and is
   `InitCompleteFlag` (`mod.rs:2270`), consumed by `require_init_phase`
   (`mod.rs:663`).
3. **In `--lib` test builds neither line runs.** User config is never
   loaded and the flag never flips. Any acceptance test asserting
   post-freeze behavior must flip it explicitly, the way
   `mod.rs:14655-14690` already does; otherwise it passes vacuously.

### Both Lua backends ship

`Cargo.toml:75-77` — `default = ["luajit"]`, with `lua54` a supported,
mutually exclusive fallback (audit F-002 pins the
`--no-default-features --features lua54` build). LuaJIT is Lua 5.1
semantics and has no native integer subtype; lua54 does. Any numeric
validation must therefore behave identically under both feature
selections and cannot rely on `math.type`. The same caution kills two
other tempting designs: GC-timed listener lifetime (Q#CR6) and `#` on a
holey array (Q#CR3).

### No GC-cleanup precedent exists

`mod.rs` contains **zero** `MetaMethod::Gc` implementations. The only
`dispose` binding is the explicit one at `mod.rs:1867` (the compile-mode
overlay handle). Resource lifetime in this codebase is
explicit-dispose, never finalizer-driven.

### Tab width — the motivating example is worse than advertised

The backlog calls this "the five hardcoded tab-width sites." The scout
found five sites across **two crates with two different values**:

- `const TAB_WIDTH: u32 = 8` in `src/text_view.rs:35`,
  `src/highlight.rs:321`, `src/diag.rs:393`, `src/completion.rs:594` —
  four independent copies of the same constant and the same
  `col += TAB_WIDTH - (col % TAB_WIDTH)` expansion.
- `pmacs-gpu/src/main.rs:6728-6733` — `advance_minimap_col` expands a
  tab to **4**, not 8.

And the GPU's main text path expands nothing at all: buffer text reaches
the frontend as raw bytes via `BufferSnapshot`/`CrdtOp`, so a literal
`\t` is handed to glyphon and shaped by the font. (The only two `'\t'`
sites in `pmacs-gpu/src/main.rs` are both in the minimap.) Tab rendering
is already inconsistent between the two frontends *before* any setting
exists. This is a rendering-parity bug wearing a config-shaped hat, and
it drives Q#CR13.

---

## Non-goals

No settings GUI. No second config-file format. No persistence outside
`init.lua`. No per-project trust or loading policy. No automatic
migration of every existing setter. No protocol messages and no
frontend-local pixel/font settings. No filesystem watching or hot reload
of `init.lua`.

---

## Decisions

### Q#CR1 — Scope: substrate, two scopes, three consumers, no wire

Stage 1 delivers the registry, the Lua surface, buffer-local scoping,
discovery, and exactly three first consumers (Q#CR8). It does **not**
deliver persistence, a `customize` UI, a settings panel, tab width,
per-language or per-project scopes, or any protocol change. One feature,
one branch, one PR.

The deliverable that justifies the PR on its own: the named backlog item
"per-buffer auto-pair toggle (config-registry-blocked)" stops being
blocked.

### Q#CR2 — `src/config_registry.rs`: a third registry, Rust-owned

New module `src/config_registry.rs` — not `config.rs`, which the
`init.lua` loader owns. Shape mirrors `hook.rs`:

```rust
pub struct ConfigDefinition {
    pub name: String,
    pub description: String,          // R42, mandatory
    pub kind: ConfigKind,
    pub default: ConfigValue,
    pub mutability: ConfigMutability, // Live | StartupOnly
    pub source: SourceLocation,
}

pub struct ConfigListener {
    id: u64,                          // generation-safe, never reused
    name: String,
    body: Function,
    source: SourceLocation,
}

pub struct ConfigRegistry {
    by_name: HashMap<String, ConfigDefinition>,
    order: Vec<String>,
    global: HashMap<String, ConfigValue>,               // overrides only
    locals: HashMap<BufferId, HashMap<String, ConfigValue>>,
    listeners: Vec<ConfigListener>,                     // registration order
    next_listener_id: u64,
    frozen: bool,                                       // Q#CR10
    definition_epoch: u64,
    value_epoch: u64,
}
```

**Override storage versus epoch advancement are two different questions
(F1).** Revision 1 conflated them and broke the flagship feature. The
rule:

- **An override is always stored**, even when it equals the value it
  shadows. `set` and `set_local` unconditionally record an entry, which
  is what makes `is_set` meaningful (Q#CR4) and what makes a
  buffer-local *pin* actually pin.
- **`value_epoch` advances, and listeners fire, only when an effective
  value changes.** Storing an override equal to the current effective
  value is observationally silent but not structurally absent.

The failure this prevents: with `editing.auto-pair` globally `true`, a
user calls `set_local(buf, "editing.auto-pair", true)` to pin that
buffer, then later `set("editing.auto-pair", false)` globally. Under
"true no-op" the local was never stored and the pinned buffer flips —
the pin silently never existed. Acceptance 11 pins this, bite-verified.

`definition_epoch` advances when a key is defined; `value_epoch` gives
future render-path consumers a single `u64` to gate on, mirroring the
split syntax/face epochs from #120 (Q#TH6).

`ConfigError` carries `EmptyName`, `MissingDescription` (R42),
`DuplicateName`, `NotFound`, `UnknownField` (R50, listing supported
keys), `TypeMismatch { name, expected, got }`, `OutOfRange`,
`NotAChoice { name, got, choices }`, `StartupOnlyLocal` (Q#CR10), and
`StartupOnlyAfterFreeze`. Behind
`SharedConfigRegistry = Rc<RefCell<ConfigRegistry>>` as Lua app data,
set beside the other five at `mod.rs:2265-2271`.

**Rust-owned, not a Lua table**, for two reasons. Rust consumers must
read a setting without borrowing Lua (the tab-width sites in Q#CR13 are
the eventual proof). And duplicate/typo/type rejection must be enforced
somewhere a user's `init.lua` cannot bypass — the argument
`command.rs:12` already makes.

**Bindings go in a new `src/lua_bindings/config.rs`, not in `mod.rs`.**
The withdrawn draft proposed an "owned section of
`src/lua_bindings/mod.rs`"; that file is 15,594 lines, the paused F-016
split lives there, and the concurrent vterm lane names it as its own
overlap file. A submodule reduces the shared-file footprint to one `mod`
line plus one `install_config(...)` call and removes almost all of the
inter-lane conflict surface.

### Q#CR3 — Closed value vocabulary, owned data, strict specs

```
boolean | integer | number | string | enum
```

`string-list` is **dropped** from stage 1 (F6). No stage-1 adopter wants
it, and "copied densely from a 1-based array" does not say what happens
to a table with holes — `#` on a holey table returns an arbitrary
border, with no guarantee LuaJIT and lua54 choose the same one.
Admitting it means either explicit hole-rejection logic or a
cross-backend nondeterminism vector that acceptance 6 would have to
chase. It returns with its first real adopter and an explicit hole rule.

`ConfigValue` is owned Rust data. Lua tables, functions and userdata are
never stored. Metadata returned to Lua (`describe`, `list`) is a fresh
table each call, never a handle onto registry state. Definition specs
are strict raw tables: unknown fields, missing fields,
metatable-provided values, wrong types, non-finite bounds, inverted
ranges, duplicate enum choices, and a default that violates its own
contract all reject **before** any mutation. Defaults pass the same
validator as user values.

Numbers must be finite; integers must be exact — checked by value, not
by `math.type`, per the two-backend ground truth. `integer` and `number`
carry optional `min`/`max`.

Tables as *values* stay out. Table-valued configuration already has a
working home — `pmacs.lsp.config`, `pmacs.pair.sets`,
`pmacs.comment.strings`, the write-through proxies — and admitting them
means answering deep-equality, merge-vs-replace, per-key validation and
per-key notification. That is a second arc.

### Q#CR4 — Two scopes: global and buffer-local. Language and project are patterns, not scopes

This is the framing's load-bearing decision and the main departure from
the withdrawn draft, which was global-only. Global-only would not
unblock a single one of the five backlog items the registry exists to
unblock — four of the five are inherently per-buffer or per-language.

```lua
pmacs.config.define { name = …, description = …, type = …, default = … }
pmacs.config.set(name, value)              -- global override
pmacs.config.set_local(buf, name, value)   -- buffer-local override
pmacs.config.get(name [, buf])             -- see resolution below
pmacs.config.reset(name [, buf])           -- drop exactly one layer
pmacs.config.is_set(name [, buf])          -- override present, not value ≠ default
```

**Resolution, pinned (F9).** `get(name, buf)` resolves
buffer-local → global → default. `get(name)` with **no buffer argument
resolves the global chain only** (global override → default) and never
consults an implicitly-active buffer. The signature is the contract;
there is no hidden ambient buffer. A consumer that wants buffer-aware
behavior must pass the buffer, and the acceptance list pins that
forgetting it yields the global value rather than a surprise.

`reset(name)` drops the global override; `reset(name, buf)` drops only
that buffer's local layer and re-exposes the global. `is_set` reports
override *presence* at the queried layer, which is well-defined
precisely because Q#CR2 always stores overrides.

Per-language and per-project behavior is achieved the way Emacs achieves
it: a callback on `buffer.after-load` reads
`pmacs.lsp.buffer_language(buf)` or `pmacs.project.detect(buf:path())`
and calls `set_local`. The registry never learns what a language or a
project is.

The claim, stated so it can be falsified: **all five backlog-blocked
features are expressible as a hook that sets buffer-locals.**
Language-aware indent, per-language comment padding and the per-buffer
auto-pair toggle are buffer-local by nature. Per-project compile
commands are a `set_local` keyed on the detected root. Tab width is
buffer-local (Q#CR13 defers it for an unrelated reason).

Mode-scoped settings are not offered because they cannot work: every
editor `KeymapStack::resolve` passes `&[]`. Offering a mode scope on an
unwired mode system would ship a knob that silently never fires.

### Q#CR5 — Buffer-local storage: registry-owned, purged at the existing choke point

`locals: HashMap<BufferId, HashMap<String, ConfigValue>>` inside the
registry, following `BufferRemoveCallbacks` (`mod.rs:133`) rather than
adding a field to `Buffer` — the buffer struct carries undo and CRDT
invariants and has no property bag, and config does not belong in it.

Purge rides `after_buffer_removed` (`mod.rs:1458`), one line beside the
keymap purge already there. No new hook is defined for this, and the
purge does **not** fire listeners (Q#CR6): the buffer is gone, so there
is no effective value for anyone to observe.

Honest limitation, corrected in revision 2 (F8): `BufferRegistry::remove`
can be called directly without going through `remove_buffer_and_fire`
(`editor.rs:5527` does so in a test). Such a path leaks that buffer's
locals **permanently** — not "until the `BufferId` is reused", because
`BufferId`s come from a global counter and are never reused
(`buffer_registry.rs:82-84`). The leak is therefore bounded and can
never alias onto a future buffer, which makes it a memory footnote
rather than a correctness hazard. Stage 1 pins the contract with
acceptance 13 rather than restructuring `BufferRegistry`.

### Q#CR6 — Change delivery: post-commit listeners, borrow released

Carried forward from the withdrawn draft, whose design here is better
than a plain hook and matches the compile-mode `handle:dispose()`
precedent (`mod.rs:1867`). The registry never runs arbitrary Lua while
mutably borrowed.

```lua
local handle = pmacs.config.on_change('editing.auto-pair', function(new, old, buf)
  -- receives already-validated owned values
end)
handle:dispose()  -- idempotent, generation-safe
```

Set/reset flow: resolve and validate the candidate with no mutation →
**store the override unconditionally** (Q#CR2) → advance `value_epoch`
iff the effective value changed → **release the registry borrow** →
if the effective value changed, invoke listeners in registration order
with copied values → log a failing listener to the normal Lua error sink
and continue the rest.

**Dispatch semantics, pinned (F2):**

- **(a)** A global `set` that changes the global effective value fires
  once with `buf = nil`.
- **(b)** It does **not** additionally fire per-buffer. A buffer holding
  its own override is shadowed — its effective value did not change — so
  a listener that cares about a specific buffer must re-resolve with
  `get(name, buf)`. The notification says "the global changed", not
  "every buffer changed".
- **(c)** The `after_buffer_removed` purge (Q#CR5) fires nothing.
- **(d)** `on_change` on an undefined name raises `NotFound`, matching
  the define-before-set posture of Q#CR10.

None of the three stage-1 adopters consumes `on_change` — `pair.lua`
reads at insert time, `editops.lua` at save, `autosave.lua` per tick —
so the acceptance tests are the only exercise these semantics get. That
is precisely why the contract is written out here rather than left to
the first consumer to discover.

**Listeners persist until explicitly disposed (F3).** Revision 1 said a
garbage-collected handle stops firing; that is dropped. There is no
`MetaMethod::Gc` anywhere in `mod.rs`, the compile-mode precedent is
explicit-dispose only, and GC timing differs between LuaJIT and lua54 —
importing finalizer nondeterminism would cut directly against the
cross-backend exactness this framing demands elsewhere. A user who
registers `on_change` and drops the handle keeps the listener; that is
the same bargain `pmacs.hook.add` already makes.

A listener error does not roll the value back: earlier listeners may
already have applied side effects, and rollback would create two sources
of truth. Re-entrant `set`/`reset` is permitted after borrow release and
produces a later notification epoch; a per-dispatch recursion bound
stops an accidental listener cycle from hanging the editor. Listener ids
are never reused, so a stale handle can never dispose a newer listener.

The borrow-release step is the one to bite-verify —
`HookRegistry::snapshot` exists for exactly this reason, and the
statusline arc's three-phase borrow-released transaction is the recent
precedent for getting it wrong being expensive.

### Q#CR7 — No wire surface; protocol stays at v18

Every stage-1 setting is read daemon-side, in Lua. Nothing new goes on
the wire and `SUPPORTED` is untouched.

Settings whose consumer lives in a frontend already have their own
authoritative facts channel — `ThemeFacts` (v16), `FontFacts` (v17),
`StatuslineSegments` (v18), `LineNumbers` (v13/v14). That
one-channel-per-concern shape is deliberate, with an
authoritative-per-attachment contract and a snapshot/baseline reset
contract behind it (#120 rounds 2-5); a generic "config facts" channel
would have to re-derive all of that and would fit worse than what those
four already do. The registry is not a transport.

### Q#CR8 — Three first consumers, chosen to prove three shapes

| Setting | Type | Proves | Status |
| --- | --- | --- | --- |
| `editing.auto-pair` | boolean | buffer-local resolution | **new** — unblocks the named backlog item |
| `editing.trim-on-save` | boolean | migration behind a stable API | migrates `editops.lua:844` |
| `autosave.interval-ms` | integer, min 1000 | validation + live re-read each tick | migrates `autosave.lua:42` |

All three are consumed entirely in Lua, on the daemon, which is what
makes Q#CR7 true. The withdrawn draft deferred adopter selection to
implementation time; naming them now is what lets the acceptance list
below be written before any code exists.

**No existing surface is removed**, and the wrappers keep their legacy
coercion (F4). This is the subtle part of the migration. Today
`pmacs.autosave.interval_ms(1500.7)` succeeds and floors to `1500`
(`autosave.lua:48`), and `pmacs.editops.trim_on_save("yes")` sets true
(`on ~= false`, `editops.lua:846`). A *thin* wrapper over a strict
`config.set` would reject both, because Q#CR3's `integer` demands
exactness and `boolean` demands a real boolean. So the wrappers coerce
first — `math.floor` / `~= false` — and then call `set` with an
already-conforming value. The registry stays strict; the legacy API
stays lenient; acceptance 27-28 pin both directions on inputs no current
test covers.

One honest divergence from "the wrapper's shape stays exactly as it
was", found in review round 1: `pmacs.autosave.interval_ms(1e30)`
previously stored the float, and now raises `NonIntegral` because the
floored value exceeds `i64` range. The old behavior stored a nonsense
interval; the new one refuses it. An improvement, but a behavior change
at the extreme rather than a pure no-op migration, so it is recorded
here rather than claimed away.

Richer feature-specific APIs are explicitly not deleted:
`pmacs.gpu.set_font` remains the font preference API until a separately
framed migration decides how a daemon-global preference and
frontend-local resolution interact.

`pair.lua` reads `editing.auto-pair` in its insertion predicate. It
loads before `lsp.lua` by an existing ordering contract (Q#AP7,
`editor.rs:319` and `:325`), so the config surface must be installed
before `pair.lua` (Q#CR14).

### Q#CR9 — Names are dotted and kebab-cased

`editing.auto-pair`, `autosave.interval-ms`, and — when it arrives —
`editor.tab-width`. Lowercase ASCII segments, total length bounded at
128 bytes.

Segment grammar, tightened in revision 2: `[a-z][a-z0-9]*(-[a-z0-9]+)*`.
Revision 1's `[a-z][a-z0-9_-]*` admitted `auto-` and `a--b`; this form
forbids a trailing hyphen and a doubled hyphen while accepting every
name we actually want.

The withdrawn draft used snake_case (`editor.tab_width`). Kebab matches
the two registries that already exist — `buffer.before-save`,
`buffer.after-switch`, `buffer.self-insert`, `buffer.kill-this` — and
setting *names* are strings in the registry vocabulary, not Lua
identifiers. Lua field names stay snake_case as they are today.

### Q#CR10 — Define before set; `Live` vs `StartupOnly`

`pmacs.config.set` on an undefined name raises `NotFound`, exactly as
`pmacs.hook.add` does. Silent acceptance of an unknown name is how typos
become permanent mysteries.

Definitions are immutable after first registration, except that a
byte-for-byte identical redefinition succeeds (idempotent reload); a
conflicting redefinition fails and leaves the original exactly as it
was.

`mutability` is `Live` or `StartupOnly`. `StartupOnly` keys accept
writes while user config is loading and freeze when `set_init_complete`
runs at the tail of `EditorState::new()` (`editor.rs:465-476`); a later
write returns an error naming the key and the policy. This generalizes
the posture `require_init_phase` (`mod.rs:663`) already hard-codes for
`pmacs.attach`, and gives it a declarative home.

**`StartupOnly` and `set_local` are mutually exclusive (F5).**
Buffer-locals are set at runtime, from `buffer.after-load` hooks that
fire long after the freeze — so a `StartupOnly` key could never carry a
buffer-local override, and a `set_local` against one would be dead code
that looks live.

**Corrected in revision 3.** Revision 2 said "`define` rejects the
combination outright", which is not implementable: `define` cannot know
about a future `set_local`, and `StartupOnly` is perfectly legal on its
own. The rejection lives where the combination actually manifests —
**`set_local` returns `StartupOnlyLocal` when the named definition is
`StartupOnly`**, unconditionally and independent of freeze state, so the
error is the same before and after startup rather than changing shape
mid-session.

`reset(name, None)` after the freeze is also refused for a `StartupOnly`
key: dropping a frozen override would let the default silently reassert
itself post-freeze, which is the same hazard `set` is blocked for.
Buffer-local `reset` needs no such check, since such a key can never
hold a local override in the first place.

The ordering that makes define-before-set safe is the corrected sequence
in ground truth: builtins define during `EditorState::new()`, user
config runs, then the freeze — all inside the same function. Packages
loading after init cannot be pre-configured from `init.lua` — the
existing v0.1 posture, not a new limitation. Staging pending sets for
later-defined names is deferred.

### Q#CR11 — Discovery: `describe` + `list` + `M-x describe-setting`

`pmacs.config.describe(name [, buf])` returns a fresh table with `name`,
`description`, `type`, `default`, `choices`, bounds, `mutability`,
`value`, `global`, `buffer_local`, and `source`.

**`buffer_local`, not `local` (F7)** — `local` is a Lua keyword, so
`info.local` is a syntax error and every consumer would be forced to
write `info["local"]`. The field is present only when a buffer argument
is given and that buffer holds an override.

`pmacs.config.list()` returns fresh metadata tables in **definition
order**, matching `names()` and the command/hook registries' "stable
listing" rule. Neither `describe` nor `list` ever exposes an internal
table or a listener function.

Revision 3 clarification: revision 2 said "deterministic by key", which
implementation read as possibly meaning sorted-by-name. The requirement
is *determinism* — never `HashMap` iteration order — and definition
order satisfies it while staying consistent with the two sibling
registries. A UI that wants alphabetical sorts at the presentation
layer; the substrate does not decide that. Bounds are exposed as flat
`min` / `max` fields, not a nested `bounds` table.

`M-x describe-setting` renders through `src/help.rs`, following
`render_hook` / `format_hook_text` (`help.rs:160`, `:170`) so the
`*help*` buffer, its link spans and its view-rebuild path work
unchanged. The withdrawn draft had `describe`/`list` as Lua-only; wiring
the `*help*` surface is what makes the registry discoverable to a user
rather than to a script.

A `list-settings` listview panel is deferred — the machinery exists from
Arc 1b, but it is UI scope on a substrate PR.

### Q#CR12 — No persistence in stage 1

Emacs's `custom-file` problem — the init file says one thing, the
persisted state file says another, and the user cannot tell which won —
is a genuine design hazard deserving its own framing round. The
`$XDG_STATE_HOME/pmacs` machinery from Arc 3 exists
(`install_state_dirs`, `editor.rs:551`), so this is a question about
what we want, not about plumbing.

### Q#CR13 — Tab width is stage 2, and the reason is not config

Tab width is the backlog's headline example, and the withdrawn draft
used `editor.tab_width` as its running example and candidate adopter. It
is deliberately out of stage 1 here.

Per the ground truth: the daemon has four `TAB_WIDTH = 8` constants, the
GPU minimap expands tabs to 4, and the GPU's main text path does not
expand tabs at all. So `editor.tab-width` cannot be honored on the GPU
by defining a setting. It needs frontend tab expansion, a decision about
whether the value crosses the wire or the frontend reads its own, and a
rendering-parity fix that stands on its own merits. Note that the
withdrawn draft's own non-goals excluded protocol messages and
frontend-local settings — which its headline example required. Stage 1
resolves that contradiction by deferring the example, not the non-goal.

Stage 2: unify the four daemon constants into a resolved value threaded
through the display-column functions as a parameter — they are pure
functions today and should stay pure — then answer the GPU question.

### Q#CR14 — No runtime chunk; each module defines its own keys

**Revised in revision 3.** Revision 2 put a "friendly Lua surface" in
`builtin/runtime/config.lua`, loaded after `fs.lua`. Implementation
established there is nothing for that file to hold, so **it does not
exist**.

Two facts kill it. First, `pmacs.config` is installed entirely from
Rust by `attach_editor`, which runs before *any* `builtin/runtime/*.lua`
chunk is evaluated in `EditorState::new()` — so the bindings are already
the friendly surface, with no raw underscore layer needing a Lua wrapper
the way `pmacs._async` needs `async.lua`. `pmacs.command` and
`pmacs.hook` are the precedent: both are pure-Rust surfaces with no
runtime chunk. Second, the one helper such a file might plausibly hold —
a shared `define`-wrapping convenience — would actively break acceptance
9, because `SourceLocation` is captured from Lua debug info at the
`define` call site, so routing every builtin define through a helper in
`config.lua` would make every builtin setting report `config.lua` as its
source instead of its owning module.

The module documentation that revision 2 assigned to that chunk lives in
the `src/lua_bindings/config.rs` module header instead.

Builtin `define` calls live with their owning modules (F10), not
centralized. `pair.lua` defines `editing.auto-pair`, `editops.lua`
defines `editing.trim-on-save`, `autosave.lua` defines
`autosave.interval-ms`. `pair.lua`'s load-before-`lsp.lua` contract
(`editor.rs:319`, `:325`) is untouched, and because the bindings install
ahead of every chunk, no load-order constraint is added by this arc at
all.

Consequence worth noting for the concurrent vterm lane: this arc now
touches `src/editor.rs` **zero** times.

**Builtin `define` calls live with their owning modules (F10)**, not
centralized in `config.lua`. Revision 1 centralized them, which would
have pointed every builtin setting's `SourceLocation` at `config.lua` —
weakening exactly what acceptance 9 celebrates — and coupled
`config.lua` to autosave's floor, pair's default and editops' semantics.
The hook precedent is owner-defines: `builtin/hooks/default.lua` defines
hooks, but each module owns its own behavior. So `pair.lua` defines
`editing.auto-pair`, `editops.lua` defines `editing.trim-on-save`, and
`autosave.lua` defines `autosave.interval-ms`. Ordering still works —
every builtin chunk precedes `init.lua` regardless.

The Rust `install_config` call sits with the other installs in
`attach_editor`. Total footprint in the two files the vterm lane also
touches: one install call plus one chunk load in `editor.rs`, one `mod`
line plus one call in `lua_bindings/mod.rs`.

### Q#CR15 — Threading and hot paths

Main-thread `Rc<RefCell<_>>`, matching the syntax, statusline, command
and hook registries. Reads return borrowed or copied data internally and
allocate only when crossing into Lua metadata. Render hot paths cache a
typed value or gate on `value_epoch`; they must never build a Lua table
or do a string lookup per cell or per frame.

No worker-thread mutation. Workers receive copied settings in job specs,
so a live change affects future jobs only, unless the owning feature
explicitly cancels and restarts work.

---

## Bets

1. **Global + buffer-local is sufficient.** Falsified if any of the five
   backlog-blocked features cannot be expressed as a hook that calls
   `set_local`. The most likely falsifier is per-project compile
   commands, where the natural key is a project root — if a
   project-scoped value must survive with no buffer open, this fails and
   a third scope is needed.
2. **No protocol change is needed.** Falsified if any stage-1 consumer
   turns out to have a frontend-side reader. Checked against all three:
   `pair.lua`, `editops.lua`, `autosave.lua` all run daemon-side.
3. **Scalars are sufficient for stage 1.** Falsified if a first consumer
   wants a table- or list-valued setting. The three chosen adopters are
   two booleans and an integer, so this is near-certain for stage 1 and
   says nothing about stage 2.
4. **`after_buffer_removed` catches every buffer death that matters.**
   Falsified by a production path that removes a buffer from the
   registry without it. The known exception is test-only today
   (`editor.rs:5527`), and per F8 its blast radius is a bounded,
   non-aliasing leak.
5. **Migrating two settings behind unchanged public functions is
   invisible to users.** Falsified if any observable behavior of
   `trim_on_save` / `interval_ms` changes — including the coercion
   behavior on non-conforming inputs, which is why F4's pins exist.

Revision 1's bet 6 (a real `StartupOnly` freeze point) is **confirmed**
and retired into ground truth: `set_init_complete` at
`editor.rs:465-476`, inside `EditorState::new()`, covering both entry
points.

---

## Deferred (named)

- **Persistence** and the `custom-file` split-brain question (Q#CR12).
- **Tab width and the GPU tab-expansion parity fix** — stage 2
  (Q#CR13), including the GPU minimap's divergent width of 4.
- **`string-list`** (F6) — returns with its first adopter and an
  explicit hole-rejection rule.
- **Per-language and per-project first-class scopes**, if bet 1 falls.
- **Mode scope** — blocked on wiring the mode system (every editor
  `resolve` passes `&[]`); a cross-cutting backlog item in its own right.
- **Window-local scope** — `line_numbers` is the existing precedent and
  the first migration if a second window-local setting appears.
- **Table-valued settings** (Q#CR3), and with them any migration of
  `pmacs.lsp.config`, `pmacs.pair.sets`, `pmacs.comment.strings` or the
  `pmacs.parse.*` write-through proxies.
- **Migrating the remaining scalar surfaces** — `async_config` (×2),
  `killring.max`, `autosave.enable`, `recentf.enable`,
  `saveplace.enable`, `session.desktop_mode`.
- **Migrating `pmacs.gpu.set_font`** — needs its own framing for the
  daemon-preference / frontend-resolution split (Q#CR8).
- **Deprecating the getter-when-nil functions** once migration
  completes; stage 1 keeps every one of them, coercion included.
- **Pending-set staging** for names defined after `init.lua` runs
  (Q#CR10), which would also give packages a configuration story.
- **`M-x list-settings`** as a listview panel (Q#CR11).
- **A `customize`-style editing UI**, and a `:set`-style minibuffer
  command.
- **A `scope = "global"` define flag** (review round 1, finding 2).
  `set_local` currently succeeds for any `Live` setting, including ones
  whose consumer only ever reads the global chain. `editing.trim-on-save`
  was fixed by making its consumer buffer-aware — the save hook now
  resolves against the buffer being saved — but a per-buffer
  `autosave.interval-ms` is *semantically* meaningless (there is one
  sweep timer, not one per buffer) and the API still accepts it, stores
  it, and reports it from `describe`. That is a stored value nothing
  reads, the same shape F1 exists to prevent. The fix is a define-time
  scope declaration letting the registry refuse `set_local` outright,
  exactly as `StartupOnlyLocal` already refuses another meaningless
  combination. Deferred rather than taken in review round 1 because it
  adds public API surface after review.
- **Field-naming in bound-parse errors** — a bad `min` reports the
  config name and the expected type but not *which* of `min`/`max`
  offended. Cheap, but wants its own error variant rather than a
  synthesized pseudo-name.
- **`reset(name, buf)` symmetry for `StartupOnly`** — `set_local` is
  refused with `StartupOnlyLocal` but the buffer-local `reset` is
  allowed. Unreachable-harmless today (no such local can exist), so a
  rejection would be symmetry rather than a fix.

---

## Acceptance

Registry semantics (unit, `src/config_registry.rs`):

1. Valid definitions round-trip every value kind and produce a fresh
   metadata table.
2. `define` rejects an empty name, a missing or whitespace-only
   description (R42), a duplicate name, an unknown spec key (R50), a
   metatable-provided field, non-finite bounds, an inverted range,
   duplicate enum choices, and a default violating its own contract —
   each **without** adding a definition or advancing either epoch, and
   each with a message naming the offending field.
3. `define` rejects the name grammar's edge cases: trailing hyphen
   (`auto-`), doubled hyphen (`a--b`), empty segment, leading digit, and
   over-length (Q#CR9).
4. An identical redefinition is idempotent; a conflicting redefinition
   leaves the original exact.
5. Values and definitions are deep-copied from Lua — mutating the
   caller's table afterwards cannot alter registry state.
6. Integer and number finite/boundary cases are exact under **both**
   `--features luajit` (default) and
   `--no-default-features --features lua54`.
7. `list` returns definition order (never `HashMap` order);
   `describe`/`list` never expose an internal mutable table or a
   listener function, and each call returns a fresh table.
8. `names()` / `list()` order is stable across ≥3 defines.
9. `SourceLocation` is captured from the *defining module's* chunk and
   renders `file:line` — `editing.auto-pair` reports `pair.lua`, not
   `config.lua` (Q#CR14).

Scoping, storage and buffer-local lifecycle:

10. `get(name, buf)` resolves buffer-local → global → default;
    `reset(name, buf)` drops only the local layer and re-exposes the
    global; `reset(name)` drops only the global.
11. **An equal-valued override is still stored (F1).** With the global
    value `true`, `set_local(buf, name, true)` then
    `set(name, false)` leaves `get(name, buf)` `true` and
    `get(name)` `false` — **bite-verified**: the test fails against a
    "true no-op" implementation that declines to store it.
12. A buffer-local set on buffer A does not change the value seen in
    buffer B; `is_set` reports override presence per layer, including
    for an equal-valued override.
13. A buffer-local value is dropped when the buffer is removed through
    `remove_buffer_and_fire`; removing a buffer directly via
    `BufferRegistry::remove` leaves the entry, and the test asserts that
    documented limitation rather than a fix (Q#CR5, F8).
14. `get(name)` with no buffer argument returns the global chain result
    even when the active buffer holds a different local override (F9).
15. `value_epoch` advances only on effective-value change; storing an
    equal-valued override advances neither epoch.

Listeners (Q#CR6):

16. Callbacks run after borrow release, in registration order, with
    copied `(new, old, buf)` values — **bite-verified**: the test fails
    with the borrow held.
17. A global `set` fires once with `buf = nil` (a), and does **not**
    fire for a buffer whose own override shadows the change (b).
18. The `after_buffer_removed` locals purge fires no listener (c).
19. `on_change` on an undefined name raises `NotFound` (d).
20. One raising listener is logged once and does not block later
    listeners; the committed value stays authoritative.
21. A re-entrant `set` from inside a listener creates a later ordered
    epoch without a `RefCell` panic; a recursive cycle hits the bounded
    error rather than hanging.
22. Dispose is idempotent and id-generation-safe; a stale handle never
    disposes a newer listener. A dropped-but-undisposed handle keeps
    firing (F3) — the inverse of revision 1's claim, pinned so the
    behavior cannot silently regress to GC-dependence.

Startup and mutability:

23. `StartupOnly` keys accept writes before the freeze and reject after,
    with a message naming the key and the policy. **The test flips
    `InitCompleteFlag` explicitly** (`mod.rs:14655-14690` is the
    pattern) — in `--lib` builds `set_init_complete` never runs, so a
    test that omits this passes vacuously.
24. `set_local` against a `StartupOnly` definition returns
    `StartupOnlyLocal` both before and after the freeze (F5, corrected
    in revision 3 — `define` cannot police a future call). A
    `StartupOnly` global `reset` after the freeze is refused for the
    same reason `set` is.
25. A failing `init.lua` preserves prior successful sets and still
    starts the editor; missing `init.lua`, broken `init.lua` and
    `require` package-path behavior are unchanged.
26. The `pmacs.config` table is populated before the first
    `builtin/runtime/*.lua` chunk evaluates — a builtin module's
    top-level `define` call succeeds — and `pair.lua` still loads before
    `lsp.lua` (the Q#AP7 contract is untouched). Revision 3: this
    replaces "`config.lua` is installed before `pair.lua`", which became
    vacuous when that chunk was removed (Q#CR14).

Adopters:

27. `pmacs.editops.trim_on_save(true)` and
    `pmacs.config.set("editing.trim-on-save", true)` are interchangeable
    in both directions — each observes the other's writes — and
    `trim_on_save("yes")` still sets true (F4).
28. `pmacs.autosave.interval_ms(1500.7)` still returns `1500`, and
    `interval_ms(500)` still raises on the floor — now enforced by the
    registry validator behind the wrapper's coercion. **Bite-verified**
    against the removed hand-rolled check (F4).
29. `editing.auto-pair` false globally suppresses pairing; false
    buffer-locally suppresses it in that buffer only, with pairing still
    active in a second buffer of the same language.
30. The autosave tick observes a mid-session interval change without a
    restart (the existing live-re-read contract, re-pinned), whether the
    change arrived through the wrapper or through a direct
    `pmacs.config.set`.
31. No adopter builds a Lua table or performs a string-keyed lookup
    **per cell or per rendered line**, and all three preserve their
    previous default behavior exactly. Revision 3 note: this item and
    item 30 read as contradictory in revision 2 — 30 *requires* the
    autosave tick to re-read every frame while 31 forbade a "per-frame
    lookup". Q#CR15's prohibition targets render hot paths, not a single
    O(1) scalar `get` once per tick, which is exactly what Q#CR8 asks
    autosave to prove. One scalar `get` per tick is explicitly
    conforming.

Discovery:

32. `pmacs.config.describe` returns every documented field, with
    `buffer_local` absent when no buffer-local override exists and the
    field name usable without bracket syntax (F7).
33. `M-x describe-setting` renders into `*help*` with the source
    location present.

Gates: the full standing suite per `CLAUDE.md`, plus the new acceptance
suite, plus `--no-default-features --features lua54` for item 6.

---

## Resolved review questions (round 1)

1. **Scope set** — accept global + buffer-local. Bet 1's named falsifier
   (a project value surviving with no buffer open) is real and correctly
   deferred.
2. **`string-list`** — dropped (F6). Returns with an adopter.
3. **`Live`/`StartupOnly`** — accepted; freeze point confirmed at
   `editor.rs:465-476`, covering both entry points. Test-build caveat
   (acceptance 23) and the `set_local` exclusion (F5) carried into the
   doc.
4. **No-rollback listener errors** — accepted; matches the hook
   log-and-continue posture.
5. **Three adopters, `set_font` untouched** — accepted, with F4's
   coercion pins added.
6. **Defer tab width** — accepted; the parity mess is verified real and
   the withdrawn draft's contradiction correctly diagnosed.
7. **Kebab-case** — accepted, with the grammar tightened to
   `[a-z][a-z0-9]*(-[a-z0-9]+)*` to forbid `auto-` and `a--b`.

Open for round 2: nothing blocking. F10 (owner-defines) was adopted as
recommended; if the lead prefers centralized defines for reviewability,
acceptance 9 is the item that changes.

---

## Lane coordination (config registry ↔ vterm)

Both lanes are active concurrently in separate worktrees. Assignment:

| File | Owner | Other lane |
| --- | --- | --- |
| `src/editor.rs` runtime-load block | **config** — its chunk must load before consumers | vterm appends its chunk at the tail |
| `src/lua_bindings/mod.rs` | **neither** — config adds `lua_bindings/config.rs`, vterm adds its own submodule; each takes one `mod` line + one install call | keep both footprints to one line |
| `src/lib.rs` module exports | one line each | trivial rebase conflict, resolve in favor of both |
| `src/ansi.rs` | **vterm** | config never touches it |
| `docs/agent-handoff.md`, `docs/active-work.md` | whichever merges second rewrites its §1 entry post-rebase | do not co-edit |

Neither feature imports the other's types. Recommended merge order:
config registry first (smaller, substrate, heaviest footprint in the
shared files), then vterm rebases. Per the standing constraint, vterm
keeps terminal settings hard-wired in its early stages and adopts
`pmacs.config` only after both contracts land, so neither arc is a
prerequisite for the other.

Implementation branch, after framing approval: `config-registry`, cut
from then-current canonical `main`, never from a vterm branch.
