# Project detection

pmacs identifies project roots by walking upward from a file's
parent directory looking for a marker (`Cargo.toml` for Rust,
`package.json` for Node, `.git` as a generic VCS fallback, etc.).
The walk stops at the first match, with language-specific markers
preferred over generic VCS roots when both exist at the same level.

The default behavior matches `git rev-parse --show-toplevel`: walk
all the way to the filesystem root.

## When the default surprises you

A file under `/tmp/scratch.rs` will be classified as part of a
project rooted at `/tmp` if `/tmp/.git` exists — because the walk
finds the marker before hitting the filesystem root. This is the
same surprise `git`, `cargo`, and other tools produce; it's
predictable but occasionally inconvenient.

The escape hatch is `pmacs.project.set_search_boundary(path)`. Set
this in your `init.lua` to clamp the upward walk so a stray marker
high in the tree cannot capture unrelated files.

```lua
-- Restrict project detection to walk only within ~/code.
-- Files outside ~/code will not have a project root detected.
pmacs.project.set_search_boundary(os.getenv("HOME") .. "/code")
```

The boundary is *inclusive*: a marker located at the boundary path
itself is still found. Set the boundary to the directory that
contains your projects, not to one level above. To restore the
default behavior (walk all the way to the filesystem root), pass
`nil`:

```lua
pmacs.project.set_search_boundary(nil)
```

## Symlinks

The boundary applies *after* symlink resolution. When the
boundary is `/home/user/code` and a search starts from a symlinked
path that resolves into `/home/user/code/...`, the walk respects
the boundary correctly.

This matters for two common setups:

- Corporate `/home` mounts, where `/home/user` may be a symlink to
  `/var/empire/users/user` or similar — the boundary you set
  against your visible home path still works.
- User-organized symlink farms (e.g., `~/work/foo` linked to
  `~/code/foo`) — search from the symlinked path still terminates
  at the boundary you set against the canonical location.

If the boundary path or the search start does not exist on disk,
canonicalization falls through to the literal path; the comparison
becomes lexical. This affects pre-creation tests but not normal
operation.

## API summary

```lua
pmacs.project.set_search_boundary(path)  -- set, or nil to clear
pmacs.project.search_boundary()          -- current value, or nil
pmacs.project.detect(file_path)          -- honors the boundary
```

`pmacs.project.detect(file_path)` returns
`{ root, kind, language_id }` for the detected project, or `nil` if
no marker matches before the boundary (or the filesystem root, when
no boundary is set).

## Design notes

We considered several alternatives to the unbounded walk, and
chose the opt-in boundary as the most predictable:

- **Hard-coded stops** at `$HOME` / `/tmp` / mount points break
  legitimate cases (someone's project lives under `/srv/work`,
  someone's `$HOME` is `/var/jeans` over SSH, etc.).
- **Ownership-based stops** ("walk while same uid as the start
  file") break shared-dev setups and read-only repo mounts, and add
  a `stat` per ancestor.
- **Confidence-weighted detection** (heuristically score "real
  project-ness") sacrifices the property that makes detection
  useful: predictability.

Matching `git`'s behavior keeps detection's failure mode consistent
with the rest of the user's toolchain. The boundary gives users
who care a precise, configurable opt-in without imposing a
specific policy on everyone.

## Forward planning

The boundary is workspace-scoped (one boundary per `Workspace`
instance). When project-local `init.lua` lands (post-v0.1) we may
extend this to a per-project boundary, or to a stack of boundaries
that nested project loads can push and pop. The v0.1 surface is
deliberately minimal so those future extensions don't break
existing user config: setting a single workspace-wide boundary in
your global `init.lua` continues to do exactly what it does today.
