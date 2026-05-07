# Package author's guide

This guide is for authors who want to publish a pmacs package. It
covers the manifest format, the address schemes pmacs accepts, the
audit-lint rule set every package is expected to follow, and the
mechanics of distributing a package via Git.

The user-facing side (how to *install* a package) is at
[`docs/packages.md`](packages.md). This document is the inverse:
how to *publish* one.

A reference implementation lives at
[`builtin/packages/repl/`](../builtin/packages/repl/) --- pmacs's
own bundled REPL package, the dogfood case for everything below.

---

## 1. Package layout

A package is a directory tree with at minimum a manifest and an
entry module:

```
my-package/
├── pmacs.toml          # required: manifest
└── init.lua            # required: entry module (path = manifest's `entry` field)
```

Larger packages add submodule files and supporting directories:

```
my-package/
├── pmacs.toml
├── init.lua
├── core.lua            # exported submodule
├── ui.lua              # exported submodule
├── internal/
│   └── helpers.lua     # NOT exported; only reachable from within the package
└── README.md
```

The pmacs loader resolves `require("my-package")` to `init.lua`
and `require("my-package.core")` to `core.lua` --- *if and only if*
`my-package.core` appears in the manifest's `exports` list.
Anything not exported is private to the package, even if other
packages can technically reach it on disk.

---

## 2. Manifest format (`pmacs.toml`)

The manifest is a TOML file at the package root. v1.0 fields:

```toml
name = "my-package"
version = "1.2.3"
summary = "One-line description of what the package does."
pmacs_required = ">= 0.1.0, < 1.0.0"
entry = "init.lua"
exports = ["my-package", "my-package.core", "my-package.ui"]

# Optional --- omit if empty.
[[dependencies]]
address = "github:other/utility-package"
version = "^0.4.0"

# Optional --- omit if none.
[[conflicts]]
address = "github:competing/repl"
version = "*"
```

### Field reference

| Field            | Type             | Required | Notes |
|------------------|------------------|----------|-------|
| `name`           | string           | ✓        | Lowercase, hyphen-separated. Optional `namespace/name` form (one `/`). Must start with a-z; `[a-z0-9-]` only. |
| `version`        | semver string    | ✓        | Strict semver: `MAJOR.MINOR.PATCH`. |
| `summary`        | string           | ✓        | One-line description. |
| `pmacs_required` | version-req      | ✓        | Range of pmacs versions this package supports. |
| `entry`          | path string      | ✓        | Relative to package root. Conventionally `init.lua`. |
| `exports`        | list of strings  | ✓        | Public Lua module names other packages may `require`. The package's basename should be in the list if you want `require("<name>")` to work; submodules outside the list cannot be required by other packages. |
| `dependencies`   | list of tables   | optional | Each entry has `address` (see §3) and `version` (a version-req). Defaults to empty. |
| `conflicts`      | list of tables   | optional | Same shape as `dependencies`. Use sparingly --- the resolver fails the install if any conflicting package is also being installed. |

The manifest parser rejects unknown fields silently (forward
compatibility): a v1.0 binary reading a v1.1 manifest with a new
optional field still loads it.

### `exports` and the per-package environment

Every package executes its entry chunk in a per-package `_ENV`
table. Reads of "globals" (`pmacs.buffer`, `string`, `table`, etc.)
go through `_ENV`'s metatable's `__index = _G`, so the standard
library and the pmacs API surface remain reachable. Writes to
"globals" stay local to the package: assigning to `MARKER = "x"`
inside `init.lua` does *not* pollute `_G`.

Practical implication: a package can declare module-level state
without worrying about colliding with another package's module-level
state, even if both pick the same name.

---

## 3. Address schemes (v1.0)

A package address tells the resolver where to fetch the package
from. v1.0 supports the following forms:

| Scheme              | Example                                | Resolves to                             |
|---------------------|----------------------------------------|-----------------------------------------|
| `github:`           | `github:owner/repo`                    | `https://github.com/owner/repo.git`     |
| `gitlab:`           | `gitlab:owner/repo`                    | `https://gitlab.com/owner/repo.git`     |
| `git:` over HTTPS   | `git:https://example.org/repo.git`     | `https://example.org/repo.git`          |
| `git:` over SSH     | `git:git@example.org:owner/repo.git`   | `git@example.org:owner/repo.git`        |
| `git:` over file    | `git:file:///srv/git/repo.git`         | local path (mostly for testing)         |

The shorthand schemes (`github:`, `gitlab:`) are sugar for the
underlying `https://` URL. Full URLs work too:

```lua
pmacs.packages.install { "git:https://forge.invalid/me/pkg.git", version = "^1" }
```

### Forge aliases (extension path)

`github:` and `gitlab:` are baked in. Other forges (Codeberg,
Forgejo instances, internal corporate forges) are not. The pmacs
extension path for adding a new alias is documented at
`src/packages/address.rs`: a single match arm in `Address::parse`
plus a unit test. PRs welcomed; the criteria are (a) a stable
public forge URL pattern, (b) consensus in the issue tracker, and
(c) audit-lint coverage of the parser change.

The recommended interim path for one-off forges is to use the full
`git:https://...` form rather than waiting on an alias.

---

## 4. Versioning and the lockfile

pmacs is a semver-disciplined ecosystem:

* Packages declare `version` as a strict semver.
* Users pin via version constraints (`^1.2.3`, `>=1, <2`, `~0.5`).
* The resolver picks the highest tag matching the constraint set.
* The lockfile (`pmacs.lock`, written next to the user's config or
  project root) records the exact commit hash, the resolved version,
  and a content hash (SHA-256 over `git archive --format=tar`).

When you publish a new version:

1. Update `version` in `pmacs.toml`.
2. Tag the commit. The tag name **must** match the `version` field
   prefixed with `v` (i.e., `v1.2.3` for `version = "1.2.3"`).
3. Push the tag to the upstream.

Users who installed your package with `version = "^1.0"` will see
the new tag the next time they run `pmacs.packages.update`.

---

## 5. Audit-lint rule set (T M7.9)

Every package should pass `pmacs-audit` cleanly. The lint runs
declarative tree-sitter queries against your `*.lua` files and
emits findings at three severity levels:

* **Error** --- forbidden patterns. The CLI exits non-zero; CI gates
  on this.
* **Warning** --- patterns that require capability declaration.
  Currently always fires for fs / process operations; will gate on
  a future manifest `permissions` field.
* **Info** --- patterns that need human classification (currently
  cross-package dotted requires).

### v1.0 rules (15 patterns across 7 spec classes)

The full list lives at
[`audit/audit-rules.scm`](../audit/audit-rules.scm) (the published
contract) and `src/audit/rules.rs` (the metadata table). Summary:

| Severity | Class                  | Rules                                                                         |
|----------|------------------------|-------------------------------------------------------------------------------|
| Error    | private surface        | `no-private-surface-require`, `no-private-surface-identifier`                 |
| Error    | FFI / native loader    | `no-ffi-call`, `no-package-loadlib`, `no-package-cpath-mutation`              |
| Error    | debug-cancellation     | `no-debug-sethook`, `no-debug-setmetatable`                                   |
| Error    | environment escape     | `no-rawget-rawset-on-globals`, `no-setfenv-getfenv`                           |
| Warning  | filesystem mutation    | `no-fs-mutation-io-open-write`, `no-fs-mutation-os`                           |
| Warning  | process spawning       | `no-process-spawn-io`, `no-process-spawn-os`, `no-process-spawn-pmacs`        |
| Info     | reach-around           | `reach-around-require`                                                        |

### Common Error rules and their fixes

* **`no-private-surface-*`** --- a package tried to `require` a
  module under `pmacs._internal.*` or `pmacs.core.*`, or referenced
  an identifier prefixed with `_pmacs_internal_` or `_core_`. These
  surfaces are not API; use the documented `pmacs.X.Y` namespaces
  instead.
* **`no-ffi-call`** --- LuaJIT's FFI escapes the Lua sandbox. If
  you genuinely need native code, propose the surface as a pmacs
  API addition rather than reaching past the boundary.
* **`no-debug-sethook`** --- pmacs uses the debug hook for
  cooperative C-g cancellation (T M7.8); installing your own hook
  disables it editor-wide. Use `pmacs.async`-friendly patterns or
  schedule work explicitly.
* **`no-rawget-rawset-on-globals`** --- the per-package `_ENV`
  table is the API. Routing through `_G` defeats the sandboxing.

### Running the lint locally

```sh
cargo install --git https://git.levineuwirth.org/neuwirth/pmacs \
    --bin pmacs-audit pmacs
pmacs-audit --pretty .
```

Exit code 1 means at least one Error finding; 2 means an I/O or
configuration failure; 0 means clean (Warnings/Info OK).

### Sample CI workflows

Drop-in templates for three forges live at
[`audit/ci/`](../audit/ci/):

* `github-actions.yml`
* `gitlab-ci.yml`
* `forgejo-actions.yml`

Each builds `pmacs-audit` from a pinned pmacs revision, runs it,
and uploads the JSON report as a job artifact.

---

## 6. Distribution

pmacs has no central registry. Packages are published by tagging a
commit in a Git repository. The address scheme picks the forge
(or generic Git URL); the resolver's tag enumeration picks the
version.

### Publishing checklist

1. Author the package in a fresh repository.
2. Write `pmacs.toml` with `version = "0.1.0"` (or wherever you
   start).
3. Author `init.lua` and any submodules listed in `exports`.
4. Run `pmacs-audit --pretty .` and fix any Error-severity
   findings.
5. Commit, tag `v0.1.0`, push to the forge.
6. Document the install command in your README:
   ```lua
   pmacs.packages.install { "github:you/your-package", version = "^0.1.0" }
   ```

For breaking changes: bump `MAJOR`, document the migration in your
CHANGELOG, and consider declaring a `[[conflicts]]` entry in your
new manifest against the old `version = "<1.0.0"` so users who
mass-update don't end up with both shapes resolved at once.

---

## 7. Dev-loop APIs (T M8.1)

Iterating on a package without restarting pmacs uses three
init-time APIs and one runtime API. Together they let you point
the editor at a working tree on disk, edit source files, and
reload to observe new behavior.

### `pmacs.packages.install_local(path)`

Symlinks a working tree into the install root instead of
fetching+extracting from a Git remote.

```lua
pmacs.packages.install_local("/srv/dev/your-package")
```

* Init-time-only (like the other `pmacs.packages.*` install APIs).
* Validates the source has a readable `pmacs.toml` and that the
  package's `pmacs_required` matches the running version.
* The install dir at `<install_root>/<basename>` becomes a symlink
  to your source. Edits to your source files are immediately
  visible to subsequent loads, with no copy step.
* **Skips the lockfile.** Local installs are explicitly ephemeral
  and not reproducible across machines. The lockfile records only
  fetched installs; users sharing a project rely on
  `pmacs.packages.install` for that.
* If the install path already holds a *real* directory (a previous
  fetched install), `install_local` refuses with a clear error so
  you don't accidentally clobber an installed tree with manual
  edits.
* Calling `install_local` twice for the same name swaps the
  symlink; before the swap, the prior install's `on_unload` hooks
  fire (see below). If a hook fails, the swap is aborted with disk
  unchanged.

### `pmacs.packages.reload(name)`

Re-runs the package's chunk against whatever's currently on disk.
Returns the new module table.

```lua
-- After editing /srv/dev/your-package/init.lua:
local m = pmacs.packages.reload("your-package")
```

What reload does, in order:

1. Runs every registered `on_unload` hook for the package. Hooks
   fire in registration order; a hook that fails leaves the
   remaining unrun hooks (and the failed one) in the registry for
   a retry.
2. Drops `package.loaded[name]` (and every `package.loaded[name.<sub>]`
   for declared submodule exports) so the next `require` actually
   re-runs the chunk.
3. Drops the cached per-package `_ENV` table. Globals removed in
   the new source disappear from the env, instead of lingering
   from the prior chunk's writes.
4. Calls `require(name)` to load the freshly-readable bytes.

`reload` works against any installed package, not only
`install_local`-installed ones — if you've fetched a package and
then edited its on-disk install root, `reload` picks up the
changes too. (Whether that's a good practice is a separate
question; `install_local` is the supported way to author against
working trees.)

### `pmacs.packages.on_unload(fn)`

Registers a per-package teardown hook. Called from inside your
package's chunk:

```lua
-- In your package's init.lua:
local worker = pmacs.workers.spawn(...)

pmacs.packages.on_unload(function()
  worker:terminate()
end)

return M
```

* The owning package is recovered from the calling chunk's `_ENV`;
  no manual basename argument required.
* Calls from non-package code (top-level `init.lua`, the REPL)
  error with a pointer at `pmacs.hook.add('editor.before-quit', ...)`
  as the right venue for editor-shutdown cleanup that doesn't
  belong to a single package.
* Hooks fire on `reload(name)` and during an `install_local`
  swap that's replacing a prior install at the same name.

**Idempotence contract.** Hooks must be safe to call more than
once. If a hook fails, the next reload (or install_local
replacement) re-attempts that exact hook — pmacs doesn't skip
past a failed cleanup. A package whose `on_unload` is
`worker:terminate()` followed by an assertion that the worker is
gone will need to handle "worker already terminated" on the
retry.

**Packages that define commands must unregister them.** Any
package whose chunk calls `pmacs.command.define` at top level
needs an `on_unload` hook that hands those slots back, otherwise
`reload(name)` (and `install_local` replacement) will hit
`DuplicateName` on the second chunk run. The inverse is
`pmacs.command.unregister(name)`:

```lua
-- In your package's init.lua:
local OWNED = {}
local function define_owned(spec)
  pmacs.command.define(spec)
  OWNED[#OWNED + 1] = spec.name
end

define_owned { name = "mypkg.go", description = "…", fn = function() end }

pmacs.packages.on_unload(function()
  for _, n in ipairs(OWNED) do pmacs.command.unregister(n) end
  OWNED = {}
end)
```

`pmacs.command.unregister(name)` returns `true` if a command was
removed and `false` if `name` wasn't registered (so the loop above
is safe even after a partially-successful prior reload). Unlike
`install_local`, `unregister` is **not** init-phase-gated: it has
to work whenever `define` works (parity), and packages need to
call it from `on_unload` hooks that fire on post-init `reload(name)`
calls.

### `pmacs.fs.*` — worker-dispatched filesystem primitives

The four async fs operations packages need without reaching for
`io.*` or `os.*`. Each returns a Handle (the M3 worker pattern);
`:await()` from inside `pmacs.async(...)` yields the result.

```lua
pmacs.async(function()
  local entries = pmacs.fs.read_dir(path):await()
  -- entries[i] is a table:
  --   { name=, kind=, size=, mtime=, mtime_nsec=, mode=, symlink_target= }
  -- kind is "file" / "dir" / "symlink" / "other".
end)
```

| Function | Shape | Notes |
|----------|-------|-------|
| `read_dir(path [, opts])` | array of entry tables | Filesystem iteration order; package owns sort. lstat-based — symlinks reported as `symlink` with `symlink_target` set. |
| `stat(path [, opts])` | single entry table | Same shape as a `read_dir` entry. lstat-based. |
| `rename(from, to)` | nil on success | Atomic on the same filesystem; cross-fs returns EXDEV. |
| `chmod(path, mode)` | nil on success | **Follows symlinks** (per `chmod(2)`); changes the target's mode, not the link's. Asymmetric with `read_dir`/`stat` which use lstat. |
| `remove(path)` | nil on success | File or empty dir. Non-empty dirs fail; recurse at the package layer. Symlinks removed as symlinks (target survives). |

`opts.supersede = "<key>"` chains read ops (`read_dir`, `stat`)
into the M3 supersede semantics: a later op under the same key
cancels the earlier one's `:await()` with `{ tag = 'cancelled' }`.
Mutating ops (`rename`/`chmod`/`remove`) intentionally don't
accept `opts.supersede` — a "cancelled" syscall may have already
mutated disk.

**UTF-8 constraint.** v0.1's `pmacs.fs` requires UTF-8 paths and
entry names. A directory containing a non-UTF-8 entry surfaces a
`failed` status from `:await()` with the parent path and offending
raw bytes named. Byte-preserving paths are post-v0.1 work.

### A complete dev-loop example

```lua
-- ~/.config/pmacs/init.lua

-- Author your package at /srv/dev/pmacs-mypkg.
pmacs.packages.install_local("/srv/dev/pmacs-mypkg")

-- After editing files in /srv/dev/pmacs-mypkg, evaluate this
-- inside the running pmacs (e.g. from the scratch buffer):
--   pmacs.packages.reload("pmacs-mypkg")
-- The new code runs without restarting pmacs.
```

---

## 7a. Edit interception (`pmacs.buffer.add_intercept`)

For packages that present a buffer as a *projection of external
state* — dired, wdired, magit-style — every user edit needs to be
validated, translated to a real-world side effect, or rejected.
The edit-intercept chain is the primitive for this.

```lua
local handle = pmacs.buffer.add_intercept(buf, function(op)
  -- op.kind is "insert" | "delete" | "replace"
  -- op.bytes (insert/replace only) is the proposed bytes as a Lua string
  -- op.pos (insert) or op.start/op["end"] (delete/replace) are byte positions

  if op.kind == "insert" and op.bytes:find("\n", 1, true) then
    error("newlines not allowed in this view")  -- rejects the edit
  end
  return nil  -- nil = pass through unchanged
end)

-- Later:
pmacs.buffer.remove_intercept(handle)
```

The intercept body returns:

* `nil` — pass the edit through unchanged. Most common case.
* a table of the same `kind` with new positions — override where
  the edit lands (bytes are not mutable through the chain).
* `error(msg)` — reject the edit; the user sees `msg` as the edit's
  error.

`bytes` is surfaced as a Lua string (byte-clean: arbitrary 8-bit
content round-trips, not just UTF-8). The dired-class wdired layer
uses this to validate permission-column edits against the rwx
alphabet at intercept time, before any chmod syscall.

Multiple intercepts may be attached to the same buffer; they run in
attach order, threading the (possibly position-modified) op through
the chain.

---

## 8. The bundled REPL as a worked example

The REPL in [`builtin/packages/repl/`](../builtin/packages/repl/)
is pmacs's own first-party package. It demonstrates:

* A real (non-trivial) `pmacs.toml` with `name = "repl"`,
  `entry = "init.lua"`, `exports = ["repl"]`.
* A package that legitimately spawns processes (the `pmacs.process.spawn`
  warning is classified as expected; see
  [`tests/m7_11_acceptance.rs`](../tests/m7_11_acceptance.rs)).
* The same load path third-party packages take: at editor start,
  the bootstrap registers it in the install roster, and
  `require("repl")` resolves through the M7.7 searcher with the
  manifest's `exports` whitelist enforced.

If you're stuck modeling something in your own package, comparing
against the REPL's source is usually the fastest way to understand
the expected shape.
