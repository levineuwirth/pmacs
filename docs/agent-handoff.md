# Agent handoff — cross-machine continuity

**Last updated: 2026-07-23, with the approved one-command GPU invocation
broker implemented on open PR #141, after the documentation refresh (#140),
Vterm Stage 3 (#135, protocol v19 and native GPU terminal), tab-width
rendering parity (#137), locals-query processing (#134), modeline detection
(#132), mode system wiring (#129), config registry (#127), Vterm Stages 1–2
(#126/#130), and completed Themes Arc 4 (#120/#124/#125).**
This file is the
bridge between development machines. If you are an agent reading
this on a fresh clone: this document plus the `docs/*-framing.md`
files ARE your memory. Read this fully before taking on work, seed
your persistent memory from it, and **update this file (and commit
it) whenever project state changes materially** — the next machine
reads it the way you just did.

For volatile branches, checkpoints, verification, and recovery
commands, read `docs/active-work.md` immediately after this file.

## 1. Where the project stands (2026-07-23)

- `main` @ `96d0bae` (documentation refresh #140 atop Vterm Stage 3 #135
  and tab-width parity #137), protocol **v19** (`SUPPORTED=[6..=19]`; v16 =
  `ThemeFacts`, v17 = `FontFacts`, v18 = `StatuslineSegments`, v19 = terminal
  frames/events).
- **One-command GPU invocation is implemented on OPEN PR #141, not yet on
  `main`** (`gpu-invocation`, implementation checkpoint `6fd5834`, framing
  Revision 4). The additive public path is `pmacs --gpu [--socket NAME|PATH]`;
  bare `pmacs [FILE]` remains the TUI. Root owns the CRDT gate, socket
  resolution, sibling-first GPU discovery/PATH fallback, and GPU outcome.
  The separate `pmacs-gpu` binary owns connect-or-start, a five-second /
  50-ms retry window, daemon process-group isolation, and named child
  reaping. Direct `pmacs-gpu --attach RAW_PATH` remains strict and never
  auto-starts. No protocol change.
- **Config registry LANDED — #127** (`docs/config-registry-framing.md`
  rev 3; merge `2e37c04`; two review rounds). `pmacs.config` is the
  typed, introspectable options registry the backlog ranked first, and
  it closes the "config-registry-blocked" deferrals below. It was built
  as a PARALLEL LANE alongside vterm in a sibling worktree; the files
  were assigned per-lane up front and the rebase had zero conflicts.
  - **Third registry** beside `CommandRegistry`/`HookRegistry`
    (`src/config_registry.rs`), same R42/R50/duplicate-rejection/
    `SourceLocation` vocabulary; Lua surface in
    `src/lua_bindings/config.rs`. **No protocol change (still v18) and
    ZERO changes to `src/editor.rs`.**
  - **An override is ALWAYS stored**, even when equal to the value it
    shadows; only `value_epoch` and listener dispatch key on effective
    change. The "equal-value set is a no-op" reading silently voids a
    buffer-local pin: nothing is stored, and a later global `set` flips
    the very buffer the user pinned.
  - **Two scopes: global and buffer-local.** `get(name, buf)` resolves
    local → global → default; **`get(name)` resolves the GLOBAL CHAIN
    ONLY** and never consults an ambient buffer. Per-language and
    per-project are *patterns* (a hook calling `set_local`), not scopes
    the registry knows about. Mode keymaps now resolve through #129, but
    `pmacs.config` deliberately remains global + buffer-local.
  - Buffer-locals live in a registry side table purged at
    `after_buffer_removed`, beside the keymap purge.
  - Listeners: commit → snapshot → **drop the borrow** → re-enter Lua;
    a raising listener is logged without blocking the rest or rolling
    back; a depth bound turns a cycle into a pointed error. **Explicit
    dispose only** — there is no `MetaMethod::Gc` anywhere in the
    codebase, and GC timing differs between the two Lua backends.
  - `StartupOnly` freezes off the existing `InitCompleteFlag` at write
    time (which is why no `editor.rs` call was needed). In `--lib`
    builds `set_init_complete` never runs, so a post-freeze test must
    flip the flag explicitly or it passes vacuously.
  - Adopters own their own `define`, so `SourceLocation` names the
    owning module: `editing.auto-pair` (pair.lua, read against the
    typed edit's SOURCE buffer), `editing.trim-on-save` (editops.lua,
    read against the buffer being saved), `autosave.interval-ms`
    (autosave.lua, re-read per tick). **The migration wrappers keep
    their legacy coercion** — the registry is strict, the legacy setters
    stay lenient (`trim_on_save("yes")`, `interval_ms(1500.7)`).
  - `M-x describe-setting` renders into `*help*`.
- **Mode system wiring LANDED — #129** (`docs/mode-system-wiring-framing.md`;
  merge `b4b925d`; one review round). The existing mode-keymap substrate is
  now live without a protocol change.
  - `Buffer.major_mode: Option<String>` owns the single major mode. The
    detected language name initializes it once on `buffer.after-load`, before
    grammar gating, so server-only languages work; switches never rewrite it.
    Explicit overrides and clears survive switches. A future reload that fires
    after-load re-detects, and explicit mode state is not session-persisted.
  - Dispatch borrows the zero-or-one mode through `Option<&str>::as_slice()`
    and `&[&str]`: no hot-path mode allocation. Resolution remains
    buffer-local → mode → global, and registry/keymap borrows end before Lua
    command invocation.
  - Lua surfaces: `pmacs.buffer.major_mode` / `set_major_mode` and
    `pmacs.editor.active_modes`. `pmacs.describe.key`, `pmacs.help.show_key`,
    and followed percent-encoded `@mode:` links use the same effective context;
    `pmacs.keymap.lookup` remains raw-global.
  - The built-in `mode` statusline provider reads `ctx.buffer`, so passive
    splits render their own mode. Real-daemon acceptance covers all ten framing
    criteria across both Lua backends and Linux/macOS CI.
- **Modeline language detection LANDED — #132**
  (`docs/modeline-detection-framing.md` rev 2; merge `1dd47fc`). Fresh loads
  scan bounded Emacs `-*- mode: ... -*-` and Vim/Vi `ft=` / `filetype=`
  modelines without evaluating file content, normalize common editor aliases,
  and give explicit modelines precedence over inferred language.
  - `builtin/runtime/syntax.lua` owns one per-buffer fresh-load decision:
    modeline → bundled grammar extension → LSP filetype extension → exact
    filename → shebang. Syntax, initial major mode, pairing, comments, and LSP
    all reuse that pin; LSP retains its independent backing-path guard.
  - Editing a modeline or shebang does not switch an attached parser or make
    language-aware consumers diverge. Close/reopen re-evaluates changed file
    metadata. Explicit post-load major-mode overrides remain independent.
  - Bounded valid unknown names remain passive major modes without starting an
    unavailable parser or server. All thirteen framing criteria are covered on
    LuaJIT and Lua 5.4. No Rust or protocol surface changed; protocol stays v18.
- **Syntax-highlight / language-detection side-quest (#114–#118)
  LANDED** — a one-shot arc built in sibling worktrees off main while
  the user's themes lane (`theme-faces`) ran concurrently in the shared
  checkout. All merged. What shipped:
  - **Grammars** (`crate::syntax::BUILTIN_LANGUAGES`): every
    LSP-configured language now has one — cuda, bash, dockerfile (via
    the ABI-current `tree-sitter-containerfile`, NOT the dead
    `tree-sitter-dockerfile` which pins `tree-sitter ^0.20`), make,
    cmake, python, go, javascript (+jsx), typescript (+tsx), toml, zig.
  - **Detection chain and pin** (`builtin/runtime/syntax.lua`): modeline →
    grammar extension → LSP filetype map → filename → shebang. User-extensible
    Lua surfaces include `pmacs.parse.modeline_aliases`, `.shebangs`,
    `.filenames`, `language_from_modeline`, `language_from_shebang`,
    `language_from_filename`, and the pinned `buffer_language`.
    `builtin/runtime/lsp.lua` delegates to that shared decision after enforcing
    its backing-path requirement. Grammar name MUST equal the
    `pmacs.lsp.config.<name>` key. A buffer keeps its pinned language across
    edits/switches; close/reopen performs a fresh bounded inference.
  - **LSP configs added**: dockerfile (`docker-langserver --stdio`),
    cmake (`cmake-language-server`, config via
    `init_options.buildDirectory="build"` — it does NOT pull
    `workspace/configuration`). Make has no server.
  - **Substrate**: `LanguageEntry.highlights_query` and `.locals_query` are
    `&[&'static str]` fragments joined base-first (cuda over c/cpp; ts over
    js/jsx). Since locals-query processing #134, settle compiles the
    grammar's `LOCALS_QUERY`, resolves Tree-sitter's scope/definition/value/
    reference conventions into sorted `LocalFacts`, and stores them beside
    each layer's tree/query. Work runs once per fresh bundle and only when the
    highlight query asks about `local`; viewport rendering remains bounded.
    Both TUI and semantic/GPU producers evaluate `#is?`/`#is-not? local`
    through the shared capture walk. Non-shadowed JS/TS builtins are restored;
    shadowed definitions/references keep ordinary variable styling.
- **Multi-language injections (#122) LANDED** — the direct continuation
  of the #114–#118 highlight arc; four review rounds, framing
  `docs/multi-language-injections-framing.md` (Q#IJ1–IJ11). A buffer can
  now hold more than one language: `ParseTreeBundle` holds `Vec<Layer>`
  (root + injected children, depth-ascending, installed atomically so the
  existing `StyleGate` + highlight-cache Arc gates still work). The parse
  worker builds child trees off the static `BUILTIN_LANGUAGES` table
  (lazy load preserved) via `set_included_ranges` — child node offsets
  are ABSOLUTE, so injected spans are buffer-coordinate-native; settle
  resolves each layer's highlight query
  (`SyntaxRegistry::resolve_layer_queries`). First consumers: markdown
  fenced code + `markdown_inline` (retires the M9.7 block-only floor).
  New substrate: `LanguageEntry.injections_query`;
  `default_injection_aliases` + `SyntaxRegistry::injection_alias_snapshot`
  (case-folded fence names, Lua-extensible via
  `pmacs.parse.injection_aliases`, snapshotted into `ParseRequest` so the
  worker never touches the `Rc` registry or Lua);
  `ParseTreeBundle.injection_capped` (the 4096-layer backstop, surfaced
  once/buffer via `pmacs.error` at settle);
  `compute_highlight_spans_for(query, tree, source, local_facts, range)`
  (per-layer);
  the wire `flatten_layer_spans` event-sweep → DISJOINT effective spans
  (deeper / later-sibling / narrower wins, keyed by `(layer_index,
  capture_order)`); GPU `spans_from_segments` + `source_color_at` fold.
  Two findings to keep: injection ranges exclude only NAMED children
  (anonymous tokens ARE the injected text — excluding them shreds a block
  `inline` node; matches tree-sitter-md's own splitter), and the wire
  flattener runs over the WHOLE buffer via the file-style summary, so it
  must be an event sweep, not O(spans²).
- **JSON + YAML grammars and language servers (#123) LANDED** — bundled
  ABI-current `tree-sitter-json` / `tree-sitter-yaml` cover `.json`,
  `.yaml`, and `.yml`; the existing injection engine now highlights YAML
  frontmatter and JSON/YAML fences. Default external LSP configs are the
  pinned `vscode-json-language-server` provider and
  `yaml-language-server`; configured settings are pushed after
  `initialized`, which also supports push-model servers. The fake-server
  delivery proof and PATH-gated live JSON/YAML provider smokes cover the
  configuration contract. `.jsonc` / `.json5` remain a deliberate
  follow-up because the JSON grammar is strict.
- **Compile-mode (Arc 5 stage 1, #113) LANDED** (2026-07-14, 7 rounds;
  framing `docs/compile-mode-framing.md` rev 13). `compile.run` streams
  `/bin/sh -c "exec 2>&1; <cmd>"` into an intercept-read-only
  `*compilation*` buffer via a Lua ANSI parser; once-per-newline error
  rules; unified `error.next`/`error.previous` (M-g n/p, `` C-x ` ``,
  M-!). Substrate other code can use: `ProcessSpec.stdin/group` (group
  lifecycle: reap ledger, in-drain enforcement, cancellable poll
  readers), `buf:revision()`, jump_back fires `buffer.after-switch`,
  `pmacs.errors.claim`, `AnsiParser::finish()` (observable reset), and
  the style-overlay stack: buffer-attached `BufferStyleSpanTranslator`
  (once-per-edit, fragment-preserving), render-only window overlays with
  identity-deduped `Window::ensure_overlay` + `clone_for_split`,
  idempotent atomic `handle:dispose()`.
- **Editing-conveniences pack (editops, #111) landed** (framing
  `docs/editing-conveniences-framing.md` rev 6). goto-line, case ops,
  transpose, zap-to-char, line move/duplicate/join, region
  sort/reverse/dedupe, delete-trailing-whitespace + opt-in
  `pmacs.editops.trim_on_save`. Substrate: `pmacs.killring.kill_range` /
  `break_chain([fid])` / `arm_kill_prompt`+`commit_kill_prompt` (marker
  lifecycle), and the origin-guard pattern for chain-sensitive
  minibuffer commands.
- **Auto-pairing (#110) landed — Arc 2 COMPLETE** (framing
  `docs/auto-pairing-framing.md` rev 6). `BUILTIN_PAIR_CHARS` in
  pmacs-protocol leave both frontends' optimistic classifiers;
  `builtin/runtime/pair.lua` loads BEFORE lsp.lua (first-didChange
  ordering contract); one-shot typed-edit provenance via
  `pmacs.editor.take_typed_edit()` (buffer-revision postcondition,
  Q#AP9). Substrate: `buf:path()`, `pmacs.lsp.buffer_language(buf)`,
  `PMACS_FAKE_LSP_CHANGE_SINK`, `TestDaemon::spawn_with_config`.
- **Themes (Arc 4) stages 1–3 LANDED; Arc 4 COMPLETE ON `main`.**
  - Stage 1 (#120, `docs/theme-faces-framing.md` rev 9): named UI faces
    as reserved `ui`/`ui.*` theme entries; transactional split
    syntax/face epochs; protocol-v16 `ThemeFacts`; snapshot/baseline
    symmetry; store-sourced diagnostic-count freeze.
  - Stage 2 (#124, `docs/gpu-set-font-framing.md` rev 5):
    `pmacs.gpu.set_font` and authoritative protocol-v17 `FontFacts`;
    frontend-local family resolution, live font reload/reflow, and
    visual-run caret geometry.
  - Stage 3 (#125, `statusline-segments`,
    `docs/statusline-segments-framing.md` rev 3): composable strict
    `pmacs.statusline` providers; borrow-released per-window evaluation
    with failure latches; legacy-preserving TUI composition; a pure
    built-in LSP provider; dynamic modeline faces; protocol-v18
    `StatuslineSegments`; authoritative-empty/snapshot symmetry; and
    atomic GPU validation, face resolution, shaping, clipping, and
    cache invalidation. Acceptance 1-27 is implemented. Final gates:
    Clippy clean; 1,619 default + 1,793 CRDT library tests; 7 default +
    8 CRDT feature acceptance; 114 M4; 109 required GPU; one-invocation
    workspace sweep 2,718 passed across 78 suites (19 ignored,
    `basedpyright` filtered); `git diff --check` clean. Stage 3 landed
    as #125 and completed Arc 4 on `main`.
- **Vterm Stage 1 terminal core LANDED ON `main` — #126**
  (`docs/vterm-framing.md` rev 5; merge `643d1e1`).
  - Implementation commits: `bbc1f33` (Stage 1), `962944b` (Darwin signal
    normalization), first-review fixes `f0a235f`, `28f2e6c`, `bf972a7`, and
    second-review hardening `9797ada`; reviewed feature head `fc4e0ce` merged
    through PR #126, <https://github.com/levineuwirth/pmacs/pull/126>.
  - `AnsiParserProfile::{LineOriented, FullScreen}` preserves compile/REPL
    behavior while terminal PTYs emit the full cursor/mode/device operation
    set. `src/terminal/{screen,input,session}.rs` owns the state machine,
    encoders, and lifecycle registry.
  - Public session seam: owned strict `TerminalSpec`; owned
    `TerminalSnapshot`; `TerminalProcessState`; and
    `SharedTerminalManager = Rc<RefCell<TerminalManager>>` with
    `open/is_terminal/process_id/snapshot/tick/send/resize/terminate/prune/
    shutdown`. Stage 1 snapshots are context-free; Stage 2 adds per-view
    state without a second screen.
  - `EditorState` tick order is supervisor → terminal-owned PID drain/prune →
    `process.after-tick`. Terminal IDs are not exposed through
    `pmacs.process`; ordinary Lua/LSP/MCP ownership is unchanged. Terminal
    identity buffers are pathless, clean, empty, round-trip, and guarded
    read-only at every rope/CRDT/history mutation boundary.
  - Acceptance 1–14 is mapped in the framing. The real PTY bite splits
    ESC/CSI writes, observes alternate-screen cursor addressing, blocks and
    resumes through raw `send`, restores the main screen, and pins final
    output before exact PID/outcome annotation. One-row annotation visibility,
    TERM-ignoring shutdown, spawn rollback, buffer-kill prune, and immutable
    empty CRDT bootstrap are pinned.
  - Review round 1 added typed IND/NEL/RI with margin-correct screen behavior,
    defaults absent `TERM` to `xterm-256color`, makes shutdown liveness
    acceptance portable with `kill(pid, 0)`, and preserves custom tab stops on
    resize. Review round 2 rejects C0/C1 controls before they enter screen
    cells, preserves the released button code in SGR mouse reports, removes
    dead screen paths, and clears stale round-trip state during prune. Stage 2
    now uniquifies default terminal buffer names transactionally.
  - Exact CUU/CUD and out-of-range DECSTBM clamping, combining across controls,
    xterm alternate-screen details, legacy non-SGR mouse, printable ASCII and
    CSI-dispatch allocation fast paths, and scrollback-cap naming are explicit
    post-arc deferrals in the framing.
  - Final from-start rerun after review round 2: Clippy clean; 1,661 default +
    1,837 CRDT library tests (3 ignored each); 9 default + 10 CRDT vterm
    acceptance; M4 114 passed (3 ignored, 1 filtered); required GPU 109;
    workspace 2,769 passed across 79 suites (19 ignored, 1 filtered); diff
    check clean. `scripts/bite HEAD^ src/terminal/screen.rs --test
    vterm_stage1_acceptance terminal_cells_reject_child_control_characters`
    is a clean behavioral bite. The parser dispatch has its independent clean
    behavioral bite; the original `main`/crate-root bite remains explicitly
    weaker compile-time API evidence.
  - **Stage 3 GPU/protocol LANDED ON `main` — #135** (merge `cac4961`;
    `docs/vterm-framing.md` Revision 9, criteria 28–37).
    Protocol **v19**: `InstanceMessage::TerminalFrame` (discriminant 26,
    daemon-gated) plus `FrontendEvent::TerminalResize` (11) and
    `TerminalPointer` (12), both frontend-gated — the first bump gating in
    BOTH directions. `SUPPORTED=[6..=19]`.
  - `pmacs-protocol/src/terminal.rs` now owns the shared terminal bounds,
    `TerminalProcessState`, `TerminalSelectionSpan`, and the single
    structural policy `TerminalFrame::validate`; `src/terminal/*`
    re-exports them so no duplicate type exists. `unicode-width` is a
    workspace dependency so the screen and the validator measure glyph
    columns identically. `MAX_TERMINAL_FRAME_GLYPH_BYTES = 8 MiB` bounds
    the payload instead of widening the transport cap; the measured
    maximum legal frame encodes to 13,437,863 bytes under the unchanged
    16 MiB `MAX_FRAME_BYTES`. Over-bound snapshots are rejected, never
    truncated or silently chunked.
  - **The `Viewport` gate keys on the AUTHENTICATED SOURCE'S ACTIVE
    BUFFER, not the buffer the message names.** `Viewport` also aligns the
    window to the buffer it declares, so a stale document viewport in
    flight when a command opens a terminal drags the frontend back off it
    - the terminal then never paints, with no error anywhere. The weaker
    "is the declared buffer a terminal" reading looks right and fails
    exactly this way.
  - Suppression compares the COMPLETE ordered payload, never
    `screen_generation`: scroll, selection, viewport, and process state all
    change without advancing it.
  - GPU: `pmacs-gpu/src/terminal.rs` is a pure cell-space paint planner
    (testable without a GPU); the renderer builds one shaped buffer per
    text run so a wide/cluster advance can never choose the next column's
    origin. `pmacs-gpu --headless-probe` drives the real attach client
    without winit (`attach::connect_with_sink`), which is how criterion 37
    gets one real daemon + real PTY + real wgpu path.
  - #135 integrated canonical `main` after #137 landed. The one code conflict
    joined `TAB_STOP_COLUMNS` with the terminal imports in
    `pmacs-gpu/src/main.rs`; terminal geometry remains fixed-cell and never
    consumes the document tab-stop projection.
  - Final post-integration gates: strict Clippy; 1,768 default + 1,944 CRDT
    library tests; Vterm Stages 1/2/3 at 9/10, 4/4, and 5/7 default/CRDT;
    statusline 7/8; tab-width 2/2; M4 121; required GPU 139; workspace 2,946
    passed across 84 suites; formatting and diff check clean. The first
    macOS/LuaJIT CI run timed out waiting for `VTERM_ALT_READY` in the Stage 2
    real-TUI smoke; the complete failed-job rerun passed all 12 checks.
  - **Stage 2 TUI LANDED ON `main` — #130** (merge `86fc1bc`;
    `docs/vterm-framing.md` Revision 7, criteria 15–27). `TerminalViewKey` keys
    per-frontend/window projection state over one shared process/screen; logical
    row anchors retain

    scroll/selection through reflow. One authenticated frontend controls at
    most one session, with atomic replacement and release on
    focus/switch/kill/detach.
  - The strict `pmacs.terminal` Lua surface owns open/state/view/send/terminate
    and context-implicit scroll/copy commands; the latter error unless the
    invoking frontend's active window is a terminal. Fixed `C-c` is the
    per-frontend terminal escape: only its next key reaches terminal-local
    editor bindings, while unescaped bound keys pass through to the child.
    `C-c C-c` sends one literal interrupt. Copy drains through the acting
    frontend's clipboard path; active BELs drain once locally and per daemon
    frontend, while historical/passive bells are baseline-suppressed.
  - TUI composition paints owned terminal cells/styles only inside each
    window's content rectangle, suppresses document overlays, and keeps sibling
    splits independent. Daemon key/mouse/paste/focus/resize/detach routing uses
    the authenticated connection source rather than client-claimed IDs.
    `builtin/runtime/terminal.lua` provides the terminal command, view commands,
    and pure `ui.modeline.terminal` process/scroll segment.
  - `tests/vterm_stage2_acceptance.rs` maps Lua transactionality, shared-view
    isolation, clipboard/modeline behavior, and a hermetic real `/bin/sh` TUI
    PTY smoke. Stage 2 changed no wire schema or GPU renderer; protocol
    remained v18 until Stage 3.
  - PR #130 review round 1 (`8702791`) aligned dispatch with the approved
    escape-prefix contract, closed the non-terminal Lua error path, made
    controller replacement atomic, retained zero-area view anchors, removed
    duplicate detach work, and replaced per-view deep scrollback clones with
    borrowed live/published row projections. Focused child-input coverage pins
    both unescaped bound-key passthrough and `C-c C-c`.
  - PR #130 review round 2 (`b9a7e40`) clamps anchors into the first
    surviving cell when eviction cuts through a wrapped logical line, prevents
    ambient `active_frontend` from minting interactive Lua authority, names
    malformed explicit-context fields, restores dispatcher rationale, and
    removes owned cell snapshots from terminal mouse routing. The framing now
    records the transient v18 semantic-controller boundary and bracketed-paste
    injection deferral.
  - Current-main integration (`3f0252f`) preserved per-frontend terminal
    dispatch while applying the landed mode-scoped keymap, and exposed the
    `mode`, `terminal`, and `lsp` statusline providers together. PR #130 merged
    at `86fc1bc`.
  - Final integrated gate: `cargo fmt --check`; strict workspace Clippy;
    1,753 default + 1,929 CRDT library tests (3 ignored each); mode-system
    acceptance 1 default + 1 CRDT; Stage 1 acceptance 9 default + 10 CRDT;
    Stage 2 acceptance 4 default + 4 CRDT; statusline acceptance 7 default +
    8 CRDT; M4 114 passed (3 ignored, 1 filtered); required GPU 109;
    workspace 2,882 passed across 82 suites (19 ignored, 1 filtered);
    `git diff --check` clean.
- **Tab-width rendering parity LANDED — #137** (merge `2625ec7`;
  `docs/tab-width-parity-framing.md` rev 2).
  Source tabs remain one byte while every buffer
  renderer follows the shared fixed `pmacs_protocol::TAB_STOP_COLUMNS = 8`.
  - `src/display_width.rs` owns allocation-free Unicode/tab-aware byte-to-column
    accounting for plain text, syntax, diagnostics, completion anchors,
    buffer-style overlays, and search washes.
  - The GPU rich-chunk projection expands source/adornment tabs before
    cosmic-text shaping and retains first-class source-tab provenance.
    Carets, hits, selections, peer washes, and diagnostic geometry share the
    same source/projected boundary rules, including a soft wrap inside one
    expanded tab.
  - GPU minimap widths use the same tab/Unicode rule and refresh in the accepted
    text-edit transaction. No config, wire shape, negotiation, or protocol
    version changed. Local gates: 1,763 default + 1,939 CRDT + 1,763 Lua 5.4
    library tests; 2 focused acceptance; M4 121; required GPU 119; workspace
    2,911 across 83 suites; strict Clippy and diff check clean.
  - This closes the standing "**tab width is a rendering-parity bug, NOT a
    config gap**" deferral in §5: one shared constant now drives every
    renderer. Terminal cells are deliberately OUTSIDE it — a terminal's
    columns come from the child, so `pmacs-gpu`'s terminal geometry uses
    the monospace advance and never `TAB_STOP_COLUMNS`.
- **PARKED: kill-ring browser + persistence.** Revision 2 framing is
  preserved on branch `kill-ring-browser`, but its `0efb5cd` scout is stale
  and must be repeated before implementation. No PR or implementation is
  active.
- Roadmap: `docs/roadmap-2026-07.md` (ranked arcs). Position:
  - **Arc 1 (LSP utility surface) COMPLETE** — completion popup
    (#92/#93), panels/references/outline/hover (#94–#96), plus
    hardening follow-ups (#102, #105, #106).
  - **Arc 2 (editing table stakes) COMPLETE** — query-replace (#97),
    kill ring + `M-y` (#103/#105/#106), comment-toggle (#107),
    auto-indent (#109), auto-pairing (#110).
  - **Arc 3 (persistence) COMPLETE** — saveplace/recentf (#98),
    desktop-save (#99), autosave/crash-recovery (#100), save-clobber
    fix (#101).
  - **Arc 4 (themes + extensibility) COMPLETE** — named UI faces (#120),
    live GPU font preferences (#124), statusline providers (#125).
  - **Arc 5 terminal stage COMPLETE** — compile mode (#113), Vterm terminal
    core (#126), TUI frontend (#130), and protocol/GPU Stage 3 (#135) landed.
  - **Config registry COMPLETE (#127)** — not a numbered arc; it was the
    cross-cutting substrate ranked first on
    `docs/side-quest-backlog.md`'s north star, and it unblocks the
    editing/indent/comment items that were config-blocked.
  - **Mode system wiring COMPLETE (#129)** — major-mode keymaps,
    introspection, lifecycle initialization, and statusline display shipped.
  - **Locals-query processing COMPLETE — #134** — grammar locals metadata,
    lexical resolution, settled per-layer facts, shared TUI/GPU
    local-predicate filtering, and a registry-wide locals-query invariant
    shipped without a protocol change.
  - Remaining ranked arcs: 6 folding, 7 DAP, 8 GPU splits, plus the
    `.ipynb` arc (its JSON-grammar prerequisite shipped in #123).

## 2. How we work (the part that must not drift)

The user is expert and reviews deeply — they falsify framings and find
real bugs in round after round. The cadence that has worked for ~40 PRs:

1. **Scout ground truth** in the code before proposing anything.
2. **Write a framing doc** — `docs/<feature>-framing.md`, numbered
   decisions (`Q#XY1…`), explicit "Ground truth", "Bets", "Deferred
   (named)", and an acceptance-test list. Present it and **wait for
   explicit approval** ("Go for it" / "Ready to roll"). Expect 1–3
   rounds of findings first; revise the doc, don't argue.
3. **Branch off main** (one feature = one branch = one PR). Commit the
   framing as the first commit.
4. Implement. **Every reviewer finding gets a bite-verified fix** — a
   test that fails without the fix. Watch for vacuously-passing tests.
5. Run the full gate suite (§3). Open the PR with `gh`.
6. The user replies with "Findings" lists on the PR rounds too. Same
   discipline. They say when to merge — **never merge unprompted**.
7. After merge: update this handoff + your memory.

Commit/PR conventions: commit messages via `git commit -F <file>` (no
inline backticks through the shell). **Authorship trailers and PR
attributions must be truthful:** do not add a Claude co-author trailer or
Claude Code attribution unless Claude actually contributed. Clippy runs as
its own step, never `&&`-chained.

## 3. Gate suite (all green before any PR)

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings   # own step
cargo test --lib                                        # ~1500
cargo test --lib --features crdt                        # ~1672
cargo test --test <the new/touched acceptance suites>
cargo test --test m4_acceptance -- --skip basedpyright
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu             # 58
cargo test --workspace -- --skip basedpyright           # full sweep
git diff --check
```

Machine-specific caveats — re-verify on a machine you haven't used
before trusting them:

- **basedpyright**: the DESKTOP's local binary is broken and HANGS the
  `m4_5_basedpyright` tests — hence the `--skip` there. The LAPTOP has
  a working basedpyright 1.39.9 (verified 2026-07-10: the m4_5 test
  passes in 0.18s), so the skip is droppable on the laptop.
- **GPU on the laptop**: AMD Radeon 780M (RADV) — native Vulkan,
  `PMACS_REQUIRE_GPU=1` works without lavapipe.
- **Flaky-under-load tests — rerun isolated before treating a sweep
  failure as a regression.** The m8 daemon tests and the m6 process/PTY
  tests (`m6_1_pty_mode_lifecycle_started_then_exited`,
  `m6_8_supervisor_reaps_all_children_across_cycles`) are timing-based;
  `editor::composition_overhead_under_ten_percent` is a render-ratio
  microbenchmark that fails ~1/3 even isolated single-threaded (already
  `cfg!(macos)`-disabled). Vterm Stage 3's merge CI saw one macOS timeout in
  `real_tui_terminal_smoke_restores_host_after_output_input_resize_scroll_copy_and_bell`;
  the complete failed-job rerun passed. The required-GPU gate also failed once
  in `headless_diag_face_recolors_band_counter_despite_unchanged_text`, then
  passed both an isolated single-thread rerun and the full 139-test rerun. A
  lone timing failure → rerun the test alone (`-- --test-threads=1`) before
  investigating. Run the workspace
  sweep as ONE `cargo test` invocation piped to a full log — a double
  invocation + `grep -c "test result: ok"` can mask real failures with a
  misleading `0`.
- **GPU tests** need a Vulkan device. `PMACS_REQUIRE_GPU=1` makes
  absence a hard failure instead of a silent skip. Headless option:
  lavapipe (see `docs/repository-audit-2026-07-03.md` for the CI
  harness how-to). On a laptop without discrete GPU, mesa/lavapipe
  works.
- **The desktop's shell is fish** (no `$(...)`, use `(...)`; no
  `$UID`, use `(id -u)`). Check `$SHELL` here before assuming.

## 4. Substrate invariants (do not undo; tests enforce most of these)

**Command boundaries (Arc 2 kill-ring substrate)** —
`EditorCore.command_history: HashMap<FrontendId, CommandBoundary{this, last}>`,
per frontend. Rotate on: keybound command, self-insert, menu invoke,
`invoke_interactive` (the M-x path). Break on: unbound key, GPU
optimistic CRDT edits, pointer gestures (wheel scroll deliberately does
NOT break), unified paste. **Plain `pmacs.command.invoke` stamps
nothing** (programmatic API); `invoke_interactive` rotates-then-invokes
(Emacs M-x semantics). Single-codepoint optimistic CRDT inserts
classify as `buffer.self-insert` (exact decode; `"a("` breaks instead).
Lua: `ed.this_command()` / `ed.last_command()`.

**Effective-edit returns** — `buf:insert/delete/replace` return the
post-intercept `(start, end, inserted_len)`. Callers that care
(killring, comment) compare EXACTLY against the request; length-delta
and text-at-position checks are documented defeated patterns. Always
`pcall` the mutator: a rejecting intercept must report, not throw
through, and failed ops must leave no state (kill chains, yank
sessions).

**Kill ring** — entries `{id, text}` with stable monotonic ids; chains
and yank sessions are per-frontend and id-checked (an index is not
stable under other frontends' pushes). OS clipboard mirrors to the
acting frontend only. Paste is a unified daemon arm keyed by the
dispatcher's AUTHENTICATED source — never a payload frontend_id.

**LSP outbound positions** — every Position/Range builder in
`src/lsp.rs` routes through `outbound_position` (byte → negotiated
encoding). Any new request builder must too; UTF-16 servers reject raw
byte columns on non-ASCII text. Semantic tokens: `full`, `full.delta`,
and `range` are three INDEPENDENT capabilities — gate each.

**Persistence (Arc 3)** — state-dir wiring lives in
`install_state_dirs()` on real entry points only, NOT `EditorState::new()`
(tests stay hermetic); `PMACS_STATE_HOME` overrides. Autosave: one
buffer owns a path's recovery slot; only recover/discard release
unclaimed crash data; adopt clears the old owner's skip cache.

**Protocol** — encoding-breaking bumps are deliberate and versioned
(`SUPPORTED=[6..=19]`). v15 = `CompletionPopup` +
`StatusFacts.message`; v16 = `ThemeFacts`; v17 = `FontFacts`; v18 =
`StatuslineSegments`; v19 = the vterm terminal family. New wire surface ⇒
bump + both-frontends support + acceptance. An APPENDED variant must be
guarded by a byte pin on the PREVIOUS final variant — its own round-trip
cannot detect a discriminant shift.

**Fake LSP** (`src/bin/pmacs_fake_lsp.rs`) modes: `fullonly`,
`rangeonly`, `rangeonly16` (UTF-16 + fail-closed bounds validation),
`sighelp`. Use these for capability-matrix tests, not real servers.

## 5. Hard-won ops lessons

- **The checkout may be shared with the user.** Check `git status` for
  foreign uncommitted work before any stash/checkout/branch surgery;
  never assume dirty files are yours. (Their uncommitted fix was nearly
  orphaned once.) A clean status goes stale within minutes when two
  lanes are active — for parallel work, `git worktree add` a sibling
  directory off main instead of switching the shared checkout.
- **Never `git stash` in this repo.** The stash namespace is
  REPO-GLOBAL — one list shared across every worktree and with the
  user; a scripted push/pop can pop a human's years-old stash into
  your tree (happened during #111: a failed `stash push` chained
  into `stash pop`, which grabbed the user's PR-#17-era entry). For
  run-tests-against-an-old-version swaps, use `scripts/bite` — a
  trap-guarded one-file swap over read-only `git show`, with an
  inverted verdict (exit 0 iff the tests FAIL against the old
  version), making bite-verification machine-checkable.
- **Stacked PRs**: retarget the child to main BEFORE merging the
  parent — GitHub auto-closes a PR whose base branch is deleted and
  cannot reopen it (#104 → re-opened as #105).
- **Frozen reviewed PRs do not absorb moving overlapping work.** For #135 and
  #137, the approved/frozen #137 landed first; the larger #135 lane then
  merged canonical `main`, retained its review anchors, and reran every gate.
  Derive that integration surface from `git diff <base>..main`, not the other
  PR's file list — concurrent landed work added an overlap the original
  two-PR comparison missed.
- **Scripted edits (sed/python) in files with repeated similar blocks**
  (`src/lsp.rs` JSON builders): anchor on a unique line or you will
  silently edit the wrong block. This produced a vacuously-passing test
  and cost an hour.
- **Acceptance fixtures that open `.rs`/`.py` files** must empty
  `pmacs.lsp.config` first — the after-load hook spawns real servers
  (rust/python/c have default configs). Language *detection*
  (grammars + `pmacs.lsp.filetypes`) is unaffected.
- Test scratch buffers have no path ⇒ no language; use tempdir files
  when language matters.
- **Dual-purpose session state is a reset-contract trap** (#120
  rounds 2–5): `last_status` doubled as the peer emission baseline
  AND the stale-diag count freeze, so resetting baselines on
  `BufferSnapshot` zeroed mid-edit counts. Knowledge about a buffer
  belongs in shared stores (`DiagnosticStore` severity totals), never
  in per-session baselines; and any daemon-side reset needs its
  frontend mirror audited in the same round (the GPU snapshot arm
  missed search/menu/status the first time).
- **Tab width is a rendering semantic, NOT a config gap.** The implementation
  on `tab-width-parity` fixes the width at the TUI's established 8 columns,
  shares that constant through `pmacs-protocol`, and expands tabs only in each
  display projection. Defining `editor.tab-width` could not have fixed the GPU:
  source text and semantic spans stay byte-addressed while cosmic-text needs
  projected spaces plus an inverse hit/caret map. A future configurable width
  would require a buffer-effective frontend fact and cache invalidation; do not
  re-plan it as a scalar config-only change.
- **A test that never runs passes.** Two #127 review-round tests passed
  vacuously at first: `pmacs.editor.save()` is the RAW save, while
  `buffer.before-save` fires inside the `buffer.save` COMMAND
  (`builtin/commands/default.lua`), and `save()` no-ops on an
  unmodified buffer — so a fixture that opens a file and saves it
  asserts on bytes nothing rewrote. Dirty the buffer with a real edit
  and go through `pmacs.command.invoke("buffer.save")`. Caught only
  because the *other* case failed and the cause was chased instead of
  the assertion adjusted.
- **A message that ALIGNS state cannot be gated on the state it names.**
  `FrontendEvent::Viewport` both declares a byte range and switches the
  frontend's window to the buffer it names. Gating the vterm v19 dual
  declaration on "is the DECLARED buffer a terminal" therefore left a
  stale in-flight document viewport free to drag a frontend straight back
  off a terminal a command had just opened — the window oscillated, the
  terminal declaration was refused every time, and no frame ever arrived.
  Nothing errored. The gate has to key on the authenticated source's
  ACTIVE buffer. Generalizes: when two messages declare competing views of
  "what am I showing", the arbiter is the daemon's own state, never the
  claim inside either message.
- **A pass that sets a mode flag must clear it on EVERY exit.** The
  semantic producer's terminal pass returned early via `?` when no
  declaration existed, leaving `terminal_active` set — and the daemon uses
  that flag to suppress `CursorByte` and the presence sweep, so a frontend
  that went back to a document silently lost both. Caught by an acceptance
  assertion, not by any type.
- **Sub-crate acceptance needs a real seam, not a fixture.** `pmacs-gpu`
  depends only on `pmacs-protocol`, so "real daemon + real PTY + real
  wgpu in one path" could not be an in-crate test. Generalizing
  `attach::connect`'s reader sink (`connect_with_sink`) and adding
  `--headless-probe` gave the acceptance the REAL handshake, outbox,
  writer, and `render_to_view` — which is the whole point; a
  decoded-message fixture would have proved none of the three fit
  together. The probe found two real defects the in-process tests did not.
- **Real-grid acceptance must budget for macOS startup and path width.**
  A 100 ms first-Hello timeout failed under loaded macOS CI; use the normal
  five-second handshake window, then short polling reads. An 80-column split
  also clipped a custom statusline segment after macOS's long
  `/var/folders/...` temp path while passing on Linux; size the grid for the
  longest supported fixture path (the mode-system test uses 160 columns per
  split).


## 6. Named deferrals (the standing backlog, consolidated)

Editing: word kills (`M-d`/`M-BS` — need bytes-returning deleters +
prepend-on-backward append), `C-SPC` set-mark, `C-u C-y` / `C-M-w`,
kill-ring browser + persistence, clipboard watching, block comments +
mid-line comment spans, comment-dwim append-at-EOL, per-language
comment padding. Pairing (framing "Deferred"): wrap-region on opener,
pair-aware backspace, RET-inside-pair closer-on-own-line,
in-string/in-comment inhibit (needs node-at-byte `pmacs.parse`),
undo amalgamation (pair = one step), balance-aware quotes
(the per-buffer toggle SHIPPED in #127 as `editing.auto-pair`).
Editops deferrals (full
list in its framing): recenter (blocked on viewport facts — the GPU
never consumes daemon `view_top`), Unicode case/word classes,
region-spanning move/duplicate, locale collation for sort-lines,
ensure-final-newline on save.
Substrate: buffer-aware edit epoch (after-edit currently compares the
ACTIVE buffer only), wire provenance for CRDT self-insert
classification, Lua intercept probe, completion.lua still on the old
cursor-delta heuristic (migrate to `this_command`), the TUI's
nonempty-selection optimistic type-over gate, generated-buffer search
invalidation, cross-peer chronological undo arbitration (mixed
source/daemon history; pinned by auto-pairing acceptance),
origin-pinned `buffer.after-edit` fan-out (a context-switching
intercept changes what later callbacks — LSP, completion — observe).
LSP/persistence: hidden-buffer LSP attach, daemon desktop-restore, the
*warning* half of external-change detection (verify-visited-file-
modtime).
Config registry (SHIPPED #127; these are its own named deferrals):
persistence of settings and the `custom-file` split-brain question,
`M-x list-settings` as a listview panel, a settings completion source
for the minibuffer (`minibuffer.read`'s `source` is a fixed Rust-side
vocabulary), table-valued settings (so `pmacs.lsp.config`,
`pmacs.pair.sets`, `pmacs.comment.strings` and the `pmacs.parse.*`
write-through proxies stay raw Lua), migrating the remaining scalar
setters (`async_config` ×2, `killring.max`, the `enable` booleans) and
`pmacs.gpu.set_font`, pending-set staging for names defined after
`init.lua` runs, and a `scope = "global"` define flag — `set_local` is
currently accepted for `autosave.interval-ms`, where a per-buffer
value is meaningless.
**Tab width is NOT a config gap** — see §5.
Mode system (SHIPPED #129): minor modes, `buffer.after-mode-change`,
mode-scoped settings, `describe-mode`, and persistence of explicit major-mode
overrides/clears across sessions.
Modeline detection (SHIPPED #132): bounded first/last-line Emacs `-*-`
and Vim `ft=`/`filetype=` parsing, explicit-over-inferred precedence,
alias normalization, and shared fresh-load language pinning for
syntax/highlight/LSP startup.
Highlight/detection (from the #114–#118 side-quest + injections #122):
~~locals-query processing~~ **SHIPPED #134**; remaining injection follow-ups
now that the engine landed (#122) —
`injection.combined` (many matches → one shared parse; PHP-in-HTML, some
comment schemes), child-tree incrementality + range-scoped layer rebuild
(child layers cold-reparse on every settle today), injectable
runtime/Lua-registered languages (v1 resolves only against
`BUILTIN_LANGUAGES`), and the next injection *consumers* gated on new
grammars — HTML/CSS/GraphQL/SQL (`<script>`/`<style>`, JS/TS template
literals, doc-comment code);
~~modeline detection as a 5th layer (`-*- mode: … -*-` /
`# vim: ft=…`)~~ **SHIPPED #132**;
byte-accurate multibyte cursor placement in `move_active_cursor_to`
(still steps one codepoint per LSP byte column). A full Jupyter `.ipynb`
setup (reader → editable → kernel execution) now has its JSON grammar
prerequisite, but remains a real arc, not a one-shot.
GPU: auto-reconnect after daemon restart, splits/multi-buffer, gutter
riders (whitespace guides, folding, git markers).
Themes (full list in theme-faces framing rev 9 "Deferred (named)"):
popup/menu/dropdown bg + selected-row faces, `ui.background` /
`ui.caret`, `ui.modeline.inactive`, minimap chrome, peer-cursor
palette (+`ui.selection` for peer rects), `ui.inlay_hint` (needs the
epoch treatment on its producer), wire alpha, `Indexed` palette
unification, named-theme registry / light theme / persistence
(the registry exists now, #127; theme persistence still waits on
settings persistence), grid-vs-wire `default_style` asymmetry,
mask widening (gutter bg, wash glyph recolor, statusline bg echo
surface, chrome bold/italic/underline re-shaping).
Housekeeping: F-016 `lua_bindings/mod.rs` split paused mid-way
(tranches 0–2 landed, ~5–8 PRs left; see
`docs/lua-bindings-split-framing.md`).
Full cross-cutting index of the non-themes backlog (this list + every
framing doc's Deferred section + a code sweep, themes excluded, with a
prioritization north star): `docs/side-quest-backlog.md`.

## 7. Machine-local facts (desktop) that do NOT travel

Three untracked files live only on the desktop working tree and are
deliberately never committed: `docs/pmacs-gpu-editing-perf-handoff.md`,
`docs/session-5-stale-styling-handover.md`, `python_experiment.md`.
Don't expect them in a clone; on the desktop, never delete them.

## 8. Update protocol for this file

When a PR merges, an arc opens/closes, or a decision lands: edit the
snapshot (§1), append lessons (§5) and deferrals (§6) as they arise,
bump the date line at the top, and commit — usually riding the same PR
as the work. Keep durable architecture here; put branch hashes,
machine-local tools, incomplete verification, and recovery commands in
`docs/active-work.md`. This is a briefing, not a log: prune sections
that stop being true.
