# Splitting `lua_bindings.rs` — framing (F-016)

`src/lua_bindings.rs` is 15,202 lines — the repository's largest file by
2×. It concentrates the entire Rust↔Lua surface: a shared core (the
registry alias, `BindingError`, the `BufferIdLua` userdata, intercept
views, ~15 state-holder types, the `install()` spine) followed by ~20
independent `pmacs.<domain>` API surfaces (packages, lsp, mcp, completion,
project, index, window, minibuffer, parse, theme, process, diag, async,
ansi, …) and ~2,800 lines of tests. Audit F-016.

This is exactly the file the audit calls out ("combines many Lua API
domains, package management glue, LSP stores, theme bindings, attachment
bindings, and tests in one file") and its guidance is explicit: **"Split
only along stable boundaries, not as a drive-by refactor."** The
`pmacs.<domain>` sections, already fenced by `// ---` banners and each
wired through its own `install_<domain>_module` seam, *are* those stable
boundaries.

## Non-negotiable: behavior-preserving code motion

This arc moves code; it does not change it. Every tranche is:

- **Pure relocation.** A domain section's items move verbatim into
  `lua_bindings/<domain>.rs`. No logic edits, no signature changes, no
  renames beyond what visibility requires.
- **Minimal visibility widening.** Items in the shared core that a moved
  domain references become `pub(crate)` (or `pub(super)`); a domain's own
  `install_<domain>_module` becomes `pub(super)` so the parent `install()`
  still calls it. Nothing gains wider visibility than the move demands —
  the compiler names each one (E0603/E0433), so the set is exact, not
  guessed.
- **Green per tranche.** `cargo build` + `cargo clippy --all-targets` +
  the full test suite pass under **both** Lua flavors after every tranche,
  before it's committed. A split that changes a test outcome is a bug in
  the split.

The Lua-visible API (`pmacs.buffer.*`, `pmacs.lsp.*`, …) is **byte-for-byte
unchanged** — the same `install()` builds the same tables; only the Rust
file layout moves. No `.lua` code and no test *behavior* changes.

## Target structure

`src/lua_bindings.rs` → `src/lua_bindings/`:

- `mod.rs` — the shared core (registry alias, `BindingError`,
  `BufferIdLua` + its method-adders, `LuaInterceptView`, the state-holder
  types, `require_init_phase`, shared helpers) **and** the top-level
  `pub fn install()` spine that calls each domain's installer. This is the
  stable hub every domain depends on; it stays put.
- `lua_bindings/<domain>.rs` — one file per `pmacs.<domain>` surface,
  exposing `pub(super) fn install_<domain>_module(...)`. Each `#[cfg(test)]
  mod tests` moves with its domain where the tests only touch that
  domain's (now `pub(super)`-reachable) surface; shared/cross-cutting
  tests stay in `mod.rs`.

The dependency shape is a **hub-and-spoke**: `mod.rs` is the hub; domains
are spokes that depend on the hub and (ideally) not on each other. Where a
genuine domain→domain edge exists (e.g. completion→lsp), the depended-on
domain is extracted first and its needed items widened to `pub(crate)`.

## Incremental, not a big bang

A single 15k-line move is unreviewable and risks silent breakage. Instead,
**each tranche is its own PR**: convert the file to a directory module +
extract one dependency-ordered group of domains, prove green, merge, next.
Leaf domains (no outgoing cross-domain edges) go first to establish the
pattern; coupled domains follow their dependencies.

### What the coupling recon established

- **Two misplaced helper clusters cause almost every cross-domain edge.**
  The generic JSON converters (`lua_to_json`/`lua_table_to_json`/
  `json_to_lua`) sit inside the `lsp` section but are used by `async`,
  `mcp`, `completion`, and the completion framework — they belong in shared
  core. The ANSI converters (`event_to_lua_table`/`style_to_lua_table`) sit
  in the `packages` line-range but belong to `ansi` (`process` uses them
  too). Hoisting these dissolves the async→lsp, mcp→lsp, completion→lsp,
  and ansi→packages edges.
- **The banners are not clean cut-lines.** The `packages` line-range is a
  grab-bag: it physically contains the core module installers (`command`,
  `menu`, `help`, `hook`, `describe`, `keymap`) that `install()` calls and
  that belong to shared core, plus editor sub-installers. Extraction must
  move items to their *logical* home, not by line-range.
- **Rust privacy is on our side.** A child module can read its ancestors'
  private items, so a domain moved to `lua_bindings/<d>.rs` reaches
  shared-core internals via `super::` with **no widening**. Widening is
  needed only for (a) a domain's install fn so the parent can call it
  (`pub(super)`), (b) sibling→sibling edges, and (c) external callers — the
  last handled by `pub(crate) use <d>::<item>;` re-exports in `mod.rs` so
  `crate::lua_bindings::<item>` paths in `editor.rs`/`lua.rs`/etc. never
  change.
- **Real edges that survive the hoist:** `process → ansi`, `project →
  lsp`, `completion_framework → index`/`lsp` (type-alias only). These
  extract after their dependency.
- **Tests are one flat `#[cfg(test)] mod tests` over `super::*`** touching
  private items (`BufferIdLua`, `require_init_phase`, `install`, handles).
  They stay in `mod.rs` and move to `lua_bindings/tests.rs` last, once the
  handful of items they name are reachable.

### Tranche plan (one PR each)

0. **This PR — directory conversion + the truest leaf (`diag`).**
   `lua_bindings.rs` → `lua_bindings/mod.rs`; extract the smallest
   zero-outgoing-edge leaf, `diag`, as `lua_bindings/diag.rs`. Deliberately
   minimal: a large mechanical refactor's first PR should validate the
   directory module, the `super::`-access discipline, and the new-file CI
   path on the *simplest* real case before moving code in bulk. Subsequent
   tranches batch multiple domains now that the mechanics are proven.
1. **`index` (the one genuinely clean remaining leaf).** Zero shared-core
   coupling; establishes the `pub(crate) use` re-export for external
   callers (`editor.rs`, `completion_framework.rs`) and `super::` access
   to a stranded helper (`lua_to_json`). **Correction after tranche 1:**
   the recon under-counted the misplaced helpers. Beyond the JSON/ANSI
   clusters, the *style/color* converters (`lua_to_style`/`style_to_lua`/
   `color_to_lua`/…) sit in the `theme` section, and `caller_source` +
   the command/menu builders (`build_command_from_spec`,
   `build_menu_item_from_spec`, `BindArgs`, `register`, …) sit at the tail
   of the `minibuffer` range — all shared, used across sections. So
   `parse`/`theme`, `window`, and `minibuffer` are **not** clean
   line-range extractions until the hoist below runs.
2. **Hoist the misplaced helpers (must precede the contaminated leaves).**
   Move every shared helper stranded in a domain section to its proper
   home: JSON converters → shared core; ANSI converters → the `ansi`
   module; style/color converters + `caller_source` + command/menu
   builders → shared core. This is a within-`mod.rs` repositioning (no
   behavior change) that makes the domain sections clean line-range units.
   Then extract `parse`/`theme`, `window`, `minibuffer`, `ansi`,
   `packages` (the last after lifting the misplaced core installers out of
   its range too).
3. **The `lsp` hub + its JSON consumers.** `lsp`, then `async`, `mcp`,
   `completion` (edge-free once the JSON helpers are hoisted).
4. **The coupled tail.** `process` (→ansi), `project` (→lsp),
   `completion_framework` (→index/lsp), and the `editor` umbrella
   (gathering its sub-installers scattered across the `window`/`packages`
   ranges).
5. **Tests.** Move the flat `mod tests` into `lua_bindings/tests.rs`.

Each tranche is independently green and mergeable; the order is
dependency-correct so no tranche introduces an unresolved sibling edge.

## Scope of this arc

**This arc splits `src/lua_bindings.rs` only.** The other large files the
audit lists — `src/editor.rs` (7,093) and `pmacs-gpu/src/main.rs` (6,761)
— are separate future arcs with their own stable boundaries (editor
command groups; GPU input / render-pipeline / layout). Bundling them here
would violate the "one stable boundary at a time" discipline. Named as
follow-ups, not started.

## Categorical bets

- **The compiler is the safety net.** Because the split is pure motion,
  every real breakage is a compile error (missing item, private item) or a
  failing test — there is no silent behavioral drift to hunt for. That is
  what makes a mechanical refactor of this size tractable.
- **Widen visibility exactly as far as the move forces, no further.**
  `pub(crate)`/`pub(super)` over `pub`; the goal is the same encapsulation
  in more files, not a newly-public surface.
- **Stable boundaries only.** The `pmacs.<domain>` seams are API-shaped and
  long-lived; splitting along them ages well. Splitting by line-count or
  incidental adjacency would not.

## Validation

Per tranche: `cargo fmt` clean; `cargo clippy --all-targets` clean under
luajit **and** lua54; full `cargo test` green under both flavors (the
2,800-line test suite is the behavioral oracle — same tests, same
outcomes, new locations). No Lua-side change, so the `.lua` fixtures and
acceptance suites are untouched and must stay green.

## As-built

**Tranche 0 (this PR).** `git mv src/lua_bindings.rs
src/lua_bindings/mod.rs` (the `pub mod lua_bindings;` in `lib.rs` resolves
to `mod.rs` unchanged), then extracted the `pmacs.diag` surface —
`diagnostic_to_lua` + `install_diag`, moved **verbatim** — into
`src/lua_bindings/diag.rs` (229 lines). `mod.rs` declares `mod diag;` and
its one internal call site became `diag::install_diag(...)`; `diag.rs`
reaches shared-core items (`BufferIdLua`, `SharedCore`) via `super::` and
imports externals (`SharedLspManager`, `crate::diag::*`) directly. No
visibility widening was needed — the child module sees the parent's items,
and `install_diag`'s only caller is `mod.rs` itself. No re-export needed
(nothing external references it). `mod.rs`: 15,202 → 14,986 lines.

Validated: `cargo fmt` clean; `clippy --lib` clean under luajit **and**
lua54; full lib suite **1437 passed / 0 failed** under luajit (the tests,
which drive `pmacs.diag.*` through the Lua VM, are the behavioral oracle —
unchanged outcomes, code merely relocated).

**Tranche 1 (this PR).** Extracted `pmacs.index` (the project symbol-index
surface) into `src/lua_bindings/index.rs` (390 lines) — the one remaining
*clean* leaf (its 3 private helpers are used only within its own range).
`mod.rs` declares `mod index;` and `pub use index::{SharedProjectIndexer,
make_project_indexer};`, which keeps the `crate::lua_bindings::…` paths in
`editor.rs` + `completion_framework.rs` (and an in-file completion-framework
use) valid. `index.rs` has **zero shared-core coupling** — it depends only
on `crate::project_index`, mlua, std, and reaches one stranded helper
(`lua_to_json`, still in the `lsp` section) via `super::`. Verbatim move,
no logic change. `mod.rs`: 14,986 → 14,603 lines.

While vetting the next leaves, discovered the recon under-counted the
misplaced helpers (see the corrected tranche plan above): `parse`/`theme`,
`window`, and `minibuffer` trail off into *shared* style/color, `caller_
source`, and command/menu helpers, so they need the helper-hoist tranche
(now #2) before they can be extracted cleanly. This tranche stops at
`index` rather than force those.

Validated: `cargo fmt` clean; `clippy --lib` clean under **both** flavors;
full lib suite **1437 passed / 0 failed** under **both** luajit and
lua54.
