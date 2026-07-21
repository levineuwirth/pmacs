# Agent handoff — cross-machine continuity

**Last updated: 2026-07-21, after Vterm Stage 1 review round 1 was addressed
and fully gated on `vterm-core` (awaiting follow-up; not merged). Vterm Stages
2 and 3 are not implemented.** This file is the bridge between development
machines. If you are an agent reading on a fresh clone: this document
plus the `docs/*-framing.md` files ARE your memory. Read this fully
before taking on work, seed persistent memory from it, and **update this
file (and commit it) whenever project state changes materially** — the
next machine reads it the way you just did.

## 1. Where the project stands (2026-07-21)

- Canonical `main` @ `7bc0c61` (#125 merged), protocol
  **v18** (`SUPPORTED=[6..18]`).
- **Syntax-highlight / language-detection side-quest (#114–#118)
  LANDED** — a one-shot arc built in sibling worktrees off main while
  the user's themes lane (`theme-faces`) ran concurrently in the shared
  checkout. All merged. What shipped:
  - **Grammars** (`crate::syntax::BUILTIN_LANGUAGES`): every
    LSP-configured language now has one — cuda, bash, dockerfile (via
    the ABI-current `tree-sitter-containerfile`, NOT the dead
    `tree-sitter-dockerfile` which pins `tree-sitter ^0.20`), make,
    cmake, python, go, javascript (+jsx), typescript (+tsx), toml, zig.
  - **Detection chain** (`resolve_active_language` in syntax.lua /
    `buffer_language` in lsp.lua): extension → LSP filetype map →
    filename → shebang. New user-extensible Lua surfaces
    `pmacs.parse.shebangs` / `.filenames` and
    `pmacs.parse.language_from_shebang` / `language_from_filename`.
    Grammar name MUST equal the `pmacs.lsp.config.<name>` key (grammar
    detection wins over the filetype map, so it fixes the LSP id too).
    A buffer keeps its first-attached grammar across edits/switches (no
    re-sniff of a since-edited shebang).
  - **LSP configs added**: dockerfile (`docker-langserver --stdio`),
    cmake (`cmake-language-server`, config via
    `init_options.buildDirectory="build"` — it does NOT pull
    `workspace/configuration`). Make has no server.
  - **Substrate**: `LanguageEntry.highlights_query` is now
    `&[&'static str]` — fragments joined base-first, for grammars whose
    bundled highlights are a `; inherits:` delta (cuda over c/cpp; ts
    over js/jsx). `compute_highlight_spans` FAILS CLOSED on the
    `#is?`/`#is-not? local` property predicate (no locals processing) —
    drops those captures so shadowed builtins aren't mis-styled.
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
  `compute_highlight_spans_for(query, tree, source, range)` (per-layer);
  the wire `flatten_layer_spans` event-sweep → DISJOINT effective spans
  (deeper / later-sibling / narrower wins, keyed by `(layer_index,
  capture_order)`); GPU `spans_from_segments` + `source_color_at` fold.
  Two findings to keep: injection ranges exclude only NAMED children
  (anonymous tokens ARE the injected text — excluding them shreds a block
  `inline` node; matches tree-sitter-md's own splitter), and the wire
  flattener runs over the WHOLE buffer via the file-style summary, so it
  must be an event sweep, not O(spans²).
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
- **Vterm Stage 1 terminal core IMPLEMENTED ON `vterm-core`, FULLY GATED,
  AWAITING REVIEW FOLLOW-UP, NOT MERGED** (`docs/vterm-framing.md` rev 4).
  - Implementation commits: `bbc1f33` (Stage 1), `962944b` (Darwin signal
    normalization), and review fixes `f0a235f`, `28f2e6c`, `bf972a7`; pull
    request: #126, <https://github.com/levineuwirth/pmacs/pull/126> (open,
    non-draft, targeting `main`).
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
    resize. Stage 2 must uniquify default terminal buffer names. Exact CUU/CUD
    margin clamping, combining across controls, xterm alternate-screen details,
    legacy non-SGR mouse, and the printable ASCII allocation fast path are
    explicit post-arc deferrals in the framing.
  - Final from-start rerun after review round 1: Clippy clean; 1,660 default +
    1,836 CRDT library tests (3 ignored each); 8 default + 9 CRDT vterm
    acceptance; M4 114 passed (3 ignored, 1 filtered); required GPU 109;
    workspace 2,767 passed across 79 suites (19 ignored, 1 filtered); diff
    check clean. The first review Clippy pass found only identical LF/IND match
    arms; after consolidation the complete sequence restarted at gate 1.
    `scripts/bite HEAD^ src/ansi.rs --lib
    parser_split_points_produce_identical_screen` is a clean behavioral bite;
    the original `main`/crate-root bite remains explicitly weaker compile-time
    API evidence.
  - Stage 2 reviews require a durable focus/input resize owner, owning
    `FrontendId` for the global `C-c` continuation, and local clipboard/BEL
    signal drainage. Stage 3 additionally owns `pmacs-gpu/src/attach.rs`,
    authenticated source routing, protocol-owned wire types/limits, and a
    deliberate complete-frame limit decision: 16 MiB is insufficient; use a
    measured legal-worst cap or aggregate bound, never silent chunking.
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
inline backticks through the shell); end with the Claude co-author
line. PR bodies end with the Claude Code attribution. Clippy runs as
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
  `cfg!(macos)`-disabled). A lone failure of one of these → rerun that
  test alone (`-- --test-threads=1`) before investigating. Also: run the
  workspace sweep as ONE `cargo test` invocation piped to a full log — a
  double invocation + `grep -c "test result: ok"` can mask real
  failures with a misleading `0`.
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
(`SUPPORTED=[6..15]`). v15 = `CompletionPopup` + `StatusFacts.message`.
New wire surface ⇒ bump + both-frontends support + acceptance.

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

## 6. Named deferrals (the standing backlog, consolidated)

Editing: word kills (`M-d`/`M-BS` — need bytes-returning deleters +
prepend-on-backward append), `C-SPC` set-mark, `C-u C-y` / `C-M-w`,
kill-ring browser + persistence, clipboard watching, block comments +
mid-line comment spans, comment-dwim append-at-EOL, per-language
comment padding. Pairing (framing "Deferred"): wrap-region on opener,
pair-aware backspace, RET-inside-pair closer-on-own-line,
in-string/in-comment inhibit (needs node-at-byte `pmacs.parse`),
undo amalgamation (pair = one step), balance-aware quotes,
per-buffer toggle (config-registry-blocked). Editops deferrals (full
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
modtime), config registry (no unified config surface yet).
Highlight/detection (from the #114–#118 side-quest + injections #122):
locals-query processing (run each grammar's LOCALS_QUERY so
`#is?`/`#is-not? local` is honored instead of the current fail-closed
drop — restores `.builtin` styling for non-shadowed console/require
etc.); **injection follow-ups now the engine landed (#122)** —
`injection.combined` (many matches → one shared parse; PHP-in-HTML, some
comment schemes), child-tree incrementality + range-scoped layer rebuild
(child layers cold-reparse on every settle today), injectable
runtime/Lua-registered languages (v1 resolves only against
`BUILTIN_LANGUAGES`), and the next injection *consumers* gated on new
grammars — HTML/CSS/GraphQL/SQL (`<script>`/`<style>`, JS/TS template
literals, doc-comment code); modeline detection as a 5th layer
(`-*- mode: … -*-` / `# vim: ft=…`);
byte-accurate multibyte cursor placement in `move_active_cursor_to`
(still steps one codepoint per LSP byte column). **JSON + YAML PR #123
OPEN** (grammar-gap style, `tree-sitter-json`/`-yaml`; LSP configs
`vscode-json-language-server` with provider pin
`@t1ckbase/vscode-langservers-extracted@2.0.2`, plus
`yaml-language-server`). The public and checkpoint branches are both at
fully gated `5c202c5`, rebased onto `f8096ff`; the JSON and YAML
PATH-gated pmacs smokes both passed, and the PR awaits user review.
Once merged, YAML `---` and TOML `+++` markdown frontmatter highlight via
the #122 engine and the Jupyter reader → editable → kernel arc has both
grammar prerequisites.
GPU: auto-reconnect after daemon restart, splits/multi-buffer, gutter
riders (whitespace guides, folding, git markers).
Themes (full list in theme-faces framing rev 9 "Deferred (named)"):
popup/menu/dropdown bg + selected-row faces, `ui.background` /
`ui.caret`, `ui.modeline.inactive`, minimap chrome, peer-cursor
palette (+`ui.selection` for peer rects), `ui.inlay_hint` (needs the
epoch treatment on its producer), wire alpha, `Indexed` palette
unification, named-theme registry / light theme / persistence
(config-registry-blocked), grid-vs-wire `default_style` asymmetry,
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
as the work. Keep it under ~250 lines: this is a briefing, not a log;
prune sections that stop being true.
