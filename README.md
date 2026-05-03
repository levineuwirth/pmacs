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

**v0.1.0 --- preview.** The design described above is implemented and
working. Solo development through 1.0; public contributions are
deferred until then. Use, evaluate, and file issues; pull requests
will get a friendly "thanks, see you at 1.0" until that gate.

## Build

Requires Rust `1.85` or newer (edition 2024). Lua flavor selectable
between `luajit` (default) and `lua54`; both pass the full test suite.

```sh
cargo build --release             # produce target/release/pmacs
cargo run --release -- <file>     # build and run on a file
cargo test                        # unit + integration tests
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Release-only perf gates (M5 keystroke-to-render, M6 ingest/RSS/cancel
and scrollback navigation/search) are `#[ignore]`'d during normal
test runs and exercised in CI under dedicated jobs.

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
