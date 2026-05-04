# Package installation

pmacs's package surface lives at `pmacs.packages.*` in Lua. Two
install variants ship in v0.1: `install` (user-config scope) and
`install_project` (project scope). Both run synchronously during
init; mid-session install is not supported.

## `pmacs.packages.install { ... }` — user-config scope

Installs to `$XDG_DATA_HOME/pmacs/packages/<basename>/` (or
`$HOME/.local/share/pmacs/packages/...` if `XDG_DATA_HOME` is unset).
The package's entry module is wired into Lua's require resolution so
`require("<basename>")` returns the module's table.

```lua
pmacs.packages.install {
    "github:owner/repo",
    version = "^1.0.0",
}
```

The shorthand string form is also accepted (version pins only):

```lua
pmacs.packages.install "github:owner/repo@^1.0.0"
```

### Pin kinds: `version`, `branch`, `commit`

Each install pins exactly one revision. The spec table chooses the
pin via one of three mutually-exclusive fields:

```lua
-- Highest semver tag matching the constraint. Recommended default.
pmacs.packages.install { "github:owner/repo", version = "^1.0.0" }

-- HEAD of the named branch at install time. Not reproducible across
-- time --- the upstream's branch HEAD moves --- so use sparingly.
pmacs.packages.install { "github:owner/repo", branch = "main" }

-- Exact commit. Reproducible: the same SHA always installs the same
-- snapshot. Useful for pinning to a known-good state before the
-- upstream has tagged a release.
pmacs.packages.install { "github:owner/repo", commit = "abc1234" }
```

Mutual exclusion is enforced: a spec table with two of these fields
errors with a "must specify exactly one" message naming every
conflicting field. With none of the three, the pin defaults to
`version = "*"` (any tag).

The shorthand string form (`"address@^1.0"`) is **version-pin only**.
Branch and commit pins must use the table form because there is no
unambiguous sigil that distinguishes a branch named "main" from a
malformed semver constraint without surprising users.

For version pins, pmacs additionally cross-checks that the
manifest's `version` field at the matched tag satisfies the user's
constraint, catching upstreams whose tag and `pmacs.toml` disagree.
Branch and commit pins skip that check (the user explicitly asked
for that revision regardless of what the manifest says).

## `pmacs.packages.install_project { ... }` — project scope

Installs to `<project_root>/.pmacs/packages/<basename>/`. Project
installs are prepended to `package.path` so they take precedence
over user-config installs of the same basename.

```lua
pmacs.packages.install_project {
    "github:owner/repo",
    version = "^1.0.0",
    project_root = "/abs/path/to/project",
}
```

### `project_root` is required

`install_project` requires an explicit `project_root`. There is **no
fallback to the process CWD**: at init time CWD is whatever shell
directory the user happened to invoke pmacs from, which is almost
never a meaningful project root.

If you omit the field you get a typed error that names two concrete
patterns for filling it in. Pick whichever fits:

#### Pattern A: an environment variable (CI, scripts, multiple machines)

```lua
pmacs.packages.install_project {
    "github:owner/repo",
    version = "^1.0.0",
    project_root = os.getenv("PMACS_PROJECT"),
}
```

The user (or the CI runner) sets `PMACS_PROJECT=/path/to/project`
before invoking `pmacs`. The path is stable across invocations
regardless of where the shell happened to be.

#### Pattern B: a path relative to the loading `init.lua`

```lua
pmacs.packages.install_project {
    "github:owner/repo",
    version = "^1.0.0",
    project_root = ".",
}
```

Relative paths in `project_root` resolve against the directory
**containing the loading `init.lua`**, *not* against CWD. So
`project_root = "."` means "alongside this init.lua";
`project_root = "subdir"` means a subdirectory of the init.lua's
directory.

This works because pmacs's loader stamps each chunk with a `@<path>`
source label (the standard Lua convention for file-loaded chunks);
the install binding reads that label back from a per-eval app-data
slot to recover the chunk's directory.

Edge case: when running pmacs's package API from a Lua chunk that
was *not* loaded from a file (e.g., via `pmacs --eval ...`, or from
the M-x command-line evaluator), there is no source label. Relative
`project_root` values then fall through to "as-is," matching the
pre-v0.1 CWD interpretation. This is intentionally ad-hoc: the only
flow that matters in v0.1 is init.lua, and string-loaded chunks
that need an exact path can use Pattern A or pass an absolute path
literally.

### Forward planning: project-local `init.lua`

When project-local `init.lua` lands (post-v0.1), the project loader
will set a "current project root" before evaluating the project's
init.lua, and `install_project` from inside that init.lua will pick
up the project root automatically — no `project_root` field needed.
The user-global init.lua path will continue to require an explicit
field, since it has no implicit project context.

The change will be relaxation, not breakage: code that explicitly
passes `project_root` keeps working unchanged.

## How `require` resolution works

pmacs uses Lua's standard require machinery, augmented at install
time:

1. **Path-based search.** Each install prepends
   `<install_root>/?.lua;<install_root>/?/init.lua` to
   `package.path`. Packages with the conventional layout
   (`<basename>.lua` or `<basename>/init.lua`) resolve via this
   path with no further machinery — exactly as a hand-written Lua
   project would.

2. **Custom searcher.** When the path-based search misses (e.g.
   the manifest declares `entry = "main.lua"` or `entry =
   "lib/foo.lua"`), a custom searcher pmacs registered in
   `package.searchers` (Lua 5.4) / `package.loaders` (LuaJIT and
   Lua 5.1) consults the install roster, finds the matching
   package by basename, and returns a loader for the exact entry
   path declared in the manifest.

The searcher iterates the roster in install order, most-recent
first, so a project-scope install of a basename overrides a prior
user-scope install of the same basename — mirroring the
"newer-installs-prepend-to-path" semantics of the path-based
search.

When `require` cannot find a name through any searcher, the
combined error message names every searcher's contribution; the
custom searcher's contribution looks like:

```
no installed pmacs package named 'whatever'
```

so a user with a typo can spot it without digging into pmacs's
internals.

## `pmacs.packages.installed()`

Returns an array of records describing every package installed
during the current init pass. Each record has the same shape as
`install`'s return value:

```lua
{
    name = "samplepkg",            -- manifest's name
    version = "1.0.0",             -- manifest's declared version
    tag = "v1.0.0",                -- resolution descriptor (see below)
    commit = "abc...",             -- full SHA of the installed snapshot
    install_path = "...",
    entry = "...",
    scope = "user",                -- or "project"
    summary = "...",
    pin = {                        -- structured user request
        kind = "version",          -- or "branch" or "commit"
        value = "^1.0.0",          -- echoes the spec field exactly
    },
}
```

The `tag` field is a stable, non-empty descriptor:

- For version pins: the matched tag (`"v1.2.3"`).
- For branch pins: `"branch:<name>"`.
- For commit pins: `"commit:<short-sha>"`.

The `pin` table is the source of truth for "what did the user
request." The flat fields (`tag`, `version`, `commit`) record the
resolution. They differ for branch/commit pins, where the resolved
commit is what got installed but the user's request was the branch
name or SHA prefix.

## `pmacs.packages.update(...)`

Stubbed for v0.1. M7.6 implements re-resolution and lockfile
regeneration. Until then, re-running `install` with a new constraint
upgrades in place.

## Errors and how they're shaped

Every install error message names the operation, the input that
caused the failure, and (where applicable) the workaround. Examples:

- `pmacs.packages.install_project requires an explicit project_root field. Pass project_root = "/path/to/your/project" (often os.getenv("PMACS_PROJECT") or a path relative to the directory containing your init.lua).`
- `package at tag v2.0.0 of github:owner/repo requires pmacs ">= 2.0.0", but this pmacs is "0.1.0". Upgrade pmacs, or pin a package version compatible with "0.1.0".`
- `no tag for github:owner/repo satisfies "^99.0.0". Available tags: ["v1.0.0", "v1.1.0"]`

The convention is that the error stands on its own — read in a CI
log or stack trace, the user can see what to do without context.
