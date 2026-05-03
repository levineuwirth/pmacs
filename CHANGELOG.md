# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] --- 2026-05-03

First public preview. Solo development through 1.0; public contributions
deferred until then.

### Added

#### Editor core (M1)

- Persistent rope with O(log N) insert / delete / slice / snapshot.
- Buffers with chained `View` trait subscribers and intercept-edit hooks.
- Undo / redo history with edit grouping.
- Atomic file load / save.
- Crossterm-driven TUI; cell-grid renderer abstracted for future GUI.
- Single-buffer text editing as the M1 deliverable: open a file, navigate,
  edit, undo, save.

#### Lua and command system (M2)

- Embedded Lua VM (LuaJIT default; Lua 5.4 supported via feature flag;
  cross-flavor test gates on both).
- `pmacs.command.define` / `invoke` / `exists`; named introspectable
  commands for every M1 primitive.
- `pmacs.keymap.bind` with global / mode / buffer scopes; multi-key
  sequences (e.g., `C-x C-s`); buffer-scope wins over mode wins over
  global.
- `pmacs.hook` with typed kinds: `all-must-succeed`, `first-non-nil`,
  `last-write-wins`. Built-in hook surface (`buffer.before-save`,
  `process.after-tick`, etc.).
- Minibuffer implemented as a buffer (validates the universality of
  the buffer abstraction early).
- `describe-key` / `describe-command` for self-introspection.

#### Async, workers, message bus (M3)

- Worker pool with stealable queues; coroutine-based `pmacs.async`
  surface for Lua.
- Typed message bus; cancellation tokens that propagate through
  awaiting coroutines; `supersede` keys for "newer query cancels older."
- Acceptance test: parallel grep across a directory tree with cancel
  on new query, p99 cancel under 50 ms.

#### Language services (M4)

- Tree-sitter highlighting as a worker-pool service; per-buffer parse
  caches; incremental reparse on edit.
- LSP client (initialize / didOpen / didChange / didSave / completion /
  hover / definition / formatting / signature / diagnostics) with
  per-server async coroutines.
- Project indexing service; symbol search across 1M+ symbols completes
  under a second.

#### Frontend partition and remote attach (M5)

- Daemon mode (`pmacs --daemon`) owning buffers, processes, language
  services as a long-lived instance.
- Typed protocol over framed Unix-socket transport;
  `FrontendCapabilities`, `Hello`, `AttachRequest`, `FrontendEvent`,
  `InstanceMessage` (CellDelta / Cursor / ModeLine / Bell).
- Cell-delta diffing on the instance side; only changed cells go on
  the wire.
- SSH transport variant for remote attach; reconnect-on-drop modeled
  on `mosh`. Buffers, cursors, and process state preserved across
  laptop suspend / network drop / SSH server restart.
- Keystroke-to-render p99 ≤ 10 ms over loopback `LocalSocket`
  (M5.9c perf gate).

#### REPL package and ANSI (M6)

- ECMA-48 ANSI parser: SGR (8/16/256/24-bit color, bold, italic,
  underline variants, strikethrough, reverse), intra-line cursor
  motion, line-level erase, OSC title-setting, bracketed paste,
  alt-screen suppression. Recovery rule: skip to end of CSI/OSC
  structure, drop to ground at next `ESC` or after 1 KiB, never
  crash/hang/corrupt.
- PTY-aware process supervisor: raw / canonical line-discipline modes,
  SIGWINCH delivery on resize, child-exit cleanup, per-generation
  bounded byte channel (1 MiB / 128 slots) with per-tick coalescing.
- REPL package (`builtin/runtime/repl.lua`, 691 lines): three-region
  buffer (history / prompt / input) with read-only enforcement on
  history; PTY-spawned shells (bash, zsh, fish, lua); buffer-scoped
  RET (submit) / C-c (SIGINT) / C-d (close stdin or delete-char-
  forward); exit markers with basenames and symbolic signal names;
  multi-REPL coexistence with per-handle parser/scrollback/binding
  state.
- Output backpressure perf gates: sustained ingest ≥ 100 MB/s over
  10 s; RSS delta ≤ 200 MB during the run sampled at 100 Hz; cancel
  response p99 ≤ 100 ms over 100 trials.
- Scrollback management: configurable retention (10000 lines /
  16 MiB defaults); single-pass tick-driven truncation removing
  oldest complete command-output blocks; navigation latency p99
  ≤ 16 ms across 10000-line scrollback; search p99 ≤ 100 ms;
  truncation correctness across 50 events.
- Multi-REPL: three concurrent REPL buffers render and respond
  independently; supervisor handles multiple PTY children without
  resource leaks (verified across 10 spawn-close cycles).
- API audit (the v0.1 ship gate): REPL package source 691 lines
  (46% of 1500-line ceiling); 15 external functions, all through
  documented `pmacs.X.Y` namespaces; zero direct Rust core access;
  zero reach-around patterns. Seven findings classified; one
  promoted-and-fixed in-audit (supervisor-record leak across
  spawn-close cycles, closed by restructuring the close path so
  `_on_exit` is the single point of teardown including
  `pmacs.process.forget`).

### Project posture

- `forbid(unsafe_code)` across the entire crate. Syscalls that need
  unsafe (e.g., `sysconf`) are bridged via `/bin/sh` / coreutil /
  binary trampolines.
- Cross-flavor CI: every test runs under both `luajit` and `lua54`.
  Perf gates run release-only under dedicated CI jobs
  (`m4-perf-gates`, `m5-perf-gates`, `m6-perf-gates`).
- 914 lib tests + per-milestone integration suites
  (m1-acceptance through m6_8, m6_perf, m4-acceptance, m5_5..m5_8,
  m6_4..m6_7).

[0.1.0]: https://example.invalid/pmacs/releases/tag/v0.1.0
