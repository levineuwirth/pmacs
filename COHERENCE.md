# Product Coherence for Pmacs

## Status of this document

This document has two jobs. It states the **product-coherence thesis**
for pmacs, and it records the **audited ground truth** of how the
codebase measures against that thesis, so that no future agent or
contributor has to re-excavate it.

- The vision prose is durable. The **Ground truth** subsections were
  established **2026-07-25** by a four-lane code audit (discoverability,
  interaction islands, packages/workers, first-run journey) plus a
  distribution check, on branch `lsp-multi-root-affinity`
  (= `main` @ `0827dd1` plus the multi-root LSP work).
- Citations name **symbols first, `file:line` second**. Line numbers
  drift with the tree — `docs/keybindings.md` drifted by 250–1000 lines
  within days of its "last verified" stamp (§24) — so treat the symbol
  name and the structural claim as authoritative and the line number as
  a hint. Re-grep before relying on a number.
- Grades used below: **Strong / Partial / Weak / Missing** (and
  **Broken** where something actively fails).
- Update protocol is §25. When a PR changes any audited claim here,
  updating this file rides that PR, the same way `docs/agent-handoff.md`
  does.

Relationship to the other required documents: `docs/agent-handoff.md`
carries durable project state and working method; `docs/active-work.md`
carries volatile branches and recovery; `docs/side-quest-backlog.md`
carries item-level deferrals. This document carries **product direction
and the measured distance to it**. It is not a second backlog; it is the
standard the backlog gets ranked against.

---

## Purpose

Pmacs already has an unusually strong technical foundation for an editor
at its stage of development. Its daemon/frontend split, semantic
rendering protocol, CRDT-based editing, structured worker runtime, Lua
programmability, language tooling, package resolver, terminal support,
and remote-capable architecture all point toward a system with genuine
long-term differentiation.

The next challenge is not primarily adding more isolated capabilities.
It is making the existing and planned capabilities converge into a
coherent product.

Visual Studio Code is used throughout this document as a reference point
because it is an exceptionally successful modern editor. Pmacs is
obviously not trying to become VS Code. Its goals are substantially
different: live programmability, inspectability, stronger concurrency
semantics, frontend plurality, and deeper user control are central to
Pmacs in ways they are not central to VS Code. The useful lesson is
therefore not to copy VS Code's interface or architecture wholesale, but
to understand how a technically complex system can become immediately
useful, progressively discoverable, and easy to adopt.

The deeper reference point is Emacs. Emacs's beauty comes from its
ontological unity: the editor is text, Lisp, commands, buffers, and a
running system that the user can interrogate and change. Its enduring
achievement is not any single feature, but that it created the kind of
environment in which generations of users could build almost anything.

Pmacs should preserve that unity while correcting the accidental
historical constraints beneath it: cooperative rather than general
parallelism, unclear ownership, global mutation, difficult unloading,
rendering coupled too closely to the core, opaque latency, implicit
remote context, and inconsistent package lifecycle.

Pmacs does not need to contain everything Emacs contains before it can
be considered a successor. It must instead remain the kind of system in
which everything Emacs contains could eventually be built — with clearer
ownership, stronger concurrency, richer frontends, explicit execution
locations, and fewer historical traps.

That places the VS Code comparison in its proper role. VS Code
demonstrates how a complex development environment can be coherent,
approachable, and immediately useful. Emacs demonstrates how an editor
can become a live, fertile, user-transformable world. Pmacs should
combine the adoption discipline of the former with the programmability
and unity of the latter.

The core product objective should be:

> **Pmacs should be immediately excellent, progressively understandable,
> completely inspectable, and ultimately replaceable.**

A user should receive a polished workstation before they become an
editor engineer. If they choose to become one, the entire system should
remain open to them.

---

## 0. Scorecard (audited 2026-07-25)

| § | Concern | Grade | One-line state |
|---|---|---|---|
| 2 | Golden product journey | **Runs to step 10** | `pmacs .` opens the directory (1a); the interface introduces itself (1b-3); a missing language server says so (1b-2, #204); a build is bound and prefilled (1b-1, #203). Steps 1, 11 and 12 remain the thin end |
| 3 | Zero-configuration state | **Partial** | Defaults genuinely strong; missing-tool failure is silent, not graceful |
| 4 | Progressive disclosure | **Inverted** | The advanced level is real; the beginner level is the missing one |
| 5 | Unified discoverability | **Substrate without surface** | Best-in-class registration metadata; almost no way for a user to reach it |
| 6 | Interaction islands | **Weak, and growing** | Six hardcoded key-interception shadows; no transient-keymap mechanism exists |
| 7 | First-class workspaces | **Missing (conventions only)** | Marker walk + four independent consumers; no workspace object |
| 8 | Execution locations | **Missing (architecture ready)** | SSH attach works; "location" is not a value anywhere |
| 9 | Worker ownership | **Mechanism without identity** | Cancellation solid; no owner/purpose/hierarchy; four disjoint activity views |
| 10 | Extension trust classes | **Missing (one class)** | Shared Lua state, `__index = _G`; MCP is the one out-of-process seam |
| 11 | Config layering + provenance | **Partial (foundation only)** | Typed registry is right; 5 settings live in it; no value provenance |
| 12 | Profiles | **Missing** | One hardcoded default keymap; not a named concept |
| 13 | Package lifecycle UX | **Resolution without lifecycle** | Mature resolver/lockfile; init-only install; no uninstall/disable/search |
| 14 | Workbench primitives | **Partial (best trajectory)** | Listview is a real primitive but only 3 call sites, all LSP panels; buffer-list and search re-implement it; bottom panel complete on BOTH frontends (#155 + Stage 2) |
| 15 | Contextual affordances | **Weak** | Right-click menu only; code actions apply first-blindly; no git integration at all |
| 16 | Semantic frontend | **Strong** | v6..=v21 schema support; production attach remains v20 during the dark panel slice; degradation practiced |
| 17 | Distribution | **Missing** | CI is test-only; no binaries, channels, checksums, or update path |
| 18 | Onboarding | **Partial** | Journey Stage 1b-3: an unconfigured launch greets in `*scratch*` naming `M-x` and four real bindings, and `M-x help` renders a cheat sheet. Still no tutorial and `C-h` still deletes a word — deliberately, see §18 |
| 19 | Coherence acceptance tests | **Started** | `tests/journey_acceptance.rs` carries 45 pins over steps 2, 3, 4, 5, 6 and 9 — the ratchet is real and stages add rows to it. The other five §19 scenarios (workspace lifecycle, worker ownership, config provenance, package lifecycle, extension isolation) are still unwritten |

Three cross-cutting patterns explain most of the table; they are
detailed in §1.1–§1.3: **substrate without surface**, **the silence
asymmetry**, and **per-arc coherence debt**.

Coherence-shaped work already in flight at audit time: find-file /
dired Stage 0 (`C-x C-f`, merged #162, `docs/dired-framing.md`) and its
Stage 1 directory view (merged #165), bottom panel Stage 1 (merged #155),
multi-root LSP affinity (merged #161), the config registry foundation
(merged #127).

---

## 1. The Product Problem

Pmacs is building many difficult things correctly and in parallel. That
is appropriate for an early systems project. The risk is that the
project succeeds architecturally while remaining fragmented
experientially.

A technically sophisticated editor can still feel incoherent when:

- installation requires repository knowledge;
- capabilities exist but are difficult to discover;
- subsystems expose unrelated interaction conventions;
- project, process, terminal, language-server, and remote state are
  modeled separately;
- configuration is powerful but provenance is unclear;
- packages can extend the editor but cannot be understood, controlled,
  or attributed;
- background work is concurrent but not meaningfully owned;
- new users must configure the system before they can experience its
  strengths.

The relevant distinction is between **capability completeness** and
**product coherence**. Capability completeness asks "can pmacs do X?".
Product coherence asks whether a user naturally encounters X at the
right time, whether X behaves by shared conventions, whether the user
can understand why X is active, and whether X feels like part of one
editor rather than an adjacent demonstration.

Pmacs is well on its way toward capability completeness in several major
areas. Product coherence must now become an explicit development track
rather than an emergent consequence of subsystem work. The 2026-07-25
audit found that every one of the eight bullet points above is true of
pmacs today, and that they share three structural causes.

### 1.1 Ground truth: substrate without surface

The single most consistent audit finding, appearing independently in all
four lanes: **the mechanism layer is disciplined, often best-in-class;
the product surface that would make it perceptible is missing.** The
July 2026 roadmap named an instance of this "dark matter — built but
unwired" and treated it as a one-time backlog. It is not one-time; it is
the project's default failure mode. The audited inventory of complete,
working, unreachable capability:

- **The entire rich help system.** `src/help.rs` implements a
  self-navigable `*help*` buffer with `[command:]` / `[key:]` /
  `[mode:]` / `[hook:]` / `[buffer:]` / `[view:]` cross-reference links
  and `follow_link_at`, installed as `pmacs.help.show_command` /
  `show_key` / `show_buffer` / `show_mode` / `show_hook` / `show_view`
  (`install_help_module`, `src/lua_bindings/mod.rs:5597`). `grep -rn
  "pmacs.help" builtin/` returns **zero hits** — no command, no
  keybinding, no caller.
- **File-name completion.** `CompletionSource::Files { root }`
  (`src/minibuffer.rs:589`) is reachable from Lua as `source = "files"`
  + `source_root` — zero builtin callers.
- **Command availability.** `Command.predicate` is stored on every
  command and **never evaluated** by `invoke`, `invoke_interactive`,
  keymap dispatch, M-x filtering, or the menu (§5).
- **Package ownership.** `CurrentlyLoadingPackage` is a stack correctly
  pushed/popped around every package chunk
  (`src/lua_bindings/mod.rs:3953-3966`) and consulted by exactly one
  binding (`on_unload`'s fallback). Every registrar ignores it (§13).
- **LSP health.** `LspManager::status_buffer_text()` (`src/lsp.rs:1204`)
  is Lua-bound; no builtin command opens `*lsp*` (§2, §9).
- **Interactive file opening.** `pmacs.buffer.find_or_open`
  (`src/lua_bindings/mod.rs:3103`) had no interactive caller at audit
  time; a complete 1,384-line dired existed only as a frozen test
  fixture (`tests/fixtures/pmacs-dired/init.lua`). **Fixed:** dired
  Stage 0 opens a path (`C-x C-f`, merged #162) and Stage 1 ships the
  browsing view as a builtin (`C-x d` / `C-x C-j`, merged #165). The fixture
  stays frozen — its `install_local` + `require` routing *is* the M8
  package-universality proof (Q#DR1) — and shrinking it is scheduled
  after Stage 3.

The strategic consequence: **most coherence gaps in pmacs are doors,
not engines** — deliberately deferred surface, not design error. That is
the cheap kind of gap, and it should change how the remaining work is
costed.

### 1.2 Ground truth: the silence asymmetry

Synchronous, user-initiated failures report well: `M-x` errors surface
as `"M-x error: <first line>"` (`builtin/commands/default.lua:633-641`),
compile spawn failures print in-buffer and on the status line
(`builtin/runtime/compile.lua:850-855`), and `pmacs --gpu` with no
`pmacs-gpu` binary produces the best missing-tool message in the
codebase — it names both the sibling path it tried and the PATH fallback
(`src/main.rs:367-379`).

Automatic, background failures are swallowed. The canonical case, hit on
**every file open** when a language server is preconfigured but not
installed: `Command::spawn` ENOENT propagates up through
`LspManager::spawn` and raises in Lua — where `ensure_server` `pcall`s
it, and the `buffer.after-load` hook `pcall`s the whole attach. Net
user-visible result used to be nothing: no status message, no `*errors*`
entry, no modeline marker (the LSP segment is gated on an attachment
record existing, so absence was indistinguishable from "unsupported file
type"). Working tree-sitter highlighting **actively masks** the failure —
the user sees colored text and assumes language intelligence is on.

**Journey Stage 1b-2 answers this specific case**
(`docs/journey-stage1b2-lsp-guidance-framing.md`): the failure is
reported once per `(language, root, command)` with the command, the
language and the errno; `M-x lsp.status` renders a durable `*lsp*` panel;
and the modeline says `LSP:!` instead of nothing. **The asymmetry itself
is not retired** — the rule below still needs adopting site by site, and
`pmacs.error` is still undefined.

Post-crash is the same shape and is **not** covered:
`LspEventKind::Crashed` is pushed and no builtin subscriber surfaces it.
A server that started and then died is a different failure with a
different message.

*(Citation note: this paragraph carried three stale line references —
`ensure_server` was cited at `:614-626` when the spawn `pcall` is at
`:658-674`, and the `buffer.after-load` hook at `:895-897` when it is at
`:1019-1021`. Symbols are authoritative per §25; the numbers are dropped
rather than re-pinned.)*

This directly contradicts the product thesis (§23): the "without
freezing" half is delivered; the "without becoming opaque" half is
currently false for exactly the failures a new user will hit first.

**The reporting channel the runtime believes it has does not exist.**
Fifteen call sites — `async.lua` (5), `syntax.lua` (4), and one each in
`lsp.lua`, `mcp.lua`, `fs.lua`, `editops.lua`, `autosave.lua`, and
`commands/default.lua` — report background failures through
`pmacs.error`, each guarded as `if pmacs.error then pmacs.error(...)`.
**`pmacs.error` is never defined in production** — the only assignment
in the tree is a test stub (`src/editor.rs:9881`), and
`type(pmacs.error)` is `nil` in a fresh `EditorState`. So every one of
those fifteen reports is dead: the guard makes the silence look
deliberate and keeps it from ever being noticed. `pmacs.errors` (plural,
`builtin/runtime/compile.lua:45`) is an unrelated namespace and is not
it. This is the silence asymmetry one level deeper than §1.2 first
recorded — not "the failure isn't surfaced" but "the surface was
written, guarded, and never built." Found while landing PR #161, which
nearly added a sixteenth; that one reports via
`pmacs.editor.set_status` (which exists) with the `pmacs.error` arm
riding along for when the channel is built.

**Rule to adopt:** anything that fails automatically must leave a
user-visible trace with a named owner. A `pcall` around background
wiring must log attributed failure, never discard it. Corollary from the
above: report through a channel with a **test that observes it**, or the
guard is indistinguishable from the silence it was meant to fix.

**Frequency note — corrected by Stage 1b-2.** This previously said the
failure fires "once per project root". It did not: `LspManager::spawn`
returns early *before* both `status_tracker.ensure` and
`clients.insert`, so a failed spawn left **no record at all**,
`pmacs.lsp.list()` could not see it, and `ensure_server`'s affinity loop
re-spawned. The real rate was **once per file open** — strictly worse
than recorded, and the reason the fix memoizes the *report* while still
retrying the spawn. Surfacing it stays
Priority 1 work with its own framing — it is a user-visible product
behavior (what message, where, with what guidance), not a substrate fix
to smuggle into an affinity PR.

### 1.3 Ground truth: coherence debt compounds per-arc

Three audited growth patterns show subsystem work accruing coherence
debt with no counter-pressure:

- Each new modal UI **extended the shadow family** instead of building
  the keymap-layer mechanism (menu → completion → query-replace, §6) —
  and each addition must hand-sync three guard lists (`dispatch_key`,
  `dispatch_idle_for`, `dispatch_paste`).
- Each new subsystem **added its own activity view** (`*workers*`,
  `pmacs.process.list`, `*lsp*`, the terminal-private id set, §9),
  because no common identity key exists to join them.
- Each new option **individually decides** whether to adopt the config
  registry; five have, everything else has not (§11).

The framing-doc workflow (scout → framing → approval → acceptance
criteria → bite-verified review) is exactly the right tool to reverse
this — no framing has ever carried a product-coherence acceptance
criterion. Adding them is a process change, not an engineering arc, and
it is what makes this document *required* rather than advisory.

---

## 2. The Golden Product Journey

Pmacs should maintain one protected end-to-end experience against which
all major work is tested:

1. Install Pmacs.
2. Launch it without prior configuration.
3. Open a real project.
4. Understand the visible interface.
5. Edit immediately.
6. Receive language intelligence.
7. Find a symbol or file.
8. Open a terminal.
9. Build or test the project.
10. Inspect and act on an error.
11. Understand what background work is running.
12. Close and later restore the workspace.

This does not need to exercise every advanced feature. It exists to
prove that the editor's components form a usable whole. A strong initial
target is a Rust project, because Rust stresses many of pmacs's intended
strengths: project detection, toolchain discovery, language-server
lifecycle, async diagnostics, build/test integration, terminal use,
large compilation workloads, symbol search, background indexing,
structured error presentation.

```text
Install Pmacs
    ↓
Run `pmacs .`
    ↓
Project root detected
    ↓
Rust mode activated
    ↓
rust-analyzer found or installation guidance shown
    ↓
Files, diagnostics, terminal, and project actions available
    ↓
Build or test command discoverable
    ↓
Errors become navigable structured results
```

This journey should become a release gate. New architectural work should
be evaluated partly by whether it improves, preserves, or complicates
the journey.

### Ground truth: the journey today

**Grade: reaches step 5; thin from step 6 on.** Was **broken at step 3**
at audit time:

```
$ ./target/release/pmacs .
pmacs: Is a directory (os error 21)
EXIT=1
```

The literal first arrow of the diagram above failed. `load_file`
(`src/file_io.rs:81-87`) does `File::open` (succeeds on a directory)
then `read_to_end` → EISDIR, which is not `NotFound`, so
`EditorState::open` returned `Err` and `main` printed and exited.

**Journey Stage 1a fixed that arrow** (`docs/journey-stage1a-framing.md`).
`resolve_target_buffer` now answers `ResolvedTarget::Directory` *ahead*
of the load, `pmacs .` lists the directory in dired, `RET` visits a
file, and a self-insert lands in it — steps 3 and 5 run end to end,
pinned by `tests/journey_acceptance.rs`. Which surface opens a directory
is a `path.open-directory` chain with dired as a replaceable fallback,
so this did not grow a second directory surface.

Still true: multiple file arguments are rejected (`"multiple files not
yet supported"`, `src/main.rs:227`), and everything from step 6 onward
is gated on a file being open — but the zero-config way to open one is
no longer "already know the path".

Full verdict table:

| # | Step | Verdict | Evidence |
|---|---|---|---|
| 1 | Install | **Partial** | Source build only: `cargo build --release --workspace --features pmacs/crdt` (`README.md`). No binaries, no packaging. Runtime deps (`/bin/sh`, git, tar, coreutils) documented, never checked at runtime |
| 2 | Launch unconfigured | **Works** | `EditorState::new()` → empty `*scratch*`; missing config is not an error (`src/config.rs:7-9`); recentf/saveplace/autosave default-on |
| 3 | Open real project | **Works at the CLI** | Journey Stage 1a: `resolve_target_buffer` answers `ResolvedTarget::Directory` before the EISDIR-producing load, and `EditorState::open` / the daemon bootstrap dispatch the `path.open-directory` chain, whose fallback is dired (#165's buffer, reached rather than duplicated). Startup no longer fails: an unreadable directory, a crashed resolver, and a cleared handler all report on the status line and leave the session running. Because the listing is async and the bootstrap is synchronous, the commit runs against a destination captured at request time (`pmacs.window.commit_to`) rather than against the ambient frontend |
| 4 | Understand interface | **Partial** | Mode line gives name/modified/L:C/scroll + mode/LSP/terminal segments. Journey Stage 1b-3 adds a welcome in `*scratch*` and `M-x help`; **still Partial** because `C-h` deletes a word (deliberately — §18) and there is no tutorial |
| 5 | Edit | **Works** | Full CUA + Emacs keymap in 161 lines (`builtin/keymaps/default.lua`); isearch, query-replace, kill ring, undo/redo, auto-indent/pair/comment, atomic save. Genuinely excellent zero-config |
| 6 | Language intelligence | **Partial** | Rust grammar bundled and auto-attaches; rust-analyzer preconfigured (`builtin/runtime/lsp.lua`). **Journey Stage 1b-2 (#204) ended the silence** for a server that fails to *start*: the status line names the command, language and errno once per `(language, root, command)`; the modeline reads `LSP:!` instead of nothing; and `M-x lsp.status` renders `*lsp*` over the `status_buffer_text()` renderer that had existed since M4.8 with no caller. **Still Partial**, for a reason unaffected by that landing: a server that starts and then *crashes* is still unsurfaced — `LspEventKind::Crashed` is pushed and no builtin subscriber handles it |
| 7 | Find symbol / file | **File: fixed (open by path merged #162; browsing #165). Symbol: works but undiscoverable** | No find-file/dired/picker existed at audit. Now `C-x C-f` opens a known path and `C-x d` / `C-x C-j` browse (flat listing, `dired` mode keymap); `M-.`/`M-?`/`C-c o` still bound but advertised nowhere and server-gated; no workspace-symbol command; `pmacs.index.*` has no UI |
| 8 | Open terminal | **Works** | Full PTY with scrollback + modeline segment, bound to `C-c t` and configurable through three registered settings (`terminal.default-profile`, `terminal.scrollback-rows`, `terminal.escape-key`) plus named `pmacs.terminal.profiles` (PR #173), and searchable through `M-x terminal.copy-mode` / `C-c C-t`, which materializes the retained scrollback into an ordinary read-only buffer (Stage 2). Named limitations: `C-c t` is unreachable from *inside* a terminal window, where `C-c` is consumed as the escape — `M-x terminal` still works there; and there is still **no close/kill command**, which is the remaining half of this step's discoverability gap. *Was broken outright on the GPU frontend until the double terminal-layout sync was fixed: the child took a `SIGWINCH` storm at tick cadence, so typing into it was impossible while output still flowed.* |
| 9 | Build / test | **Works** | Journey Stage 1b-1 (#203): `C-c c` runs `compile.run`, and the first prompt is prefilled from the detected project kind (`pmacs.compile.defaults`, seeded `rust = "cargo build"`, extensible from `init.lua`) via `ProjectKind::Rust` — **not** `Cargo`, see §24. The prompt **captures** its directory rather than re-resolving at accept time, so the command it offers and the directory it runs in cannot drift while the minibuffer waits. Still defaults cwd to the detected project root and parses Rust `-->` errors. Named limitation: after `pmacs <dir>` the active buffer is dired's and pathless, so the cwd falls back to the process cwd — §8's execution-location model owns that, and the degradation stays coherent (no suggestion is offered for a directory with no detected Cargo project) |
| 10 | Inspect error | **Partial (good once reached)** | `E:n W:n` modeline counts, underlines, `M-g n/p` + ``C-x ` `` walking a unified compile/grep/diag source, message echo, `RET` visits. Gated entirely on step 6 or 9 succeeding first |
| 11 | See background work | **Works but undiscoverable** | `*workers*` view via `M-x editor.list-workers`; `C-c C-k` cancel-at-point. No keybinding, no statusline spinner/progress indicator anywhere (§9) |
| 12 | Close + restore | **Partial** | Per-file cursor+scroll (saveplace), recent files, minibuffer history, autosave recovery all restore zero-config. Open-buffer set and window layout do **not**: desktop-save is opt-in (`pmacs.session.desktop_mode(true)`) *and* a documented no-op under a daemon (`src/desktop.rs:323-326`, `:353-356`, Q#DS9) |

A journey observation worth keeping verbatim from the audit:
**keybinding coverage is inverted relative to frequency** — `C-c @
C-M-s` opens all folds, while opening a file, opening a terminal, and
running a build have no bindings at all.

All three of that observation's examples have now been answered —
opening a file by `C-x C-f` (#162), opening a terminal by `C-c t`
(#173), and running a build by `C-c c` (Journey Stage 1b-1, #203).
**The quote stays as written**: it names a standing bias in how new work
gets bound, not three isolated omissions, and three fixes do not retire
a bias. What has changed is that the bias no longer has an uncontested
example in the golden journey — a new surface that ships without a
binding would be evidence the pattern is live again, and should be read
that way.

---

## 3. A Strong Zero-Configuration State

Pmacs should not require configuration before it becomes pleasant. The
default experience should demonstrate the editor's thesis: responsive
editing, visible asynchronous work, coherent project awareness, language
intelligence, integrated terminal and task execution, helpful
diagnostics, discoverable commands, graceful failure when external tools
are absent.

Configuration should be an escalation path:

1. The editor works.
2. The user notices a preference.
3. The relevant setting or command is easy to find.
4. The user changes it.
5. The editor explains where the effective value came from.
6. Advanced users can replace the behavior entirely.

### Recommended default surface

The graphical frontend should have a deliberate default workspace with a
restrained number of visible regions: main editor area; compact
statusline; optional project/files surface; bottom panel for terminal,
build output, diagnostics, and other transient tools; command palette;
contextual actions; unobtrusive background activity indicator. The TUI
should express the same conceptual model within terminal constraints.
The goal is not identical geometry across frontends — it is shared
nouns, commands, lifecycle, and state.

### Ground truth

**Grade: partial — the defaults half is strong, the graceful-failure
half fails.**

What already works with zero configuration, and is a real asset:

- Missing config is **not an error by contract** (`src/config.rs:7-9`);
  no config directory is created or required; a *broken* `init.lua`
  does not block startup — the error lands in `*errors*` and the status
  line (`src/config.rs:10-13`).
- Default-on persistence: recentf (`builtin/runtime/recentf.lua`, cap
  50, `C-x C-r`), saveplace (`builtin/runtime/saveplace.lua`, restores
  cursor + view on `after-load`), autosave every 30 s with next-session
  recovery (`builtin/runtime/autosave.lua:24`), per-bucket minibuffer
  history. State root: `PMACS_STATE_HOME` → `$XDG_STATE_HOME/pmacs` →
  `~/.local/state/pmacs` (`user_state_dir`, `src/state.rs:54-70`),
  wired only in real entry points (`install_state_dirs`) so tests stay
  hermetic.
- Atomic saves, full editing surface, bundled grammars for every
  preconfigured LSP language.

What fails the escalation path:

- Step 3 ("easy to find") fails for both settings and commands (§5).
- Step 5 ("explains where the value came from") is **unanswerable
  today**: config overrides are stored as bare values with no source
  (§11).
- "Graceful failure when external tools are absent" is the silence
  asymmetry (§1.2). The `--gpu` message (`src/main.rs:367-379`) is the
  pattern to replicate; LSP auto-attach is the anti-pattern.

---

## 4. Progressive Disclosure

Pmacs should support several levels of use without requiring users to
inhabit the most advanced one. These levels should be different
presentations of the same underlying objects — a command selected from a
context menu, invoked through `M-x`, bound to a key, called from Lua, or
triggered by an agent should be the same command object.

### Ground truth

**Grade: inverted.** The advanced level is largely real; the beginner
level is the one missing. Audited level-by-level:

**Beginner** (should see: files, buffers, search, diagnostics, terminal,
build actions, menus, missing-tool guidance):

- files ✓ since #162 / #165 (`C-x C-f` opens a path, `C-x d` browses;
  neither is advertised anywhere but the keymap) · buffers ✓ (`C-x
  b`, `*buffer-list*`) · search ✓ (`C-s`/`C-r`/`C-M-s`; project.search
  is M-x-only) · diagnostics ✓ once a server runs · terminal ✓ but
  M-x-only · build ✓ but M-x-only with empty prompt · menus △
  (right-click only, 11 items) · missing-tool guidance ✗ (§1.2).

**Intermediate** (should discover: palette, keybinding search, workspace
settings, profiles, package management, task definitions,
frontend/language settings):

- palette △ (`M-x` fuzzy over bare names, §5) · keybinding search ✗ (no
  list-keybindings/where-is commands) · workspace settings ✗ (no
  workspace scope, §11) · profiles ✗ (§12) · package management ✗
  in-session (§13) · task definitions ✗ · frontend customization △
  (themes, `pmacs.gpu.set_font`, statusline providers — all Lua-only) ·
  language settings △ (raw Lua tables, outside the registry).

**Advanced** (should be able to: inspect implementations, redefine live,
create packages, new views, providers, keymap layers, workspace policy,
orchestrate workers, replace interaction models):

- inspect ✓ (SourceLocation on everything; no jump-to-source command
  though) · redefine live ✓ (`unregister` + `define`) · packages ✓
  (authoring is real, §13) · new views ✓ (listview is Lua-usable) ·
  providers ✓ (statusline; completion/minibuffer sources are a fixed
  Rust vocabulary) · keymap layers ✗ (§6 — the mechanism does not
  exist) · workspace policy ✗ · orchestrate workers △
  (`pmacs.workers.register` funnels into builtin dispatchers, §9) ·
  replace interaction models ✗ (the shadows, §6).

The "same command object" principle largely holds where surfaces exist —
menu items, keybindings, and M-x all resolve command names into the one
registry — with one caveat: menu items are a **parallel registry of
labels** whose command references are unvalidated (§5).

---

## 5. Unify Discoverability

Pmacs already has the beginnings of a strong command registry. This
should become the center of a broader discovery model. Every meaningful
action should eventually expose: stable symbolic identity, title,
description, category, aliases, current keybindings, provenance,
applicability predicate (with an explanation when unavailable), argument
schema, destructive/asynchronous/reversible flags, locality, related
commands and settings, and source location. Settings should expose name,
type, description, default, effective value, provenance, scope,
validation rules, listeners, related commands. Packages and workers
should expose the analogous sets (§13, §9).

This suggests a general pmacs principle:

> **Anything that can affect the user should be discoverable as a
> structured object with identity, provenance, ownership, and
> lifecycle.**

### Ground truth

**Grade: substrate without surface — the sharpest instance of §1.1.**

**What the substrate already has (genuinely strong):**

- `Command` (`src/command.rs:66-79`) = `{ name, description, source,
  body, predicate }`. Description is **mandatory and validated** (R42);
  duplicate names are a hard error, not an overwrite; `SourceLocation
  { file, line }` is auto-captured from Lua debug info on **every**
  command, hook, menu item, config definition, config listener, and
  keybinding — the user cannot forge it. ~147 `pmacs.command.define`
  sites across `builtin/`.
- `ConfigDefinition` (`src/config_registry.rs:396-410`) is **richer
  than `Command`**: name, mandatory description, `ConfigKind`
  (Boolean/Integer/Number/String/Enum with bounds, choices,
  allow_empty), default, `Live`/`StartupOnly` mutability, source.
  `pmacs.config.list()` returns full descriptor tables.
- Reverse keybinding lookup exists as data: `KeymapStack::iter_all()`
  (`src/keymap_stack.rs:295-311`) enumerates every binding;
  `pmacs.describe.command(name).key_bindings` computes where-is on
  demand.
- `pmacs.describe.*` (`src/lua_bindings/mod.rs:6042-6162`) returns
  structured tables for command/key/buffer/view/mode/hook, and
  `describe.key` resolves against the **active buffer + major mode**.
- M-x matching is fuzzy (case-insensitive subsequence with
  boundary/consecutive bonuses, `fuzzy_score`,
  `src/minibuffer.rs:637-666`).

**What is missing, itemized:**

- `Command` has **no title, no category, no aliases, no argument
  schema, no destructive/async/reversible flags**. The dotted-name
  prefix (`buffer.`, `lsp.`) is convention, not data. MCP tooling works
  around the missing schema by stuffing rendered JSON schema text into
  the description string.
- **`Command.predicate` is dead metadata.** It is read in exactly two
  places (a literal line in the unreachable help renderer, and a test)
  and **never evaluated** by `invoke`, `invoke_interactive`, dispatch,
  M-x filtering, or the menu. The doc comment's claim that "the command
  palette (T M2.7) uses it to gray out unavailable entries" describes
  something that never shipped.
- **M-x shows bare name strings.** `CompletionSource::Commands` returns
  `Vec<String>` of names; the wire type `MinibufferPrompt.candidates`
  is `Vec<String>` (`pmacs-protocol/src/message.rs:994-1006`). No
  description, no keybinding, no category alongside candidates — while
  `CompletionPopupRow` (`:1231`) already carries `kind` and `detail`,
  proving richer rows are a solved wire problem in this codebase.
- **The entire Rust help layer is orphaned** (§1.1). Consequence: two
  parallel `*help*` implementations exist — `help.rs`'s
  cross-referenced renderer and the Lua `show_help_text` in
  `builtin/commands/default.lua:1103-1136` — and the one users can
  actually reach (`M-x editor.describe-command`) renders **less** than
  the unreachable one (no source, no scope, no predicate note).
- **Missing as commands entirely:** describe-key, describe-mode,
  describe-hook, describe-buffer, where-is, list-commands,
  list-settings, list-keybindings, apropos. What exists:
  `editor.describe-command`, `editor.describe-setting`,
  `editor.describe-instance[-buffer]`, `editor.list-buffers`,
  `editor.list-workers`. `M-x describe-setting` prompts **free-text
  with no completion source** (deliberately skipped —
  `builtin/commands/default.lua:1180-1185`); a typo yields a status
  line error.
- **No help prefix key.** `C-h` is `buffer.delete-word-backward`
  (`builtin/keymaps/default.lua:86`, with a comment noting the key "was
  free"). No `F1`, no `C-h k/f/b`.
- **Settings value provenance is absent.** Overrides are stored as bare
  values (`global: HashMap<String, ConfigValue>`,
  `src/config_registry.rs:693-708`); `describe-setting`'s "Source:" is
  the *definition* site. "Why is this setting 4 and who set it?" is
  unanswerable (§11).
- **Menu items are a parallel registry.** `MenuItem`
  (`src/menu.rs:56-78`) carries its own hand-written `label` duplicating
  the command's description, with a lazily-resolved `command` name
  string that is **never validated to exist** — a typo'd item silently
  does nothing when clicked. The wire row is label + separator only
  (`MenuPromptRow`): no key hints, no grayed state. Note the asymmetry:
  `pmacs.menu.list` reports `has_predicate`; `pmacs.describe.command`
  does not.
- **The two key-lookup APIs disagree.** `pmacs.keymap.lookup` is
  global-only (it resolves with no buffer and no modes,
  `src/lua_bindings/mod.rs:6294-6307`) while `pmacs.describe.key` is
  context-aware. `pmacs.keymap.list` erases `source` and renders
  `Scope::Buffer(id)` as bare `"buffer"` (id erased), so full-fidelity
  enumeration requires per-command `describe.command` calls. There is no
  which-key-style prefix surface.

**Shape of the fix:** roughly (a) three metadata additions on `Command`
(title, category, predicate actually evaluated + reported), (b) value
provenance in the config registry, (c) a dozen interactive commands and
richer M-x candidate rows over introspection that **already exists**.
This is the highest payoff-per-effort concern in the document.

---

## 6. Eliminate Hardcoded Interaction Islands

Pmacs's public programmability story will be strongest when all major
interaction layers pass through ordinary registries and extension
points. Temporary or modal interfaces — incremental search, query
replace, minibuffer prompts, completion menus, context menus, transient
selectors — should eventually use inspectable keymap layers rather than
special Rust-level interception. A general transient keymap model
includes priority, activation condition, owner, lifetime, fallback
behavior, discoverability, help labels, and cancellation behavior.

### Ground truth

**Grade: weak, and growing by one island per modal feature.**

Everything funnels through one function: `EditorInstance::dispatch_key`
(`src/editor.rs:901`), a single input-precedence state machine (its own
`#[allow(too_many_lines)]` says as much). The audited precedence order:

| # | Surface | Guard site | Decoder | Kind |
|---|---|---|---|---|
| 0 | popup-vs-modal auto-close | `editor.rs:917-925` | — | pre-step |
| 1 | Context menu | `editor.rs:933` | `MenuKey::from_chord` (`editor.rs:3005`) | **full shadow** |
| 2 | isearch | `editor.rs:939` | `SearchKey::from_chord` (`editor.rs:2902`) | **full shadow** |
| 3 | query-replace | `editor.rs:945` | `QueryReplaceKey::from_chord` (`editor.rs:2967`) | **full shadow** |
| 4 | Minibuffer | `editor.rs:951` | `MinibufferAction::from_chord` (`src/minibuffer.rs:468`) | **full shadow** |
| 5 | Completion popup | `editor.rs:958-971` | `CompletionPopupKey::from_chord` (`editor.rs:3056`) | **partial shadow** (control chords only; skipped while a multi-key prefix is pending) |
| 6 | Terminal transport + configurable escape | `editor.rs:973-1010` | `EditorState::terminal_escape_chord` → `TerminalManager::escape_chord` (`src/terminal/session.rs`) | **partial, transport-level** |
| 7 | Ordinary dispatch | `editor.rs:1018-1032` | `KeymapStack::resolve` | the only inspectable layer |

Facts that define the gap:

- **Full shadows eat every key**, including unrecognized ones (each
  decoder has an `Ignore`/`Dismiss` fallback arm). While a terminal
  buffer is focused and unescaped, *all* keys encode to the child —
  bindings led by the escape chord are **structurally unreachable** in
  a terminal buffer. Since #173 that chord is `terminal.escape-key`
  rather than a hardcoded `C-c`, so a user can *move* which prefix is
  eaten; they cannot make the shadow stop eating one.
- **A worked example that a modal-*looking* feature need not become a
  shadow.** Terminal copy mode (Stage 2 of the terminal-config arc) is
  the case that most invited a seventh rung: it wants motion, search and
  its own `g`/`q` inside a surface where every unescaped key otherwise
  goes to a child process. It resolves to the buffer-local keymap idiom
  instead, by **materializing** the retained scrollback into an ordinary
  read-only document buffer. The keys-must-not-reach-the-child problem
  then dissolves structurally rather than being guarded: the transport
  arm keys on `is_terminal(buffer_id)`, and a snapshot buffer is not a
  terminal, so the arm never fires. No new precedence rung, no new
  hand-synced guard-list entry, and `describe-key` keeps reporting the
  truth — pinned by asserting exactly that for the snapshot's `g` and
  `q`, which is the observable difference between the idiom and a
  shadow. **The count stays at six.**

  The transferable rule: when a feature wants a keymap over *content*,
  ask whether the content can become a buffer. The shadows that exist
  are the cases where it genuinely cannot (a minibuffer prompt, a
  live search prompt) — not the cases where nobody tried.
- **No transient-keymap mechanism exists to migrate to.** `KeymapStack`
  has exactly three fixed scopes — `Buffer(BufferId)`, `Mode(String)`,
  `Global` (`src/keymap_stack.rs:37-44`); resolution order buffer →
  mode → global with cooperative prefix-pending across scopes
  (`resolve`, `keymap_stack.rs:235-291`). No layer stack, no push/pop,
  no priority, no lifetime. The Lua scope accept-list hard-rejects
  anything else. So this is not "migrate the shadows to the layer
  system" — **the layer system must be built first.** (`active_modes`
  is also at most one mode today; minor modes are unbuilt.)
- **`describe-key` lies while a shadow is active.** With the completion
  popup open, `describe-key C-n` reports `cursor.down @global`; the
  literal arm `'n' => Some(Self::Next)` fires instead. Introspection
  has zero awareness of the shadows; Lua can observe only a boolean per
  surface (`popup_visible`, `search_active`, `query_replace_active`,
  minibuffer-active).
- **This is deliberate and documented** — rationale R51
  (`docs/keybindings.md`, `src/minibuffer.rs:470`): the shadows are
  intentionally not user-configurable. The completion framing
  considered and rejected buffer-local binds on teardown-lifecycle
  grounds (`docs/in-buffer-completion-framing.md:93-105`) — the
  objection was a *leaked binding outliving its session*, which is an
  argument for a lifetime-owning layer handle, not against layers.
- **Three hand-synced guard lists** must be updated per shadow:
  `dispatch_key`, `dispatch_idle_for` (`editor.rs:791` — deliberately
  omits the partial popup shadow; load-bearing for CRDT frontends'
  optimistic-apply correctness), and `dispatch_paste`
  (`editor.rs:1129-1140`).
- Off-path hardcodes: client-side **F12 detach** (`is_detach_key`,
  `src/attach.rs:997-1006`) and the replica frontends' **optimistic key
  classifiers** — classification, not routing, and kept honest by
  `dispatch_idle_for`. There are **two, one per replica frontend**, and the
  original audit named only one: `crate::optimistic::classify_key` belongs to
  the **`pmacs --attach` TUI** replica (`src/attach.rs:843` is its only
  consumer), while `pmacs-gpu` has its own, unrelated
  `optimistic_insert_text` / `optimistic_crdt_insert`
  (`pmacs-gpu/src/main.rs:2694`/`3306`). The "kept honest by
  `dispatch_idle_for`" claim was **verified for both** while investigating the
  GPU terminal input defect: a focused terminal buffer is in
  `round_trip_buffers` (`src/terminal/session.rs:338`), so `dispatch_idle_for`
  reports false and neither classifier can fire there.

**The counter-example that proves the idiom:** the entire picker/panel
family — listview (references, outline, lsp-help), project-search, buffer-list,
compile-mode, REPL, terminal scroll commands — uses ordinary
**buffer-local keymaps** via `pmacs.keymap.bind { scope = "buffer" }`
(`builtin/runtime/listview.lua:76-88` and siblings). These are
inspectable, correctly reported by describe-key, and rebindable from
`init.lua`. Roughly half the transient UI already lives on the right
side of the line.

**The concrete missing primitive** is small and well-scoped: a transient
overlay consulted before buffer scope (a `Scope::Transient` or an
overlay `Vec<Keymap>`), with (a) push/pop tied to session lifetime via a
lifetime-owning handle (RAII on the Rust side), (b) a full-shadow vs
partial-shadow flag (isearch eats everything and falls back to
search-self-insert; the popup intercepts eight chords and falls
through), and (c) `dispatch_idle_for` **derived** from the stack ("any
active layer is full-shadow") instead of hand-maintained. With that, the
six ladder rungs collapse into "session pushes a layer on open, pops on
close," and describe-key becomes truthful for free.

---

## 7. First-Class Workspaces

Project-root detection is useful, but pmacs needs a richer workspace
object. A project answers "which root contains this file?"; a workspace
answers "which persistent development environment owns this set of
activity?" A workspace should eventually own:

```text
Workspace
├── identity
├── one or more roots
├── execution location
├── environment and toolchain
├── configuration layers
├── trust policy
├── enabled packages
├── language-server instances
├── indexes
├── terminals and processes
├── tasks
├── debugger sessions
├── open buffers and views
├── frontend layout state
└── persistence and restoration policy
```

This matters for multi-root language servers, monorepos, generated
files, remote projects, containers, HPC environments, per-project
packages, task ownership, session restoration, and project-specific
trust. The workspace should be a core runtime entity, not an informal
convention shared across unrelated subsystems.

### Ground truth

**Grade: missing — what exists is a marker walk plus four independent
per-subsystem conventions.**

- **Detection**: `src/project.rs` — `default_markers()` is
  `Cargo.toml`, `go.mod`, `package.json`, `.git` (directory), with the
  deliberate rule that **language markers outrank `.git`** at the same
  ancestor level; upward walk, innermost wins; `set_search_boundary`
  honored; `ProjectKind` (e.g. `Cargo`) exists and is **consumed by
  nothing** user-facing.
- **Four independent consumers**, each resolving on its own:
  LSP root (`project_root_for`, `builtin/runtime/lsp.lua:554-570`:
  config override → marker walk → file's own directory, returning
  `root, source` where source ∈ config/detected/fallback); compile cwd
  (`project_root_of_active`, `builtin/runtime/compile.lua:600-608`);
  project-search root (`resolve_search_root`,
  `builtin/commands/default.lua:843-857`, falls back to `"."`); the
  project symbol index (`.pmacs/index.json`, `src/project_index.rs`).
- **There is no "current project" independent of the active buffer's
  path.** With only `*scratch*` open, every consumer above returns
  nil/`"."`. Nothing owns the set {roots, servers, terminals, tasks,
  layout} — which is why desktop-save under a daemon had nothing
  principled to attach to (Q#DS9, §2 step 12).
- **First slice landed (PR #161)**: the multi-root LSP server-affinity
  work makes *(language, found-root)* the server identity — the first
  time a root functions as an identity key rather than a spawn
  parameter. It also establishes the rule that a *fallback* root (the
  file's own directory, when no marker was found) is deliberately **not**
  an identity, so markerless files keep sharing one server per language.
  Note it is again per-subsystem: LSP learns roots; compile, search,
  index, and trust do not share the object.

A workspace entity is a **model gap** (real arc), not wiring. It is also
the prerequisite that keeps §8 (locations), §9 (task ownership), §11
(workspace config scope), and step 12 of the journey from each inventing
their own ownership story.

---

## 8. First-Class Execution Locations

Pmacs's daemon/frontend architecture gives it an excellent basis for
remote development. The next step is to model execution location
explicitly — a value that can be inspected and assigned, not an
implementation detail hidden inside file access or process spawning:

```text
Location
├── local
├── ssh://host
├── container://name
├── slurm://allocation
├── daemon://session
└── custom provider
```

Filesystem roots, processes, terminals, language servers, workers,
debuggers, indexers, package services, and build/test tasks should all
carry a location. That makes answerable: where is this server running?
where will this build execute? is this terminal local? can this worker
migrate? what happens if the remote daemon disconnects?

### Ground truth

**Grade: missing as a model; the architecture half already works.**

What exists: `pmacs --attach user@host` (remote TUI over SSH),
`ssh:user@host/instance` / `local:/path.sock` addressing, mosh-modeled
reconnect-on-drop, and the daemon/frontend split itself — i.e.
`daemon://session` exists implicitly and robustly. What does not exist:
any `Location` value. Every `ProcessSpec` spawn, LSP server, terminal
PTY, and worker is implicitly daemon-local; no resource carries a
location field; nothing can be asked "where is this running?". No
container/slurm/provider concept anywhere.

This concern is deliberately *after* §7 in dependency order: a location
without a workspace to scope it has nothing to attach to. For the
research/HPC ambition (§12's Research Workstation profile), this pair is
the long-lead differentiator — nothing else in the editor market models
it well.

---

## 9. Extend the Worker Model into Structured Concurrency

Pmacs's worker system is one of its most distinctive strengths.
Cancellation, supersession, streaming, frame-aware draining, and the
`*workers*` view provide a strong basis. The next step is ownership and
hierarchy: every substantial task should have an owner, a workspace, an
optional buffer/view, a parent, children, a latency class, a
cancellation scope, a resource budget, an execution location, progress,
and failure attribution.

```text
Workspace: pmacs
└── Command: project-build
    ├── Task: save-dirty-buffers
    ├── Task: cargo-check
    │   ├── Process: cargo
    │   └── Stream: compiler-diagnostics
    └── Task: refresh-diagnostics
```

Cancelling `project-build` should cancel its children. Closing a
workspace should terminate or detach workspace-owned work. Reloading a
package should stop package-owned tasks. The activity view should answer
what is running, why, who owns it, where, what depends on it, and what
cancellation will affect. That turns parallelism into a product feature
rather than an implementation claim.

### Ground truth

**Grade: mechanism without identity.**

**The mechanism layer is solid:** cooperative per-job cancellation
tokens with panic isolation (`src/worker.rs:13-28`); supersession with
correct settle-time pruning (`src/async_runtime.rs:688-701`) — a
genuinely good primitive; a completions ring (cap 64); `register_external`
so non-pool work (LSP requests, MCP) appears uniformly; one shared
`ProcessSupervisor` under everything (`src/editor.rs:341`); the
`*workers*` view (`src/workers_buffer.rs`, opened by `M-x
editor.list-workers`, auto-refreshing, `C-c C-k` cancel-at-point).

**The identity layer is absent:**

- `PendingJob` (`src/async_runtime.rs:365-392`) carries `{cancel,
  state, supersede_key, stream_buffer, max_batch, kind,
  dispatched_at}`. **No owner. No purpose string. No
  workspace/buffer association. No parent.** The one buffer link that
  exists (parse job → buffer) lives in a `SyntaxCoordinator` side map,
  invisible to the workers view.
- `JobKind` is a **closed 12-variant enum** (Sleep, ComputeSum, EmitN,
  Grep, Parse, FsReadDir, FsStat, FsRename, FsChmod, FsRemove,
  McpRequest, LspRequest). `pmacs.workers.register` funnels Lua jobs
  into existing Rust dispatchers, so **every third-party job renders
  under a builtin's label**.
- Supersession is opt-in per dispatch site and underused: `"search"`
  (grep) and `lsp:{method}:{sid}:{uri}` use it; **parse jobs and all
  MCP requests pass `None`** — a fast typist stacks parse jobs.
- Cancellation scopes: per-id and per-key only. No cancel-all,
  by-kind, by-buffer, by-owner, or by-subtree — there is no scope to
  range over.
- **Four disjoint activity planes with no join key:**

| Plane | Surface | What it misses |
|---|---|---|
| Async jobs | `*workers*` | processes, servers, terminals |
| OS processes | `pmacs.process.list` (no buffer view exists) | **filters to `LineOriented` only — terminal PTYs are invisible**; `spawn_terminal` bypasses the public path entirely |
| LSP servers | `*lsp*` status text | **no builtin command opens it**; LSP sets `RestartPolicy::Never` on the supervisor and runs its own restart logic |
| Terminals | private id set drained after the supervisor tick | user-visible in none of the above |

  A terminal PTY appears in **no** user-visible activity view. An LSP
  server appears in `*lsp*` (unreachable) and `list()`; its requests
  appear in `*workers*`; nothing joins them.
- **No progress indicator exists anywhere** — no statusline spinner,
  no busy count (grep for progress/spinner/busy in `src/statusline.rs`
  is empty). "Visible asynchronous work" (§3) is currently false unless
  the user knows to run `M-x editor.list-workers`.
- `ProcessSpec.label` is the nearest thing to attribution: caller-
  supplied, unvalidated convention (`lsp:{name}`, terminal buffer
  name).

The audit's conclusion, worth preserving verbatim: *because identity is
missing, scoped cancellation has nothing to scope over and a unified
activity view has nothing to group by — the four views exist precisely
because there is no common key to merge them on.* Owner/purpose/parent
fields on the job and process specs are the prerequisite; the unified
view and the ownership tree fall out of them.

---

## 10. Define Extension Trust and Isolation Classes

Pmacs should preserve live, low-friction programmability — it should not
force all extensions into rigid out-of-process APIs. At the same time,
namespace isolation inside a shared Lua state is not enough for fault
containment, security, latency containment, memory accounting,
native-code isolation, reliable unloading, or project-local trust. Pmacs
should define extension classes before the ecosystem becomes large:

- **10.1 Trusted core packages** — in-process, deep API access,
  distributed with pmacs or explicitly trusted.
- **10.2 Normal Lua packages** — shared/managed runtime, declared
  capabilities, owned registrations and workers, execution budgets,
  measurable latency, reloadable lifecycle, package-level error
  attribution.
- **10.3 Isolated service extensions** — separate process, typed RPC,
  crash recovery, resource accounting, explicit fs/process/network
  permissions.
- **10.4 Project-local / untrusted** — explicit approval, restricted
  capabilities, strong isolation, workspace-scoped trust, easy
  revocation.

### Ground truth

**Grade: missing — one class exists.**

Every package today is a 10.1/10.2 hybrid with none of 10.2's
machinery: in-process, per-package `_ENV` with `__index = _G`
(namespace hygiene, not containment), full API access, no capability
declarations, no budgets, no latency measurement, no owned-registration
lifecycle (§13). The only containment primitive in the tree is the
instruction-count hook that can cancel a hot-looping main-thread chunk
(`src/lua_isolation.rs:1-39`) — a runaway guard, not an isolation class.

Two real assets to build on: the loader's `exports` gating (the package
searcher is deliberately inserted at position 1 of `package.searchers`
so exports are enforceable, `src/lua_bindings/mod.rs:3891-3900`), and
**MCP as the existing 10.3 seam** — packages can already spawn MCP
servers and consume their tools over a typed transport
(`docs/mcp-for-package-authors.md`), which is exactly the
separate-process/typed-RPC shape 10.3 asks for. Project-local trust
(10.4) has a natural anchor once §7's workspace exists.

Sequencing note: 10.2's "owned registrations, reloadable lifecycle,
error attribution" is the same work as §13's ownership gap — do it once,
under one arc.

---

## 11. Configuration as Typed, Layered Data

Pmacs's typed configuration registry is the correct foundation. It
should develop into a layered system with explicit provenance. Likely
layers: built-in defaults; profile defaults; user settings;
machine-local; remote-location; workspace; root/folder; language/mode;
buffer-local; session overrides. A setting inspection view should show
the full chain and the active source:

```text
setting: editor.tab-width
effective value: 4
type: integer
scope: workspace

defined by:
  built-in default: 8
  Rust profile: 4
  user setting: 2
  workspace override: 4

active source:
  ~/src/pmacs/.pmacs/settings.lua
```

Pmacs should also preserve three distinct levels — **settings** (typed
declarative data), **behavioral customization** (commands, hooks,
keymaps, Lua), **package construction** (new capabilities) — so that
users do not need executable Lua for ordinary preferences, while
advanced users can still replace the mechanism.

### Ground truth

**Grade: partial — the foundation shipped (#127) and is correct; the
layering, provenance, and adoption have not followed.**

- The registry is typed, described, duplicate-rejected, freeze-aware
  (`StartupOnly`), listener-bearing, and introspectable — see §5. Its
  design decisions (always-store overrides, explicit buffer, no ambient
  scope) are recorded in `docs/config-registry-framing.md`.
- **Two scopes exist** of the ten layers listed above: global and
  buffer-local. Per-language and per-project are patterns (a hook
  calling `set_local`), not scopes. No profile, workspace, machine, or
  remote layer.
- **Value provenance is absent** (§5): overrides are bare
  `ConfigValue`s; `describe-setting`'s "Source:" names where `define()`
  ran. The inspection view sketched above is currently impossible to
  render.
- **Adoption is nine settings**: `editing.auto-pair` (pair.lua),
  `editing.trim-on-save` (editops.lua), `autosave.interval-ms`
  (autosave.lua), `window.panel-height` + `window.min-height`
  (window.lua), `terminal.default-profile` +
  `terminal.scrollback-rows` + `terminal.escape-key` (terminal.lua,
  #173), and `lean.abbrev` (lean_input.lua, Arc 8 Stage 4b) — a
  `live` boolean read against the typed edit's SOURCE buffer, the
  `editing.auto-pair` shape including its correction to resolve
  `rec.buffer` rather than the active buffer. Everything else a user might set — theme, fonts, LSP
  server config, killring size, recentf/saveplace/desktop enables,
  pair sets, comment strings, `pmacs.parse.*` — lives in raw Lua
  outside the registry and is therefore invisible to `describe-setting`
  and any future settings UI. The migration list is already written:
  `docs/config-registry-framing.md` "named deferrals" (table-valued
  settings are the hard prerequisite for LSP/pair/comment tables).
- **The table-valued gap now has a named, shipped instance.**
  `pmacs.terminal.profiles` (#173) is a raw Lua table sitting beside
  three registered scalars *for the same feature*, because a profile is
  inherently `{ command, args, cwd, env }` and the registry stores four
  scalars. It is the clearest evidence yet that table-valued settings
  are the blocking prerequisite: the terminal is now half-registered,
  and no settings UI can render the half that matters most.
- **The missing `scope = "global"` flag has its second live case.** After
  `autosave.interval-ms`, the terminal's two *open-time* settings —
  `terminal.default-profile` and `terminal.scrollback-rows` — are read
  before their terminal's identity buffer exists, so a buffer-local
  override can never be consulted. The registry accepts `set_local` on
  them anyway, because `Live` mutability is all it can express. Nothing
  breaks; the setting simply has no effect, which is the worst shape a
  configuration surface can take. `terminal.escape-key` is the contrast
  that shows this is a real distinction rather than a blanket wish: it
  *deliberately* supports buffer-locals, and per-terminal escapes are a
  feature. So the argument for both deferrals is now **cumulative and
  concrete** rather than hypothetical — two adopters, two distinct
  missing primitives, one feature.
- **No persistence**: settings changed at runtime do not survive
  restart (the `custom-file` split-brain question is a named deferral).
- The three-level separation holds in principle today (registry /
  hooks+keymaps / packages), but with nine settings registered, level 1
  is effectively empty — users need executable Lua for nearly every
  ordinary preference, which is the exact failure the section warns
  about.

---

## 12. Profiles as Product-Level Bundles

Pmacs should offer a small number of official profiles bundling default
keymaps, visible interface regions, package recommendations, settings,
task conventions, discovery hints, and onboarding: **Pmacs Standard**
(approachable graphical workstation), **Emacs** (familiar bindings,
minibuffer-centered), **Minimal**, and later **Research Workstation**
(terminals, remote machines, Slurm, proof assistants, long-running
builds). Profiles must not create separate products — they exercise the
same registries and primitives.

### Ground truth

**Grade: missing.** Not a named concept anywhere in the tree. There is
one hardcoded default: a single 161-line keymap
(`builtin/keymaps/default.lua`) that is already a de-facto hybrid of the
"Standard" and "Emacs" profiles (CUA selection + Emacs kill/yank/isearch
chords). No profile object, no bundle format, no selection mechanism, no
per-profile defaults layer (§11's missing profile scope is the same
gap). Prerequisites: the config profile layer, and enough registry
adoption that a profile has something declarative to set.

---

## 13. Package Experience, Not Merely Package Resolution

Pmacs already has serious package-resolution machinery. Product
coherence requires a package *lifecycle* experience: search,
installation, updates, disable, reload, uninstall, version inspection,
dependency graph, compatibility warnings, capability declarations,
ownership inspection, error history, active-worker inspection, resource
use, trust state. Installation should work during a running session.
Users should be able to install coherent capability bundles ("Rust
Development") rather than individual packages. Marketplace sequencing:
stable format → ownership/reload lifecycle → in-editor manager → curated
registry → bundles → publisher identity → public marketplace.

### Ground truth

**Grade: resolution without lifecycle — the artifact layer is mature,
the lifecycle layer assumes a single author iterating on their own
package.**

**Mature (keep):** `pmacs.toml` manifest (validated name, semver,
`pmacs_required`, dependencies/conflicts, entry, exports); git-address
installs (`github:`/`gitlab:`/URL; auth delegated to git config; no
registry service); iterate-to-fixed-point resolver with deterministic
ordering and honest unsatisfiability errors (documented no-backtracking
tradeoff); merged SHA-256 lockfile; per-package `_ENV`; `exports`
enforced by a position-1 searcher; bundled packages through the
identical path.

**The lifecycle facts:**

| Operation | State |
|---|---|
| `install` / `install_project` / `install_local` / `update` | exist, **init.lua-only** — `require_init_phase` raises `InitOnlyApi` mid-session; the error text admits there is no CLI equivalent ("restart with an updated init.lua") |
| `reload(name)` | **works in-session and is well-built**: unload hooks → loaded-table invalidation (name + `name.` prefixes) → env clear → re-require |
| `installed()` / `describe(name)` / `load(name)` / `on_unload(fn)` | work in-session; `describe` returns manifest metadata only |
| uninstall / remove | **absent** — the documented procedure is `rm` in a shell (`src/packages/installer.rs:1178-1180`) |
| disable / enable | **absent** — no concept |
| search / list-available | **absent** — no registry, no index; you must already know a git URL |
| inspect contributions | **absent** — `describe` cannot say which commands/hooks/keys/settings a package contributed; no `*packages*` view exists |

**Structural findings that any lifecycle arc must address:**

- **The roster is in-memory per session**, rebuilt from `init.lua`
  calls. A package on disk that init.lua doesn't `install` is invisible
  to `require`/`installed()`. And because `do_install` unconditionally
  runs the resolver, **every startup runs `git fetch --prune --tags`
  per package** before the idempotent fast-path can trigger — a
  first-launch latency and offline-use problem. (The Rust-side
  `UpdatePolicy::Frozen` that would fix offline installs is
  **unreachable from Lua** — dead code.)
- **Ownership is not tracked.** `SourceLocation` is path attribution,
  not package attribution (for `install_local` the path may be the dev
  tree, not the install root); nothing indexes registrations by source;
  there is no "what did package X register" query and no
  bulk-unregister. The correct signal (`CurrentlyLoadingPackage`)
  already exists and is ignored by every registrar (§1.1).
- **The teardown surface is incomplete in a way that makes the
  documented convention unsatisfiable: `pmacs.hook.remove` does not
  exist** (`install_hook_module` exposes define/add/list/run;
  `HookRegistry` has no removal method at all). A package that calls
  `pmacs.hook.add` leaks a callback on every reload, permanently. The
  package-author guide's hand-rolled `OWNED = {}` cleanup pattern
  (`docs/package-author-guide.md:379-415`) cannot be followed for
  hooks. (Independently rediscovered by the Lean 4 arc scout.)
- **Error attribution exists on exactly one code path** —
  `packages.load` wraps require and logs `[package <name>] load failed`
  to `*errors*` — **and nothing in `builtin/` uses it**. Plain
  `require` from init.lua attributes only by traceback; a failing
  `install` aborts the whole init.lua with no per-package isolation.
- Install-root directory names are the manifest name's last segment, so
  same-basename packages collide on disk (knowingly accepted,
  `installer.rs:44-50`).

Sequencing: ownership + `hook.remove` + attribution is the same work as
§10's class 10.2 and is the prerequisite for disable/uninstall/inspect;
in-session install requires reworking the init-phase gate; search/
bundles/marketplace remain correctly last.

---

## 14. Coherent Workbench Primitives

Pmacs should resist implementing each subsystem with a custom UI
vocabulary. It should provide a small set of reusable view primitives —
editable text view, virtual list, tree, structured table, inspector,
output channel, diagnostics collection, task/progress view, diff view,
transient selector, contextual popup, side panel, bottom panel,
help/documentation view — and packages should provide structured models
to them. Git status, project files, symbol outlines, package
dependencies, and worker trees should share one tree model with
consistent selection, expansion, filtering, action discovery, mouse and
keyboard behavior, persistence, and accessibility.

### Ground truth

**Grade: partial, with the best trajectory of any concern.**

Primitive-by-primitive against the list above:

- **Editable text view** ✓ — the buffer itself, everywhere.
- **List** ◐ — listview is a real shared primitive with a shared
  buffer-local keymap idiom (RET/SPC visit, n/p, g refresh, q quit)
  that is inspectable and rebindable (§6's counter-example). **But its
  adoption is narrower than this document claimed, and the correction
  matters more than the grade.** Measured at `ad41cf1`: there are
  exactly **three** `pmacs.listview.open` call sites, **all three in
  `builtin/runtime/lsp.lua`** — `*references*` (`:2056`), `*outline*`
  (`:2102`) and `*lsp-help*` (`:2513`). The three other `listview`
  mentions under `builtin/` are comments in `compile.lua` and
  `dired.lua` citing "the listview idiom", which is a *pattern being
  copied*, not the primitive being used.
  - **`*buffer-list*` and project-search do NOT use it**, contrary to
    the previous wording here. `*buffer-list*` is built independently
    in `builtin/commands/default.lua` — its own comment calls it "a
    regular buffer named `*buffer-list*`" (`:348`, `:367`). This
    document previously asserted both that buffer-list uses listview
    and, twenty lines below, that it does not; the second was right.
  - **So the asset is one subsystem's, not the UI layer's.** Every
    consumer is an LSP panel, and the two surfaces most often cited as
    proof of sharing re-implement the pattern by hand. A primitive
    three sibling call sites deep in one file is a good primitive with
    an adoption problem, which is a different remediation from a
    missing one: the work is migrating `*buffer-list*` and search
    onto it, not building it.
  - **Copying the idiom propagated a defect**, which is the concrete
    cost of counting imitators as adopters: the erroring-intercept
    pattern the comments copy is exactly the one the Output-channel
    bullet below records as *not* read-only.
- **Output channel** ✓ — the compile-mode `*compilation*` model
  (streamed, intercept-read-only, error-rule parsing), reused by grep
  and shell-command. **Caveat found in terminal copy mode's review
  (Stage 2): "intercept-read-only" is not read-only.** `Buffer::undo`
  reaches the rope through `ensure_writable` without consulting the
  intercept chain, so `M-x buffer.undo` empties such a buffer — and
  rebinding the undo *chords* buffer-locally does not close it, as
  `compile.lua`'s own comment admits ("command/menu undo stays
  dispatchable"). `Buffer::set_generated_contents` (write + discard
  history + assert `read_only`, in one authorized call) now fixes this
  for the terminal snapshot; **four writer mechanisms have not yet adopted
  it and remain emptiable** — listview panels; `compile.lua`'s
  `ensure_slot`, which serves `*compilation*` **and** `*shell-command*`;
  the independent `*search-results*` panel in
  `builtin/commands/default.lua`; and dired buffers. All pair an erroring
  intercept with `bypass_intercept` writes over a still-writable rope.
  (`*workers*`, `*help*` and `*buffer-list*` are generated but do not use
  this idiom.) **A second half of the same
  caveat, found in round 3: a rope write is only half of an edit.** The
  owner-authorized write must be fanned out to the windows showing the
  buffer and queued for replica mirrors, or the displaying window keeps
  a line index describing the previous contents and the next paint
  indexes the new rope with stale ranges. Adoption is therefore not a
  one-line swap — and the three appending buffers (`*compilation*`,
  `*shell-command*`, `*search-results*`) need a streaming variant of the
  primitive that does not exist yet. Listview and dired already write
  whole-buffer replaces and are the cheap half.
- **Diagnostics collection** ✓ — `DiagnosticStore` + signs + unified
  `error.next` source.
- **Transient selector** ✓ — the minibuffer (though its `source`
  vocabulary is fixed Rust-side).
- **Contextual popup** ✓ — completion popup, context menu (each a
  shadow, §6).
- **Bottom/side panel** ✓ — Stage 1 (#155) gave the substrate:
  `WindowParams` side/fixed_rows/dedicated, `display = "current" |
  "panel"` adopted by listview/compile/terminal, quit-action, divider
  drag. **Stage 2 completed it on the second frontend** (2A #177, 2B-1
  #184, 2B-2 #187, 2B-3): the GPU frontend now renders a real panel
  band with its own divider, pointer routing, and resize drag, and a
  semantic session that negotiates the panel wire is panel-capable.
  Both frontends therefore share one placement primitive rather than
  the GPU silently taking the Stage 1 non-side fallback. Stage 3 — the
  adopter default flip, so omitting `display` resolves to the panel
  policy — is the remaining step.
- **Task/progress view** △ — `*workers*` exists but joins nothing
  (§9).
- **Help view** △ — exists twice (§5); needs unification, not
  invention.
- **Tree** ✗ — none. The named future consumers (project files, symbol
  hierarchy, package dependency graph, worker trees, git status) will
  each need it; building it once *before* dired's directory view and
  the workers tree harden their own conventions is exactly this
  section's point. Dired Stage 1 (merged #165) landed **without** inventing
  one: its listing is flat (Emacs parity), and the recursive
  in-buffer case — `i` insert-subdirectory — is a named deferral in
  `docs/dired-framing.md` §13, which is where a shared tree primitive
  would land.
- **Structured table / inspector / diff view** ✗ — none. (`describe.*`
  tables are the inspector's data model without a view; the
  wire-declared `ResourceOffer` family was reserved for diff/blame
  sources and remains unproduced.)

---

## 15. Contextual Affordances

Pmacs should remain excellent for keyboard-driven users while making
capabilities visible to users who do not know their names: a diagnostic
should offer code actions; a test definition run/debug; a Git change
stage/revert/diff; a missing formatter configuration guidance; a symbol
references/rename/definition/documentation; a remote workspace its
location; a long-running task progress and cancellation. Affordances
should invoke ordinary commands, never separate logic paths.

### Ground truth

**Grade: weak.**

What exists: the right-click context menu — 11 items in 4 groups
(edit/symbol/diagnostic/history, `builtin/menus/default.lua:117-142`),
with a closed context vocabulary (`always`/`selection`/`symbol`/
`diagnostic`, `src/menu.rs:44`) and per-item predicates that *are*
evaluated (unlike command predicates). It correctly invokes ordinary
commands by name. Its limits: right-click only (no keyboard path in),
no key hints on rows, invisible items filtered rather than grayed, and
the unvalidated command references of §5.

What does not:

- **Code actions apply the first action blindly** — no picker (a
  roadmap "dark matter" item still true at audit).
- **There is no Git integration at all** — no status, stage, diff,
  blame, or gutter markers anywhere in the tree (gutter git riders and
  the `ResourceOffer` diff/blame family are named deferrals). The Git
  affordance list above has nothing to attach to yet.
- No test run/debug affordances (DAP is a future arc,
  `docs/dap-debugging-framing.md`).
- No missing-tool guidance affordances (§1.2 — the diagnostic that
  *should* say "rust-analyzer not found — install with rustup" says
  nothing).
- No remote-location display (§8 — nothing carries a location).
- Task progress/cancellation affordances exist only inside `*workers*`
  (§9); a long-running task shows nothing at the point of origin.

---

## 16. Productize the Semantic Frontend Architecture

The semantic protocol should be visible as a product advantage: native
frontend rendering, frontend-specific typography, high-quality
decorations, efficient incremental updates, accessible semantic
information, multiple simultaneous frontends, stable remote attachment,
frontend experimentation without reimplementing editor semantics. To
preserve coherence: core commands frontend-neutral; stable semantic
identities; explicit capability negotiation; graceful degradation;
layout state separated from semantic state; no frontend becoming the de
facto privileged implementation.

### Ground truth

**Grade: strong — the healthiest concern in this document, and most of
its asks are already practiced.**

- Versioned protocol schema `SUPPORTED=[6..=21]` with deliberate
  encoding-breaking bumps, both-frontends support required per bump,
  and byte-pin discipline for appended variants (handoff §4). The v21
  bottom-panel family landed with Stage 2B-1 (#184) and is **live in
  production since Stage 2B-3** — activated without an incompatible
  handshake change. Because `Hello` is server-first, the daemon's
  advertised version is now a permanent compatibility **baseline**
  (still 20) and the session's real version is settled one message
  later by the frontend's `AttachRequest` counter-offer. That split —
  advertise the floor, negotiate up — is the reusable pattern for every
  future additive family, and it means bumping the advertised version is
  reserved for a change that cannot be expressed additively at all.
- Two genuine frontends share the conceptual model; CRDT concurrent
  editing with presence across them; remote attach + reconnect.
- **Graceful per-frontend degradation is practiced, not aspirational**:
  fold projection is per-frontend (`FrontendView.fold_projection`,
  selected from the negotiated `semantic_render` bit) so a grid
  frontend collapses folds while a simultaneous GPU session does not
  skip lines (#149/#148).
  - **But it is enforced by convention, not by structure.** The GPU terminal
    input defect was a per-frontend-kind operation applied to *both* kinds:
    the dispatcher's grid and semantic terminal-layout syncs were written as
    twins and executed as siblings, so a GPU session's PTY was resized twice
    per tick forever. `sync_terminal_layouts_for_tick` now makes that one
    exclusive by construction; every other per-frontend-kind pair in the
    dispatcher remains two adjacent `if`s that a reader must notice are
    alternatives.
- The GPU frontend exceeds the TUI (minimap, squiggles, typography,
  and since #158 rendered inline math) without the TUI losing the
  model — the "no privileged frontend" rule is holding under real
  divergence pressure. Inline math is the sharpest case so far: the
  GPU shapes `$…$` spans through a bundled MATH-table font while the
  TUI shows the LaTeX source unchanged, and the TUI's distinct-face
  fallback is a **named deferral rather than an oversight**. What
  keeps it inside the rule is that the slice reserves no protocol
  version and adds no wire surface — the divergence is presentational
  only, and the semantic model both frontends read is identical.

Remaining, honestly small relative to the section's ambition: capability
negotiation is per-bit rather than a first-class declared capability
set; layout state vs semantic state separation is partial (window layout
is daemon-side; desktop restore under a daemon is unresolved, §2 step
12); and the advantage is invisible as *product* because §17 means
nobody outside the repo can try it.

---

## 17. Distribution Is Part of the Product

Pmacs should eventually be installable without repository familiarity:
reproducible release builds, Linux and macOS binaries, checksums and
signatures, stable and nightly channels, one-command update, rollback,
protocol- and package-API compatibility reporting. First launch should
create/locate config directories, explain the default profile, identify
optional external tools, and let the user open a project immediately.

### Ground truth

**Grade: missing — zero release machinery exists.**

`.github/workflows/` contains exactly one workflow, `ci.yml`, and it is
test-only (fmt/clippy/test matrix; the only `release` strings in it are
`cargo test --release` flags). No release job, no artifact upload, no
tags-to-binaries path, no checksums, no channels, no update or rollback
mechanism. Installation is `git clone` + `cargo build --release
--workspace --features pmacs/crdt` (README), which additionally requires
knowing the feature-flag matrix (luajit vs lua54 × crdt). Runtime
dependencies (`/bin/sh`, `stty`, git, tar) are documented in the README
and never checked at runtime. First launch creates nothing and explains
nothing (§18) — though by design it also *requires* nothing (§3), which
is the right half to have.

This concern is independent of every other arc and can start anytime;
until it does, every other coherence improvement is invisible outside
the repository.

---

## 18. Onboarding

Pmacs needs onboarding that teaches concepts through use: open a
project → command palette → find a file → terminal → inspect a
diagnostic → view workers → change a setting → inspect where it came
from → Lua REPL → redefine a command. That sequence communicates the
whole thesis: already useful, discoverable, visible computation,
explainable settings, programmable internals. It should be an ordinary,
restartable help workspace, not a one-time modal wizard.

### Ground truth

**Grade: partial — the cheap floor's first two items landed with Journey
Stage 1b-3.**

An unconfigured launch now greets in `*scratch*` naming `M-x` and four
real bindings, and `M-x help` renders a cheat sheet through the existing
`*help*` mechanism. The greeting happens in a launch-finalization seam
(`prepare_startup` → `EditorState::finalize_local_launch`) that runs
after config, after attach dispatch resolves to local, and after desktop
restore — no constructor is the right hook, because `EditorState::open`
calls `new` before resolving its target and the daemon constructs one
too.

Still missing: no tutorial, no first-run detection, and **`C-h` still
deletes a word — deliberately.** It is bound to
`buffer.delete-word-backward` because non-kitty terminals cannot
disambiguate Ctrl+Backspace from Ctrl+H (both produce byte 0x08,
`builtin/keymaps/default.lua:78-86`), so rebinding it to a help prefix
would break Ctrl+Backspace on every legacy terminal. The help-prefix
question is a real trade for §20 Priority 4's discovery arc to weigh
across the whole command family, not an oversight.

Note the dependency: five of the ten onboarding steps above currently
lead somewhere broken or invisible (find a file — the mechanism is fixed
since #162/#165 but is advertised nowhere except the keymap; inspect a
diagnostic — silent-failure risk; view workers — undiscoverable;
setting provenance — unanswerable). Onboarding is correctly sequenced
*after* the P1/P4 fixes, but the cheap floor — a welcome buffer in
`*scratch*` naming `M-x`, the keybinding cheat sheet as a help buffer,
and a help prefix decision — had no prerequisites at all. **The first
two are done** (Stage 1b-3); the third is deferred with its reason
recorded above.

---

## 19. Product Coherence Acceptance Tests

Pmacs should add acceptance tests that exercise product behavior across
subsystems, complementing (not replacing) subsystem tests:

- **Installation/first launch** — no config, open a directory, usable
  workspace, actionable guidance for missing tools.
- **Command discovery** — search by title and synonym; display
  keybinding, provenance, availability; invoke from palette and menu
  through the same object.
- **Workspace lifecycle** — multi-root open, servers, terminal, build,
  close, restore, ownership cleanup.
- **Worker ownership** — start completion/search/build, inspect,
  cancel a parent, confirm child cancellation and UI recovery.
- **Package lifecycle** — install in-session, inspect contributions,
  disable, confirm disappearance, reload, uninstall cleanly.
- **Remote execution** — attach to remote daemon, edit optimistically,
  remote terminal and server, disconnect/reconnect, coherent state.

### Ground truth

**Grade: started — the first suite exists; five of the six scenarios
above do not.**

At audit time zero cross-subsystem journey tests existed. **Journey
Stage 1a created `tests/journey_acceptance.rs`**, the §2 journey itself,
seeded with steps 2 (launch unconfigured), 3 (open a real project), and
5 (edit immediately), and declared a ratchet: stages add rows, none
removes them. That is the "first launch" scenario, partially — missing
tools still have no actionable guidance to assert.

The rest is unchanged. Every other acceptance suite in the tree pins one
subsystem's contract (superbly — bite-verified, falsified-by-revert,
vacuity-checked). Command discovery, workspace lifecycle, worker
ownership, package lifecycle, and remote execution have no
cross-subsystem suite; several remain *untestable* because the behavior
doesn't exist (install in-session, disable). Steps 6–12 join
`journey_acceptance.rs` as later stages make them real — that is how
"the journey is a release gate" stops being aspirational.

(Related lesson already in the handoff: `compile_mode_acceptance`
accidentally reads the real user config — an *unintentional*
whole-product test that keeps catching real coherence bugs. That is
evidence this class of test has teeth.)

---

## 20. Recommended Priority Order

Each priority is annotated with its audited state and whether the gap is
**wiring** (surface over existing machinery — cheap) or **model** (a
missing runtime entity — a real arc).

### Priority 1: Protect the golden product journey

Establish the end-to-end workflow; treat regressions as release
blockers. **State: runs to step 5; thin from step 6 (§2). Mostly wiring,
and unusually cheap:** directory-argument handling (**done**: Journey
Stage 1a); a find-file surface (**done**: #162 open-by-path, #165
browsing); surfacing the LSP spawn failure with guidance (**done**: Journey Stage
1b-2, #204, §1.2); a compile keybinding + `cargo build`/`test` default
(**done**: Journey Stage 1b-1, #203, from the existing
`ProjectKind::Rust` — **not** `Cargo`, see §24); a terminal keybinding
(**done**: `C-c t`, #173); a welcome buffer (**in flight**: Journey
Stage 1b-3). The journey acceptance
suite (§19) is the ratchet that keeps it fixed — it **exists now**
(`tests/journey_acceptance.rs`, Stage 1a), seeded with steps 2, 3 and 5,
and carrying step 9 since #203.

Journey Stage 1b is the named remainder, and it splits: **1b-1** (the
compile binding + project-kind defaults) landed as #203 and **1b-2**
(LSP spawn guidance, step 6) as #204; **1b-3**, the welcome buffer
(step 4), is in flight and completes the split.

### Priority 2: Make workspace and location explicit

Otherwise project, LSP, remote, task, and persistence accumulate
incompatible ownership models — the audit confirms four have already
diverged (§7). **State: missing; first slice in flight (multi-root LSP
affinity). Model gap:** the Workspace entity (§7), then Location values
(§8). This is the long-lead arc; start it before the fifth and sixth
subsystems grow their own root conventions.

### Priority 3: Strengthen extension ownership and isolation

**State: missing; prerequisite-shaped. Model gap, with one bug-sized
prerequisite: `pmacs.hook.remove` does not exist (§13).** The work
unit: registrations carry their owning package (the
`CurrentlyLoadingPackage` signal already exists), removal APIs complete
the set, error attribution becomes default rather than opt-in. This
single arc unblocks §13's disable/uninstall/inspect, §10's class 10.2,
and package-scoped task cancellation in §9.

### Priority 4: Unify discovery

**State: substrate without surface. Almost pure wiring — the best
payoff-per-effort in this document (§5):** a dozen interactive commands
over existing introspection, richer M-x rows (the wire pattern already
exists), title/category on `Command`, predicate evaluation, help-layer
unification, a help prefix key. Most of P1's "understand the interface"
and §18's floor ride on this.

### Priority 5: Finish the workbench convergence

**State: partial and moving (§14) — the bottom panel is now complete on
both frontends (Stage 1 #155 through Stage 2B-3), and listview is
proven.** Only the adopter default flip (Stage 3) remains on the panel
itself. Remaining elsewhere: the tree primitive (build it before dired
and the worker tree invent two), table/inspector/diff, help unification.
Wiring plus one modest model piece (the tree model).

### Priority 6: Productize configuration

**State: foundation only (§11). Model-lite:** value provenance in the
registry, then layering (profile/workspace scopes — depends on P2 for
workspace, §12 for profiles), then adoption migration (table-valued
settings are the hard prerequisite), then persistence.

### Priority 7: Build package lifecycle UX

**State: not started; correctly sequenced after P3.** In-session
install (init-gate rework), disable/uninstall over P3's ownership,
`*packages*` view over P5's primitives, then bundles and registry
sequencing per §13.

### Priority 8: Ship binaries and release channels

**State: zero (§17). Independent of everything — can start anytime.**
The editor becomes testable by users who are not repository
contributors; every other priority's value is invisible until this one
exists.

### How this maps to arcs

Candidate arc cuts, honoring one-feature-one-branch-one-PR and the
framing workflow (each needs its own scout + framing before any
implementation — this list is direction, not commitment):

1. **Journey Stage 1** (P1): split at the new-Rust-primitive line.
   **Stage 1a — landed**: directory open, the `EditorState::open` →
   `resolve_target_buffer` unification, the destination-scope substrate,
   and the first journey acceptance suite. It routes `pmacs .` into
   #165's dired buffer rather than growing a second directory surface.
   **Stage 1b-1 — landed (#203)**: the compile binding and project-kind
   defaults, with the prompt capturing its directory rather than
   re-resolving it at accept time. **Stage 1b-2 — landed (#204)**:
   LSP-failure surfacing. **Stage 1b-3 — in flight**: the welcome
   buffer and `M-x help`. With it the 1b split is complete.
2. **Discovery surface** (P4): the describe/list/where-is command
   family, M-x rich rows, help unification, help prefix.
3. **Transient keymap layer** (§6): the overlay scope + lifetime
   handle + derived `dispatch_idle`, then migrate shadows one per PR.
4. **Extension ownership** (P3): `hook.remove`, owner-carrying
   registrations, attribution-by-default.
5. **Worker identity** (§9): owner/purpose/parent on jobs and
   processes, join the four planes, statusline activity indicator.
6. **Workspace entity** (P2): the object, then location values.
7. **Config provenance + adoption** (P6).
8. **Package lifecycle** (P7, after 4).
9. **Distribution** (P8, anytime).

A standing process change accompanies all of them (§1.3): **every new
framing doc must state its coherence impact** — which journey steps it
touches, whether it adds an interaction island, whether its options
enter the config registry, whether its background work is attributed —
so the debt stops compounding silently.

---

## 21. What Pmacs Should Borrow

Proven adoption-cost reducers from successful modern editors, with
audited status: immediate usefulness (△ — editing yes, journey no);
strong defaults (✓ where they exist, §3); progressive disclosure (✗
inverted, §4); searchable commands (△ names-only, §5); integrated
language tooling (✓ data layer / △ surface); project awareness (△
conventions, §7); visible contextual actions (△ §15); coherent
task/terminal integration (✓ mechanics / ✗ visibility, §9); package
discoverability (✗, §13); configuration layering (△ foundation, §11);
remote development as core workflow (△ works, unmodeled, §8); smooth
distribution and updates (✗, §17); consistent interface primitives (△
best trajectory, §14); explicit missing-tool guidance (✗ except
`--gpu`, §1.2).

---

## 22. What Pmacs Should Preserve and Deepen

Pmacs should not trade away the qualities that justify its existence —
and the audit confirms these are today's genuine strengths: live
programmability (redefine/unregister at runtime, per-package envs);
implementation inspectability (SourceLocation on every registration,
mandatory descriptions); replaceable interaction models (aspirational —
§6 is the gap); multiple genuine frontends and semantic rendering (✓,
§16 — the strongest concern); explicit parallel work with
cancellability and observability (mechanics ✓, product visibility ✗,
§9); remote daemon architecture (✓); user control over the editor as a
running system (✓).

The goal is not to make pmacs less powerful so that it becomes
approachable. The goal is to make power **progressively available**.

---

## 23. Product Thesis

Emacs offers: *the editor is a programmable environment, and the user
may transform it completely.* VS Code offers: *the editor is already a
coherent development workstation, and extensions fill in the remaining
gaps.* Pmacs should offer:

> **The editor is already an excellent workstation, and every part of
> that workstation remains inspectable, programmable, concurrent, and
> replaceable.**

Its strongest distinctive proposition is not "Emacs in Rust" or "Emacs
with threads":

> **Pmacs is a live-programmable editor in which computation,
> interfaces, ownership, and execution locations are explicit — allowing
> local, remote, interactive, and background work to coexist without
> freezing or becoming opaque.**

The audit's one-line verdict on the thesis: **"without freezing" is
delivered; "without becoming opaque" is not yet true** — for the
failures a new user meets first (§1.2), for background work (§9), for
settings (§11), and for what a key will do while a modal surface is
active (§6). Product coherence is what will make the architecture
perceptible. Without it, pmacs risks becoming an impressive collection
of subsystems. With it, pmacs becomes a workstation whose complexity is
available without being imposed.

---

## 24. Known documentation drift (as of 2026-07-25)

Found during the audit; fix opportunistically, ideally before this
document is wired into CLAUDE.md/AGENTS.md as required reading:

- **§1.2's frequency note was wrong**, not merely stale: it recorded the
  missing-server failure as firing once per project root when the real
  rate was once per file open, because a failed spawn leaves no record
  for the affinity loop to find. Corrected in place by Journey Stage
  1b-2, along with three stale line citations in the same paragraph.
- **This document named a `ProjectKind` variant that does not exist**,
  in two places: §2's step-9 row and §20 Priority 1 both said
  "`ProjectKind::Cargo` existing (`src/project.rs:77`)". Line 77 is the
  *doc comment*; the variant on line 78 is **`ProjectKind::Rust`**,
  produced by the `Cargo.toml` marker. The audit read the comment and
  named the comment. Corrected by Journey Stage 1b-1 — which also
  establishes that the Lua side never sees the variant at all:
  `pmacs.project.detect` returns the **tag string** `"rust"`. Kept here
  rather than silently fixed, because a wrong type name in the document
  work is evaluated against costs a scout a real detour.

- `docs/keybindings.md` — every `src/editor.rs` line citation in §3 is
  stale by ~250–1000 lines despite a "last verified @ `f8096ff`
  (2026-07-20)" stamp; its shadow list also omits the terminal `C-c`
  escape (reports 5 shadows, actual 6).
- `builtin/api/packages.lua` (EmmyLua annotations) — missing
  `install_local`, `reload`, `load`, `describe`, `on_unload`; claims
  `update` is unimplemented (it is implemented).
- `CHANGELOG.md` (~line 300) — claims a `describe-key` command for
  self-introspection; no such command ever shipped (the Lua API
  `pmacs.describe.key` exists; the interactive command does not).
- `docs/config-registry-framing.md` (~658) — claims `describe-setting`
  renders through `src/help.rs`; it hand-builds its own text in
  `builtin/commands/default.lua`.
- `src/workers_buffer.rs` module doc — says the completions ring caps
  at 32; `COMPLETED_RING_CAP` is 64.
- `src/command.rs` doc comment on `predicate` — describes palette
  gray-out behavior (T M2.7) that never shipped.

---

## 25. Update protocol for this document

- **When a PR changes any audited claim here, updating this file rides
  that PR** — flip the grade, rewrite the fact, note the PR number.
  Same discipline as `docs/agent-handoff.md`.
- Line numbers are hints; symbols are authoritative. When touching a
  section anyway, re-verify its citations; do not let this document
  accumulate the drift §24 catalogs in others.
- Grades change only with evidence (a landed PR, a re-audit), never
  aspirationally.
- The **Ground truth** subsections are a snapshot dated 2026-07-25. If
  a future comprehensive re-audit is performed, update the date in the
  header and prune superseded facts rather than appending — this is a
  briefing, not a log.
- Framing docs for coherence-affecting work should cite the section
  they serve (e.g. "COHERENCE §6") and state their coherence impact per
  §20's standing process change.
