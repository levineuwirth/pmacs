# Repository Audit - 2026-07-03

Scope: repository-wide audit of the current `pmacs` worktree at
`/home/jeans/Repos/active/pmacs`.

This audit covers Rust crates, Lua built-ins and fixtures, package-manager code,
GPU frontend code, documentation, repository hygiene, and verification commands.
It audits the files currently on disk.

## Worktree Context

Excluding this newly added report, `git status --short` reported these
untracked worktree files at the end of the audit:

```text
?? #run.sh#
?? docs/pmacs-gpu-editing-perf-handoff.md
?? docs/semantic-frontend-protocol.md.local-bak
?? docs/session-5-stale-styling-handover.md
?? pmacs
?? python_experiment.md
```

The audit did not revert or normalize this state.

## Verification Summary

Commands run:

| Command | Result | Notes |
|---|---:|---|
| `cargo fmt --all --check` | Pass | Formatting is clean. |
| `cargo clippy --all-targets -- -D warnings` | Pass | This only checked the root default package from this workspace location. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Fail | `pmacs-gpu` has 3 denied clippy warnings. See F-001. |
| `cargo test -p pmacs-gpu --bin pmacs-gpu` | Pass | 39 tests passed on the current worktree. |
| `cargo test -p pmacs-protocol` | Pass | 12 tests passed; 0 doctests. |
| `cargo test -p pmacs --no-default-features --features lua54 --lib` | Pass | 1431 passed, 0 failed, 3 ignored. |
| `cargo test -p pmacs --features crdt --lib` | Pass | 1601 passed, 0 failed, 3 ignored. |
| `cargo test --workspace --all-targets --all-features` | Fail | `mlua-sys` rejects enabling both `luajit` and `lua54`. See F-002. |
| `cargo run --quiet --bin pmacs-audit -- --pretty builtin tests/fixtures` | Pass with warnings | 0 errors, 3 warnings, 10 infos. |

Sandbox caveat: an initial `cargo test --workspace --all-targets` run failed in
three attach/socket tests with `PermissionDenied: Operation not permitted`.
Rerunning the attach lib-test group with normal socket permissions passed
(`44 passed`). I did not treat that sandbox failure as a product defect.

## Findings

### F-001 - High - Workspace clippy fails, while the documented clippy command misses workspace members

Evidence:

- `README.md:37-43` documents:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
- `cargo metadata --no-deps --format-version 1` reports `workspace_default_members`
  as only the root `pmacs` package.
- `cargo clippy --all-targets -- -D warnings` passed, but
  `cargo clippy --workspace --all-targets -- -D warnings` failed.
- The workspace clippy failures are in `pmacs-gpu/src/main.rs`:
  - `pmacs-gpu/src/main.rs:2712`: two `clippy::cast_possible_wrap` errors from
    `centered as i64 - self.scroll_top as i64`.
  - `pmacs-gpu/src/main.rs:2948`: `clippy::format_push_string` from
    `readout.push_str(&format!(...))`.

Impact:

The documented release/developer clippy command can give a false green while
`pmacs-gpu` is not clippy-clean under the workspace's lint policy. This makes CI
and local release validation vulnerable to missing frontend regressions.

Recommendation:

Fix the three `pmacs-gpu` lints, then update README and CI to use workspace-aware
commands, e.g. `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace --all-targets`. Alternatively set explicit workspace
`default-members` if the intent is for plain `cargo test` and `cargo clippy` to
cover all first-party crates.

Resolution (PR #75, then extended): the three `pmacs-gpu` lints were fixed and
CI now runs `cargo clippy -p pmacs-gpu --all-targets` (README uses `--workspace`).
That closed the *clippy* blind spot but not the symmetric *test* one — the
default-member scope also means `cargo test` skips `pmacs-protocol`. F-014's
GPU-render job later ran `pmacs-gpu`'s tests; this change adds
`cargo test -p pmacs-protocol --all-targets` to the test job so the shared wire
format (its ~12 encode/decode + transport-framing tests) is finally exercised in
CI. All three first-party crates now run in CI.

### F-002 - Medium - `--all-features` is not buildable because Lua features are mutually exclusive

Evidence:

- `Cargo.toml:63-75` defines `default = ["luajit"]`, `luajit`, and `lua54`.
- `cargo test --workspace --all-targets --all-features` fails in `mlua-sys` with:

```text
You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau
```

Impact:

Generic CI, packaging automation, `cargo hack`, and distro tooling often starts
with `--all-features`. For this workspace, that produces a hard build failure
even though both Lua flavors pass independently.

Recommendation:

Document the supported feature matrix explicitly and avoid `--all-features` in
CI. Prefer a matrix such as default LuaJIT, `--no-default-features --features
lua54`, and `--features crdt`. If possible, add a clearer crate-local compile
error for simultaneously enabled Lua flavors so users see a pmacs-specific
message before the `mlua-sys` failure.

Resolution (PR TBD): documented the feature matrix explicitly — a table in
`README.md` §Build (the two Lua flavors + orthogonal `crdt`, the supported
build lines, and an explicit "don't use `--all-features`"), a `# Lua flavor
features` section in `src/lib.rs`'s crate docs, and an expanded `Cargo.toml`
`[features]` comment. CI already avoids `--all-features` (it iterates the
flavors explicitly), so no CI change was needed.

The suggested crate-local `compile_error!` was **investigated and rejected as
unreachable**: the flavor check lives in the `mlua-sys` *build script*, which
cargo compiles before the `pmacs` crate, so any mis-set flavor (both, or
neither) fails there first and `pmacs`'s own `compile_error!` never evaluates
— confirmed empirically for both cases. A dependent crate cannot preempt a
dependency's build failure, so the honest mitigation is the documented matrix
rather than a guard that can never fire. The docs state that the actual error
surface is the `mlua-sys` message.

### F-003 - Medium - `pmacs-gpu` can appear hung when attached to a non-CRDT daemon

Evidence:

- `pmacs-gpu/src/attach.rs:119-128` documents the failure mode directly: without
  daemon `crdt`, negotiation succeeds, no `BufferSnapshot` arrives, and the GPU
  window sits on `(connecting...)` forever.
- `pmacs-gpu/src/attach.rs:129-148` sends an `AttachRequest` requesting
  `multi_frontend`, `crdt_replica`, and `semantic_render`, but there is no
  explicit capability rejection path in the GPU client after the request.

Impact:

This is a confusing operational failure. A user can start the daemon without the
right feature and get an inert GPU window instead of a clear compatibility error.

Recommendation:

Make capability negotiation explicit. The daemon should reject required
capability mismatches with a structured `Goodbye` or handshake error, and the GPU
frontend should render a fatal status explaining that the daemon must be started
with `--features crdt`.

### F-004 - Medium - Current GPU command-chord forwarding can break AltGr/international text input

Evidence:

- `pmacs-gpu/src/main.rs:4674-4688` reduces winit modifiers to Shift, Ctrl, Alt,
  and Super/Meta. There is no AltGraph/AltGr distinction in the protocol
  modifier set (`pmacs-protocol/src/message.rs:92-115`).
- `pmacs-gpu/src/main.rs:4775-4779` treats any `Char`, `Enter`, or `Tab` with
  Ctrl or Alt held as a command chord.
- The current tests assert that this generalized command-chord path forwards
  `Char` plus Ctrl/Alt combinations (`pmacs-gpu/src/main.rs:5822-5853`).

Impact:

On many keyboard layouts, printable characters are entered through AltGr, which
toolkits may expose as Ctrl+Alt or as an Alt-related modifier. Those characters
can be misclassified as command chords and forwarded to the daemon keymap instead
of inserted as text. This affects non-US layouts and users typing characters
such as `@`, `[]`, `{}`, `\`, `|`, or currency symbols depending on layout.

Recommendation:

Use winit's text/IME path or an explicit AltGraph signal to distinguish printable
text input from command chords. Add tests for AltGr-style printable input. If the
wire protocol needs it, add an AltGraph modifier bit rather than folding it into
Ctrl/Alt.

Resolution (PR #76, then hardened): a keypress that produced printable
`key.text` while a command modifier was held is reclassified as text input and
the modifiers are stripped (`is_layout_text`). The first cut gated on *any*
command modifier, which over-reached: on macOS the `Option` key is reported as
`Alt` and emits printable text for most letters, so `Option+x` was stripped to a
plain insert — swallowing every GUI Meta binding (`M-x`, `M-f`, …) on that
platform. The gate is now the true AltGr signature — **both `Ctrl` and `Alt`**
(the LCtrl+RAlt the OS synthesizes for AltGr on Windows) — so `Alt`-alone forwards
as a Meta chord again while Windows AltGr still inserts. A protocol AltGraph bit
was *not* added; the text-presence heuristic is sufficient and layout-agnostic.
Not locally validatable on macOS (no box) — a narrowing of when we strip, so
low-risk on Linux/Windows.

### F-005 - Medium - Package install paths collide for namespaced packages with the same basename

Evidence:

- `src/packages/installer.rs:44-49` states that install directories are named by
  package basename and accepts collisions for v0.1.
- `src/packages/installer.rs:1040-1051` strips a namespace prefix with
  `package_basename`.
- `src/packages/loader.rs:43-48` and `src/packages/loader.rs:164-176` route
  require lookup by basename, with most-recent install winning on collisions.

Impact:

Two distinct packages such as `owner/magit` and `other/magit` can target the same
install directory and require namespace. Depending on order, one can shadow or
block the other. A resolver can reason about canonical names while the installer
and loader collapse them to the same basename.

Recommendation:

Reject duplicate basenames during resolution/install until a namespace-aware
install layout exists, or install under a namespace-preserving path. If basename
requires remain a compatibility goal, add an explicit aliasing rule rather than
silently collapsing distinct package names.

### F-006 - Medium - Atomic save has file-metadata and crash-durability gaps

Evidence:

- `src/file_io.rs:139-148` creates a new sibling temp file, writes bytes,
  `sync_all`s the temp, then renames it over the target.
- The save path does not copy the existing file mode/permissions onto the temp
  before rename.
- The save path does not fsync the parent directory after rename.
- `src/file_io.rs:156-174` uses PID plus subsecond nanoseconds for the temp name
  and relies on `create_new` failing if a collision occurs; it does not retry.

Impact:

Saving an existing executable or otherwise specially-permissioned file can replace
it with default temp-file permissions. On POSIX filesystems, a crash after rename
but before the parent directory entry is synced can lose the rename despite the
file bytes having been synced. The temp-name collision path is unlikely, but a
same-process collision would surface as a spurious save failure.

Recommendation:

Before writing, read existing metadata and apply the target's mode to the temp
file where supported. After a successful rename, fsync the parent directory on
Unix. Add a bounded retry loop for temp-name collisions.

### F-007 - Medium - Minibuffer dropdown height is unbounded and can render off-screen

Evidence:

- `pmacs-gpu/src/main.rs:3160-3164` computes dropdown `top_y` as
  `band_top - n as f32 * MB_DROP_ROW_HEIGHT`.
- `pmacs-gpu/src/main.rs:3176-3193` draws a background rect and selected row for
  all candidates without clipping/windowing the candidate list.

Impact:

Large completion lists can extend above the top of the window. The selected row
can be outside the visible area, and hit testing/rendering can become inconsistent
with what the user can see.

Recommendation:

Cap visible rows based on available height, keep a scroll/window offset around
the selected candidate, clamp `top_y`, and make hit testing use the same visible
window.

### F-008 - Medium - GPU attach outbound queue is unbounded

Evidence:

- `pmacs-gpu/src/attach.rs:156` uses `std::sync::mpsc::channel`.
- `pmacs-gpu/src/attach.rs:186-199` writes queued events on a separate writer
  thread to avoid blocking the UI thread.
- `pmacs-gpu/src/attach.rs:326-333` sends every `FrontendEvent` into that
  unbounded queue.

Impact:

If the daemon or socket stalls, the UI thread can continue enqueueing key,
pointer, paste, viewport, and CRDT events without backpressure. This can grow
memory and can also deliver stale pointer/viewport traffic after the daemon
recovers.

Recommendation:

Use a bounded channel with a clear policy. Coalesce superseded events such as
viewport and pointer motion. Consider failing fast or entering a degraded state
when the writer cannot keep up.

### F-009 - Low - Package fetch cache key uses non-cryptographic 64-bit FNV-1a

Evidence:

- `src/packages/fetcher.rs:161-165` derives the bare-repo cache path from a hash
  of the normalized URL.
- `src/packages/fetcher.rs:510-518` implements 64-bit FNV-1a and describes it as
  "collision-resistant enough for a cache key."

Impact:

The package cache is keyed by potentially attacker-controlled repository URLs.
A deliberate or accidental hash collision could make two URLs share one bare
mirror path and lock file, mixing refs or producing confusing installs. This is
not a content-integrity bypass by itself, but it is avoidable risk in
security-adjacent package infrastructure.

Recommendation:

Use SHA-256 or another cryptographic digest of the normalized URL for cache
directory names. Store the normalized URL in a sidecar file for diagnostics.

### F-010 - Low - Timed-out git subprocesses return before stdout/stderr drain threads are joined

Evidence:

- `src/packages/fetcher.rs:599-604` spawns stdout/stderr drain threads.
- `src/packages/fetcher.rs:611-617` kills and waits for the child on timeout,
  then returns `FetchError::Timeout` immediately.
- `src/packages/fetcher.rs:630-631` only joins the drain threads on the normal
  completion path.

Impact:

Repeated fetch timeouts can leave short-lived detached reader threads behind
until their pipe reads finish. It is unlikely to leak permanently after the child
is killed, but it makes timeout behavior less deterministic and harder to test.

Recommendation:

After killing and waiting for the child, join the drain threads before returning
the timeout error, or restructure process execution around a timeout-aware
`wait_with_output` helper.

### F-011 - Low - `ResolvedPackage::commit` is documented as a SHA but can hold a tag string

Evidence:

- `src/packages/resolver.rs:143-144` documents `ResolvedPackage.commit` as a
  "Full 40-character commit hash."
- `src/packages/resolver.rs:1036-1044` returns the tag string from
  `TagCandidate::commit_for_tag`.
- `src/packages/lockfile.rs:573-610` compensates by resolving the commit-ish to a
  SHA before writing the lockfile.

Impact:

The lockfile path is protected, but the internal type contract is misleading.
Downstream code that trusts `ResolvedPackage.commit` as a SHA can accidentally
re-resolve a moving commit-ish or use the field in error messages and markers as
if it were immutable.

Recommendation:

Either resolve tags to SHA inside the resolver before constructing
`ResolvedPackage`, or rename the field to `revision`/`commitish` and update docs
and call sites to reflect the actual contract.

### F-012 - Low - Resolver topological sort contains dead/shadowed indegree work

Evidence:

- `src/packages/resolver.rs:705-715` builds and mutates an `indegree` map.
- `src/packages/resolver.rs:735-738` immediately shadows it with the actual
  `indegree` map used by the algorithm.

Impact:

This is a small inefficiency and a readability hazard in dependency-ordering
code. The surrounding comments already show the algorithm was corrected in place,
but the abandoned block remains.

Recommendation:

Delete the first `indegree` construction block and keep only the final outgoing
dependency-count implementation.

### F-013 - Low - GPU documentation and crate metadata are stale relative to implementation

Evidence:

- `pmacs-gpu/Cargo.toml:6` still describes the crate as "session 2:
  hello-world."
- `docs/pmacs-gpu-design.md:3-5` says the GPU work is pre-implementation with
  sessions queued.
- The current GPU code has attach, CRDT, rendering, minimap, diagnostics,
  minibuffer/menu, mouse, and editing code.

Impact:

New contributors and future audit passes start from incorrect project state.
This also makes it harder to decide which TODOs are live versus historical.

Recommendation:

Update `pmacs-gpu` metadata and the top of the design note to point at the
current status and the active per-phase framing/audit docs.

### F-014 - Low - GPU visual behavior lacks a committed screenshot/pixel regression harness

Evidence:

- `pmacs-gpu/src/main.rs:5670-6750` contains 39 unit tests focused on helper
  logic and vertex construction.
- The design note explicitly deferred acceptance-test shape to golden-frame or
  screenshot comparison (`docs/pmacs-gpu-design.md:356-360`).
- I did not find an automated headless GPU screenshot/pixel test in `tests/` or
  `pmacs-gpu/src`.

Impact:

Layout and rendering regressions can pass the current unit suite. This matters
because the frontend has many pixel-local responsibilities: hit testing, dropdown
layout, minimap projection, diagnostic underlines, selection/current-line
backgrounds, and status/minibuffer composition.

Recommendation:

Add at least one deterministic headless render smoke test or recorded-frame
pixel/golden harness. Start with narrow cases: nonblank frame, text visible,
dropdown visible and clipped, minimap visible, diagnostic underline visible, and
resize behavior.

### F-015 - Low - Repository contains backup/build artifacts and untriaged untracked files

Evidence:

- `find` located backup/editor artifacts:
  - `#run.sh#`
  - `docs/semantic-frontend-protocol.md.local-bak`
- `git status --short` also showed an untracked root-level `pmacs` file, which
  appears likely to be a local build/run artifact.
- Several untracked docs are present and may be intentional, but they need
  explicit triage before commit.

Impact:

Backup and binary artifacts increase review noise and can be committed by
accident. Untracked docs make it unclear which design notes are canonical.

Recommendation:

Remove or `.gitignore` editor backups and local binaries. Triage untracked docs
as either real project documents or local handoff notes.

### F-016 - Low - Very large source files are maintenance hotspots

Evidence from line counts:

```text
15202 src/lua_bindings.rs
 7093 src/editor.rs
 6761 pmacs-gpu/src/main.rs
 3729 src/lsp.rs
 3495 src/semantic_render.rs
 3178 src/buffer.rs
 2988 src/editor_core.rs
```

Impact:

These files concentrate unrelated behavior, make review difficult, and increase
the chance of hidden coupling. `src/lua_bindings.rs` in particular combines many
Lua API domains, package management glue, LSP stores, theme bindings, attachment
bindings, and tests in one file.

Recommendation:

Split only along stable boundaries, not as a drive-by refactor. Good candidates:
Lua package APIs, Lua LSP APIs, Lua theme APIs, GPU input handling, GPU render
pipelines, GPU layout/dropdown logic, and editor command groups.

## Lua Audit Notes

The Lua audit command reported no errors. Warnings were:

- `builtin/packages/repl/init.lua:274`: `pmacs.process.spawn(spec)`.
- `tests/fixtures/pmacs-magit/gestures.lua:357`: `io.open(tmpfile, "w")`.
- `tests/fixtures/pmacs-magit/status.lua:55`: `pmacs.process.spawn { ... }`.

These are not automatically defects. The REPL package needs process access, and
fixture warnings are expected if those packages deliberately exercise filesystem
or process APIs. They should still be checked against package manifests whenever
fixtures graduate into real packages.

## Suggested Priority Order

1. Fix F-001 and update workspace verification commands.
2. Fix F-003 and F-004 before treating `pmacs-gpu` as user-ready.
3. Fix F-006 before relying on pmacs for editing permission-sensitive files.
4. Decide the package basename-collision policy in F-005 before wider package
   ecosystem growth.
5. Add the GPU visual regression harness in F-014 before large layout changes.
