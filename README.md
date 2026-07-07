# Pmacs

Parallel Emacs --- a Rust-cored, Lua-scripted editor in the Emacs tradition.

Pmacs runs the editor's hot path (rope, buffers, views, async runtime,
process supervision) in Rust, and exposes the rest --- commands, keymaps,
hooks, packages --- through an embedded Lua VM. The design follows Emacs
in shape (configurable, introspectable, programmable from inside) but
discards the single-threaded substrate; workers, message bus, and a
coroutine-based async surface are core primitives, not bolt-ons.

The editor is partitioned into a long-lived **instance** (the daemon
that owns buffers, processes, and language services) and thin
**frontends** that attach over a typed protocol (currently v14). Two
frontends ship today:

- a **TUI** (crossterm cell grid), attachable locally over a Unix
  socket or remotely over SSH, with reconnect-on-drop modeled on
  `mosh`; and
- **pmacs-gpu**, a GPU frontend (wgpu + winit + glyphon) that renders
  from a *semantic projection* of editor state --- style spans,
  decorations, inlay adornments --- rather than a character grid, and
  edits optimistically against a local CRDT replica for
  latency-free typing.

Buffers are optionally CRDT-backed (`loro`, behind `--features crdt`),
so multiple frontends --- TUI and GPU, local and remote --- can edit
the same buffers concurrently with live cursor/selection presence.

## Status

**v1.0.0 --- stable core, active development.** The v1.0 gate (the
instance/frontend partition, the Lua surface, and a REPL package
audited to use zero direct Rust core access) shipped some time ago;
development since has landed the GPU frontend at near input/render
parity with the TUI, the semantic-frontend protocol (v6 → v14), the
LSP feature arc, in-buffer search, the context menu + OS clipboard,
the line-number gutter with diagnostic signs, and the package-manager
hardening pass. Current direction lives in `docs/roadmap-2026-07.md`.
Public contributions are open: use, evaluate, file issues, send pull
requests.

## Highlights

**Editing & UI.** CUA-style region editing plus Emacs kill/yank
bindings; linear undo/redo; incremental search, substring and regex
(`C-s` / `C-r` / `C-M-s`); line-number gutter with absolute, relative,
and hybrid modes; diagnostic gutter signs; right-click context menu;
OS clipboard integration (OSC 52 in the TUI, native in the GPU);
minibuffer with completion dropdown and per-bucket persisted history;
buffer-list mode (`C-x C-b`); self-navigable help system
(`describe-command`, `describe-key`); atomic saves (temp + rename +
parent fsync, mode-preserving).

**Language intelligence.** LSP client with async, never-blocking
requests: diagnostics (severity-colored underlines/squiggles, gutter
signs, statusline counts, `M-g n`/`M-g p` navigation), rename with
`prepareRename`, go-to-definition including cross-file navigation,
hover, signature help, references, document symbols, code actions,
buffer formatting, semantic tokens, and inlay hints (rendered inline
in the GPU frontend). Servers are preconfigured for rust-analyzer,
clangd (C/C++), basedpyright, gopls, typescript-language-server
(TS/TSX/JS/JSX), lua-language-server, bash-language-server, taplo
(TOML), and zls (Zig). Syntax highlighting is dual-authority:
bundled tree-sitter grammars (Rust, Lua, Markdown, C, C++) paint
lexical structure and LSP semantic tokens refine it --- languages
without a bundled grammar still get full semantic coloring. A
persistent project symbol index (`.pmacs/index.json`) rides the same
worker infrastructure.

**Collaboration & frontends.** With `--features crdt`, buffers are
CRDT-backed and any number of frontends attach to one daemon and edit
concurrently; peers see each other's cursors and selections as
translucent washes. The GPU frontend adds a live minimap (click to
jump, drag to scrub), wavy diagnostic squiggles, a status band with
live diagnostic counts, and optimistic local editing that rebases
in-flight edits through authoritative frames.

**Extensibility.** ~37 `pmacs.*` Lua namespaces cover buffers,
windows, commands, keymaps (global/mode/buffer scope), hooks, themes
(truecolor-capable syntax palette), tree-sitter, LSP stores, async
workers, and a PTY-aware process supervisor with an ECMA-48 ANSI
parser. A package manager installs from git (`github:owner/repo`,
version/branch/commit pins) with transitive dependency resolution and
a SHA-256 lockfile. Pmacs is also an **MCP client**: packages can
spawn MCP servers and consume their tools, resources, and prompts ---
AI integrations are packages over a transport, not a built-in
feature. The bundled REPL package (PTY shells, ANSI rendering,
multi-REPL, scrollback retention) is written entirely against the
public Lua API.

## Running

Single-process TUI:

```sh
pmacs [FILE]                 # TUI; -nw reserved for when a GUI default lands
```

Daemon + attached frontends (build with `--features crdt` for
multi-frontend editing and the GPU frontend):

```sh
pmacs --daemon --socket NAME           # foreground daemon; bare NAME →
                                       #   <runtime>/pmacs/NAME.sock
pmacs --attach --socket NAME           # TUI frontend; F12 detaches
pmacs --attach user@host               # remote TUI over SSH
pmacs-gpu --attach /run/user/$UID/pmacs/NAME.sock   # GPU frontend
```

`pmacs --attach` also understands `ssh:user@host/instance`,
`local:/path.sock`, and bare hostnames (treated as SSH). See
`pmacs --help` for the full matrix.

User configuration is plain Lua at
`$XDG_CONFIG_HOME/pmacs/init.lua` (default `~/.config/pmacs/init.lua`),
loaded after the builtin runtime so plain assignments override
defaults --- keybindings, `pmacs.lsp.config`, theme overrides, and
package installs all live there.

## Build

Builds on the toolchain pinned in `rust-toolchain.toml` (Rust
`1.95.0`, edition 2024); rustup selects it automatically.

```sh
cargo build --release             # target/release/pmacs (LuaJIT flavor)
cargo build --release --features crdt        # + CRDT buffers (daemon use)
cargo build --release -p pmacs-gpu           # the GPU frontend binary
cargo run --release -- <file>     # build and run on a file
cargo test --workspace            # unit + integration tests (all crates)
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings   # incl. pmacs-gpu
```

### Feature matrix

Cargo features fall into two independent axes. **Do not use
`--all-features`** — it enables both Lua flavors at once, which cannot
build (see below).

| Feature  | Axis       | Notes                                                       |
| -------- | ---------- | ---------------------------------------------------------- |
| `luajit` | Lua flavor | **Default.** LuaJIT backend via `mlua` (vendored).         |
| `lua54`  | Lua flavor | Lua 5.4 fallback for hosts without LuaJIT (big-endian, …). |
| `crdt`   | Buffer     | Opt-in CRDT-backed buffer mode (adds the `loro` dep). v1.0 builds enable it; orthogonal to the flavor. |

**Exactly one Lua flavor must be enabled** — `luajit` *or* `lua54`, never
both (and never neither). They map to `mlua`'s mutually-exclusive Lua
backends, so `--all-features` (or `--features luajit,lua54`, or
`--no-default-features` with no flavor) fails in the `mlua-sys` build
script with *"You can enable only one of the features: …"*. That check
lives in a dependency cargo builds first, so pmacs can't replace it with a
friendlier error — the fix is to build a specific flavor. Supported build
lines:

```sh
cargo build --release                                   # luajit (default)
cargo build --release --no-default-features --features lua54
cargo build --release --features crdt                   # luajit + crdt
cargo build --release --no-default-features --features lua54,crdt
```

CI, `cargo hack`, and distro tooling should iterate the flavors
explicitly (`--no-default-features --features <flavor>[,crdt]`) rather
than reaching for `--all-features`. Both flavors pass the full test suite;
CI runs the matrix on every push.

Release-only perf gates (M5 keystroke-to-render, M6 ingest/RSS/cancel
and scrollback navigation/search) are `#[ignore]`'d during normal
test runs and exercised in CI under dedicated jobs. The GPU frontend
has headless render tests (offscreen wgpu, pixels read back) that run
in CI under lavapipe and skip gracefully on machines without a Vulkan
adapter (`PMACS_REQUIRE_GPU=1` turns a missing adapter into a hard
failure).

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

The GPU frontend additionally needs a Vulkan-capable driver stack
(any real GPU driver, or lavapipe for software rendering); its font
(JetBrains Mono, OFL-licensed) is bundled into the binary.

## Layout

The workspace has three first-party crates:

```
src/                 pmacs — the core + TUI + daemon
  rope.rs              persistent byte-sequence backing every buffer
  buffer.rs            buffer + view chain + undo/redo
  editor_core.rs       cursor + commands + edit dispatch
  crdt.rs              loro-backed CRDT buffer state (feature `crdt`)
  daemon.rs            instance side of the frontend partition
  attach.rs            frontend side; transports + reconnect
  semantic_render.rs   semantic-frame producer (StyleSpans, Decorations, …)
  lsp.rs               language-server client
  diag.rs, highlight.rs  diagnostic + syntax/semantic-token rendering
  syntax.rs            tree-sitter integration
  search.rs            incremental search (substring + regex)
  minibuffer.rs        prompt, completion, persisted history
  menu.rs              context-menu model
  file_io.rs           atomic saves + external-modification detection
  async_runtime.rs     worker pool + message bus
  process.rs           PTY-aware process supervisor
  ansi.rs              ECMA-48 parser
  project.rs, project_index.rs  project detection + symbol index
  packages/            resolver, fetcher, installer, lockfile, loader
  mcp.rs               MCP client (packages speak to MCP servers)
  lua_bindings/        pmacs.* Lua surface installers
  text_view.rs         cell-grid renderer
  frontend.rs          crossterm TUI
  main.rs              entry point (TUI / daemon / attach modes)

pmacs-protocol/      wire types + framing codec shared by all frontends
pmacs-gpu/           the GPU frontend (wgpu + winit + glyphon)

builtin/             Lua runtime shipped with the binary
  commands/default.lua  named commands for every editor primitive
  keymaps/default.lua   default key bindings
  hooks/default.lua     built-in hook definitions
  menus/default.lua     context-menu items
  runtime/              async, lsp, syntax, mcp, fs runtimes
  packages/repl/        the bundled REPL package

docs/                design notes, framing docs, and the roadmap
tests/               integration tests (acceptance gates per milestone)
```

## License

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
