# Pmacs

Parallel Emacs --- a Rust-cored, Lua-scripted editor in the Emacs tradition.

Pmacs runs the editor's hot path (rope, buffers, views, async runtime,
process supervision) in Rust, and exposes the rest --- commands, keymaps,
hooks, packages --- through an embedded Lua VM. The design follows Emacs
in shape (configurable, introspectable, programmable from inside) but
discards the single-threaded substrate; workers, message bus, and a
coroutine-based async surface are core primitives, not bolt-ons.

The editor is partitioned into a long-lived **instance** (the daemon
that owns buffers, processes, and language services) and a thin
**frontend** that attaches over a typed protocol. Frontends can run
locally over a Unix socket or remotely over SSH; reconnect-on-drop
modeled on `mosh` keeps remote sessions alive across laptop suspends.

The first-class package is a **REPL package** written entirely against
the public Lua API: PTY-spawned shells (bash, zsh, fish, lua), an
ECMA-48 ANSI parser, multi-REPL coexistence, and scrollback management
with line/byte retention. Successful completion of an audit verifying
the package uses zero direct Rust core access was the v0.1 ship gate.

## Status

**v1.0.0 --- stable.** The design described above is implemented and
working. Solo development carried the project to 1.0; public
contributions are open from this release. Use, evaluate, file issues,
and send pull requests.

## Build

Builds on the toolchain pinned in `rust-toolchain.toml` (Rust
`1.95.0`, edition 2024); rustup selects it automatically. Lua flavor selectable
between `luajit` (default) and `lua54`; both pass the full test suite.

```sh
cargo build --release             # produce target/release/pmacs
cargo run --release -- <file>     # build and run on a file
cargo test --workspace            # unit + integration tests (all crates)
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings   # incl. pmacs-gpu
```

Release-only perf gates (M5 keystroke-to-render, M6 ingest/RSS/cancel
and scrollback navigation/search) are `#[ignore]`'d during normal
test runs and exercised in CI under dedicated jobs.

## Runtime requirements

The pmacs binary depends on a small set of POSIX command-line tools
at runtime. The dependency exists because the project enforces
`#![forbid(unsafe_code)]` everywhere, including in tests; calls that
would otherwise need `unsafe` (PTY raw-mode setup, signal name
translation) are routed through trampolines that exec these tools.

- **`/bin/sh`** (POSIX shell). Used for the PTY raw-mode trampoline:
  `/bin/sh -c 'stty raw -echo </dev/tty 2>/dev/null; exec "$@"' --`
  configures the controlling TTY's line discipline before exec'ing
  the actual subprocess. Required by the REPL package and any other
  caller that spawns a process in raw PTY mode.
- **`stty`** (coreutils). The line-discipline configurator invoked
  by the trampoline above.
- **`coreutils`** more broadly. The M6 process-supervisor tests
  spawn `cat`, `yes`, and `which`; absent these the test suite (not
  the editor itself) degrades. `which` is also used by the M6.5
  shell-locator helper to find `bash` / `zsh` / `fish` for
  per-shell integration tests. The M7.2 fetcher's timeout test
  uses `sleep`.
- **`git`** (added in M7.2). Required for any package operation:
  the package fetcher shells out to `git` to clone, fetch, and
  resolve refs, with a deterministic environment
  (`GIT_TERMINAL_PROMPT=0`, `GIT_CONFIG_NOSYSTEM=1`, `LC_ALL=C`,
  inherited `GIT_*` variables stripped). Authentication for
  private repositories rides the user's existing git configuration
  (credential helpers, SSH agent), so packagers do not need a
  separate auth story. Pre-M7 builds without package operations
  do not need git.
- **`tar`** (added in M7.3). Required for `pmacs.packages.install`:
  the installer materializes a snapshot via `git archive --format=tar`
  piped into `tar -x -C <dest>`, which keeps the on-disk install
  directory self-contained (no `.git` linkage back to the bare
  cache, no working-tree state). GNU tar and bsdtar both work.
  Pre-M7 builds and any path that doesn't call
  `pmacs.packages.install{...}` do not need tar.

Distribution packagers should ensure these are runtime dependencies
of the pmacs package. On a typical Linux distribution, busybox or
GNU coreutils plus a shell of any kind satisfies the requirement; on
macOS the system shell and `/usr/bin/stty` are both standard.

The Lua VM (LuaJIT or Lua 5.4) is statically vendored via `mlua`'s
`vendored` feature, so there is no external Lua dependency at
runtime.

## What v0.1 ships with

- **Editor core.** Persistent rope with O(log N) edits and snapshots;
  buffers with chained intercept-views; undo/redo; atomic file I/O;
  crossterm-driven TUI.
- **Lua surface.** Embedded LuaJIT (or Lua 5.4) with `pmacs.command`,
  `pmacs.keymap` (global / mode / buffer scopes), `pmacs.hook`
  (typed kinds: all-must-succeed, first-non-nil, last-write-wins),
  `pmacs.buffer`, `pmacs.window`, `pmacs.editor`. Minibuffer is itself
  a buffer. `describe-key` and `describe-command` for self-introspection.
- **Async runtime.** Worker pool + message bus + coroutine-based Lua
  async surface (`pmacs.async`). Cancellation is provably correct
  under load.
- **Language services.** Tree-sitter highlighting and LSP integration
  ride the worker/message infrastructure. Project indexing as a third
  service. Symbol search across 1M+ symbols completes under a second.
- **Frontend partition.** Daemon mode with local Unix-socket transport;
  cell-delta diffing on the instance side; SSH transport variant for
  remote attach; reconnect-on-drop preserves session state across
  laptop suspend / network drop.
- **REPL package.** A 691-line Lua package that wires the M6 ANSI
  parser to PTY-spawned shells with raw-mode line discipline. Three-
  region buffer (history / prompt / input) with read-only enforcement;
  RET / C-c / C-d bindings; multi-REPL coexistence; scrollback
  retention with line- and byte-bounded truncation. Published
  alongside an audit verifying zero direct Rust core access.

## Layout

```
src/                 Rust core
  rope.rs              persistent byte-sequence backing every buffer
  buffer.rs            buffer + view chain + undo/redo
  editor_core.rs       cursor + commands + edit dispatch
  async_runtime.rs     worker pool + message bus
  process.rs           PTY-aware process supervisor
  ansi.rs              ECMA-48 parser
  daemon.rs            instance side of the frontend partition
  attach.rs            frontend side; protocol + reconnect
  lsp.rs               language-server client
  syntax.rs            tree-sitter integration
  project_index.rs     symbol / file indexing
  text_view.rs         cell-grid renderer
  frontend.rs          crossterm TUI
  lua_bindings.rs      pmacs.* Lua surface installers
  main.rs              entry point (TUI + daemon modes)

builtin/             Lua runtime shipped with the binary
  commands/default.lua  named commands for every editor primitive
  keymaps/default.lua   default key bindings
  hooks/default.lua     built-in hook definitions
  runtime/              packages (async, lsp, repl, syntax)

tests/               integration tests (acceptance gates per milestone)
```

## License

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
