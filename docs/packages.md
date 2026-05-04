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

The shorthand string form is also accepted:

```lua
pmacs.packages.install "github:owner/repo@^1.0.0"
```

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

## `pmacs.packages.installed()`

Returns an array of records describing every package installed
during the current init pass. Each record has the same shape as
`install`'s return value (`name`, `version`, `commit`,
`install_path`, `entry`, `scope`, `summary`).

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
