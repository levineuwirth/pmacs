# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **This file is frozen at 1.1.0** (released 2026-08-01). Nothing is
> added below this note; the record of what ships from here is the
> [GitHub releases page](https://github.com/levineuwirth/pmacs/releases).
> The sections that follow are kept as history and describe work that
> shipped before 1.1.0.

## [Unreleased]

### Added

#### Semantic-frontend protocol scaffolding (M11.1)

First milestone of the M11 semantic-frontend arc (see
`docs/semantic-frontend-protocol.md`). Wire-format scaffolding only —
no producer or consumer yet; mechanically identical to the M10.5 CRDT
wire declaration.

- `PROTOCOL_VERSION` bumped 2 → 3; `SUPPORTED_PROTOCOL_VERSIONS` now
  `[1, 2, 3]`. The slice-membership handshake keeps v0.1/v1.0 binaries
  connecting to v1.1 binaries unchanged.
- New `semantic_render` capability bit on `FrontendCapabilities`,
  `InstanceCapabilities`, and `NegotiatedCapabilities`
  (`#[serde(default)]`; instance default `false` until the M11.2
  projection seam lands). `negotiate_capabilities` AND-combines it and
  enforces the `semantic_render ⇒ crdt_replica` dependency (a semantic
  session is also a text replica) — rejected as a
  `CapabilityMismatch`, never silently degraded.
- `InstanceMessage` gains the `SemanticFrame` family: `StyleSpans`,
  `Decorations`, `InlineAdornments`, `BlockAdornments`, `FoldState`,
  `ResourceOffer`; `FrontendEvent` gains `Viewport`. All anchored in
  byte offsets — the instance never learns a pixel.
- `PMACS_INSTANCE_SEMANTIC_RENDER` env override mirrors the existing
  per-capability test overrides.

#### Semantic projection seam (M11.2)

The first real producer. The instance now projects syntax styling to
`semantic_render` sessions without rasterizing to a cell grid.

- New `SemanticRenderState` (`src/semantic_render.rs`), sibling of
  `instance_render::RenderState`: reads the same `EditorState` but
  emits `InstanceMessage::StyleSpans` — tree-sitter spans mapped
  through the active `Theme`, scoped and clipped to the byte range the
  frontend declared via `FrontendEvent::Viewport`. Emits nothing until
  a viewport is declared; suppresses byte-identical frames (true
  span-granularity diffing is M11.4).
- `StyleSpans.generation` is anchored to `CrdtState::version_scalar()`
  — the oplog version vector summed to one monotonic non-decreasing
  scalar, letting a frontend discard styling that predates an edit it
  already applied optimistically.
- Dispatcher selects the projection **per session**: a semantic
  session gets a `SemanticRenderState` and never `CellDelta`/grid
  `Cursor` (it lays out locally) but still receives `CursorByte`,
  `BufferSnapshot`, `CrdtOp`, and presence. A grid and a semantic
  frontend can attach to the same buffer simultaneously.
  `FrontendEvent::Viewport` is consumed (routed by authenticated
  source, like `CrdtOp`).
- `InstanceCapabilities` default `semantic_render` flipped to
  `cfg!(feature = "crdt")` — the "M11.2 enables semantic" moment,
  analogous to the M10.8 Day-4 `multi_frontend`/`crdt_replica` flip.

#### Decorations projection (M11.3)

`SemanticRenderState` now also projects `InstanceMessage::Decorations`
from the editor state pmacs actually has instance-side:

- **Selection** — per-window (per-frontend) byte-native state, scoped
  to the session's active window for the declared buffer and clipped
  to the viewport. `DecorationKind::Selection`.
- **Diagnostics** — the shared `DiagnosticStore`, keyed by the file
  URI the LSP glue opened the document under (`lsp::path_to_file_uri`
  is now `pub(crate)`, byte-identical to the Lua `file_uri_for`).
  LSP `(line, col)` is converted to a byte range against the buffer
  source; severity maps to `DiagnosticError`/`Warning`/`Info`/`Hint`.
- `StyleSpans` and `Decorations` suppress unchanged frames
  independently — a selection move does not force a styling re-send.
- `Decorations::SearchMatch`/`SearchMatchActive`/`CurrentLine` are not
  emitted: pmacs has no instance-side search-hit store, and
  current-line is a pure cursor derivation the frontend already owns
  via `CursorByte` (emitting it would breach the contract boundary).
- `InlineAdornments`/`BlockAdornments`/`FoldState` remain unproduced
  by design — pmacs has no inlay-hint/blame/lens/fold/diff source
  yet. The wire variants exist (M11.1); their producers are wired
  when those features land. Honest stubs, not fabricated data.

#### Segment diffing (M11.4)

Coarse whole-payload re-sends replaced with a `CellDelta`-style diff,
lifted from positional cells to byte-anchored ranges.

- `StyleSpans`/`Decorations` refined: `{ buffer_id, generation,
  full: bool, segments: Vec<…Segment> }`. `full = true` is a resync
  (frontend discards prior state for the buffer); `full = false`
  ships only the dirty byte regions, each `StyleSegment` /
  `DecorationSegment` replacing styling within its range. Bytes in no
  segment keep prior state; an unchanged span overlapping a dirty
  range is faithfully reconstructed (segments carry *all* current
  items intersecting the range, not only changed ones). `Decorations`
  also gains `generation` for parity with `StyleSpans`.
- The diff: symmetric difference of the previous/current ordered
  item sets, changed ranges coalesced into maximal disjoint dirty
  intervals. First frame and any viewport-region change force `full`;
  an unchanged frame ships nothing. Byte offsets cascade on edits, so
  an incremental frame after an edit dirties `[edit, viewport_end)` —
  bounded; no-edit frames (cursor/scroll/selection) stay free.
- `ResourceOffer` remains an honest stub (no resource-bearing
  adornment producer exists yet) — same discipline as M11.3.

#### Semantic frontend↔instance glue (M11.5)

The arc's consumer side and end-to-end coverage. pmacs has no GUI
toolkit, so — per the design note's testability strategy — the
deliverable is the bounded testable glue, not a GPU renderer.

- New headless `SemanticClient` (`src/semantic_client.rs`, `crdt`-
  gated): composes the `BufferMirror` rope replica (M10.10) with a
  tile-based `SemanticModel` that reconstructs styling/decorations
  from the `full` + dirty-segment deltas (M11.4). Self-contained:
  no terminal, no pixels. Emits `FrontendEvent::Viewport`; exposes
  read-back accessors (`text`, `effective_style_at`,
  `decoration_kinds_at`, tile ranges). The M11.4 contract (segments
  carry every current item intersecting their range) makes a tile
  self-contained, so incremental application is a clean per-tile
  replacement with edge-clipping, not cross-span surgery.
- `tests/m11_5_semantic_acceptance.rs`: (a) reconstruction-
  equivalence — an incrementally-driven client is asserted byte-for-
  byte identical to a fresh full projection across a scripted
  viewport/edit/selection sequence including a viewport jump (the
  golden discipline without a snapshot crate); (b) end-to-end —
  a real daemon routes `StyleSpans`/`Decorations` to a semantic
  session (after it declares a `Viewport`) and never to a grid
  session, and `CellDelta` vice versa, validating the M11.2
  per-session projection through the socket.

This completes the M11 semantic-frontend arc (M11.1–M11.5): wire +
capability scaffolding, the instance-side projection seam,
decorations, segment diffing, and the consumer-side glue with
end-to-end coverage. `InlineAdornments`/`BlockAdornments`/`FoldState`/
`ResourceOffer` remain honest stubs pending their source features.

## [1.0.0] --- 2026-05-18

First stable release. Builds on the 0.1.0 preview (M1–M6) with the
M7–M10 arc: third-party packages, the universality proof across three
shape-distinct workloads, MCP, and multi-frontend CRDT collaboration.
Solo development through 1.0; public contributions open from this
release.

### Added

#### Third-party package system (M7)

- Package install at user-config and project scope
  (`pmacs.packages.install` / `install_project`) with `version` /
  `branch` / `commit` pins and a lockfile.
- Per-package environment with `exports`; `require` resolution scoped
  so packages can declare module-level state without cross-package
  collision.
- Edit intercepts: packages observe and transform buffer mutations;
  multiple intercepts thread in attach order.
- Audit-lint rule set (the package-trust surface): error/warning/info
  patterns across environment-escape, fs-mutation, process-spawn, and
  reach-around classes; runnable locally and in CI.

#### Filesystem API and bundled packages (M8)

- `pmacs.fs.*` worker-dispatched surface: `read_dir`, `stat`,
  `rename`, `chmod`, `remove` with M3 `supersede` chaining on read
  ops; UTF-8 path constraint surfaced as a structured failure.
- Three shape-distinct bundled packages proving the buffer
  universality claim: dired (directory listing), magit (section
  view), outline / outline-aggregate (derived structure views).
- Dev-loop APIs: `pmacs.packages.install_local`, `reload`,
  `on_unload`.

#### MCP for package authors (M9)

- Model Context Protocol surface so package authors can expose
  editor capability to AI assistance over the same protocol-uniform
  path as the rest of the system (see `docs/mcp-for-package-authors.md`).

#### Multi-frontend CRDT collaboration (M10)

- CRDT layer (loro-backed, feature-gated) with an optimistic-apply
  keystroke path; daemon owns the authoritative replica, frontends
  mirror it.
- Two laptops attach to one daemon instance, both edit one file,
  edits converge within a frame; per-frontend undo; presence and
  per-frontend cursor rendering.
- Convergence verified under simulated network jitter
  (`PMACS_INSTANCE_LATENCY_JITTER_MS`); automated synthetic-frontend
  and doubled-PTY acceptance plus a two-laptop manual checklist.

#### Pulled-forward v0.2-prerequisite APIs

These planned v0.2 promotions were implemented and shipped in 1.0
after the public-API ceiling was raised by operator decision:

- `pmacs.buffer.from_file`, `pmacs.buffer.on_removed` (with an
  idempotent `:remove()` handle; buffer-local keymaps pruned on
  removal).
- `bypass_intercept` option on `buffer.insert` / `delete` / `replace`
  (skips the intercept chain only; undo/dirty/view/CRDT bookkeeping
  preserved).
- `pmacs.fs.watch` (polling watcher; `:cancel()` /
  `:is_cancelled()`).
- `pmacs.async.yield_to_next_tick` (worker-free next-tick yield).
- `pmacs.editor.move_to_line` (0-based, clamps out of range).
- `pmacs.outline.query` (cross-package outline structure query).
- Audit-lint rule 16 `reach-around-require-field` (info).

### Changed

- **SSH remote attach now carries the wire protocol over the SSH
  *stderr* channel by default** (previously stdout). On at least one
  tested environment (`OpenSSH_10.3p1`), a non-PTY SSH session does
  not forward a long-lived remote process's stdout until that process
  exits, while stderr streams in real time; the `--daemon-attach`
  bridge is exactly such a process, so a stdout-carried protocol hung
  indefinitely (the daemon and bridge were correct throughout). The
  default flip resolves this with no user action required. Override
  per invocation with `PMACS_ATTACH_SSH_PROTOCOL=stdout|stderr` (the
  legacy `PMACS_ATTACH_SSH_PROTOCOL_STDERR=0/1` is still honored).
  Breadth is currently a single environment; the default is a
  one-line internal switch so it can be revisited without downstream
  impact.

### Fixed

- SSH remote attach no longer fails with a host-side
  `send Hello failed: Broken pipe`: the daemon-liveness probe in the
  `--daemon-attach` auto-start path was a disposable connect that
  could consume the daemon's server-speaks-first `Hello`; the
  established connection is now reused (regression-tested).

### Known limitations

- **Per-frontend undo history does not persist across reattach.**
  After a frontend disconnects and reconnects, it is issued a fresh
  collaboration identity; its pre-disconnect edits remain in the
  converged document (CRDT state is fully restored) but are no longer
  reachable by that frontend's undo command — undo only reaches
  edits made since reconnecting. Workaround: remove unwanted
  pre-disconnect content by editing it out. Preserving undo history across
  reconnect remains a v0.2 follow-up.
- **macOS REPL ctrl-c / exit-marker timing.** Two M6.5 REPL
  acceptance tests (`m6_5_ctrl_c_sends_sigint`,
  `m6_5_exit_marker_uses_basename_with_leading_newline`) time out on
  macOS CI runners. The core daemon, CRDT, and remote-attach paths —
  including the Mac-host collaboration scenario — build and pass on
  macOS (the lib suite is green there); the limitation is confined
  to bundled-REPL signal/exit-marker timing. Tracked for v0.2.

### Project posture

- `forbid(unsafe_code)` maintained across the crate; syscalls that
  need unsafe are bridged via `/bin/sh` / coreutil / binary
  trampolines or safe `nix` wrappers.
- Rust toolchain pinned (`rust-toolchain.toml`, 1.95.0) so local and
  CI build the same validated compiler; bumped deliberately.
- Cross-flavor CI: every test runs under both `luajit` and `lua54`;
  clippy `-D warnings` clean on both flavors and the `crdt` lane;
  release-only perf-gate jobs.
- Library suite ~1223 tests (non-crdt) / ~1377 (crdt), zero failing
  on the pinned toolchain, plus per-milestone integration suites
  through M10.

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

[1.0.0]: https://example.invalid/pmacs/releases/tag/v1.0.0
[0.1.0]: https://example.invalid/pmacs/releases/tag/v0.1.0
