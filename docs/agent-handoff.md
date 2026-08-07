# Agent handoff — cross-machine continuity

**Last updated: 2026-08-06.** `main` is **`db1bbe9`** — the tree
primitive **#217**, atop **#216**, which completed the macOS CI
signal-integrity arc by retiring R2 and R4 with discriminating
witnesses, atop **#215**, which built the registry. **That arc is
retired**; its live residue (R1, R3, and the newer R5 and R6) is
re-homed to the async-runtime, reap-ledger, and readiness-helper-audit
lanes in `docs/active-work.md`.

The live CI-triage rule in §5 points at `docs/ci-red-signatures.md`,
which keys on **signature, not test name** — and, since 2026-08-06,
on **signature plus date**: a red matching a retired row is a
recurrence only if it *postdates* the retirement, and an earlier one
corroborates instead. The hazards list this file used to carry is
retired, and its two unevidenced entries are audit notes there.
Previously **2026-08-04, as bottom-panel Stage 3 #213 — the adopter
default flip, which COMPLETES ARC 7: omitting `display` now means the
panel, and the workbench's panel half is done on both frontends. Beneath
it the post-release accuracy pass #212 and Distribution Stage 1 #211 —
released as v1.1.0, the first pmacs release with prebuilt binaries,
which makes journey step 1 reachable without cloning the repository —
and the CI CRDT coverage lane #209, the
first time CI has ever compiled and run the `crdt` half of the test
corpus, closing a gap that left 279 tests (including a REQUIRED
`CLAUDE.md` gate) unexecuted for the project's whole life. Development
moved to the laptop in the same session; the docs absorption #208 sits
beneath it. Previously 2026-07-29, as bottom-panel Stage 2B-3 — the GPU panel
band, compatible protocol-v21 activation, and the negotiated
`panel_capable` flip, completing Arc 7 Stage 2 — opens atop `e003b81`.
Beneath it, bottom-panel Stage 2B-2 (#187) — the
daemon panel projection and the epoch machine — lands on `6c9e765`,
which is the dired Stage 2 framing (#171) atop the resource-op delete
guard framing (#186). Those two are framing-only: both are approved
documents with no runtime code, and neither has begun implementation.
Beneath them, the docs-only coherence listview
correction (#189) and landed-state refresh (#185); the canonical
landed base beneath those is `7586905`. The runtime anchor beneath them is the M4
config-sink race fix (#174) and bottom-panel Stage 2B-1 (#184). #174 is
test-only. #184 is the substantive one — the reserved protocol-v21
bottom-panel wire family, dark by construction, with the production
handshake deliberately still advertising v20 — following the Journey/GPU
directory-target ratchet (#183), following Journey Stage 1a (#182),
which made directory
startup one coherent local/daemon/GPU path and incorporated the terminal
configuration + copy mode landed-doc work (#180); following terminal
copy mode (#178) — `C-c C-t`
materializes a terminal's whole retained range into an ordinary buffer,
plus `Buffer::set_generated_contents`, the first genuinely immutable
generated-buffer write path — and its landed-doc pair (#168); following
Lean 4 Stage 4a (#179) — the typed-edit
consumer chain — and bottom-panel Stage 2A (#177), the classified census
routing that makes every Projection-class consumer ask
`primary_document_window`; the bottom-panel Stage 2 framing
(#175), terminal configuration Stage 1 (#173) — profiles, scrollback, a
per-terminal configurable escape key, and the `C-c t` opening binding —
Lean 4 stages 3a and 3b (#167, #170), pmacs' first Lean language server;
the GPU terminal input fix (#166), the double terminal-layout sync that
made a GPU terminal untypable; the CRDT undo repro (#157), the
inline-math landed-doc refresh (#172), the inline-math slice (#158), the
first mathematical typesetting in pmacs; dired Stage 1 (#165), Lean 4
Stage 2 (#161), the dired framing pair (#163/#164), find-file (#162) —
the dired arc's Stage 0 — COHERENCE.md (#163), Lean 4 Stage 1 (#160), the
minimap blank-slab fix (#159), bottom-panel Stage 1 (#155), the
inline-math re-scout (#154), the vterm PTY-flake fix (#153), and the
GPU initial-target doc refresh (#152); and before that GPU
initial-target (#148, protocol v20),
following folding Stage 2 (#149) and its landed-doc refresh (#150),
web grammars HTML+CSS (#146), the LaTeX Stage 1 / inline-math framing pair
(#144/#145), folding Stage 1 (#142), one-command GPU invocation (#141), the
documentation refresh (#140), Vterm Stage 3 (#135), tab-width rendering
parity (#137), locals-query processing (#134), modeline detection (#132),
mode system wiring (#129), config registry (#127), Vterm Stages 1–2
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

## 1. Where the project stands (2026-08-06)

- **`main` @ `db1bbe9`.** The **tree primitive #217** — `listview` rows
  take optional `depth`/`id`, collapse is primitive-owned, folding is
  **local projection state and not a refresh protocol**, and the LSP
  outline is the sole adopter (`COHERENCE.md` §14 ◐; §20 says adoption,
  not construction). Atop `2657568` **#216**, which completed the macOS
  CI signal-integrity arc, atop `12f2970` **#215**, which built
  `docs/ci-red-signatures.md`. Beneath those, `f186253`: bottom-panel
  Stage 3 **#213** completes Arc 7,
  atop the post-release accuracy pass #212 and `000b6cd` / **v1.1.0**.
  Beneath that, Distribution Stage 1 #211
  lands atop the docs absorption #210, the CI CRDT coverage lane #209,
  the absorption #208 and `cfc1710`. Beneath that, nine PRs landed in
  this order: the ledger absorption #199, the process-signal diagnostic
  #200, the test ambient-root isolation **framing** #201, the
  reap-ledger diagnostic #202, Journey Stage **1b-1** #203, **1b-2**
  #204 and **1b-3** #205, the ambient-root isolation **implementation**
  #206, and discovery Stage 1 #207. Each has its own bullet below; this
  line is the head-of-`main` anchor and nothing else.
- **Bottom panel Arc 7 COMPLETE — Stage 3 (#213), the adopter default
  flip.**
  Omitting `display` now means the panel for listview, compile and
  terminal; **dired keeps `"current"`** because
  `pmacs.path.directory_handler` calls it with no `display` and a flipped
  default would open `pmacs .` in a bottom panel. Four hand-written
  copies of the validator collapsed into one
  `resolve_adopter_display(operation, raw, default)` — the default is a
  **parameter**, which is what makes dired's exemption visible at its
  call site instead of hidden in a divergent copy. Durable facts:
  - **A visit FROM a panel must never use the raw switch.**
    `pmacs.window.switch_buffer` replaces the buffer in the ACTIVE
    window, so from a panel it clobbers the panel itself. The outline's
    `on_visit` still did this; the references panel had been migrated to
    `display_file` when the arc landed and the outline was missed,
    because nothing exercised it from a panel until the default flipped.
    **Q#BP11c is the contract**: after RET, `M-,` must FOCUS the
    still-present panel, not clone its buffer into the document — and an
    assertion on the active buffer name alone cannot tell those apart.
  - **An opt-out that does not survive replay is not an opt-out.**
    `compile._last` stored `{cmdline, cwd}` only, so `g` re-resolved
    `display` and silently reverted an explicit `"current"` to the new
    default. Anything that replays a stored invocation must store the
    escape hatch with it.
  - **Compile's chords are PANEL-LOCAL, deliberately.** All are bound
    `scope = "buffer"`, so with `select = false` none dispatch from the
    document — `C-c C-k` included. `M-x compile.kill` still works
    anywhere via its `or compile_slot()` fallback. A global chord is a
    command-surface decision, framed separately.
  - **Two `q` mechanisms coexist by design**: presentation history
    chains in the side slot (`C → B → A → delete`, Q#BP2c), while
    `p.prev` prevents raw-switch and capability-fallback listview loops.
    Neither supersedes the other.
  - **A capability fallback must strip the QUIT ACTION too**, not just
    the side parameters — a quit action stranded on a document window
    makes a later `q` try to restore a presentation that never happened.
- **The tree primitive ships — `listview` gained depth, collapse and
  identity** (P5, §14's last missing workbench primitive; implemented,
  PR held). Rows carry **optional** `depth` and `id`; absent, a row
  behaves exactly as before, which is what leaves the flat consumers
  untouched. The LSP outline is the one adopter. Durable facts:
  - **Folding is local projection state, not a refresh protocol.**
    Collapse only *hides* rows and never changes a surviving row's
    depth, and consumers emit parents before children, so descendants
    are a **contiguous run**. The primitive therefore re-renders from
    its own array **without calling the consumer** — which is the only
    reason the anchor consumer works, because **the outline has no
    `on_refresh` at all**. A design requiring the consumer to re-supply
    rows on every fold would have fitted no existing consumer.
  - **Identity is consumer-supplied and the primitive never derives
    one**, but it is **not opaque**: `row.id` must be a **string or
    number, unique among rows, and not NaN**, enforced by `check_ids`
    where rows enter (`open` and `refresh`). Review narrowed this from
    "opaque, compared by equality", which was **two contracts wearing
    one name** — selection compares with `==`, honouring `__eq`, while
    collapse state stores ids as **table keys**, and Lua indexes tables
    by raw identity consulting no metamethod. A table id satisfied one
    half and silently failed the other. Uniqueness and not-NaN came
    from the same pass: every lookup resolves to the **first** match,
    so a duplicate makes selecting the later row toggle the earlier,
    and `0/0` is a `number` Lua refuses as a key. The outline uses
    `line:col`, because the `::` parent chain collides on overloads and
    same-named siblings — exactly where a stale expansion would
    reattach to the wrong node. **Selection is re-seated by id, not by
    line**, since a fold inserts or removes rows above the cursor.
  - **`item` is optional in the API and was mandatory in practice.**
    `line_to_item` is sparse when a row omits it, and `seat_cursor`
    took `#` of that map — a display-only tree stranded the cursor on
    the header. It counts visible rows explicitly now. Every existing
    test supplied `item`, so none could reach it.
  - **`has_children` must read the FULL row array, not the rendered
    subset.** A collapsed node's children are absent from the rendered
    map by construction, so asking the view would answer "no" for every
    collapsed node and make expanding impossible — a self-sealing bug
    that looks like fold working and unfold silently not.
  - **A bite that passes validates the pair, not the test.** The first
    byte-identity injection used `row.depth or 0`; flat rows have no
    depth, so it changed nothing and the test "passed" against a
    regression the flat path is immune to. Ask which defect you
    injected before believing a green bite.
- **pmacs is installable without cloning — Distribution Stage 1, #211,
  released as v1.1.0.** A `v*` tag builds `pmacs` and `pmacs-gpu` on
  pinned `ubuntu-22.04` / `macos-15` and publishes a GitHub Release with
  `SHA256SUMS`. **Journey step 1 works for the first time**, and
  `COHERENCE.md` §17 moves missing → Partial. Scope was deliberately
  binaries-only; channels, update, rollback and signing are stated
  non-goals. Five durable facts:
  - **A release build can produce FIVE binaries and three must never
    ship.** Cargo auto-discovers `src/bin/*.rs`, so `pmacs-audit`,
    `pmacs_fake_lsp` and `pmacs_fake_mcp` appear alongside the two real
    ones. Exclusion is **two-layered** — explicit `--bin` targets *and*
    an explicit staged asset list — and layer 2 is not belt-and-braces:
    building the branch, `target/release` still held all three from an
    earlier `cargo test --release`, and `Swatinem/rust-cache` restores
    exactly that in CI. **Archiving `target/release` would have published
    a fake language server.**
  - **`env!("CARGO_PKG_VERSION")` expands in the crate being COMPILED.**
    `InstanceIdentity::for_running_process` lived in `pmacs-protocol` and
    read it there, so the daemon reported the *protocol* crate's version
    to every frontend under a field named `pmacs_version`. Three tests
    asserted the right thing and **could not fail** while both crates
    read 1.0.0 — diverging them is what made the tests discriminating.
    *A test can be correct and still prove nothing when the two things it
    compares are equal for an unrelated reason.*
  - **Pin release runners, never `-latest`.** `ubuntu-latest` (glibc
    2.39) silently produces binaries that fail to load on Ubuntu 22.04
    and Debian 12; `macos-latest` drifts the minimum supported macOS with
    no commit to point at. And **a pinned runner proves nothing about the
    artifact** — the glibc floor is asserted by reading versioned symbols
    out of the binary (`objdump -T | grep GLIBC_`), so a bad runner
    change fails in CI instead of shipping.
  - **A tag pushed before its workflow is on the default branch does
    nothing, silently.** `on: push: tags` resolves the workflow file at
    the tagged commit, and GitHub registers workflows from the default
    branch. No run, no error — indistinguishable from "not started yet",
    which is the worst possible shape for a release step. Confirm a run
    actually appeared.
  - **Verify a release from the DOWNLOADED artifact, and use a negative
    control.** Checking CRDT presence by counting `loro` strings means
    nothing without building a non-CRDT binary and getting zero.
- **CI runs the CRDT half of the corpus for the first time — #209.**
  `.github/workflows/ci.yml` had never enabled the `crdt` feature, so
  every `#[cfg(feature = "crdt")]` test was **not compiled** — not
  skipped, not filtered, not reported. **279 tests had never executed in
  CI, 186 of them in the library**, behind the
  `cargo test --lib --features crdt` invocation `CLAUDE.md` lists as a
  **required** pre-PR gate: a required gate CI had never run. `Test
  (crdt)` now runs 3,766 tests, `M10 Perf Gates (crdt)` covers two
  suites no workflow had named, and `clippy --features crdt` is
  enforced. **275 of 279 recovered**; the other four are excluded with
  stated reasons. Five durable facts, each of which contradicted
  something previously recorded:
  - **`gpu-render` runs a DIFFERENT PACKAGE** (`cargo test -p
    pmacs-gpu`) from the root-package GPU suites. The long-recorded fix
    shape — "move the GPU-requiring crdt suites onto `gpu-render`, it
    already has lavapipe" — does not work as written. One job covers the
    whole corpus instead, which also cannot develop the hole splitting
    invites: a suite added later would otherwise land in the job needing
    no GPU and skip there forever.
  - **`PMACS_REQUIRE_GPU` is not uniform.** Present in
    `vterm_stage3_acceptance` (×2) and `bottom_panel_stage2b_gpu_
    acceptance` (×1), **absent entirely** from `gpu_invocation_
    acceptance` and `gpu_initial_target_acceptance`. It cannot serve as
    blanket proof the GPU suites ran.
  - **`m10_10_perf` is a CI-default regression tripwire, not a bench.**
    Its header says its bounds are "generous … to catch catastrophic
    regressions, not to verify a tight perf claim", so the *absence* of
    `#[ignore]` is the design. Giving it a perf job would have shipped a
    coverage reduction inside a coverage lane. Classifying the three
    `_perf` suites by filename and marker counts got the one whose name
    disagreed with its purpose exactly backwards.
  - **`--keep-going` is what turns a clippy run into an inventory.**
    Clippy abandons remaining targets at the first failure, so any
    un-`--keep-going` list is a lower bound. The recorded seven-item
    list was stale in *both* directions: one finding had been fixed
    incidentally, one was new, and every line number had moved.
  - **A feature can matter to a crate a per-test census scores as
    unaffected.** `pmacs-protocol`'s `crdt` feature gates no tests — 17
    either way — but changes `cfg!(feature = "crdt")` *expressions*
    inside the capability defaults, so the same tests exercise different
    runtime values. Worse, the crate's only use of those defaults was a
    **transport round-trip, which is invariant to the values**: an
    all-false default passes identically. Running code is not testing
    it, and that gap survived the first review round.
- **Two arcs completed in that run, and their lanes are gone from
  `docs/active-work.md` accordingly** (rule 4 removes a lane once its
  arc is done *and* its facts are here): **Journey Stage 1** — 1a plus
  the whole 1b split — and **test ambient-root isolation**. Discovery
  and reap-ledger merged a stage each and keep rewritten lanes.
### 1a. Outstanding work — the whole board, 2026-08-01

**Read this before picking anything up.** It is the only place the
remaining work is enumerated in one view; the per-arc bullets below give
the detail. Nothing here is in flight: **zero PRs are open** at this
anchor, so every item is startable.

#### Arc state, against `COHERENCE.md` §20's priority order

| P | Arc | State | What is next |
|---|---|---|---|
| 1 | **Journey** | **Stage 1 COMPLETE** (1a #182/#183; 1b-1 #203, 1b-2 #204, 1b-3 #205) | Journey runs to step 10. The thin end is now steps **1** (install — that is P8), **11** (background work: visible but no ownership model, §9) and **12** (session restore: desktop-save is opt-in *and* a documented no-op under a daemon) |
| 2 | Workspace + location | Missing; model gap | The long-lead arc. Start before a fifth subsystem grows its own root convention — four have already diverged (§7) |
| 3 | Extension ownership | Missing; prerequisite-shaped | **`pmacs.hook.remove` does not exist.** That one bug-sized gap blocks §13's disable/uninstall, §10's trust classes, and package-scoped cancellation |
| 4 | **Discovery** | **Stage 1 MERGED (#207)** | Stage 2 candidates, in rough dependency order: richer M-x rows (**protocol change** — `MinibufferPrompt.candidates` is `Vec<String>`; `CompletionPopupRow` already proves the pattern), `Command` gaining title/category/aliases/flags/arg-schema (~147 definition sites), predicate evaluation, help-layer unification, and the help-prefix decision |
| 5 | Workbench convergence | Partial; **Arc 7 COMPLETE** (#213) and **the tree primitive is implemented** (PR held) | The bottom panel is finished on both frontends and the adopter default is flipped. The tree primitive has landed on a held PR: §14's Tree moves ✗ → ◐ with the LSP outline as its one adopter. **Next: adoption** — dired's `i`, then DAP's variables view, which is why it was built first |
| 6 | Config productization | Foundation only | Value provenance, then layering, then adoption migration (**table-valued settings are the hard prerequisite** — `ConfigValue` is four scalars) |
| 7 | Package lifecycle | Not started | Correctly sequenced after P3 |
| 8 | **Distribution** | **Stage 1 SHIPPED (v1.1.0, #211)** | Binaries on tag, checksums, machine-checked glibc floor. **Journey step 1 now works and the "invisible until this exists" blocker is lifted.** Next is a *decision* about channels / update / signing, not a queued plan |

#### Open lanes (branch exists, work not finished)

- **CRDT half of the corpus is dark in CI — FIXED, merged as #209.**
  Now a lane kept only for three follow-ons (macOS leg, the `--lib
  --features crdt` flake, the `crdt_replica` serde divergence). See the
  arc bullet in §1.
  **Re-measure before quoting a number**, and there is now a tool:
  `scripts/feature-census luajit luajit,crdt`. Measured 279 dark at
  `4223dd3`; 275 recovered, 4 excluded with stated reasons. Two
  corrections it forced are worth carrying: **`m10_10_perf` is a
  CI-default regression tripwire, not a bench** (its bounds are
  deliberately generous, so `#[ignore]`ing it to give it a perf job
  would *reduce* coverage), and **`gpu-render` runs a different
  package** (`pmacs-gpu`) from the root-package GPU suites, so the
  long-recorded "move them onto gpu-render" fix-shape does not work as
  written.
- **Generated-buffer immutability** — Stage 1 merged (#191); Stage 2
  not started. Four writer mechanisms have still not adopted
  `set_generated_contents`; key the inventory by *writer*, not buffer.
- **Bottom panel** — Stage 2 complete; Stage 3 is the adopter default
  flip.
- **Folding** — Stages 1–2 merged; Stage 3 (GPU) next, with
  mirror-clear-on-snapshot a named obligation.
- **Reap-ledger** — diagnostic merged (#202); every *disposition* change
  is still parked (below).
- **Kill-ring browser + persistence** — parked by choice.

#### Deferred work, by the framing that parked it

Each was a deliberate decision with a stated reason; none is a to-do
someone forgot.

- **From LSP guidance (#204):** build `pmacs.error` (**fifteen guarded
  call sites reporting through a channel that does not exist**);
  promote the spawn-failure record into Rust's status model so `*lsp*`
  shows failures natively; surface `LspEventKind::Crashed` — a server
  that *started then died* is unsurfaced and is a different message.
- **From the welcome (#205):** the help prefix — **`C-h` is not free**;
  it deletes a word because non-kitty terminals cannot disambiguate
  Ctrl+Backspace from Ctrl+H (both byte 0x08), so rebinding breaks
  Ctrl+Backspace on every legacy terminal. Also: make `show_help_text`
  adopt the generated-buffer write invariant; onboarding proper (§18's
  ten-step sequence).
- **From discovery (#207):** everything in the P4 row above, plus
  settings **value provenance** (§11) and **closed-set acceptance
  semantics** — completion today is *assistance*, not validation:
  `resolve_accepted_value` returns the literal typed text when no
  candidate is selected, so a typo still reaches the handler.
- **From the reap ledger (#202):** Q#RL1's strict-`ESRCH` probe and
  Q#RL2's retry policy — **neither can move alone**, because
  `shutdown()`'s loop exits when the ledger empties, so a strict probe
  without a loop change converts a silent early exit into a guaranteed
  2 s stall at every editor exit. Also parked: retargeting to the
  measured pgid, `signal_target`'s read-then-kill of `tcgetpgrp` (the
  "most likely real fix site"), and **why** a group-directed `kill`
  returns `EPERM` against a live owned child — still unknown.
- **From ambient isolation (#206):** production still resolves ambient
  roots by default (only *construction* gained a parameter); nothing
  audits what previously wrote into `~/.local/share/pmacs`.

#### Side quests and standing hazards

- **`pmacs.error` is undefined in production.** Fifteen `if pmacs.error
  then` guards make the silence look deliberate. Report through
  `pmacs.editor.set_status` until the channel is built.
- **Judging a red CI run: `docs/ci-red-signatures.md` is the authority.**
  It keys on **signature**, not test name — a failure in a listed test
  that lacks that row's fragments is a NEW incident, not a known one.
  The old rule here ("rerun before concluding") is retired: **a green
  rerun establishes intermittence only**, never environmental cause or
  harmlessness; the same signature again is a second occurrence and
  stays blocking. One row is an **unresolved possible product defect**
  that no rerun can clear.

  This list previously named three tests. The audit found one of them
  had produced **two distinct signatures** with different causes, and
  that two of the four incidents actually seen were absent from it —
  which is why name-keyed lists are not trustworthy.
- **`basedpyright` hangs forever** — always
  `cargo test --test m4_acceptance -- --skip basedpyright`.
- **The crdt sweep needs `cargo build --workspace` first**, or twelve
  `gpu_invocation_acceptance` tests fail on a missing `pmacs-gpu`
  binary. `cargo build --workspace --no-default-features --features
  luajit,crdt` is the invocation that produces both binaries.
- **A shared `CARGO_TARGET_DIR` makes concurrent sweeps unattributable.**
  Every worktree on this machine resolves to the same target directory,
  so `target/debug/pmacs` is a **shared mutable file**: a
  `cargo test --workspace` at default features in one worktree
  overwrites the binary a running `crdt` sweep is spawning, and every
  real-daemon suite then starts the wrong one. **Established once, on
  2026-08-05, in the Stage 2 hardening sweep** — seven failures across
  three real-daemon suites against a clean baseline, the failure text
  naming its own cause (*"start the daemon built with the `crdt`
  feature"*), `pgrep` confirming the rival build, and a re-run from the
  same tree with a **dedicated** `CARGO_TARGET_DIR` giving 41/41. The
  reciprocal case was seen from the other side the same day.
  **Give a second worktree its own target dir**, check for live
  worktrees before believing a sweep failure, and treat any red from a
  sweep that overlapped another build as unattributable rather than as
  evidence. *(This mechanism does NOT retroactively explain the tree
  lane's unclassified occurrence — that one's signatures were destroyed
  before being read, so it has no captured text to match against this
  one's, and it keeps two non-causal hypotheses. A mechanism
  established in one occurrence is not evidence about a different
  occurrence that was never characterized.)*
- **A local sweep leaks daemons, and they accumulate across days.**
  `gpu_invocation_acceptance`'s one-command tests leave ~3 orphaned
  `pmacs --daemon` processes per sweep, reparented to systemd with
  deleted sockets; 42 were resident at one point, the oldest four days
  old. They are a rival explanation for any load-sensitive local
  failure, so **check `pgrep -f "pmacs --daemon"` before trusting a
  local red**. Lane recorded in `docs/active-work.md`.
- **A `PROTOCOL_VERSION` bump's blast radius is every version-sensitive
  test, and NONE of them appear in the diff.** "The touched acceptance
  suites" is the standing gate, and for a protocol bump it is the wrong
  selector: long-lines Stage 3 bumped v21→v22, ran the suites it had
  edited, and broke **eight** version assertions across six suites. CI
  showed exactly **one**, because **cargo stops at the first failing
  target**; the rest surfaced only afterwards, and one at a time would
  have cost four more red rounds.

  **On any protocol bump run `cargo test --tests --no-fail-fast` in
  BOTH feature configurations.** `--no-fail-fast` because of the
  stop-at-first-target behavior above, and `--features crdt` because
  three of the eight were in crdt-gated real-daemon tests that assert
  on a live socket's negotiated version — invisible to a default sweep,
  which is the blindness the bullet below already names.

  **Sort the failures before fixing them.** A *tripwire*
  (`assert_eq!(PROTOCOL_VERSION, N)`) is meant to fire and takes a
  deliberate edit; three of the eight were these, and the one pin that
  must NEVER be edited — `ADVERTISED_PROTOCOL_VERSION == 20` — did not
  fire. The other five were defects sharing one shape: **an absolute
  contract expressed as arithmetic on, or equality with, a moving
  constant.** `PROTOCOL_VERSION - 1` for "below the panel version";
  `PANEL_MIN_VERSION == PROTOCOL_VERSION` for a coincidence true only
  while panels were newest; `PROTOCOL_VERSION == 21` standing in for
  the panel stage's own version; `"21"` for a negotiated session
  version. Anchor on the constant the contract *names* — `src/` already
  spelled it `PANEL_MIN_VERSION - 1` in five places, and every outlier
  was in `tests/`.
- **A local sweep is blind to whichever feature configuration it does
  not build.** Stage 3's census and every verification sweep ran
  `--features luajit` WITHOUT `crdt`, so no crdt-gated suite was
  exercised and `compile_mode_crdt_acceptance` reached CI broken. Sweep
  BOTH configurations before claiming a corpus is green — the crdt job
  exists precisely because that blindness is easy.
- **"My change made this fragile" is a different finding from "this was
  always flaky", and only one of them is yours to fix.** Stage 3 saw two
  CI runs on one commit fail DIFFERENT PTY/GPU suites — the
  load-sensitivity signature, on suites the flake list already names.
  The tell that it was neither: the failures kept landing on GPU
  *terminal* tests, and terminal placement was what the PR changed.
  Those fixtures opened with no `display`, so the flip shrank the very
  window whose rendered frames they assert against. **Ask which tests
  and why those, before reaching for a rerun.**
- **Never hand-roll the dark-test census — use
  `scripts/feature-census`.** libtest prints `name: test` with **no
  space before the colon**, so a filter written `/ : test$/` matches
  nothing and reports a clean zero; a target with zero tests prints its
  `Running` line and nothing else, so counting only test lines drops it
  from the diff; and both configurations need an `--ignored` pass, or
  pre-existing ignores get attributed to the feature. Each of those was
  hit while writing the script, and the second one survived two
  revisions of a framing doc.
- **A green a37 means nothing on its own** — the vterm real-daemon
  acceptance returns `ok` without running unless `pmacs-gpu` is built.

- **Previous anchor, retained for provenance:** `4cd4a7b` (the
  generated-buffer immutability framing #188, the resource-op
  delete-guard implementation #190, silent-skip arming #192/#193/#194,
  CI timeouts and concurrency #195, the process teardown stdin-deadlock
  fix #197, dired Stage 2a #196, generated-buffer immutability Stage 1
  #191, and bottom-panel Stage 2B-3 #198).
- **Previous anchor, retained for provenance:** `6c9e765` (the dired
  Stage 2 framing #171 and the resource-op
  delete guard framing #186 — both framing-only, no runtime code, no
  implementation started — atop
  the docs-only coherence listview correction #189,
  atop the docs-only landed-state refresh #185, the M4 config-sink race
  fix #174 — test-only — atop bottom-panel Stage 2B-1 #184, the
  Journey/GPU directory-target ratchet #183, Journey Stage 1a #182,
  incorporating
  terminal configuration + copy
  mode landed docs #180, Lean 4 Stage 4b #181, the dired Stage 1 landed
  docs #169 and the PTY-terminate diagnostic #176, terminal copy mode
  #178, the GPU-terminal-input landed docs #168, Lean 4 Stage 4a
  #179, bottom-panel Stage 2A
  #177, the bottom-panel Stage 2 framing #175, terminal configuration
  Stage 1 #173, Lean 4 Stage 3b #170, Stage 3a #167, the CRDT undo repro
  #157, the inline-math landed-doc refresh #172, the bottom-panel
  landed-doc refresh #156, the inline-math slice #158, dired Stage 1
  #165, the GPU terminal input fix #166, Lean 4 Stage 2 #161, the dired
  framing #164, COHERENCE.md #163, find-file #162, Lean 4 Stage 1 #160,
  minimap blank-slab #159, bottom-panel Stage 1 #155).
  **Protocol schema support is `v6..=v21`, the production server-first
  `Hello` advertises v20, and a current session nevertheless negotiates
  v21.** All three are true at once, and Stage 2B-3 is what made them
  compatible: the advertised version is a permanent **baseline** and the
  session's real version is settled one message later by the frontend's
  `AttachRequest` counter-offer. The bullets below describe the arcs in
  their own terms; this line is the head-of-`main` anchor.
- **`COHERENCE.md` is now required reading and a required framing input
  — #163.** It carries the product-coherence thesis, an audited
  scorecard, per-concern gaps, and §20's priority order, and it is the
  standard new work is evaluated against. Per `CLAUDE.md`, **every new
  framing doc must state its coherence impact** — journey steps touched,
  interaction islands added, config-registry adoption, background-work
  attribution. Its §2 grades the golden journey; **Journey Stage 1a
  moved that grade off "broken at step 3"** — see the arc bullet below.
- **Journey arc (P1) — Stage 1b-1 LANDED (#203)**
  (`docs/journey-stage1b1-compile-defaults-framing.md`).
  Journey step 9 moves **Partial → Works**: `C-c c` runs `compile.run`,
  and the first prompt is prefilled from the detected project kind via
  `pmacs.compile.defaults` (seeded `rust = "cargo build"`, extensible
  from `init.lua`). Lua, tests and docs; no Rust change, no protocol
  change. The ratchet now carries steps 2, 3, 5 and **9**.
  - **Sharing a resolver is not capturing one.** `pmacs.minibuffer.read`
    is asynchronous and nothing freezes the active window while a prompt
    is open, so having the prompt and `compile.run` call the *same* cwd
    resolver still let them disagree: the user could be offered
    `cargo build` for A and given a run in B by clicking away
    mid-prompt. The interactive command captures `pmacs.compile.context()`
    once and passes its `cwd` through to the run. **This is Stage 1a's
    `commit_to` lesson on a smaller seam** — capture at request time,
    never re-derive when the async work lands. Review found it; the
    first framing had the weaker design and said it was sufficient.
  - **A pin that never crosses the accept boundary cannot see that
    class of bug.** Every originally-proposed pin compared values the
    prompt and the resolver already agreed on, so the defect above
    passed all of them. Two pins now accept the prompt and observe a
    real process.
  - **`ProjectKind::Cargo` does not exist** — `COHERENCE.md` named it
    twice, citing `src/project.rs:77`, which is the *doc comment*; the
    variant on line 78 is `ProjectKind::Rust`. Lua never sees the
    variant anyway: `pmacs.project.detect` returns the tag string
    `"rust"`. Corrected in both COHERENCE sites and recorded in its §24.
  - **The compile fallback cwd is the process cwd, which under `cargo
    test` is the pmacs repo — itself a Cargo project.** Any pin
    asserting the *absence* of a Cargo suggestion that reaches the
    fallback reports pmacs's own `Cargo.toml`. Bite D confirmed it: with
    the kind derived from the process cwd, the plain-fixture prefill pin
    still passed and only the nested-project pin caught it.
    `set_search_boundary` is no defence — it clamps only a walk starting
    *below* the boundary.
  - **Named limitation, deliberately unfixed:** after `pmacs <dir>` the
    active buffer is dired's and **pathless** (`pmacs.buffer.create`
    assigns no path; dired compensates through its own module-local
    `handle_for_buffer`), so the cwd falls through to the process cwd.
    Launched from elsewhere that is the wrong directory. The fix is
    §8's execution-location model, not a reach into dired's private
    table. What is guaranteed is that the failure stays *coherent*: the
    suggestion always describes the directory the run will use.
- **Journey arc (P1) — Stage 1a LANDED**
  (`docs/journey-stage1a-framing.md`). `pmacs .` opens a directory
  instead of exiting 1, on **one** path: `resolve_target_buffer` gained a
  `ResolvedTarget::Directory` arm *ahead* of the load, `EditorState::open`
  became a caller of it rather than a parallel implementation, and the
  daemon/GPU bootstrap shares the same arm. Which surface handles a
  directory is the `path.open-directory` chain with dired as a
  replaceable fallback slot. `tests/journey_acceptance.rs` is the new
  cross-subsystem ratchet (steps 2, 3, 5 seeded; **stages add rows, none
  removes them**). No protocol change.
  - **A hook a builtin subscribes to can never be first-claimant-wins
    for users.** `HookRegistry::add` only appends and builtins load
    before `init.lua`, so a dired subscription would always claim before
    any user listener. That is why dired is a *slot*
    (`pmacs.path.directory_handler`) and not a subscriber — and why
    clearing the slot has to leave startup succeeding with a status,
    not exiting 1.
  - **A raise and a `false` are indistinguishable in `proceed`.**
    `run_short_circuit` returns `proceed = false` for both; only
    `HookOutcome.errors` separates them, and it decides whether to
    *report*, not whether to fall back. Getting this backwards produces a
    fallback that runs after a user's resolver crashed mid-handling.
  - **The listing is async; the bootstrap is synchronous.** The whole
    post-await commit therefore runs against a destination captured at
    request time (`pmacs.window.commit_to`), which preflights every
    precondition *before* invoking the callback — dired mutates handle
    state, `prev`, and paint long before it reaches anything that could
    refuse, so validating at display time is four mutations too late.
    Awaiting inside a commit is refused: a yield would restore the scope
    while the coroutine is still parked.
  - **The scope swaps `core.active_frontend`, not just an override** —
    `pmacs.window.buffer()`'s no-arg arm reads the ambient active buffer
    directly, so dired's `prev` capture would otherwise follow whatever
    frontend happened to be dispatching. The override *also* exists, and
    is load-bearing in exactly one case: a commit reached from inside an
    interactive command, where the origin would otherwise outrank the
    ambient value. Bite-testing found N4 green without it.
  - **`replace_active_buffer` does not drop the startup scratch buffer**,
    despite its doc comment having claimed so for as long as it has
    existed. Its body is one `switch_active_buffer` call. The comment is
    corrected here; changing the lifetime is separate work.
  - Stage 1b is the named remainder: compile binding + Cargo defaults,
    LSP spawn guidance, welcome buffer.
- **Terminal configuration + copy mode arc — COMPLETE**
  (`docs/terminal-config-and-copy-mode-framing.md` rev 4; Stage 1 #173,
  Stage 2 #178; no protocol change in either, still v20). Stage 1 ships
  profiles, scrollback, a per-terminal configurable escape key and the
  `C-c t` opener; Stage 2 ships copy mode — `M-x terminal.copy-mode` /
  `C-c C-t`.
  - **The snapshot MATERIALIZES into an ordinary buffer.** That is the
    arc's organizing decision: isearch, motion, selection and the kill
    ring work with no new substrate, and "keys must not reach the child"
    dissolves structurally, because the transport arm keys on
    `is_terminal(buffer_id)` and a snapshot is not a terminal. **The
    dispatch-shadow count therefore stays at six.**
  - **`prune` reacts to buffer removal rather than causing it** — it
    filters on `!registry.contains(buffer_id)`, so a child exiting does
    **not** remove the terminal buffer. That is what makes `on_removed` a
    sound teardown hook, and why a finished command's output stays
    readable.
  - **Ownership means "in our own handle table", never found-by-name**
    (dired's F7 rule, re-learned here): snapshot writes use
    `bypass_intercept`, so adopting a same-named foreign buffer clobbers
    user data. Snapshot identity is keyed by **comparing buffer handles
    in an array** — `BufferIdLua` implements `__eq` but each wrapper is a
    distinct table key, so comparison works and hashing does not.
  - **Profiles are a raw Lua table**, joining `pmacs.lsp.config` and
    `pmacs.pair.sets`, because `ConfigValue` is four scalars with no
    table kind. The two open-time settings resolve through the **global**
    chain (they are read before the identity buffer exists); only
    `terminal.escape-key` resolves per buffer, and its cache lives on
    **`TerminalSession`** so its lifetime is the terminal's —
    `value_epoch` alone is not a sufficient key, because it does not
    advance when focus moves between terminals holding different
    buffer-local values.
  - **Criterion 17 is deliberately unpinned, and its bite is now stated
    correctly.** A real semantic frontend proving neither copy is mutated
    needs the actual GPU binary (the optimistic apply exists only in
    `pmacs-gpu/src/main.rs`; the headless `SemanticClient` has no
    optimistic path), i.e. the `a37` footing §5 warns about. After
    `set_generated_contents` the eventual test must look for
    **unauthorized mirror mutation plus daemon refusal — divergence**,
    not the "mutates both sides silently" the criterion originally
    specified, which can no longer happen and would pass for the wrong
    reason. *A fix can invalidate a test that was never written.*
  - Test instruments worth reusing: **`cat -v` is the echo probe**,
    because the screen rejects C0 controls before they reach cells so a
    raw echoed `Ctrl-X` is invisible; and such probes must **count
    occurrences rather than test presence**, because a single-character
    probe collides with the child's own banner text.
- **PTY terminate diagnostic LANDED — #176** (merge `bf8878f`,
  2026-07-26, one review round; `docs/process-signal-tolerance-framing.md`
  rev 4, after three framing rounds). **Diagnostic only — no disposition
  changed.** Every call that failed before still fails, with no state
  transition and no reap-ledger arming; `src/process.rs` is the only
  source file touched. It exists because the `terminate` EPERM flake is
  real and nothing yet knows *why*.
  - **Why three tolerance rules were all rejected: each concluded
    something about a process from something that was not about that
    process.** Rev 1 reasoned from an errno alone (EPERM means the caller
    lacks permission, not that the id was recycled); rev 2 from
    `try_wait`, which observes the spawned **leader** while a PTY signal
    targets `-tcgetpgrp(...)` — entities that diverge exactly when job
    control has moved the terminal; rev 3 from group-directed **ESRCH**,
    which proves only that the selected foreground group vanished. This
    is the reusable shape, not a Unix trivium.
  - **Two facts that killed the original argument.** `group = true` is
    *rejected* for PTY mode at spawn, so the reap ledger never applies to
    the PTY path at all; and the ledger's own comment saying EPERM
    "cannot happen for our own children" drops the entry for **bounded
    growth**, not as a ruling that EPERM means dead. A comment stating a
    belief is not the same as code enforcing it.
  - What ships: a failing `kill` now reports five separate facts —
    target source, target kind/value, spawn-time group, errno, and the
    leader's real `try_wait` state. The test seam injects the **kill
    result only**, never the observation, so the real
    `ChildHandle::try_wait` runs against the real child.
  - **It is not "strictly additive".** `try_wait` reaps and caches, so an
    exited child may be reaped earlier than before. That is safe only
    because `portable-pty` 0.9.0 returns a `std::process::Child` on Unix
    and delegates `try_wait` to it, so `poll_one` still sees the cached
    status — pinned by an exactly-one-terminal-event test rather than
    assumed.
  - **Still open, and still the most likely real fix site:**
    `signal_target`'s read-then-kill of `tcgetpgrp`. All tolerance rules
    remain parked pending the evidence this diagnostic produces, as does
    `terminate` idempotence for an already-reaped process (a different
    failure, so a different PR).
  - **The diagnostic fired, and what it showed.** macOS CI, PR #191,
    [run 30553376486](https://github.com/levineuwirth/pmacs/actions/runs/30553376486/job/90907461258):
    `target=-8619 via group, leader_pid=8619, expected_group=-8619,
    leader=live` — the `spec.group` pipe path, not the PTY path, with the
    leader observed alive by a real `try_wait`. A rerun of the identical
    head passed 12/12, so it is intermittent.
  - **Owning the child does not license dismissing a group error, and
    the occurrence does not prove the child received EPERM.** The failed
    target was the *group* `-8619`; `try_wait` observed the *process*
    `8619`. Nothing measured `getpgid(8619)`, so the two are not known to
    refer to the same thing. What is settled is narrower and still
    enough: a group target computed from the spawn-time `pgid == pid`
    assumption returned EPERM while the leader was alive, which retires
    "EPERM cannot happen for our own children" as a reason to discard an
    arbitrary group-directed error. Attributing the errno to the child
    would repeat the exact error that killed three tolerance rules —
    concluding something about one entity from something about another.
  - **A field named like an observation can be a restatement of its
    input.** `expected_group` is `-leader_pid`, and on the spawn-group
    path the target is `-leader_pid` too, so the report printed the same
    number three times and their agreement was arithmetic. Stage B adds a
    `measured_group` from `getpgid`, which is the only field able to
    disagree — and it still establishes no identity, because it is read
    inside the same read-then-act window and no portable mechanism closes
    that for a *group* (`pidfd` covers a process; macOS has neither).
- **Discovery arc (P4) — Stage 1 LANDED (#207)**
  (`docs/discovery-stage1-command-family-framing.md`, rev 6, three
  review rounds). Eleven `help.*` commands over the existing registries,
  indexed by `M-x help`. **§5 moves substrate-without-surface →
  Partial.** No Rust: the data was all reachable from Lua, and the
  settings completion source is a Lua function through
  `CompletionSource::Custom` — correcting a `default.lua` comment that
  claimed `source` was a fixed Rust-side vocabulary.
  - **Completion is assistance, not validation.**
    `resolve_accepted_value` returns the literal typed text whenever no
    candidate is selected, so a typo still reaches `on_accept`; and a
    fuzzy near-miss would silently select a *different* value. Refusing
    a non-candidate is Rust work, deferred.
  - **`apropos` is substring, not fuzzy.** `fuzzy_score` is
    subsequence-based and descriptions are long sentences, so fuzzy
    matches nearly everything. Pinned by a fixture whose description
    contains the needle only as a non-contiguous subsequence.
  - **One owner for `*help*` writes is what the seam buys — not a
    one-site migration.** `src/help.rs` has renderers for
    command/key/buffer/mode/hook/view and **none** for settings, lists
    or apropos, and `_show_help` takes already-flattened text. Rendering
    is therefore a named per-subject function, so the future Rust work
    is enumerated per subject (three new renderers) rather than
    discovered per call site.
  - **The seam-counting pin caught a real bypass**: the two renamed
    commands were still calling the file-local `show_help_text`, so the
    funnel was fiction for exactly the two that predate it.
  - **`Command.predicate` is still never evaluated** — read only at
    `src/help.rs:76` and one test. A preservation pin registers a
    *raising* predicate and asserts the command still runs, so a stage
    that starts evaluating must change that pin knowingly.
- **Journey arc (P1) — Stage 1b-3 LANDED (#205)**
  (`docs/journey-stage1b3-welcome-framing.md`, rev 4, three review
  rounds). The last of the 1b split. An unconfigured launch greets in
  `*scratch*`; `M-x help` renders a cheat sheet. **§18 and the scorecard
  move Missing → Partial**; §2's step-4 row stays Partial.
  - **No constructor is the startup hook.** `EditorState::open` calls
    `new` *before* resolving its target, the daemon constructs one too,
    `init.lua` runs inside `new`, and desktop restore happens later
    still. `run()`'s terminal-free prefix is now `prepare_startup`, and
    the greeting is its last step.
  - **Extraction is what makes wiring testable.** With the seam called
    by hand from tests, deleting the production call left every
    assertion green while shipping no welcome — the "guard with no
    production caller" shape. `prepare_startup` is `pub` because the
    journey suite is a separate crate and the rest of the sequence
    (`run`, `new`, `open`, `install_state_dirs`,
    `restore_desktop_if_armed`) is already public.
  - **`C-h` is not free**, and §2's step-4 row used to imply it was: it
    deletes a word because non-kitty terminals cannot disambiguate
    Ctrl+Backspace from Ctrl+H (both byte 0x08). Rebinding it to a help
    prefix breaks Ctrl+Backspace on every legacy terminal. Deferred to
    the discovery arc **with the reason recorded**.
  - **Deliberately NOT `set_generated_contents`** — it would lift
    read-only, discard history and mark the buffer generated, all wrong
    for a buffer step 5 requires the user to type into immediately. The
    one place not adopting that invariant is correct.
  - **`pmacs.command.invoke` is not the M-x path.** M-x is
    `editor.execute-command`, a minibuffer with the `commands`
    completion source calling `invoke_interactive` on accept — and
    because a selected candidate shadows typed text while `accept()`
    does `session.take()`, the pin asserts
    `pmacs.minibuffer.selected() == "help"` **before** RET.
- **Journey arc (P1) — Stage 1b-2 LANDED (#204)**
  (`docs/journey-stage1b2-lsp-guidance-framing.md`, rev 4, three review
  rounds). `COHERENCE.md` §1.2's canonical silence: a preconfigured
  language server that is not installed now reports with guidance, marks
  the modeline `LSP:!`, and appears in `M-x lsp.status`. §2's step-6 row
  **stays Partial** for a reason that landing did not touch: a server
  that starts and then *crashes* is still unsurfaced.
  - **`status_buffer_text()` had existed since M4.8, exposed to Lua and
    tested, with no production caller and no `*lsp*` buffer** — several
    `src/lsp.rs` and `src/project.rs` doc comments referred to that
    buffer as though it existed. Half the stage was wiring dark matter.
  - **The reporting shape was already adopted twice in `lsp.lua`** (root
    resolvers, notification subscribers). The canonical case was silent
    because nobody had converted it, not for want of a mechanism.
  - **A failed spawn leaves NO record**: `LspManager::spawn` returns
    early before both `status_tracker.ensure` and `clients.insert`, so
    `pmacs.lsp.list()` cannot see it and the affinity loop re-spawns.
    The failure therefore recurs **once per file open**, not once per
    project root as COHERENCE recorded. Hence: **memoize the report, not
    the failure** — the spawn is still retried, so installing the binary
    mid-session recovers with nothing to invalidate.
  - **The affinity key is `(language, key_uri)` and `key_uri` is nil for
    markerless files**, which deliberately share one server per
    language. Lua cannot index by nil, so one encoding function serves
    both tables with a `u`/`n` discriminator no URI can collide with.
  - **Three tables, three lifetimes.** `reported` (never cleared,
    includes the command so repointing at another missing executable
    re-reports); `failures` (cleared on a successful spawn for that
    key); and a **buffer-keyed projection** for the modeline, because
    that provider runs for every window on every paint and deriving an
    affinity key inside it would invoke root resolvers during painting.
  - **A success must SWEEP the projections, not clear one.** Clearing
    only the succeeding buffer leaves an earlier buffer rendering
    `LSP:!` while `lsp.status` reports nothing wrong.
  - **A new per-buffer table needs its own teardown.** Nothing existing
    reaches it: the LSP resource reconciliation iterates `attachments`,
    and a failed buffer has none by construction. `pmacs.buffer.on_removed`
    is registered once per projection; rename and delete **clear** rather
    than re-key, because after a rename the failure is no longer known to
    apply at the new location.
- **Reap-ledger silent failures — DIAGNOSTIC, in flight**
  (`docs/reap-ledger-silent-failures-framing.md`). The lane #200's
  framing §5 parked and its evidence unparked. **Four `kill(2)` results
  are discarded in the group reap ledger**, and each discard has its own
  consequence — three in the persistent ledger, one in the in-drain
  twin:
  - `tick_reap_ledger`'s probe cannot tell `ESRCH` (the group is gone —
    correct) from any other errno (we could not ask — not correct), and
    `retain` deletes the entry either way, cancelling escalation.
  - The deadline escalation sets `killed = true` whether or not the
    `SIGKILL` landed, so **no later tick retries it** — that arm is
    guarded by `!entry.killed` and never fires again for the group.
  - `shutdown()`'s force-kill discards its result the same way, on the
    path written specifically to stop a leak at editor exit. **It is not
    the same failure**, and the difference is the retry: `shutdown()`
    iterates the ledger with *no* `!entry.killed` guard, so it does
    re-kill an entry the escalation arm marked. A failed escalation
    leaks the group until editor exit, where one more attempt is made; a
    failed force-kill leaks it *past* editor exit, with nothing left to
    try. Saying the escalation is "never retried by anything" collapses
    the two.
  - `final_drain_runtime`'s twin collapses every errno into "dead",
    which quiesces the drain and **cancels the readers** — truncated
    output rather than a leaked process, and terminal for that drain
    where a later tick could revisit the ledger.
  - **None of the four has been observed to fire.** #200's evidence is an
    explicit `SIGTERM` failing in `signal()`, not any ledger call. What
    it retires is the *reason* ("EPERM cannot happen for our own
    children"), not the behaviour.
  - **`shutdown()`'s loop exit depends on the silent drop, and this is
    now measured.** It runs while `any_running() || !reap_ledger.is_empty()`,
    so an early exit needs the ledger empty **and** no live managed
    record — the leader-exited-survivor case the ledger exists to serve.
    With a failed force-kill followed by an errored probe it exits in
    under 500ms instead of holding its 2s bound, having concluded
    cleanup finished because the probe failed. Making the probe strict
    without touching the loop converts that silent early exit into a
    guaranteed 2s stall at every editor exit that hits it: **the two
    cannot be changed independently.**
  - **There is no channel for a background tick to report on.**
    `ProcessEvent` is keyed by `ProcessId`, while the ledger is keyed by
    pgid and is deliberately independent of managed records, so in the
    case that matters there is no id to attribute to. Every production
    consumer polls `take_events(id)` per known id; `take_all_events`
    would sidestep the keying but **has no production consumer at all**
    (two test call sites only). `pmacs.error` is dead. Reporting is
    therefore its own lane, as the framing's Bet 4 anticipated.
  - **A test seam for a background loop has to be directed.**
    `shutdown()` signals every managed process before it reaches its
    ledger force-kill, so one undirected "next kill fails" slot is eaten
    by the wrong call and the test passes while proving nothing. The
    persistent sites take a FIFO each (the coupling pin needs two
    outcomes pending at once); the in-drain site needs one outcome that
    **repeats for a whole drain**, because a one-shot is consumed by the
    next 1ms probe and can never survive the 50ms window `quiesced`
    requires.
  - **An absence assertion needs a fixture that could have produced the
    thing.** The in-drain pin's first fixture had no `trap '' TERM`, so
    `poll_one`'s leader-exit group TERM killed the descendant before it
    wrote its late marker: the marker was absent on *both* paths and the
    pin would have stayed green with the collapse fixed. The bite caught
    it — the reverted seam failed only the consumed-plan check, not the
    content assertion. That is what the consumed-plan check is for.
- **Lean 4 arc (Arc 8) — stages 1, 2, 3a, 3b, 4a, 4b ALL LANDED**
  (`docs/lean4-mode-framing.md`; #160, #161, #167, #170, #179, #181). pmacs edits Lean 4: `arborium-lean` highlighting, a
  `lean4` major mode, `⟨⟩ ⦃⦄ ⟮⟯` pairs, and a `lake serve` language
  server with a Lake-aware outermost root, a lazy toolchain probe, a
  one-shot `lean --server` fallback, and `waitForDiagnostics`. **No
  protocol change in any stage** (still v20).
  - **Two of the four stages contained no Lean at all**, and that is the
    arc's organizing rule: *no PR mixes a cross-cutting substrate change
    with Lean feature content.* Stage 2 made LSP server affinity
    per-project-root (`ensure_server` had been reusing one server across
    roots — a correctness bug for every language, not just Lean). Stage
    3a added notification/response subscription seams to
    `handle_server_requests`, the single shared LSP event drain, plus
    `pmacs.fs.canonicalize`.
  - **Two consecutive re-scouts found that rule broken by the stage
    being scouted** — Stage 3 in round 4, Stage 4 in round 5, each time
    by a risk column that contradicted its own prose. The rule is not
    self-enforcing. Re-check every remaining stage's risk column at
    scout time.
  - **A configured LSP root must be a canonical absolute path.** It
    reaches `file_uri_for` verbatim and that URI is the affinity key, so
    one package opened by two spellings spawns two servers. Stage 3a's
    `pmacs.fs.canonicalize` is the primitive; it returns nil rather than
    a lossy path for non-UTF-8 input.
  - **`LspManager::stop` on an already-terminal client strands it in
    `ShuttingDown` forever** — `server_is_live` then counts it live so
    nothing rebuilds against it, and `forget` refuses it for not being
    terminal. *Stopping a dead server is what makes it un-replaceable.*
    Stage 3b works around it by dispatching on state (`forget` when
    terminal, `stop` when live); merely skipping the call leaves
    `next_restart_at` armed. The real fix is unframed substrate work.
  - **`elan` shims lie**: `lake --version` and `lean --version` can both
    fail ("no default toolchain configured") on a machine where Lean
    otherwise works, so `command -v lake` is worthless as a capability
    check. Lean acceptance is fake-server; live smokes must be PATH-
    **and** success-gated.
  - Stage 3b took six review rounds, and **the same defect appeared four
    times**: "the fallback silently doesn't happen," as no re-attach,
    then re-attach cleared by an unrelated buffer, then satisfied by the
    very server being replaced, then repairing one buffer while the rest
    stayed stale. Each fix was locally right; none asked what a *global*
    config swap invalidates. The durable lesson is to heal at
    **consumption** — the point where a stale record is handed out — not
    at the moment of the swap.
  - **Stage 4a (the typed-edit consumer chain) MERGED as #179**
    (branch `lean4-stage4a-typed-edit-chain`, framing rev 8; it is part
    of the main anchor above). It is substrate only:
    `builtin/runtime/typed_edit.lua` owns the
    single `buffer.after-edit` subscriber and the single one-shot read,
    `pair.lua` becomes its first registered consumer, and
    `tests/auto_pair_acceptance.rs` is unchanged by zero lines
    (criterion 46, verified at the diff). No protocol change, no Lean
    content. The three decisions that turned out load-bearing rather
    than stylistic: consumers are called **even when the record is
    nil** (three existing auto-pair tests assert the non-event through
    it, and 4b abandons stale pending state on it); each consumer gets
    its **own copy** of the record, because pairing reads `rec.char`
    and a declining consumer could otherwise forge it; and the fan-out
    iterates a **snapshot**, because a consumer that registers a
    lower-priority one shifts itself forward under `ipairs` and runs
    twice.
  - **Round 8's durable lesson: `run_all_must_succeed` does NOT abort
    the fan-out.** `src/hook.rs:332` collects each callback's error and
    continues to the remaining subscribers, marking only the run
    failed — so an uncontained throw inside a hook subscriber does not
    stop `lsp.lua` from flushing didChange. Two framing revisions
    asserted the opposite to justify a `pcall`. The guard was right and
    the reason was wrong, and by the time review caught it the wrong
    reason had been copied into a module comment, an acceptance
    criterion, a test comment, and the ledger. **Correct the source a
    rationale derives from, not only the sites that quote it.**
  - **Stage 4b (the Unicode input method) MERGED as #181**
    (framing rev 9): a vendored
    1,855-entry table generated from `leanprover/vscode-lean4@17d1d08`
    by `scripts/regen-lean-abbrev`, plus a consumer registered on the
    Stage 4a chain at priority 50, ahead of pairing. **A consumer
    cannot both edit and let a later consumer act on the same
    keystroke**: the chain hands each consumer a copy of the record made
    before any consumer ran, so an edit invalidates every copy still to
    be used. The expansion therefore runs on a SECOND
    `buffer.after-edit` subscriber after the chain — which is how a
    pair character that terminates an abbreviation still pairs
    (`\alp(` → `α()`). And **deferring work past a fan-out means
    owning which fan-out it belongs to**: these fan-outs NEST, so a
    consumer between the expander and pairing that calls
    `pmacs.hook.run` re-enters the deferred subscriber while the outer
    chain is still mid-list, and the count that recognises this has to
    come from a MINIMUM-PRIORITY consumer — the expander is optional
    (a claim can stop the chain first) and a subscriber beside the
    deferred one is too late (the nested fan-out finishes inside the
    outer chain's subscriber). Its other durable facts:
    the table must stay an ORDERED SEQUENCE (equal-length ties resolve
    by source declaration order, which a `pairs`-iterated map cannot
    express); a generator round-trip check must re-read the BYTES ON
    DISK, because comparing in-memory strings cannot see an encoding
    applied by the write itself; and an expansion that SHRINKS the
    buffer must place the point explicitly, or every later self-insert
    is silently rejected and the editor looks dead.
  - **Round 9 corrected three approved acceptance criteria** by
    simulating the state machine over all 1,855 entries rather than
    re-reading the prose. Four review rounds over the text had not
    found them, because each named an example that reads as obviously
    right and is wrong only against the data.
  - Remaining: stages 5 (goal panel), 6 (`#eval` output channel), and 7
    (module hierarchy) are framed but not scouted against current
    `main`.

- **Inline math LANDED — #158** (`docs/inline-math-slice-framing.md` rev 3;
  merge `5aa9044`). pmacs renders `$…$` as typeset mathematics in the GPU
  frontend. **No protocol change (still v20); the whole slice lives in
  `pmacs-gpu`**, because `pmacs-gpu` depends only on `pmacs-protocol` and
  never on `pmacs` — a core-crate parser would have been unreachable from
  where rendering happens.
  - `math_parse.rs` → `math_layout.rs` → a `ChunkSource::MathBox` spacer
    chunk → per-glyph mini-buffers drawn at the shaped line's real
    baseline, with fraction rules as quads. Font is bundled Latin Modern
    Math (~717 KiB) under the **GUST Font License** — not OFL.
  - **The v0 subset is narrow and deliberately so**: Greek (34 entries),
    sub/superscript, and `\frac`. Everything else — including relations
    like `\geq`, fences, big operators, and all display math (`$$…$$`,
    `\[…\]`) — is a named deferral, and an unsupported command degrades
    the **whole span** back to source rather than rendering partially.
    In a real paper most inline spans still show source; that is the
    designed behaviour, not a defect.
  - **Math is suppressed while the caret is inside its span**, so editing
    always sees source. That gate reads the effective caret plus
    selection endpoints and is fed by three separate refresh triggers;
    it is the most delicate part of the slice.
  - Selection and search washes cover the whole box rectangle, not
    sub-ranges (sub-range washes are deferred).
  - TUI shows the LaTeX source unchanged. That divergence is recorded
    against `COHERENCE.md` §16, which audits the "no privileged
    frontend" rule.
- **find-file LANDED — #162** (`docs/dired-framing.md` §10, Q#DR11; merge
  `2af1ab3`; one review round). `C-x C-f` is the dired arc's **Stage 0**:
  pmacs previously had no discoverable way to open a file by path — no
  such command existed and `pmacs.buffer.find_or_open` had no interactive
  caller. Pure Lua in `builtin/commands/default.lua`, one keymap line, an
  8-test dispatch-driven acceptance suite; no Rust, no protocol change.
  Two substrate facts it documents, both worth knowing before touching
  any minibuffer prompt:
  - **Completion over files is flat and cannot be made hierarchical from
    Lua.** A custom `source` function is called with **zero arguments**
    (`minibuffer.rs:591`) and runs synchronously outside any coroutine,
    where `Handle:await()` raises — so it can neither see the input to
    re-root on nor list a directory. Only the Rust
    `CompletionSource::Files { root }` can list, and it is
    single-directory and 1024-capped.
  - **A selected candidate SHADOWS typed text.** `recompute_candidates`
    sets `selected = Some(0)` whenever the list is non-empty
    (`minibuffer.rs:372-377`) and `resolve_accepted_value` returns the
    candidate over the typed contents (`:564-574`). So free-text accept
    fires only when the input filters every candidate away — for
    basename candidates under a subsequence filter, when it contains a
    `/`. This applies to `M-x` and `switch-buffer` too. Consequences are
    pinned as decisions, including the hole where a new bare name that is
    a subsequence of an existing entry opens the existing file, and the
    empty-input case (`fuzzy_score` gives `Some(0)` for an empty needle
    and ties break lexicographically, so dotfiles lead).
  - Also: `get_or_load_buffer` computes a normalized path but **loads
    from the raw one** (`editor_core.rs:842-856`), so a `~/…` path dedups
    against an open buffer yet fails to load one that is not open —
    find-file expands the tilde Lua-side. Loading through the normalized
    path is a named deferral.
- **dired Stage 1 — the directory view — LANDED — #165**
  (`docs/dired-framing.md` §0, S1-1…S1-12; merge `c8ec8f3`; one review
  round). pmacs now has a directory surface: `C-x d` / `C-x C-j` open a
  read-only listing, one buffer per directory named
  `*dired:<canonical path>*`, with a `dired` major mode whose
  mode-scoped keymap carries `RET`/`f`, `^`, `n`/`p`, `g`, `q`, `s`.
  Protocol unchanged at **v20**. **Stage 2 (marks and operations) and
  Stage 3 (wdired) each still need their own framing**; the frozen
  fixture shrinks after Stage 3.
  - **The Rust is confined to two things**: a per-entry-tolerant
    `read_dir` (`ReadDirTolerance {Fatal, PerEntry}` →
    `FsDirListing {entries, errors}`), because `read_dir_blocking` fails
    a whole listing on any of five per-entry conditions and the tolerant
    wrapper its own module doc delegates to package authors **cannot be
    written in Lua** (one error value, no partial vec); and
    `editor_core::normalize_buffer_path` becoming `pub`, exposed as
    `pmacs.path.canonicalize`. Only non-UTF-8 **names** stay fatal —
    byte-preserving paths would be needed. The Lua result **shape** keys
    on `errors.is_some()`, so the bare array the frozen M8.2 fixture
    consumes with `ipairs` is untouched.
  - **Exposing a core normalizer beat mirroring it in Lua.** A Lua mirror
    would have been a second canonical form — the same class of bug as
    the five tab-width constants (#137). Applies to any future Lua-side
    path reckoning.
  - **A fixed-width column must be fixed-width for every input.** The
    exported `pmacs.dired._layout` (MARK 0, KIND 2, PERMS 3–12, SIZE 13,
    MTIME 24, NAME 41) is the contract Stage 3 reads offsets from, and
    `%10d` overflows at ≥10 GB, silently shifting every column right of
    it. Sizes now fall back to a width-clamped magnitude (K/M/G/T/P/E).
  - **An ambient action must be gated on the buffer it assumes.** A
    revert's cursor re-seat settles a tick or more later, by which time
    the user may have switched buffers; the paint names its buffer and is
    safe, but seating is ambient. This is the buffer-level instance of
    the rule below that interactive origin does not survive an await.
  - **A failure IS an answer — don't probe first.** Kinds are lstat-based
    in both `read_dir` and `stat`, so nothing in an entry says whether a
    symlink points at a directory. `RET` tries to list it and treats the
    failure as the answer; an explicit probe was a second full
    `read_dir`, so a descent listed twice.
  - **Unbounded per-entry error collection needs a cap when nothing
    cancels the work.** A dired listing carries no supersede key, so
    cancellation was never the backstop the tolerant loop implicitly
    relied on (`READDIR_MAX_CONSECUTIVE_ENTRY_ERRORS = 1024`).
  - **This is the first builtin with mode-scoped keys** (#129's first
    non-detection consumer), which broke the pre-existing
    `describe_key_identifies_every_default_binding`: it asserted every
    binding resolves through `describe.key` context-free, which held only
    while the modes table was empty. It now sets the effective context
    per binding and explicitly **clears** the mode for global ones,
    because a leaked mode legitimately shadows a global chord of the same
    name (dired's `RET` shadows `edit.newline-and-indent`), plus a floor
    assertion that at least one mode-scoped binding exists.
  - **A dedicated panel does not carry its dedication across a descent**
    — the framing expected it to. `display_buffer` never replaces the
    buffer in a slot dedicated to another one; it discards every
    side-specific parameter and falls back to the document window (Q#BP3
    2.iii), and the exact-window arm errors. Dired does not unpin the
    user's panel; both arms are pinned.
  - Smaller facts worth knowing before touching this code: a path-backed
    buffer's **name is its full path**, not its basename, which matters
    for any name assertion; `pmacs.buffer.kill` (not `remove`) redirects
    windows off a doomed buffer first, so `dired.kill-when-opening` kills
    **after** the replacement is displayed; ownership is checked against
    the handle table only, never the buffer name; and `C-x d` takes **no**
    completion source on purpose (with one, `RET` on an empty field opens
    whatever sorts first, and RET-where-you-are is the gesture the binding
    exists for — the field is prefilled instead).
  - Verification at merge: 1,832 default + 2,009 CRDT library tests;
    dired acceptance 25 + 25 CRDT; the frozen m8_1 10 / m8_2 15 / m8_3 32
    unchanged, which is the additivity gate for the `read_dir` change; M4
    121; required GPU 155; isolated-`XDG_CONFIG_HOME` workspace sweep
    3,205 across 93 suites. 15 claims bite-verified.
- Canonical `main` is protocol **v21** (`SUPPORTED=[6..=21]`; v16 =
  `ThemeFacts`, v17 = `FontFacts`, v18 = `StatuslineSegments`, v19 =
  terminal frames/events, v20 = the GPU initial-target semantic
  bootstrap family, v21 = the bottom-panel band family).
  **`ADVERTISED_PROTOCOL_VERSION` stays 20, permanently, and that is the
  activation mechanism rather than a hedge** — see the Stage 2B-3 bullet.
  The rule for every future additive family: advertise the baseline,
  negotiate up from the frontend's `AttachRequest`. Moving the advertised
  version is reserved for a change that cannot be expressed additively at
  all, because a server-first `Hello` reaches a shipped frontend before it
  can identify itself.
- **Bottom panel Stage 1 (window placement + TUI side windows) LANDED —
  #155** (`docs/bottom-panel-framing.md` rev 4; merge `e745068`; two review
  rounds). **No protocol change (still v20).** Arc 7's substrate: pmacs now
  has Emacs's `display-buffer` + window parameters, and a buffer can be
  displayed in a fixed-height window pinned to the bottom of the frame that
  feature code targets **by policy** instead of by stealing the selected
  window.
  - `src/window.rs`: `WindowParams { side, fixed_rows, dedicated }` plus
    implementation-owned `quit_action` / `origin_document` (Lua reads them,
    `set_params` refuses them); `MIN_WINDOW_OUTER_ROWS = 2`;
    `Layout::compute(area, fixed)` subtracts fixed children before dividing
    the remainder by weight, preserving last-flexible-takes-the-remainder, so
    a tree with no fixed leaves computes byte-identically to before.
  - **`Layout::compute` has TWO production callers**, and both must feed the
    same shared `panel_fixed_rows` map: `window_placements` and
    `src/overlay_paint.rs`'s peer-presence pass, which derives its own
    text-area `Rect` and never routes through the first. Leaving the second
    on unfixed geometry paints every peer cursor at the row it would occupy
    with no panel open.
  - **The minimum is recursive** (`subtree_min_rows`: horizontal splits sum,
    vertical splits max). "Two rows at the root" does not give each nested
    leaf two rows. `interactive_min_rows` is the same recursion over the
    user's `window.min-height` preference, and applies to drag/keyboard
    resize ONLY — the layout pass and frame-resize reconciliation use the
    structural floor, so changing a preference can never invalidate an
    existing layout.
  - **Hiding a panel is a durable state transition, not a per-frame effect**
    (`EditorState::reconcile_panel_layout`): it moves focus out and releases
    the terminal controller, because the terminal resize path merely returns
    on zero content without releasing. It runs after attach/resize/display/
    close and defensively before input dispatch, terminal sync, and paint.
  - `FrontendView` gains `panel_capable`, `frame_geometry`
    (`None` = **unknown**, never the GPU attach request's permanent 24×80
    placeholder), and derived `panel_hidden` — each spelled explicitly at
    every construction site, preserving folding's non-`Default` discipline.
  - `EditorCore::primary_document_window` is the Q#BP14 projection seam;
    `display_buffer` is Phase 1 of the display transaction (exact target →
    side affinity → ordinary reuse, with option-valued height/dedication);
    the Lua layer owns Phase 2 (activate → hook → reconcile → revalidate →
    final-focus matrix).
  - **Optimistic input is gated per WINDOW, not per buffer**:
    `dispatch_idle_for` returns `false` whenever the acting frontend's active
    window is a side window. Marking the panel's BUFFER round-trip would be
    wrong — `round_trip_buffers` is global by `BufferId`, so it would disable
    optimistic apply for another frontend editing that buffer as its document.
  - Jump entries are per frontend and carry their origin `WindowId`; a stale
    **side** origin is SKIPPED, because degrading it to an active-window
    switch is exactly the duplicate-panel corruption the arc removes.
  - `pmacs.window.display / display_file / quit / panel / params /
    set_params / resize / display_target`, plus `builtin/runtime/window.lua`
    (`window.panel-height`, `window.min-height`, `C-x ^` / `C-x C-^`).
    Adopters take `display = "current" | "panel"`; **Stage 1 default is
    `"current"`** and Stage 3 flips it.
  - The divider is the upper subtree's existing mode-line row — no row added
    or consumed, `ui.divider` restyles every exposed segment of one boundary,
    and drag state is `HashMap<FrontendId, _>` so frontends cannot steal each
    other's gestures.
  - `open_initial_target` now shares one `resolve_target_buffer` +
    exact-window install seam with `display_file`, and reasserts into a
    document window after hooks (a startup hook can now create a panel).
  - Final gates: 1,817 default + 1,994 CRDT library tests; the new
    `bottom_panel_stage1_acceptance` 46; kill ring 30; compile 67; M4 121;
    required GPU 152; initial-target 14 CRDT; all three vterm suites; folding
    Stage 2 48. All 12 CI checks green at merge.
  - **Stage 2 (the GPU panel band) is COMPLETE: 2A #177, 2B-1 #184,
    2B-2 #187, and 2B-3 (this lane)** —
    `docs/bottom-panel-stage2-framing.md` rev 6, four
    framing review rounds, no open framing items; the rev-5
    implementation split was explicitly approved 2026-07-27 and rev 6
    records PR #184's server-first compatibility and gate correction.
    It reserved protocol **v21** and shipped as four serial
    implementation slices: **2A** classified census routing +
    per-window painter extraction (no wire change, #177), **2B-1** the
    wire (#184), **2B-2** the daemon projection and epoch machine
    (#187), then **2B-3** the GPU band, compatible v21 activation, and
    the negotiated `panel_capable` flip. Production attachment stayed
    v20 through 2B-2 and negotiates v21 from 2B-3 on. Parent acceptance
    37–55 remains authoritative. **Stage 3, the adopter default flip, is
    the arc's remaining step.**
  - **The §1.3 census is CLASSIFIED, not uniformly redirected.** Only the
    Projection class (#1–#12, #21–#22) routes through
    `primary_document_window`; focus/input (#13–#15, #23), focus chrome
    and surface-routed (#16–#19), and focus/session (#20) keep their own
    authorities. Rerouting them breaks remote-op validation and
    application, `DispatchIdle`, presence, focused
    search/menu/completion routing, and terminal bell ownership.
- **Bottom panel Stage 2B-1 (the reserved v21 wire) LANDED — #184**
  (merge `6bee09d`, 2026-07-28, two review rounds plus a gate-found
  follow-up; all 12 checks green on reviewed head `5539b6e`). It adds
  no producer, no consumer, and no capability: `panel_capable` is still
  `false` for every semantic session, so nothing about it is
  user-visible. What it establishes is durable:
  - **Schema support and production advertisement are separate facts.**
    `SUPPORTED` is now `6..=21`; the daemon's unsolicited `Hello` still
    says 20. This is not a hedge — **the handshake is server-first**, so
    a shipped v20 frontend rejects a `Hello { protocol_version: 21 }`
    *before* it can send an `AttachRequest`. Bumping the advertised
    version is therefore an incompatible act on its own, independent of
    whether any new message is ever sent. A real-daemon acceptance
    emulates that exact rejection point and then requires the
    attachment to reach its initial grid. **Stage 2B-3 discharged this
    without touching the unsolicited `Hello`** — the advertisement is a
    permanent baseline and the frontend counter-offers; see its bullet
    below. That acceptance still passes unchanged, which is the point.
  - **One shared grid validator, split along a stated boundary**
    (`pmacs-protocol/src/wire_grid.rs`). Shared: checked area, the
    visible-cell bound (262,144), cell count, cursor bounds, glyph
    legality, wide-continuation topology, the 8 MiB aggregate
    glyph-byte budget, and the attachment rejection. Terminal-only: the
    512 per-axis PTY caps, title/process metadata, selection spans, and
    the `at_bottom == (scroll_offset == 0)` coupling. Per-axis caps are
    a `WireGridLimits` **parameter**, not a constant, because a panel
    does not inherit them — a 4K surface at a small font is
    legitimately wider than 512 columns, and the *area* bound is what
    keeps the encoding inside the transport budget. The attachment
    rejection is deliberately shared: panels render no attachments
    either, so classifying it terminal-only would let a panel ship a
    cell no frontend can paint.
  - **`PanelFramePayload::Absent` is authoritative, and silence is
    not.** The receiver retains its last valid frame, so a close *or* a
    hide must send `Absent` explicitly or a stale band stays on screen
    indefinitely. Validation is likewise atomic — a bad frame is
    rejected whole and the previous valid frame is retained.
  - **Two epochs, answering two different questions.** `panel_epoch` is
    opaque and monotonic per frontend: stable across ordinary frames of
    one continuously present window/buffer, and moved on buffer
    replacement, new side-window creation, and every `Absent` →
    `Present` transition — which is what stops a stale `PanelPointer`
    from addressing a reopened panel as if it were the old one (Q#BP16).
    `geometry_epoch` answers a *frontend* declaration and moves whenever
    the frontend declares new effective cell geometry, **including a
    font or scale change that leaves `CellSize` identical** — exactly
    the case daemon-side value dedup cannot see (Q#BP2S1).
  - `PanelFrame` carries an explicit `buffer_id` (review round 1) and
    its `focused` bit is presentation and focus-chrome routing only
    (Q#BP14b) — the *keys* decision remains `DispatchIdle` (Q#BP14a).
  - The frontend half is `FrontendEvent::{FrontendCellGeometry,
    PanelResizeRows, PanelPointer}`, gated in both directions, with each
    extended enum byte-pinned on its own previous final variant so the
    v6–v20 encodings are provably unchanged.
  - **A version bump is not done until every ratchet that pins the old
    version moves.** The full gate — not review — found that the
    statusline and Vterm Stage 3 ladders still pinned v20 and rejected
    v21, in both structural and real-headless-probe form. Grep for the
    outgoing version across `tests/` before calling a bump complete.
- **Bottom panel Stage 2B-2 (daemon projection + epoch machine) LANDED
  — #187** (one review round of five findings on top of the
  implementation; 12/12 green; 22/22 mutations bite). What it
  establishes, beyond the feature:
  - **A durable transition implemented as a per-frame effect is a bug
    shape, not a style.** Four of the five review findings were one
    defect: a renderer-side conditional with a durable twin that
    disagreed with it. Wire-area exhaustion hid the rendered frame but
    left panel state live; a stale `panel_epoch` could address a
    reopened same-buffer panel; `NoMessage` cleared statusline segments
    it should have retained. The fix that generalizes is
    `presentable_panel_grid` — **one** private derivation behind both
    the renderer and `reconcile_panel_layout_core`, because two
    derivations of one predicate is *how* they drift. When touching
    panel rendering, ask of every conditional: is there a durable twin,
    and does it agree?
  - **Ask what the other frontend kind's equivalent does.** The
    semantic panel terminal missed the pre-child-drain resize its
    document sibling had; the repro is a 4×20 panel reporting 3×120.
    The new `sync_semantic_panel_terminal_layout` is disjoint from that
    sibling *by construction* — one resolves through
    `primary_document_window`, the other through `side_window_for` — so
    the SIGWINCH storm the original extraction prevented cannot recur
    through the new path.
  - **A panel may legitimately be wider than a PTY.** `>512` columns
    are legal on the wire (2B-1's `WireGridLimits` parameter), but
    `snapshot_for_view` refused the band's content rect and the band
    went `Absent` with `panel_hidden` false. Clamp the *terminal
    projection*, not the band: the child takes the columns a PTY can
    have and the remainder is band background.
  - **Mutation testing cannot reach behaviour never modelled.** All 16
    original mutations passed while five real defects stood; one of
    them even fired, removing an `Absent` the test asserted. Mutations
    prove assertions bite the code that exists. They say nothing about
    the assertion never written.
  - **One durability hole is accepted deliberately:** epoch exhaustion
    stays a per-frame `Absent`, flagged rather than hidden, because
    making it durable needs a new "presentation permanently
    unavailable" reason in `FrontendView` for a state requiring 2^64
    presentations in one session. **Stage 2B-3 did not make it cheap** —
    the frontend half latches instead, which is a different remedy, so
    the daemon-side hole stays open as recorded.
- **Bottom panel Stage 2B-3 (the GPU band, compatible v21 activation,
  and the negotiated capability flip) — the arc's Stage 2 is COMPLETE.**
  This is the slice a user can see: a semantic GPU session now renders a
  real panel band instead of taking the Stage 1 non-side fallback, which
  is what closes the journey-steps-7–10 divergence §6 of the Stage 2
  framing names. Durable facts:
  - **A server-first handshake is negotiated from the CLIENT side, not by
    moving the advertisement.** `ADVERTISED_PROTOCOL_VERSION` is now a
    permanent compatibility *baseline*; the frontend answers
    `requested_protocol_version(baseline)` — its own `PROTOCOL_VERSION`
    when the baseline is the current one, a verbatim echo of anything
    older — and the daemon records
    `negotiated_session_version(offer)`. A shipped baseline frontend
    echoes and gets a baseline session, byte-for-byte as before; a
    current frontend offers up and gets the current wire. The `Hello`
    encoding and value never change, which is why the old frontend never
    sees a version it must reject. **The daemon needed no change to
    accept the offer** — it already recorded `req.protocol_version` as
    the negotiated version, so the whole mechanism is one value the
    frontend chooses plus a documented clamp.
  - **The window this leaves open, named rather than hidden:** a daemon
    whose own `PROTOCOL_VERSION` equals the baseline rejects an offer
    above its supported range. Compatibility can be preserved for old
    *frontends* or old *daemons* — a single `AttachRequest` cannot mean
    both "I want 21" and "≤ 20" — and the frontend direction is the one
    that matters, because the daemon is what a user leaves running. It
    closes on the next daemon restart and surfaces as an explicit
    `GoodbyeReason::VersionMismatch` naming both versions.
  - **`server_protocol_version` split into two facts on the GPU client**:
    `session_protocol_version` (what the session speaks — every wire gate
    keys on this) and `baseline_protocol_version` (what `Hello` said).
    They now DIFFER in the normal case, and that difference *is* the
    compatibility property, so both headless probe reports emit both and
    the two ratchets that read them assert both directions. Asserting
    only the session version would pass if the baseline had been bumped
    too, which is the exact incompatible change the mechanism avoids.
  - **The GPU document bottom is three boundaries and each call site was
    classified individually** — 20 production sites (8 status-owned, 12
    document-owned), 1 definition, 8 test sites, 29 matches, arithmetic
    stated. `geometry_capacity_bottom` reserves the divider *while the
    panel is absent* (that asymmetry is what breaks the first-open cycle)
    while `document_text_bottom` costs the document nothing until a
    `Present` frame paints.
  - **Moving a boundary is not always sufficient.** `edge_scroll_direction`
    has no upper bound, so a pixel *inside* the band still read as
    "further down the document" and armed the document's auto-scroll —
    the exact named symptom, surviving a correct reclassification. Any
    consumer that treats "past the bottom" as unbounded needs the band as
    an explicit exclusion, not just a moved boundary.
  - **Three of the first-pass assertions were VACUOUS, and the mutation
    runs are what found them.** (a) The contrast assertion compared
    `status_band_top` before and after — a *fixed point* — so the blanket
    rewrite it exists to prevent moved both readings together and passed;
    it is now anchored to an independent formula. (b) The criterion-46
    pixel test passed with the band painting *nothing*, because
    installing a panel reshapes the document and that produced the whole
    diff; it now counts differing pixels in the divider and cell rows.
    (c) The probe-versus-document-advance fixture compared two ASCII
    documents, which in a monospace family have identical advances.
  - **"No panel frame reaches a v20 session" is defence in depth, not the
    placement gate.** The producer's peer flag and the write-loop filter
    both suppress `PanelFrame` below the panel version independently of
    `panel_capable`, so that assertion passes with the capability gate
    removed entirely. The load-bearing claim is *placement*: the adopter's
    buffer must land in the pre-panel session's own document window,
    because a side window it cannot render is simply invisible.
  - `PanelBand::presented()` is the ONE frontend-side derivation of "is a
    band on screen" — retained valid frame, matching `geometry_epoch`,
    latch clear — behind the band inset, the painter, the hit-tester, and
    the drag. A latched frontend also stops absorbing payloads, so
    `presented()` is not the only thing between a disowned declaration and
    a painted band.
  - Panel columns come from the **stable normal-face probe**, never
    `mono_advance`'s document-glyph fallback, and `panel_cell_capacity`
    carries the daemon's **virtual status row** (`frontend_area_rows` is
    `total.rows - 1`) with **no per-axis cap**, because a panel may
    legitimately be wider than a PTY.
  - `PANEL_MIN_VERSION` moved into `pmacs-protocol` so the GPU frontend
    aliases one definition instead of restating 21.
  - **Stage 3 (the adopter default flip) is the arc's last step**, and it
    is the only thing between today's state and omitting `display`
    resolving to the panel policy.
- **dired Stage 2 framing LANDED (document only) — #171**
  (`docs/dired-stage2-framing.md`, revision 9; seven review rounds).
  **Approved as a framing; no runtime code and no implementation
  started.** Load-bearing decisions a reader must not re-derive:
  - **Reconciliation is order-independent, deliberately.** Reply order
    is *not* execution order — `AsyncRuntime::tick` drains the bus with
    `try_recv`, so a worker can finish first and be descheduled before
    sending. An ordering token was rejected rather than built: no
    static rule rescues the hazard anyway (rename `dir`→`newdir` racing
    delete `dir/child.txt` needs opposite orders depending on which ran
    first), and no production path can produce it. The trigger to
    revisit is named: the first production caller that fire-and-forgets
    two interdependent mutations.
  - **A refused mid-edit kill is already destructive.**
    `kill_buffer` clears round-trip state and moves windows *before*
    `BufferRegistry::remove` can return `ConcurrentEdit`, so
    `editing_in_progress` must be preflighted rather than relied on to
    refuse cleanly.
  - **Path-backed buffer names are not full paths.**
    `get_or_load_buffer` takes the name from the path *as given* and
    normalizes only the stored path, so a relative open is named
    `foo.rs`. Rename provenance is path-*equivalence*, not equality.
  - **URI-keyed stores have uncorrelated writers.**
    `textDocument/publishDiagnostics` is absorbed unconditionally, and
    `mark_document_stale` takes no `LspServerId` at all while creating
    URI keys across three stores. A route purge keyed on request
    responses cannot cover either.
- **Resource-op delete-guard implementation LANDED — #190**, atop the
  framing #186 below. The pre-filesystem refusal now exists: a
  synchronous `apply_resource_op` delete that would destroy unsaved work
  is refused before the filesystem is touched, rather than after.
  - **A refusal that no production path can reach passes every
    direct-call test.** The guard is asserted through the outermost
    user-reachable seam and falsified by revert, not by calling the
    check directly. This is the general rule now recorded in §5.
  - **Delete refusals must be visible.** Review round 2 found them
    silent — the operation declined and the user learned nothing.
    Reporting goes through `pmacs.editor.set_status`; `pmacs.error` is
    still a channel defined only by a test stub (§5).
  - **A URI-keyed store is not one store.** Purging a route keyed on
    request responses covers neither `mark_document_stale` (which takes
    no `LspServerId` while creating URI keys across three stores) nor
    `DiagnosticView`, whose URI is fixed at construction.
- **dired Stage 2a LANDED — #196** (`docs/dired-stage2-framing.md`
  rev 9, §5/§6/§10 — the substrate transaction only, **no dired
  surface**). Rename and delete reconciliation across the path owners a
  rename actually crosses.
  - **A rename is a transaction across five owners**, and this is the
    fact that forced the 2a/2b split: the buffer path, the buffer name,
    the URI-keyed LSP stores plus `DiagnosticView`, dired's pathless
    handles, and a captured Lua local that no transaction can reach.
    Stage 2b owns everything needing new Rust primitives.
  - **New LSP resource subscribers must not swallow reconciliation
    failures**, and `forget_uri` must not leave purged requests live in
    `LspClient.pending` — both were review findings, both now pinned.
- **Generated-buffer immutability Stage 1 LANDED — #191**, adopting the
  contract framed in #188. `dired.lua`'s `paint` and `listview.lua`'s
  `render` write through `pmacs.buffer.set_generated_contents`; zero
  `bypass_intercept` writes remain in either file.
  - **Why these two families first, and it is not the cheap half.**
    `compile.lua` and `builtin/commands/default.lua` rebind all seven
    undo chords to a no-op; `dired.lua` and `listview.lua` rebind
    **nothing**, so a bare `C-/` emptied a listing and a panel. Stage 1
    closes the only two families reachable without `M-x`.
  - **The framing owns the acceptance contract; the implementation
    adopts it.** Review round 5 found #191 had locally restated Stage 1
    criteria while #188 still carried the originals. Where an
    implementation finds a criterion impossible, the framing is revised
    and re-approved first — it is not narrowed in place.
  - **Stage 2 still owes everything with new Rust in it:**
    `Buffer::apply_generated_edit`, the `{ generated = true }` option,
    Q#GB10's path-backed refusal, Q#GB15's `identity_protected`,
    Q#GB5's `ensure_slot` lock, and the remaining 13 write sites.
- **Generated-buffer immutability framing LANDED — #188**
  (`docs/generated-buffer-immutability-framing.md`, revision 7; six
  review rounds, thirty-two findings). Revision 7 is the governing
  contract for the whole arc.
- **Resource-op delete guard framing LANDED (document only) — #186**
  (`docs/resource-op-delete-guard-framing.md`, revision 5; five review
  rounds). **Approved as a framing; no runtime code.** It owns the
  urgent **pre-filesystem refusal** for synchronous `apply_resource_op`
  — which on `main` today destroys unsaved work — while #171 owns full
  post-delete lifecycle reconciliation. Carried facts:
  - The sequence is `stat/no-op -> enumerate and validate -> mutate
    filesystem -> reconcile`. Validation inspects without removing, so
    a failed deletion leaves buffers intact and `on_removed` still
    observes the path already gone.
  - **`EditorCore::find_buffer_for_path` is the wrong lookup** — it
    delegates to first-match-only `find_by_path`, and
    `pmacs.buffer.from_file` creates path-bound buffers with no dedup,
    so a clean first match can hide a modified second.
  - **LSP does not promise batch atomicity, and pmacs advertises no
    `workspace.workspaceEdit` capability at all** — no
    `documentChanges`, no `resourceOperations`, no `failureHandling`.
    Recovery is the client's declared choice, and there is no spec
    default for a client that declares none. Say only that observed
    pmacs behaviour *resembles* abort-style application.
  - **`pmacs.fs.remove` has no dirty check of its own** and neither
    lane adds one. After both land the guards sit one layer above it.
    Latent — zero production callers — but "both lanes guard deletion"
    reads as the primitive being guarded, and it is not.
- **GPU initial target LANDED — #148**
  (`docs/gpu-initial-target-framing.md` rev 3; merge `0dd16a5`; two review
  rounds). `pmacs --gpu [--socket NAME|PATH] FILE` transports exact Unix path
  bytes plus launcher cwd to the managed GPU client. Protocol v20 adds a
  semantic-session `SessionBootstrapRequest` after `AttachRequest` and an
  appended `InitialTargetResult` readiness barrier; v6–v19 wire encodings stay
  pinned. The daemon resolves the path lexically, deduplicates or loads/creates
  it in the authenticated frontend's view, runs the established load/switch
  hooks, upgrades the buffer for CRDT, and publishes every target-side CRDT
  upgrade to existing grid replicas before readiness. Semantic replicas receive
  a publication only when displaying that buffer, so a second target launch
  cannot switch an existing GPU window; one dead peer cannot fail the new
  session. Failed bootstrap writes a bounded result, shuts down the socket,
  removes provisional state, and restores the ambient active frontend. Any
  stale event from an uninstalled session is dropped before state access.
  Existing no-target managed launch, direct attach, TUI, and legacy protocol
  behavior remain intact. Folding Stage 2 integration: fold projection at
  attach is selected from the same negotiated `semantic_render` bit the target
  bootstrap uses (grid collapses, semantic/GPU stays source-line pending
  Folding Stage 3). Final gates: 1,815 default + 1,992 CRDT library tests;
  target + invocation gates 14/14 CRDT each; Folding Stage 2 48 CRDT; M4 121;
  required GPU 152; Vterm Stage 3 5 default + 7 CRDT; isolated-config workspace
  sweep 3,334 across 88 suites; two concurrent real Wayland/Vulkan GPU windows
  stayed on distinct target buffers after the second attach. All 12 CI checks
  passed.
- **Folding Stage 1 (headless fold engine) LANDED — #142**
  (`docs/folding-framing.md` rev 5; merge `c49a8c7`; three review rounds,
  round 3 clean). Arc 6's engine — instance-side and headless; **no frontend
  renders a collapse yet** (that is Stage 2). No protocol bump.
  - `src/fold.rs`: a per-buffer `FoldStore` of byte ranges attached as a
    translating/dropping `View` (the `BufferStyleSpanTranslator` pattern — it
    translates strictly-inside edits and DROPS boundary-crossers,
    provenance-blind); `FoldRegistry`/`SharedFoldRegistry` =
    `Rc<RefCell<HashMap<BufferId, {Arc<Mutex<FoldStore>>, ViewId}>>>` (the
    SyntaxRegistry per-buffer model), held on both `EditorCore`
    (`src/editor_core.rs:223`) and `EditorState` (`src/editor.rs:110`).
    Containment is **start-exclusive, end-inclusive `(start, end]`**; the
    stored range is `[end of head line, end of last hidden line]` (NOT the
    `ByteRange` struct doc's `[start,end)`).
  - Structural source: nearest enclosing block-like node ≥2 source lines →
    resolve introducer↔body → **derived head line** (the line immediately
    above the first hidden line, so wrapped signatures / `where` clauses stay
    visible) → **closer-aware tail** (a closing-delimiter line stays visible,
    e.g. `} else {`). Stale/absent parse tree refuses.
  - The six `EditorCore` edit primitives call `unfold_before_point_edit`
    first (command-path pre-edit unfold, keyed on the active frontend's
    point). Interactive Lua-command unfold (yank/query-replace/comment) is a
    Stage 2 obligation; CRDT-origin unfold is Stage 3.
  - `src/lua_bindings/fold.rs` (`install_fold`): `pmacs.fold.*` data API
    (explicit buffer, no ambient resolution, matching #127) + interactive
    commands on the **Emacs hideshow `C-c @` prefix set**;
    `builtin/runtime/fold.lua`.
  - `src/semantic_render.rs`: `fold_state_msg` PRODUCES `FoldState`
    (semantic/GPU sessions) — authoritative-empty, diff-suppressed, per-session
    baseline reset on `BufferSnapshot` (the #120 stale-mirror trap class; the
    GPU fold-mirror clear-on-snapshot is a named Stage 3 obligation).
  - Durable lesson (round 2): after wiring a cleanup into a production hook,
    PIN IT THROUGH THE REAL PATH — a direct-call unit test misses the wiring
    (falsify by revert).
  - **Stage 2 (grid/daemon collapse) LANDED — #149** (merge `6ed4fe9`; five
    review rounds; `docs/folding-stage2-framing.md` rev 4). Its
    load-bearing reframe: the TUI had **no non-identity
    source-line↔display-row map** (`view_top + row` was baked into ~13 sites),
    so Stage 2's spine is `src/fold_view.rs`'s `VisibleLineMap` — derived from
    the fold store plus a window's line offsets, never stored — that the render
    loop, gutter, diagnostics, caret, selection, peer presence,
    viewport/scroll/motion, and the mode-line indicator all route through, plus
    the interactive-Lua unfold widening. Threaded as `Option<&'a
    VisibleLineMap>` on a lifetime-bearing `Viewport<'a>` that stays `Copy`.
    Design points the review rounds forced, each a trap for Stage 3:
    - the map's unit is a **merged hidden component** (overlapping *or
      adjacent* intervals unioned, keeping the earliest visible head), not a
      fold — folds may cross, and an inner/later fold's own head can be hidden;
    - instances are **per rendered window** and **per command/event
      operation**, never per frame; a command's map follows the operation's
      TARGET window (a wheel event names a pane without activating it);
    - fold projection is **per-frontend** (`FrontendView.fold_projection`, set
      at attach from the negotiated `semantic_render` bit) — shared
      `EditorCore` motion would otherwise make a simultaneous unfolded GPU
      session's cursor skip lines it still displays;
    - a hidden cursor normalizes by **position**, not row, and `set_view_top`
      clamps in the setter rather than being repaired at render time;
    - the interactive-Lua unfold keys on the **post-intercept** edit site — a
      managed buffer intercept may legally relocate the op.
    No protocol bump. **Stage 3 (GPU) is next and has no framing yet**; its
    named obligations are GPU collapse at TUI parity, caret/hit-test
    fold-awareness, the `BufferSnapshot` **fold-mirror clear** (parent R2-4 —
    the #120 trap class), CRDT-origin / GPU-optimistic interactive unfold
    (parent R2-3), and flipping `FrontendView.fold_projection` to `true` for
    semantic frontends.
- **One-command GPU invocation LANDED — #141**
  (`docs/gpu-invocation-framing.md` rev 6; merge `63fbc66`; two implementation
  reviews). The additive public path is `pmacs --gpu [--socket NAME|PATH]`;
  bare `pmacs [FILE]` remains the TUI. Root owns the CRDT gate, socket
  resolution, sibling-regular-file GPU discovery/PATH fallback, and GPU
  outcome. The separate `pmacs-gpu` binary owns connect-or-start, a
  five-second / 50-ms retry window, pre-winit event buffering, daemon
  process-group/stdin/stdout/stderr isolation, and named child reaping with
  explicit ownership handoff. Direct `pmacs-gpu --attach RAW_PATH` remains
  strict, is documented as advanced, and never auto-starts. No protocol
  change. The initial macOS/LuaJIT CI run exceeded an unrelated outline
  performance threshold (147 ms / 100 ms); the complete failed-job rerun
  passed all twelve checks before merge.
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
  - **Arc 6 (folding) Stages 1 and 2 LANDED — #142 and #149** — the headless
    fold engine (store, structural source, Lua `C-c @` surface, command-path
    unfold, `FoldState` production), then the grid/daemon collapse (the
    `VisibleLineMap` spine, fold-aware gutter/diagnostics/caret/selection/
    presence/viewport/motion, and the interactive unfold widening). **Stage 3
    (GPU) is next**, unframed.
  - **Web grammars HTML+CSS LANDED — #146**, and **LaTeX Stage 1 — #144**
    with its inline-math parent framing **#145**.
  - **Arc 7 (bottom panel) Stage 1 LANDED — #155** — window placement,
    window parameters, TUI side windows, the divider, and the adopter
    `display` opt-in. **Stage 2 (the GPU band) is next and needs its own
    re-framing**; Stage 3 is the default flip. DAP was parked awaiting
    exactly this arc's Stage 1 and can now re-baseline its touch census.
  - Remaining ranked arcs: 6 folding Stage 3, 7 bottom-panel Stages 2–3,
    DAP, 8 GPU splits, plus the `.ipynb` arc (its JSON-grammar
    prerequisite shipped in #123).

- **GPU terminal input LANDED — #166** (`main` @ `b889873`;
  `docs/gpu-terminal-input-framing.md` rev 2; one review round). The
  dispatcher applied **both** terminal-layout syncs to **every** attached
  frontend each tick. A semantic session satisfies both conditions — a
  `term_sizes` entry from `AttachRequest` *and* a terminal declaration — so
  its PTY was resized twice per tick forever: the grid arm installed the TUI
  placement size, the semantic arm the declared content rectangle, each arm's
  `old_size == size` guard seeing only what the other had just written. The
  child took a `SIGWINCH` storm at tick cadence, which made typing into a GPU
  terminal impossible while output kept flowing. TUI was structurally
  unaffected.
  - `EditorInstance::sync_terminal_layout` is split into
    `sync_terminal_controller_liveness` (frontend-kind **neutral**: panel
    reconcile + release of a controller whose window moved away — reads only
    views/windows/controller, never a grid size) and
    `sync_terminal_grid_geometry` (**grid only**: TUI placement + resize).
    `sync_terminal_layout` survives as the composition, so `editor::run` and
    `LOCAL` are byte-identical.
  - `daemon::sync_terminal_layouts_for_tick` is the extracted loop body:
    liveness for every frontend once per tick, then **exactly one** geometry
    arm keyed on `semantic_states` membership — the same fact session
    establishment uses, so the arms cannot both fire.
  - **The trap, kept in a comment:** the release on a missing
    `window_placements` entry reads like liveness and is grid geometry. A
    semantic frontend has no placement entry at all, so moving it into the
    neutral half would release a GPU controller every tick.
  - Why not the one-line guard: the grid arm was also the **only** per-tick
    controller-liveness release a semantic frontend got, and
    `sync_semantic_terminal_layout` cannot take it over — the buffer-follow
    snapshot clears the viewport declaration, so that arm stops running in
    exactly the switch-away case that needs the release.
  - No protocol change (v20). Gates: 1,829 default + 2,006 CRDT library
    tests; vterm Stage 1/2/3 10/6/9 CRDT; bottom-panel 46; M4 121; required
    GPU 155; isolated-config workspace sweep 3,177 across 92 suites.
  - **Known gap, its own lane:** CI never enables `crdt`, so the Stage 3
    real-path acceptance (including `a37`) is not compiled there. #166's unit
    pins are not `crdt`-gated and do run. See `docs/active-work.md`.

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

- **Ambient storage roots: control all FIVE, not four.** Until the
  ambient-root isolation lane lands
  (`docs/test-ambient-config-isolation-framing.md`), the ~96 integration
  suites read the developer's real `~/.config/pmacs/init.lua` and
  **write** bundled packages into the real data root:
  `#[cfg(not(test))]` guards the crate's own unit tests only, and
  `EditorState::new` materializes packages outside every `cfg` guard.
  A local full-suite run therefore needs, all pointed at a fresh
  directory:

  ```
  XDG_CONFIG_HOME  XDG_DATA_HOME  XDG_STATE_HOME  XDG_CACHE_HOME
  PMACS_STATE_HOME
  ```

  **The fifth is not redundant.** `PMACS_STATE_HOME` outranks
  `XDG_STATE_HOME` (`src/state.rs`), so redirecting the four XDG
  variables on a machine that exports it leaves the real state root
  live. `HOME` is deliberately left alone: it is the fallback for the
  XDG roots (already covered once they are set) and separately drives
  `~`-expansion, which `find_file_acceptance` pins on purpose.
  A run isolating only `XDG_CONFIG_HOME` stops the `init.lua` reads and
  still writes through the real data root — every local gate run in this
  repo before 2026-07-31 had that hole.

  **After the lane lands** the in-process population isolates itself
  through `EditorState::new_with_roots(&crate::iso::roots())` and the
  five variables are belt-and-braces rather than required.

- **basedpyright**: the desktop binary was **never broken** — this was a
  real code defect, diagnosed and fixed 2026-07-29 (see §5, "A `Drop`
  body runs before its fields"). `RuntimeHandles::drop` joined its reader
  threads before the `stdin` field dropped, so the server never got stdin
  EOF, never exited, and kept the output pipe the readers were blocked
  on. Deterministic on the desktop, invisible on the laptop and in CI,
  which is why it read as a broken local binary for weeks.
  **How the orphan is actually made — WE make it.** basedpyright's
  console script runs bundled `node` through `subprocess.run` and
  **waits** (`nodejs_wheel/executable.py:50`, verified in 1.39.6). At
  teardown `shutdown()` SIGTERMs the *recorded* pid — the Python wrapper
  — which dies without forwarding the signal, orphaning node to `PPid 1`
  holding the pipes. An earlier revision of this entry said the wrapper
  "spawns node and exits"; that was wrong, and the refutation was already
  in hand, since the initialize handshake succeeds, which a
  wrapper that exited at spawn could not have done. The consequence is
  for the follow-up, not the fix: the orphan-management work is **stop
  orphaning them** (signal the group), not tolerate self-orphaning.
  The `--skip` above stays for now: it is still correct on any tree
  predating the fix, and — the one live reason — **CI never installs
  basedpyright at all**, so arming `PMACS_REQUIRE_PYRIGHT` would fail
  rather than test anything. The two original reasons are both gone: the
  hang is fixed, and #195 gave every job a `timeout-minutes`, so a hang
  can no longer burn six hours. Installing basedpyright in CI (a uv plus
  bundled-node download per leg) and dropping the local skip are two
  separate proposals, each owed its own evidence.
- **GPU on the laptop**: AMD Radeon 780M (RADV) — native Vulkan,
  `PMACS_REQUIRE_GPU=1` works without lavapipe.
- **Flaky-under-load tests — what this lane OBSERVED.** *(Historical
  record. This bullet no longer states a triage rule: for judging a red
  run, `docs/ci-red-signatures.md` is the authority, and its rule
  supersedes "rerun isolated" — a green rerun establishes intermittence
  only, never environmental cause.
  `m6_8_supervisor_reaps_all_children_across_cycles` is **A2** there —
  an audit note, not a matchable row, because no signature was ever
  captured.)* The m8 daemon tests and the m6
  process/PTY tests (`m6_1_pty_mode_lifecycle_started_then_exited`,
  `m6_8_supervisor_reaps_all_children_across_cycles`) are timing-based;
  `editor::composition_overhead_under_ten_percent` is a render-ratio
  microbenchmark that fails ~1/3 even isolated single-threaded (already
  `cfg!(macos)`-disabled). *(Local measurements, 2026-08-05, 16-core
  Linux, tree-primitive branch, which could not reach it — its diff
  versus `main` touches no `src/`, no crate, no manifest. **Two reds,
  both inside the full `--features crdt` lib run**, at `dispatch
  overhead` 30.7% and 10.3%; two full-suite runs at the same tips were
  green; and **10/10 green run alone**, ratios spanning -2.3% to
  +1.5%. So the in-suite/isolated split has now been seen twice, and
  the second red cleared the 10% budget by 0.3 points — the threshold
  is marginal, not comfortably clear. Recorded as measurements, NOT a
  cause: ten isolated greens establish that the ratio sits nowhere near
  the threshold when alone, not that in-suite contention is what pushed
  it over, and two reds against two greens in-suite is intermittence
  rather than a mechanism. Not a `ci-red-signatures.md` row either —
  that registry judges red **CI** runs, and these were local.)* Vterm Stage 3's merge CI saw one macOS timeout in
  `real_tui_terminal_smoke_restores_host_after_output_input_resize_scroll_copy_and_bell`;
  the complete failed-job rerun passed. The required-GPU gate also failed once
  in `headless_diag_face_recolors_band_counter_despite_unchanged_text`, then
  passed both an isolated single-thread rerun and the full 139-test rerun.
  *(That "rerun the test alone before investigating" instruction is
  retired — an isolated green reproduces nothing about a load-sensitive
  failure and establishes intermittence at most. See the registry's
  rerun rule.)* Run the workspace
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

**Generated buffers: `Buffer::set_generated_contents` is the ONE
authorized write** (terminal copy mode #178) — lift `read_only`, replace
via a single whole-buffer `Replace` skipping intercepts, discard history,
re-assert `read_only`, and **return the `Edit`**. Three things make it a
unit rather than a convenience:

- **An intercept is not read-only.** `Buffer::undo` reaches the rope
  through `ensure_writable` and never consults the intercept chain, so an
  intercept-only "read-only" buffer is emptied by `M-x buffer.undo`.
  Rebinding the undo *chords* buffer-locally does **not** close it —
  `compile.lua`'s own comment says so ("command/menu undo stays
  dispatchable"). Only rope-level `read_only` does.
- **A bare `set_read_only` would be worse than nothing**, because it also
  refuses the owner's refresh — the operation such buffers exist for.
  That is why the pairing, not the setter, is the primitive. There is
  deliberately no Lua `set_read_only`.
- **A rope write is only half of an edit.** The returned `Edit` must be
  fanned out (`notify_buffer_edit_to_windows`, which also queues the
  daemon-origin CRDT op). Skip it and a displaying window keeps a
  `TextView` line index describing the previous contents — the next paint
  indexes the new rope with stale ranges and trips
  `assertion failed: end <= self.len()` — while replica mirrors never
  import the write at all.

History clearing is load-bearing twice (nothing can pop entries
`read_only` makes unreachable, so they leak), and must clear **whichever
history the buffer has**: the v0.1 stacks are bypassed in CRDT mode, where
it lives in loro's `UndoManager`. That has no `clear`, and needs none — a
manager records only what happens after construction, so
`CrdtState::clear_undo_history` rebinds a fresh one to the same doc.

**Not yet adopted — the inventory is four writer mechanisms covering
five buffers.** *Every remaining intercept-protected writer* uses the
older idiom: an erroring intercept plus `set_round_trip_input`, written
through `bypass_intercept`, with the rope left writable. All are
emptiable by `M-x buffer.undo`:

| writer | buffers | shape |
|---|---|---|
| `builtin/runtime/listview.lua:60-61` | every listview panel | delete-all + insert |
| `builtin/runtime/compile.lua` (`ensure_slot`) | `*compilation*`, `*shell-command*` | **append** per output batch |
| `builtin/commands/default.lua:869` | `*search-results*` | reset per query, then **append** per match batch |
| `builtin/runtime/dired.lua:371` | every dired buffer | whole-buffer replace |

**Do not read `ensure_slot` as covering the search panel** — it serves
`*compilation*` and `*shell-command*` only (`compile.lua:1090,1125`).
`*search-results*` is an independent panel with its own intercept,
round-trip mark and writes, and `compile.lua` names it only in a
predicate. Nor is the scope "every generated buffer": `*workers*`,
`*help*` and `*buffer-list*` are generated too but do not use this
idiom, and the REPL package's intercept
(`builtin/packages/repl/init.lua:187`) is an op-filtering editing
policy, not a read-only panel — neither group belongs to this lane.

Adoption is not a one-line swap. It inherits the fan-out obligation, and
the three appending buffers need a **streaming variant** of the
primitive; listview and dired already write whole-buffer replaces and
are the cheap half. Recorded in `COHERENCE.md` §14.

**And it does not replace `set_round_trip_input`.** The protection is
layered across two copies: rope-level `read_only` refuses the op at the
daemon; round-trip input stops a semantic frontend applying
optimistically to its **own mirror**, which a daemon-side refusal cannot
reach — the refusal arrives after the frontend has already painted, so it
buys divergence, not prevention.

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

**Protocol** — encoding-breaking bumps are deliberate and versioned. Canonical
`main` is `[6..=20]`. The in-review bottom-panel 2B-1 schema extends support
to `[6..=21]`, while `ADVERTISED_PROTOCOL_VERSION` stays 20 until 2B-3
provides compatibility-preserving activation; the server-first `Hello`
cannot advertise 21 without stranding existing v20 clients before
`AttachRequest`. v15 = `CompletionPopup` + `StatusFacts.message`; v16 =
`ThemeFacts`; v17 = `FontFacts`; v18 = `StatuslineSegments`; v19 = the vterm
terminal family; v20 = semantic `SessionBootstrapRequest` plus appended
`InitialTargetResult`; v21 reserves the panel frame/event family. New wire
surface ⇒ bump + both-frontends support + acceptance. An APPENDED variant
must be guarded by a byte pin on the PREVIOUS final variant — its own
round-trip cannot detect a discriminant shift.

**Fake LSP** (`src/bin/pmacs_fake_lsp.rs`) modes: `fullonly`,
`rangeonly`, `rangeonly16` (UTF-16 + fail-closed bounds validation),
`sighelp`. Use these for capability-matrix tests, not real servers.

## 5. Hard-won ops lessons

- **An optional field that a data structure's shape depends on is not
  optional.** `listview` rows may omit `item`, so
  `line_to_item[n] = row.item` leaves that map **sparse** — and
  `seat_cursor` took `#` of it, which for an all-display-only tree is
  0, stranding the cursor on the header where TAB finds no row. The
  whole existing suite missed it because **every test supplied
  `item`**: the field was optional in the API and mandatory in
  practice, and nothing in a passing run distinguishes those. When a
  field is optional, write one test that omits it — the sparse-table
  `#` is a Lua-specific trap, but "optional in the signature, required
  by the shape" is not.
- **A contract two mechanisms must honour is only as strong as the
  weaker mechanism.** `listview` ids were documented "opaque, compared
  by equality". Selection compares with `==`, honouring `__eq`; collapse
  state stores ids as **table keys**, and Lua indexes tables by raw
  identity, consulting no metamethod. A table id satisfied one half and
  silently failed the other — a refresh would restore the cursor and
  lose the fold, surfacing arbitrarily later with nothing pointing at
  the id. **Narrow the contract to what both halves can honour and
  enforce it where the data enters**, rather than generalizing the
  strong half to match the weak one: equality-aware collapse lookup
  would have turned a linear render quadratic to support a key type no
  consumer wanted. The same pass found the contract said "identity"
  while accepting duplicates (every lookup takes the first match, so
  selecting the later row toggles the earlier) and "number" while
  accepting NaN (a number Lua refuses as a key). **Enforcement and
  documentation drift apart silently; only the enforcement is real.**
- **Match a red by its required fragment, never by test name or
  resemblance.** An occurrence scan on 2026-08-06 nearly filed `main`
  run 30710662474 as **R3** — same test, same `EPERM`, same
  `measured_group=unobservable(ESRCH…)`. R3 requires `leader=live`;
  that run reads `leader=exited(signal SIGUSR1)`, which is **R2's**
  exact fragment, four days before R2 was retired. The two rows are a
  live possible **product defect** and a fixed **test race** in the
  same test, and only the fragment separates them. A second red in the
  same scan was filed as **R5** rather than folded into R1 for the same
  reason: both are supersede under a deadline on macOS, and **sharing a
  subject is not sharing a signature**.
- **A retired row's recurrence rule is about time, not just text.** A
  red matching a retired row falsifies the retirement **only if it
  postdates it**. An earlier occurrence found later corroborates the
  row instead — and can still add something: 30710662474 is macOS
  *luajit* where R2's evidence was *lua54*, so the mechanism was never
  flavor-specific.
- **A gate summary assembled through a pipe can report success over a
  failure.** `cmd | tail -2` returns **`tail`'s** exit status, not
  `cmd`'s — in `fish` and `bash` alike — so a chain of
  `cargo test ... | tail -2 && cargo test ... | tail -2 && echo "ALL
  GATES CLEAN"` prints the clean line even when a suite failed. This
  is not carelessness that closer reading would catch: the failure is
  **structurally invisible** in the summary the PR then cites. It
  happened while gating the silent-skip lane, and a `pmacs-gpu`
  failure was reported as clean.

  Either check `$pipestatus[1]` in fish (`${PIPESTATUS[0]}` in bash),
  or — better — redirect each gate to a file and read the file
  afterwards, which also preserves the full log this section already
  asks you to keep. Same family as the skip-reports-`ok` lesson below
  and the double-invocation traps: **the thing that summarizes a gate
  must not be able to lose the gate's verdict.**
- **A reproduction is a measurement, and needs its own positive control.**
  The basedpyright-hang lane wrote **four** reproductions that passed
  against the *unfixed* tree, each vacuous for a different reason: the
  child exited before the join; the child never read stdin at all; the
  child's stdin was silently rebound to `/dev/null` (POSIX XCU §2.9.3
  assigns `/dev/null` to an asynchronous list's stdin when job control is
  off, so `sh -c 'cat & exit 0'` EOFs instantly); and then **the repair
  for that was also wrong** — the rule applies *before explicit
  redirections*, so `<&0` duplicates `/dev/null` onto itself. `bash`
  skips the default when a stdin redirect is present, `dash` does not, so
  `<&0` passed locally and failed in CI. The shipped test uses
  `setsid --fork`, removing the shell from the reproduction entirely.
  Every one of the four looked obviously right when written, and the
  fourth was verified locally before it failed. Note what a narrower rule
  would have missed: "check the child is still alive" catches only the
  first. Only the general form catches all four — **and the ones nobody
  has invented yet.** Note also which mechanism caught the fourth: not a
  reviewer, but the control itself, failing loudly in CI and naming its
  own cause. So: assert the precondition your reproduction
  depends on, in the test, before exercising the thing under test. In
  `teardown_closes_stdin_before_joining_readers` that is two controls
  (the recorded child has exited; both readers are still blocked in
  `read`), each with a failure message naming what its absence means —
  and a `/bin/sh` that is `bash` locally and `dash` in CI is exactly the
  sort of divergence no amount of local verification reaches.
  This is the same rule that produced #192's bite positive control and
  #194's re-read-the-artifact lesson, stated at full generality: **a
  measurement you have not controlled is a claim, not evidence.**
- **A `Drop` body runs before its fields, whatever the declaration
  order.** Cost a multi-week misattribution: `RuntimeHandles::drop`
  joined its reader threads in the drop *body*, while the `stdin` sink it
  needed to close first sat in a *field* — reachable only after that body
  returned. The child never got EOF, never exited, and kept the output
  pipe the readers were blocked on, so teardown hung forever. Reordering
  the struct's fields cannot fix this shape; the operation has to move
  into the body. Generally: **if a `Drop` body waits on anything, check
  what the waited-on party needs that only a field drop will release.**
  Corollary from the same investigation — `cancel`-flag style wake-outs
  only work where the thread actually polls them; a thread blocked in a
  raw `read` never sees one, so a flag next to a blocking syscall is
  documentation, not a mechanism.
- **A test that skips on a missing precondition reports `ok`, and a gate log
  cannot tell that apart from a pass.** `vterm_stage3_acceptance::a37` — the
  only acceptance driving a real daemon, a real PTY and a real wgpu render
  together — derives `pmacs-gpu` from `CARGO_BIN_EXE_pmacs` and, when that
  binary is absent from the target directory, prints a skip and returns.
  A fresh worktree reports the suite 9/9 **in 0.17 s having never run it**;
  a real run takes ~4 s. `PMACS_REQUIRE_GPU=1` is what promotes the skip to
  a failure, and the standing gate list applies that flag to
  `cargo test -p pmacs-gpu`, a *different package*. Two habits follow:
  build the workspace before believing any suite that reaches for a sibling
  binary, and **judge such a suite by its elapsed time**, not its verdict.
- **Before attributing a red test to your branch, run it on the merge base.**
  `a37` failed on the #173 branch, which looked like a regression; it failed
  identically on the PR's own base and on two intermediate commits, and had
  *passed* on that same base twenty minutes earlier. The variable was machine
  load from a second agent compiling continuously. Load-sensitive tests make
  both verdicts uninformative in isolation, so the base-commit run is the
  cheapest way to tell a regression from weather — and it is much cheaper
  than the bisect it replaces.
- **A daemon-side fix is not deployed until the daemon is restarted from a
  tree that contains it.** #166's reporter rebuilt and saw no change: the
  running daemon had been started from a shared checkout still on a pre-fix
  branch, and `pmacs --gpu` attaches to whatever process already owns the
  socket. Rebuilding a binary does nothing to a running process. When
  validating a daemon-side fix by hand, check the running process's binary
  path and start time against the tree you think you fixed —
  `ps -eo pid,lstart,args | grep '[p]macs --daemon'` — before concluding the
  fix failed.
- **Two operations that must be alternatives are not made alternatives by
  being adjacent.** The dispatcher applied its grid and semantic
  terminal-layout syncs to every attached frontend; a semantic session
  satisfies both conditions, so its PTY was resized twice per tick forever
  and the child took a `SIGWINCH` storm that made a GPU terminal untypable
  while output still flowed. Each arm had a correct `old_size == size`
  idempotence guard — **individually sound, jointly useless**, because each
  saw only the size the other had just written. Write mutually exclusive
  per-frontend-kind work as one `if`/`else` keyed on the same fact session
  establishment uses, and extract the loop body so a test can drive the real
  thing.
- **Bite against every pre-image the fix could plausibly have taken, not just
  `main`.** For the same defect, the obvious one-line guard (skip the grid arm
  for semantic frontends) *does* fix the storm — and silently introduces a
  controller leak, because that arm was also the only per-tick
  controller-liveness release a semantic frontend got. A single revert would
  have scored the fix complete. The pin that catches it (`acc 6`) deliberately
  **passes on `main`** and fails only against the naive guard: today's defect
  supplies the release by the accident of running an arm it should not.
- **A quiet child is an instrument.** A frame storm is invisible against a
  fixture that legitimately emits hundreds of frames, and an assertion like
  `frames >= 2` cannot see one. The same applies to geometry: a
  "did a frame at the new width arrive" readout is satisfied by a geometry
  *oscillating through* that width. Assert upper bounds over a fixed window
  against a child that produces nothing, and let the child self-report the
  signal you care about (a `SIGWINCH` trap printing a **fresh distinct**
  breadcrumb per signal — repeated identical markers paint nothing, because
  `cell::diff` skips already-matching cells).
- **`TerminalMode::Raw` makes `sh`-based input fixtures useless.** There is no
  `ICRNL`, so Enter delivers CR and a `read -r` loop waits forever for a LF
  that never comes — the test then "proves" input never arrived. Use
  `exec cat`, which copies stdin to stdout byte by byte. It is also the right
  echo instrument for the opposite reason people assume: termios `ECHO` is
  *off* in raw mode, so nothing double-echoes and one keystroke yields exactly
  one cell.
- **Draining the event stream to learn a pid also TICKS, and a tick
  reaps.** #176's
  `observing_the_leader_does_not_consume_the_exit_event` failed under
  load with "process ProcessId(26) is not running": its helper drained
  for `Started` to read the pid, `drain_until` ticks while it drains, and
  a tick can observe an immediately-exiting child and move the record out
  of `Running` — after which `signal` never reaches the code under test
  at all. It passed standalone only because the drain returned on
  `Started` before `poll_one` saw the exit. Read the pid **straight from
  the supervisor record**, which does not tick, and fail fast if the
  record has left `Running`. **Verified load-bearing under matched load:
  0/15 failures with all 16 cores saturated versus 1/10 for the ticking
  helper.** The general rule: an observation helper that advances the
  system is not an observation.
- **A wait predicate that is WEAKER than the assertion it guards is a
  race, on every platform that happens to lose it.** #174: the m4 config
  sink test waited for `contains("probe")` and then asserted
  `"probe":true` — six bytes further on. The sink is JSONL written by a
  *separate process*, so the test could read a half-written line; Linux
  won that race reliably and macOS/lua54 did not, reporting the
  truncated `{"rust":{"probe":`. **Any "wait until the file mentions X,
  then assert Y about the file" shape races whenever Y is stricter than
  X.** The fix is to wait for the exact unit the assertion reads — here
  a trailing newline, true only once a whole `writeln!` record has
  landed, and still correct if the payload's field order or spelling
  changes. Two things generalize beyond the fix:
  - **A race you cannot reproduce can still be bitten at one remove.**
    The failure needs a scheduler outcome Linux does not produce, so no
    local run falsifies it. But `pump_async` *asserts* on its deadline,
    so replacing the predicate with an unsatisfiable one proves the wait
    is load-bearing rather than decorative — and restoring the *old*
    predicate, which still passes locally, proves a green local run
    cannot distinguish the two. Bite what you can reach and say plainly
    which claim rests on structure instead of evidence.
  - **The obvious fix is sometimes worse.** The sibling `m4_26` has the
    same weak shape and was deliberately left alone: waiting for the
    expected value would convert a genuine `rootUri` regression — what
    its `assert_ne!`s exist to catch — into a five-second timeout with a
    misleading "server didn't initialize?" message, trading a precise
    assertion diff for a vague hang. Closing it properly needs a record
    terminator in the fake server first.
- **Proving a child has exited is harder than it looks on this
  codebase.** A fixed sleep is not proof; nix's `waitid` is unavailable
  on macOS; and `libc::waitid` needs `unsafe`, which
  `#![forbid(unsafe_code)]` rules out. What works is driving the
  production diagnostic in a **bounded loop until it observes the
  exit**. Relatedly, #176's round-1 review replaced every substring
  assertion with exact message equality built from the kernel-assigned
  pid — the substring forms would have accepted a hardcoded target or a
  wrong exit code.

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
  trap-guarded one-file swap over read-only `git show`, with a
  two-sided verdict (the tests must PASS now and FAIL against the old
  version), making bite-verification machine-checkable.
- **`scripts/bite` is only as good as its positive control, and it
  had none until it was given one.** Before that it ran *only* the
  swapped tree, so it could not tell "my fix is load-bearing" from
  "my test is broken": a test that fails everywhere made the swapped
  run fail, and a failing swapped run was the only thing checked, so
  it printed `bite: OK` and certified nothing. It now asserts the
  named tests pass **and** that at least one actually ran (a filter
  matching nothing exits 0, so passing alone is not enough), exiting
  3 as `NO CONTROL` otherwise. It also distinguishes `OK (assertion)`
  from `OK (COMPILE)` — an old file that does not build against the
  current tree is a much weaker result, because the tests may never
  have run at all. **Read which one it printed.**
- **A fix must be COMMITTED before it is bitten** — but *not* for the
  reason previously recorded here. This file used to say `scripts/bite`
  "restores by `git checkout --`, which reverts the file to HEAD", and
  that a review round's fixes were wiped that way during #165. **That
  mechanism description is false and was verified false:** the script
  copies the file to a `mktemp` path before the swap and restores from
  that copy, under an `EXIT INT TERM` trap, so uncommitted work in the
  bitten file survives. It has never touched git state beyond a
  read-only `git show`. The rule still stands on its own merits —
  gate results must describe the pushed tree, and a `cargo fmt` after
  a commit splits worktree from branch — but do not repeat the
  destroys-your-work claim, which will push the next reader toward
  `git stash` to protect themselves, straight into the repo-global
  trap above. **The #165 incident itself is now unexplained**, and
  that is recorded rather than papered over: work really was lost, but
  not by the mechanism this file blamed. A `SIGKILL` bypasses the trap
  and would leave the swapped file in place, which is one candidate;
  so is a stash collision, given the same round. Do not invent a
  mechanism to close the gap — an unexplained incident is safer than a
  confident wrong cause, which is what produced this correction.
  Corollary for a NEW file: the swap-over-`git show` mode
  does not apply at all, so its claims must be bitten by hand-editing.
- **A CONFLICTING PR silently runs no CI at all.** GitHub builds
  `pull_request` workflow runs against the PR's **merge ref**, which it
  does not create while the branch conflicts with its base. So pushes
  land, the branch updates, no run is ever queued, and **nothing reports
  the absence** — the checks list simply keeps showing the last
  successful run, which reads as current. Three pushes to #165 produced
  zero CI before the cause was found, and `gh pr checks` returns nothing
  usable here. On any lane that lives through a moving `main`, check
  `gh pr view <N> --json mergeable,mergeStateStatus,headRefOid` and
  confirm a run exists **for the current head sha**, not merely that a
  recent run was green.
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
- **A provisional session that fails mid-bootstrap must actually close its
  socket, not just drop its handle.** #148's dispatcher-side target-failure
  paths wrote `InitialTargetResult::Failed` and dropped the write-half
  `UnixStream` clone, but a clone shares the underlying FD — the per-attach
  reader thread stayed alive with no installed session state. A client that
  kept the socket open past `Failed` and sent any ordinary event hit an
  `.expect` reachable only through the new failure path and panicked the
  whole daemon. Fix: `shutdown(Shutdown::Both)` on every dispatcher-side
  failure path, plus a defense-in-depth session-registry membership check
  before any `FrontendEvent` touches render/size/editor state. Generalizes:
  when a new failure path can leave a handle installed without its owning
  session, dropping a handle is not the same as tearing down the connection.
- **A guard with no production caller passes every direct-call test.**
  #155 round 1: `EditorCore::try_split_active` implemented the side-window
  split refusal, but `pmacs.window.split_horizontal` / `split_vertical` — and
  therefore `C-x 2` / `C-x 3` — still called plain `split_active`. The
  acceptance test called the core method directly, so reverting the guard
  entirely would have left every test green. Same shape as folding #142
  round 2. Assert through the outermost user-reachable seam
  (`try_exec(&s, "pmacs.window.split_horizontal()")`), then falsify by
  revert. When the test shares a file with the code it pins, `scripts/bite`
  cannot swap it — break the production line by hand and `git checkout --`.
- **A geometric readout is not a state predicate.**
  `TerminalViewStatus::at_bottom` is defined as `scroll_offset == 0` — "the
  viewport currently reaches the tail", not "this view follows the tail". A
  still-anchored view satisfies it whenever it happens to be tall enough, so
  asserting it could not detect that Q#BP7's growth re-arm had never been
  implemented (#155 round 2): the next rows the child printed pushed the
  anchored view back into history. Pinning *following* requires advancing the
  world — feed more child output through a filesystem gate — and asserting the
  view came along. Related: `scroll_offset` is viewport-relative, so
  "unchanged across a height change" is vacuous or wrong; the invariant is
  the frozen ANCHOR.
- **A PTY in the default mode does not translate LF to CRLF.** An
  `echo`-driven test fixture staircases rightward, and past the viewport
  width every row clips to blanks — so `assert_eq!(top_before, top_after)`
  compares `"" == ""` and passes for any regression (#155 round 2). Emit
  `printf '...\r\n'`, and guard text comparisons with
  `assert!(!observed.is_empty())` the same way the panel daemon pin guards on
  `!panel_hidden`.
- **Widening an ambient resolver into a scoped one can make a total function
  partial.** #155 round 2 resolved both arms of `pmacs.window.buffer()`
  through the acting frontend "for uniformity". `acting_frontend` follows the
  interactive origin, which can name a frontend with no registered view (a
  bare `dispatch_key` from an unattached peer), so the no-argument arm began
  raising instead of answering. No runtime caller `pcall`s it, so killring,
  syntax, autosave, pair, indent and comment silently dropped operations —
  `kill_ring_acceptance` went 30/30 to 25/5 on every CI platform. The ambient
  resolver's fallback is what makes it *total*; keep it, and document that as
  deliberate. Uniformity is not free when the paths have different totality.
- **An upgrade decision must be tracked independently of the outcome that
  triggered it.** #148 published a target's fresh `BufferSnapshot` to
  existing grid replicas only when the buffer was `newly_loaded ||
  newly_created` — but a target can dedup onto an already-existing,
  not-yet-CRDT-backed buffer (e.g. one a startup hook created via
  `find_or_open` and never activated), silently upgrading it without telling
  pre-attached replicas. The later F29 lazy-upgrade sweep then saw an
  already-backed buffer and never broadcast it, permanently stranding those
  replicas on v0.1 round-trip for that buffer. Fix: have the upgrade helper
  report whether it performed the upgrade, and OR that into the publish
  decision rather than inferring it from the caller's own load/create
  branch.


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
