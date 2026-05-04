--- @meta pmacs.packages
---
--- EmmyLua-style annotations for the `pmacs.packages.*` Lua surface
--- (T M7.3, spec §sec:packages-future). This file is a documentation
--- artifact: editor tooling (lua-language-server, EmmyLua) consumes it
--- to power completion and hover; the runtime implementation lives in
--- Rust (`src/lua_bindings.rs::install_packages_module`).
---
--- The runtime never `require`s this file. It is shipped under
--- `builtin/api/` so packagers know to include it alongside the
--- binary; users who want IDE completion add this directory to their
--- workspace's lua-language-server `Lua.workspace.library`.

--- A package install spec.
---
--- Two accepted shapes:
---
--- - Table with positional address at `[1]`. The pin is one of three
---   mutually-exclusive fields:
---     - `version = "<semver>"` (default; constrains to a tag).
---     - `branch = "<name>"` (HEAD of the named branch at install time).
---     - `commit = "<sha>"` (specific revision; full or partial SHA).
---   Specifying more than one of these errors with a "must specify
---   exactly one" message naming every conflicting field. With none
---   specified, the pin defaults to `version = "*"` (any tag).
---   The `install_project` variant additionally **requires**
---   `project_root = "..."` (no default).
--- - Shorthand string `"github:owner/repo@^1.0.0"`. **Version-pin only.**
---   The separator is the **last** `@` in the string, so addresses
---   containing an `@` (SSH shorthand `git@host:path`) parse correctly.
---   Branch and commit pins must use the table form. The shorthand
---   form is also not accepted by `install_project` (no place to put
---   `project_root`).
---
--- @class PackageInstallSpec
--- @field [1] string                   Positional address (e.g., `"github:owner/repo"`).
--- @field address string|nil           Alternative to the positional `[1]`.
--- @field version string|nil           Semver constraint (e.g., `"^1.0.0"`, `"=1.2.3"`, `"*"`). Mutually exclusive with `branch` / `commit`. Defaults to `"*"` when none of the three are specified.
--- @field branch string|nil            Branch name (e.g., `"main"`). The install resolves the branch's HEAD at install time; not reproducible across time. Mutually exclusive with `version` / `commit`.
--- @field commit string|nil            Commit SHA (full or partial). The install pins to that exact revision. Mutually exclusive with `version` / `branch`.
--- @field project_root string|nil      `install_project` only: REQUIRED project root. Absolute paths used as-is. Relative paths resolve against the directory of the loading `init.lua` (not against CWD). Common patterns: `os.getenv("PMACS_PROJECT")`, or a literal subdirectory like `"."` for "alongside this init.lua".

--- The pin info on an [`InstalledPackage`].
---
--- @class InstalledPackagePin
--- @field kind "version"|"branch"|"commit"  Which pin kind the user supplied.
--- @field value string                       The user-supplied value: the semver constraint (e.g. `"^1.0.0"`), the branch name (e.g. `"main"`), or the commit SHA. Echoes the field on `PackageInstallSpec` exactly.

--- A successful-install record returned by `install` and listed by `installed`.
---
--- @class InstalledPackage
--- @field name string                  Package name from `pmacs.toml` (e.g., `"samplepkg"` or `"user/samplepkg"`).
--- @field version string               Manifest-declared version of the installed snapshot (canonical semver, e.g. `"1.2.3"`).
--- @field tag string                   Resolution descriptor: matched tag (`"v1.2.3"`) for version pins, `"branch:<name>"` for branch pins, `"commit:<short-sha>"` for commit pins. Always non-empty.
--- @field commit string                40-character commit SHA of the installed snapshot.
--- @field install_path string          Absolute on-disk install directory.
--- @field entry string                 Absolute path to the package's `entry` Lua module.
--- @field scope "user"|"project"       Which scope the package was installed under.
--- @field summary string               One-line description from the manifest.
--- @field pin InstalledPackagePin      Structured pin info (the user's request, distinct from the resolved descriptor).

local pmacs = pmacs or {}
pmacs.packages = pmacs.packages or {}

--- Install a package to the user-config root (`$XDG_DATA_HOME/pmacs/packages/`).
---
--- Synchronous: clones / fetches the address, picks the highest semver tag
--- matching `version`, materializes the snapshot via `git archive | tar -x`,
--- and makes the package's entry module requireable as
--- `require(<package-name-basename>)`.
---
--- Resolution path: standard layouts (`<basename>.lua`,
--- `<basename>/init.lua`) are found via `package.path`. Non-standard
--- entries (e.g. `entry = "main.lua"` or `entry = "lib/foo.lua"`) are
--- found by a custom searcher pmacs registers in `package.searchers`
--- (Lua 5.4) / `package.loaders` (LuaJIT, Lua 5.1), which consults
--- the install roster and returns the manifest's exact entry path.
---
--- **Init-time-only.** Calling outside `init.lua` raises an error pointing
--- at the workaround (restart pmacs after editing `init.lua`). Mid-session
--- install is not supported in v0.1; M7.6 adds `pmacs.packages.update(...)`
--- for in-place version changes.
---
--- @param spec PackageInstallSpec|string  Spec table, or shorthand string `"address@constraint"`.
--- @return InstalledPackage
--- @throws "init-only" if called after init has finished.
--- @throws "no matching version" if no tag satisfies `version`.
--- @throws "already installed" if a different commit occupies the install path.
function pmacs.packages.install(spec) end

--- Install a package to a project-scoped root (`<project_root>/.pmacs/packages/`).
---
--- Identical to `install` except for the on-disk root. **Requires** an
--- explicit `project_root` field in the spec table. Absolute paths are
--- used as-is; relative paths resolve against the directory of the
--- loading `init.lua` (not against the process CWD, which is rarely a
--- meaningful project root). The shorthand string form is not accepted.
---
--- Project installs override user installs of the same package basename
--- in `package.path` (project entries are prepended).
---
--- **Init-time-only.** See `install` for the gate.
---
--- @param spec PackageInstallSpec
--- @return InstalledPackage
--- @throws "init-only" if called after init has finished.
--- @throws "no matching version" if no tag satisfies `version`.
--- @throws "already installed" if a different commit occupies the install path.
--- @throws "missing project_root" if the spec table omits the `project_root` field. The error message names two patterns for filling it in: `os.getenv("PMACS_PROJECT")` for an env-var-driven setup, or a path relative to the loading `init.lua`'s directory.
function pmacs.packages.install_project(spec) end

--- Snapshot the in-memory roster of packages installed during this init pass.
---
--- Each entry is the same shape as `install`'s return value. The list is
--- ordered by install order (first call first).
---
--- @return InstalledPackage[]
function pmacs.packages.installed() end

--- Re-resolve and update an installed package to the latest commit
--- matching its constraint.
---
--- **Implemented in M7.6** (lockfile + resolver). v0.1 / current builds
--- raise an error pointing at the workaround: re-run `pmacs.packages.install`
--- with the new constraint to upgrade in place.
---
--- @param name string|nil  Package name to update; omit to update all.
--- @throws "unsupported" until M7.6 ships.
function pmacs.packages.update(name) end

return pmacs.packages
