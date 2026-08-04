# Framing — Bottom panel Stage 3: the adopter default flip

**Revision 4.** Status: **IMPLEMENTED — all six branch-plan steps done;
Arc 7 complete.** Branch `bottom-panel-stage3`, based on
`githubsucks/main` @ `21de0b2`. Full sweep 3449 passed / 0 failed
against a 3447 baseline.

**Revision 3 → 4** records what implementation found. The plan held; the
surprises were all defects the flip exposed rather than design changes:

- **Three defects, each fixed rather than tested around** — the outline's
  raw-switch visit (§1.7a), compile's non-replayed opt-out (§1.7b), and
  a nil-`opts` regression of my own that `journey_acceptance` caught.
- **The census counted failures, not causes** (§1.6d): 13 listview
  failures had ONE root cause, a missing frame-geometry declaration.
- **Compile's chords are panel-local** (§1.5a), pinned by `acc34`.
- **The two `q` mechanisms are complementary** (§1.7c), pinned by
  `s1_12` and the new `s3_1`.

**Revision 2 → 3** adds the measurement and its classification. Two
findings changed the plan rather than confirming it: the census needs
`--no-fail-fast` or it under-reports by an order of magnitude, and
**`m4_acceptance` is a transitive adopter via listview that no table
named**.

**Revision 1 → 2**, all from review and all verified rather than
accepted:

- **Dired is a FOURTH copy of the validator, not merely a fourth
  adopter** — and it must keep `"current"` as its default, because the
  `pmacs .` path reaches it with no `display` at all (§1.1a). Revision 1
  had this as an open question with a leaning; it is now a decision with
  a mechanism.
- **Unify narrowly, not wholesale** (§2 Q#S3-1). A shared
  `resolve_adopter_display(operation, raw, default)` covering vocabulary,
  error text and default policy — with terminal's `window`
  mutual-exclusion staying in its Rust wrapper, because the parsers are
  *not* identical and pretending otherwise is its own defect.
- **A negative criterion is added** (acceptance 9): omitted `display`
  must still mean document placement for direct dired **and** for
  `pmacs .`.
- **The suite fallout is named, not just counted** (§1.6a): two Stage 1
  tests assert the old default deliberately and must be revised
  knowingly.

Stage 3 is **Arc 7's last step**. Stages 1, 2A, 2B-1, 2B-2 and 2B-3 are
all on `main` (#155, #175, #177, #184, #187, #198); the panel is complete
on both frontends and every mechanism this stage needs exists. What
remains is the decision the arc deferred on purpose: **omitting `display`
should mean the panel, not the selected window.**

The parent framing already decided the policy (Q#BP12) and wrote the
acceptance sketch (criteria 56–58). This document re-scouts that plan
against current `main` and records where it has drifted.

---

## 0. Coherence impact (COHERENCE §20)

- **Concern: §14 Coherent Workbench Primitives**, graded *"partial, with
  the best trajectory of any concern."* This closes the panel half of
  P5. §14's bottom-panel bullet names Stage 3 explicitly as what is
  left.
- **§6 Interaction islands — this REMOVES one.** Today every adopter
  that wants a panel must say so at each call site, and three separate
  code paths decide what silence means. After Stage 3 the policy is the
  default and the call sites stop carrying it.
- **Journey steps touched:** none directly, though steps 6 and 9 (LSP
  panels, compile output) change where their output lands.
- **Config registry adoption:** none — Q#BP12 is explicit that this is
  **not a hidden global setting**. It is a resolved default, not a
  preference.
- **Background-work attribution:** none.
- **Enables:** DAP. Q#BP12's adopter table **already contains a
  `DAP stack/variables` row**, so this stage settles the debugger's panel
  policy before the debugger exists. That is the intended order.

---

## 1. Ground truth (measured at `000b6cd`)

### 1.1 The flip is three sites in two languages

The parent framing says *"Stage 3 is not one line per consumer."* True —
but the sharper reason is that **the option was never parsed in one
place.** Each adopter validates and dispatches `display` itself:

| site | adopter | omission today |
|---|---|---|
| `src/lua_bindings/window_panel.rs:269` | `pmacs.terminal.open` | `None \| Some("current") => AdopterPlacement::Current` |
| `builtin/runtime/listview.lua:235` | listview | `if display == "panel" … else switch_buffer` |
| `builtin/runtime/compile.lua:899` | compile | same, plus an `already_in_panel` special case |

A **fourth** copy validates the same vocabulary without being a
default-flip site: `builtin/runtime/dired.lua:645`. It is the one that
must NOT flip (§1.1a).

`parse_adopter_placement` looks like the shared parser its doc comment
implies, but **it has exactly one caller** — the terminal. listview and
compile each re-implement the same three-value validation in Lua,
including their own copy of the error message.

**The three default-resolution branches are the flip; the fourth
validator is not.** `window_panel.rs:269`, `listview.lua:235` and
`compile.lua:899` all move. Changing only the Rust parser would leave
**both** Lua adopters resolving omission to the current window — a
half-flip that would look done and behave inconsistently per adopter.

Four copies of one rule is also how the next adopter gets it subtly
wrong — and the next adopter is **DAP**, already named in Q#BP12's
table. Hence Q#S3-1's narrow unification.

### 1.1a Dired must NOT flip, and the reason is the golden journey

`dired.lua:645` validates the same `"current" | "panel"` vocabulary with
the same error shape, so it is a fourth copy of the rule. **Its default
must stay `"current"`**, and the mechanism is specific rather than
stylistic:

```lua
pmacs.path.set_directory_handler(function(path, dest)
  open_async(path, { dest = dest }, nil, "dired")
end)
```

The `pmacs .` path reaches dired through that slot with **`{ dest =
dest }` and no `display` key at all** — so it resolves by omission. If
dired's default flipped with the others, **`pmacs .` would open the
directory listing in a bottom panel**, which is wrong for journey step
2 and for every subsequent step that navigates from it. Journey Stage
1a made `pmacs .` open a directory at all; putting the result in a
panel would undo the point of it.

This is the difference between an adopter and a surface. listview,
compile and terminal produce *output the user consults*; dired produces
*a document the user works in*, like a buffer. The panel default is
right for the first kind and wrong for the second.

### 1.2 `select` is not cosmetic for listview, and the citation drifted

Q#BP12 requires `select = true` for interactive listview, citing
`listview.lua:64` for `seat_cursor`. **That line is now
`NAME_VARIANT_LIMIT`; `seat_cursor` is at line 130.** The constraint
itself is intact and verified:

```lua
local function seat_cursor(p, line)
  ...
  for _ = 1, target do
    pmacs.editor.move_down()
  end
```

`pmacs.editor.move_down()` acts on the **active window**. A listview
panel displayed without `select` would seat the cursor in whatever
window is selected — the user's document. `listview.refresh` has the
same property. **This is a data-corruption-shaped bug, not a focus
annoyance**, and it is why the table's `select` column differs per
adopter rather than being uniform.

Compile takes `select = false` deliberately (passive output); terminal
takes `select = true`.

### 1.3 Compile has already been prepared for this stage

`compile.lua:894` carries a comment written for Stage 3:

> Gated on OMISSION, never on an explicit value: `display = "current"`
> is the documented user-facing opt-out from the Stage 3 default flip,
> so it must reach the raw switch even when the previous run was
> panel-placed.

and the condition is already
`display == "panel" or (display == nil and already_in_panel(slot.buf))`.
So compile's *recompile* path already keeps a panel-placed buffer in the
panel on omission. **Stage 3 makes the first run behave like the
recompile.** Re-read this comment before editing: it encodes a
distinction (omission vs explicit `"current"`) that the flip must
preserve, and the `already_in_panel` branch may become redundant.

### 1.4 What Stage 3 owes beyond the flip

Q#BP12's table is a per-adopter contract, not a single switch. For each
of listview / compile / terminal:

- **panel placement** with that adopter's `select` value;
- **`dedicated = false`** — a dedicated panel refuses to host anything
  else, and dired Stage 1 already established that `display_buffer` will
  not replace a buffer in a slot dedicated to another (Q#BP3 2.iii);
- **quit action**: delete the panel if this adopter created it, restore
  the panel it replaced otherwise;
- **visit path onto `display_file` / `display_target`** with `select`
  per the table.

`display_file` already exists and is used by dired (`dired.lua:831`,
`:840`) and `default.lua:729`, so the visit half has a proven caller
shape to copy.

### 1.5 Capability fallback still applies, and must be re-proved

The Stage 3 default resolves as a *panel request*, so it passes through
Q#BP13 capability fallback exactly as an explicit `"panel"` does. On a
pre-panel semantic frontend the request degrades, and criterion 57
requires that the degraded path leave **no side parameters and no quit
action on the document window**.

This is the criterion most likely to be quietly wrong, because the
fallback is invisible from the adopter's side.

### 1.6 What is NOT established

- **The blast radius is MEASURED — §1.6b: 37 failures across 5 suites**,
  classified per test in §1.6c. Stage 1 shipped the mechanism opt-in
  precisely so existing suites kept their meaning; flipping the default
  changes where output lands for every suite that exercises listview,
  compile or terminal **without** passing `display`.
- **Steps 1 and 2 of §7 are done** (`0224c68`, `a2f4411`). The flip
  itself, the test revisions, and the capability-fallback criterion are
  not.
- ~~Interaction with dired.~~ **DECIDED — §1.1a and Q#S3-2: dired keeps
  `"current"`**, passed explicitly to the shared resolver so the
  exemption is visible at its call site.

---

### 1.6a Two Stage 1 tests assert the OLD default deliberately

These are not collateral damage; they encode intent and must be revised
knowingly.

- **`tests/bottom_panel_stage1_acceptance.rs:1223`** —
  `acc19_adopters_place_side_affinely_through_real_entry_points` opens a
  listview with **no `display`** specifically to seed the panel buffer
  into a DOCUMENT window first, *"so side-affine placement cannot be
  vacuous."* After the flip that setup no longer produces a document
  window, and the test's own anti-vacuity guarantee is what breaks. It
  needs a new way to seed, not a `display = "current"` bolted on.
- **`tests/bottom_panel_stage1_acceptance.rs:1308`** —
  `acc19b_recompile_reuses_the_panel_instead_of_duplicating_into_the_document`
  is built entirely around a recompile reaching `start_run` with no
  `display`. Its subject survives the flip, but its mechanism (`§1.3`'s
  `already_in_panel` gate) may not — see Q#S3-3.

**The rule for the fallout sweep:** a test whose *subject* is placement
must assert the **new** default. A test whose subject is compile or
terminal behaviour opts out with `display = "current"` **only when its
setup genuinely requires the document window**. Mass-adding the opt-out
to make a suite green converts a behavioural change into an invisible
one, which is the failure this stage's inverted ordering exists to
avoid.

### 1.5a Compile's chords become PANEL-LOCAL — a deliberate contract

**Discovered while revising the fallout, and decided rather than
absorbed.** Every compile-mode chord is bound
`scope = "buffer", buffer = slot.buf` (`compile.lua:221`): `RET`, `n`,
`p`, `q`, **`C-c C-k`**, `g`, and the seven undo no-ops. They dispatch
only when `*compilation*` is the focused buffer.

Before Stage 3 compile switched in place, so the user was *in* that
buffer and the chords worked. **Stage 3 keeps `select = false`** — Q#BP12
is explicit, and passive build output stealing focus mid-edit would be
worse than the alternative — so the user stays in their document and
none of those chords reach compile-mode without focusing the panel
first.

**The contract, stated so it is deliberate rather than an accidental
reachability loss:**

1. A default `compile.run` opens **passively**; document focus remains.
2. **Buffer-local compile chords require focusing the panel** (`C-x o`,
   or a click).
3. **The capability is not lost.** `compile.kill` is reachable through
   `M-x` from anywhere, because its body is
   `slot_for_buffer(pmacs.window.buffer()) or compile_slot()` — the
   `or` arm falls back to the current compilation slot precisely when
   the caller is not in a compile buffer (`compile.lua:1124`). Verified,
   not assumed; it is what makes panel-local chords acceptable rather
   than a lost feature.

**A global `C-c C-k` is deliberately NOT part of this stage.** It is a
command-surface decision — which chords earn global scope — and it would
ride in on a placement flip without its own reasoning. Framed
separately or not at all.

Pinned by a default-placement test asserting `C-c C-k` from the document
window does **not** dispatch to compile (acceptance 10), so a future
change that quietly makes it global has to change a test that says why
it was not.

### 1.6b The fallout census — MEASURED, and it found an adopter nobody named

Taken as branch-plan step 1: record a baseline, apply the three-site
flip as a throwaway edit, sweep, revert. The flip is **not** in the
tree; only this table survives it.

**A census that stops at the first failing binary is not a census.**
The first sweep reported **2 failures in 1 suite** and looked
comfortingly small — `cargo test` halts after a failing test binary, so
everything alphabetically past `bottom_panel_stage1_acceptance` never
ran. With `--no-fail-fast` the real figure is **37 failures across 5
suites**. Any future re-measurement must pass that flag or it will
under-report by an order of magnitude, in the same shape as this arc's
other silent-success traps.

| suite | base → flipped | failures | share of suite |
|---|---|---:|---:|
| `listview_acceptance` | 17 → 4 | **13** | **76%** |
| `compile_mode_acceptance` | 72 → 55 | **17** | 24% |
| `vterm_stage2_acceptance` | 6 → 3 | **3** | 50% |
| `bottom_panel_stage1_acceptance` | 46 → 44 | **2** | 4% |
| `m4_acceptance` | 149 → 147 | **2** | 1% |
| | | **37** | |

**`m4_acceptance` was predicted by nobody** — not the parent framing,
not Q#BP12's adopter table, not revision 1 of this document. Its two
failures are `hover_doc_panel_shows_full_contents_via_binding` and
`outline_panel_opens_visits_and_restores`: **the LSP panels are
listview consumers**, so flipping listview's default reaches the LSP
suite transitively.

This materially improves the adopter map. Q#BP12 lists four rows
(listview, compile, terminal, DAP) as though they were the population.
They are the *direct* population; **the real one includes everything
built on listview**, and the LSP hover/outline panels are the proof.
Anything added on listview later inherits the panel default without
appearing in any table — which is the intended behaviour, but only if
the map says so.

**The proportions invert the obvious reading.** `compile_mode` has the
most failures and the least placement content: `acc01_spawn_streams…`,
`acc05_kill_reaps_backgrounded_descendant`,
`acc21_kill_produces_signaled_marker` and the `r1f*`/`r5f*`/`r6f*` group
are process-lifecycle and styling tests that merely *use* compile and
now find its output elsewhere. `listview_acceptance` has fewer failures
but loses **three quarters of its suite**, and its failures
(`open_seats_cursor_and_ret_visits_the_row`, `panel_rejects_typing`,
`dispatch_idle_is_false_while_a_panel_is_focused`) are placement in
substance.

### 1.6c Classification, decided

Applying §1.6a's rule to the measured set. **Placement-subject tests
assert the NEW default; only genuine document-window setups opt out.**

| test | classification |
|---|---|
| `bottom_panel_stage1::acc19` | **placement** — needs a new way to seed a document window (§1.6a) |
| `bottom_panel_stage1::acc19b` | **placement** — subject survives; mechanism may not (Q#S3-3) |
| `m4::outline_panel_opens_visits_and_restores` | **placement** — *almost a direct realization of criterion 58*: open → visit → jump-back → quit. Assert the panel/document split |
| `m4::hover_doc_panel_shows_full_contents_via_binding` | **placement** — a listview panel lifecycle test. Keep the omitted default; assert the new placement **plus** its existing content and quit guarantees |
| `compile::acc15_ret_visits_error_and_jump_back_returns` | **placement** — Q#BP12 explicitly requires compilation panel → RET source → `M-,` back to the still-present panel with the document window intact |
| `compile::acc16_n_p_walk_error_lines_without_wrap` | **opt out**, explicit `display = "current"` — its subject is cursor navigation *within* compile output, which genuinely needs the compilation buffer selected |
| `listview_acceptance` ×13 | **placement**, predominantly — assert the new default |
| remaining `compile_mode` ×15 | **incidental** — process lifecycle and styling; opt out only where the setup needs the document window |
| `vterm_stage2` ×3 | **to classify at implementation** — terminal's `select = true` makes these likelier placement than incidental |

The `acc15` / `acc16` split is the one worth remembering: **two
neighbouring tests in the same suite land on opposite sides**, because
one is about where output goes and the other is about moving within it.
A sweep that classified per *suite* rather than per *test* would have
got both wrong.


### 1.6d The census counted failures, not causes

The measured 37 was accurate as a count and misleading as a work
estimate. **Thirteen listview failures had one root cause:** a panel is
derived-hidden while frame geometry is unknown, and
`listview_acceptance` never declared any — it never needed to while
listview defaulted to the current window. One helper took it from 13 to
2. `m4_acceptance` and `vterm_stage2_acceptance` were the same.

Read a census as "how many assertions move", never "how many decisions
are required". The two differed here by an order of magnitude.

### 1.7a Defect: the outline visited through the RAW switch

`lsp.lua`'s outline `on_visit` called `pmacs.window.switch_buffer`,
which replaces the buffer in the **active** window. Harmless while the
outline opened into a document window — the switch simply reused it.
Once the panel became the default, the active window WAS the outline
panel, so **RET clobbered the panel with the source file** and left
nothing for `M-,` to return to.

The references panel (`visit_location`) was migrated to `display_file`
when the arc landed; the outline was missed because **nothing exercised
it from a panel until the default flipped**. Its own neighbouring
comment states the rule it violated: *"a visit FROM a panel must land in
the document target and leave the panel intact."*

Q#BP11c names the corruption precisely, and it is why both the outline
and compile tests now assert `M-,` **focuses** the still-present panel
rather than cloning its buffer into a document window — the previous
assertion, on the active buffer name alone, could not tell those apart.

### 1.7b Defect: an opt-out that did not survive replay

`pmacs.compile._last` stored `{cmdline, cwd}` and no `display`, so `g`
reached `start_run` with the value omitted and took the new default. A
user who ran `compile.run{display="current"}` was moved into a panel the
moment they recompiled.

**An opt-out that reverts on the next replay is not an opt-out.**
`display` is now stored and replayed, with `nil` kept as `nil` so an
omitted value still resolves to the current default rather than freezing
at the first run's resolution. The general form: *anything that replays
a stored invocation must store the escape hatch alongside it.*

### 1.7c The two `q` mechanisms are complementary

Stage 3 made a latent conflict live. `listview.lua`'s `p.prev` captures
the previous buffer only if it is not a panel; `QuitAction::Restore`
(Q#BP2c) deliberately chains `C → B → A → delete`. With two listviews
sharing one bottom slot, the second replaces the first and `q` walks the
restore chain.

**The restore chain wins**, per criterion 20 — listview `q` routes
through `window.quit` and must not exempt itself from the panel
contract. The mechanisms are then complementary rather than competing:
**presentation history chains in the side slot; `p.prev` prevents
raw-switch and capability-fallback loops.** `s1_12` keeps its Q#GB18
name-keyed-identity bite by pinning its panels in document windows,
isolating `p.prev`; the new `s3_1` pins the chain.

## 2. Questions

- **Q#S3-1 — DECIDED: unify NARROWLY.** A shared
  `resolve_adopter_display(operation, raw, default)` owns exactly three
  things: the **vocabulary**, the **error text**, and the **default
  policy**. Its four callers are listview, compile, terminal and dired —
  the last passing `default = "current"`, which is what makes dired's
  exemption a parameter rather than a divergent copy.

  **Terminal's `window` mutual-exclusion stays in its Rust wrapper.**
  The parsers are *not* identical, and a helper that pretended otherwise
  would be its own defect: only terminal accepts a `window` id, and only
  it must reject `window` combined with `display = "panel"`.

  **One normalization must be named rather than absorbed.** Terminal
  reads `spec_table.get::<Option<String>>("display")?`, so a non-string
  value raises **mlua's type error before** the custom "unknown display"
  message is ever reached. The Lua callers instead `tostring()` whatever
  they got and report it inside their own error. These are different
  observable behaviours for the same bad input, and unifying the error
  text without deciding this would silently change one of them.

  **RESOLVED and PINNED at step 2.** The custom error wins, because it
  names the legal vocabulary where mlua's type error does not.
  Non-strings render by **type alone** (`unknown display (integer)`),
  never quoted, so the message cannot imply a string was passed. Pinned
  in `bottom_panel_stage1_acceptance::acc19` at the **terminal** entry
  point — the one adopter whose behaviour actually changed — asserting
  the shared error *and* that nothing is created, exactly as the
  unknown-string case already does.

  **The type SPELLING is deliberately not pinned.** Lua 5.4 reports
  `integer`; LuaJIT has no integer subtype. Asserting either literal
  would pass on one CI flavor and fail on the other, so the test pins
  the shape (`unknown display (`, the vocabulary, and the *absence* of a
  quoted value). Verified 46/46 under both flavors.

  Consequently step 2 is **default-preserving with one intentional
  normalization**, not "behaviour-preserving": every adopter kept its
  default, but invalid-input behaviour moved on purpose.
- **Q#S3-2 — DECIDED: dired does not flip.** It keeps `"current"` as
  its default, expressed as the `default` argument to the shared
  resolver so the exemption is visible at the call site rather than
  implied by a fourth copy of the parser. §1.1a records the mechanism —
  the `pmacs .` handler passes no `display` — and acceptance 9 pins it
  from both entry points.
- **Q#S3-3 — does `already_in_panel` survive?** Once omission means
  panel, compile's special case may be dead code. **Leaning: measure,
  then delete if dead** — but check the explicit-`"current"` path first,
  because that arm is what the comment says the gate protects.
- **Q#S3-4 — how many existing suites move?** Unknown (§1.6). This must
  be measured **before** the flip, so the diff to acceptance files can be
  read as intended-vs-collateral rather than discovered afterwards.
- **Q#S3-5 — is there a user-facing escape beyond per-call `display`?**
  Q#BP12 says this is deliberately not a setting. Someone who dislikes
  panels has no global opt-out, only per-call `display = "current"`,
  which they do not control for builtin commands. **Leaning: accept for
  this stage and record it**, since a setting is §11 work and panel
  persistence is already blocked on settings persistence.

---

## 3. Bets

- **Bet 1 — the flip itself is small; the suite churn is the work.**
  Three dispatch sites, each a few lines. The cost is criterion 58's
  per-adopter open→visit→return→quit suites plus whatever §1.6's
  measurement turns up.
- **Bet 2 — the capability-fallback criterion (57) is where a defect
  hides.** It is invisible from the adopter side and only observable on a
  pre-panel semantic frontend.
- **Bet 3 — `select` gets one adopter wrong.** The values differ per
  adopter for real reasons (§1.2), and a uniform `select = true` would
  look correct and break compile's passive-output behaviour.

---

## 4. Acceptance

Inherits the parent framing's criteria 56–58, made concrete:

1. Omitting `display` from listview, compile and terminal entry points
   resolves to the Q#BP12 panel/select policy on a panel-capable grid
   **and** semantic frontend.
2. Explicit `display = "current"` preserves each adopter's pre-arc
   selected-window behaviour, including compile's raw-switch path.
3. **Per-adopter `select` matches the table** — listview `true`,
   compile `false`, terminal `true` — asserted individually, not by a
   shared helper that would pass with a uniform value.
4. An interactive listview panel seats its cursor **in the panel**, not
   in the previously selected window (§1.2's real failure mode).
5. On a pre-panel semantic frontend the omitted default takes capability
   fallback with **no side parameters and no quit action** left on the
   document window; visit and `q` remain the existing non-side paths.
6. Per-adopter open→visit→return→quit suites, not a generic helper
   (criterion 58), preserving Stage 1's unknown-value rollback
   assertions.
7. **The unknown-`display` error still fires before anything is
   created** — buffer, session, process or wrapper — for all three
   adopters, whichever parser survives Q#S3-1.
8. The count of existing suites whose behaviour changes is **stated**,
   and each change is classified intended or collateral — **not
   silenced by mass-adding `display = "current"`** (§1.6a).
9. **NEGATIVE criterion — compile's chords are panel-local.** With the
   default placement, `C-c C-k` pressed in the **document** window does
   not dispatch to compile (§1.5a). Tests whose subject is
   compile-BUFFER behaviour opt out with `display = "current"` and say
   why. `M-x compile.kill` still works from anywhere.
10. **NEGATIVE criterion — omission still means the document for dired.**
   Both entry points are pinned: a direct `pmacs.dired.open(path)` with
   no `display`, **and** the `pmacs .` launch path through
   `pmacs.path.directory_handler`. Neither may place into a panel. This
   is the criterion that would catch a well-intentioned "make all four
   consistent" change, and it guards journey step 2.

---

## 5. Parked

- Everything in the parent framing's §6 "Deferred (named)" — left/right/
  top side windows, multiple slots, `no_other_window`, manual
  hide/show, `display-buffer-alist`-style user rules, panel persistence
  (blocked on settings persistence), GPU document splits.
- **A global panel preference** (Q#S3-5) — §11 work.
- **The tree primitive.** Not part of this stage, and the next thing to
  scope: §14 grades Tree ✗, and DAP's variables view is its next
  would-be inventor.

---

## 6. Gates

The standing `CLAUDE.md` suite, with the touched acceptance suites being
at minimum `bottom_panel_stage1_acceptance`, `bottom_panel_stage2a`,
both `stage2b` daemon/GPU suites, plus the listview, compile-mode and
terminal suites — the last three are where §1.6's unmeasured churn will
land.

---

## 7. Branch plan

One branch, `bottom-panel-stage3`:

1. **Measure first** (Q#S3-4): run the full suite with the flip applied
   as a throwaway edit, record which suites move, revert. *The
   measurement is the first commit's evidence, not the flip.* Classify
   each mover per §1.6a's rule before writing a line of the fix.
2. **Land `resolve_adopter_display`** (Q#S3-1) with all four callers
   still passing their CURRENT defaults, so nothing flips yet. It is
   **default-preserving with one intentional normalization**, not
   "behaviour-preserving" — every adopter keeps its default and the
   suite is byte-identical to baseline, but terminal's invalid-input
   behaviour moves deliberately (§1.6b). Decide and pin that
   normalization here, since no existing assertion can catch it.
3. **Flip the three sites** by changing only the `default` argument at
   listview, compile and terminal — dired keeps `"current"` — with
   per-adopter `select`.
4. **Per-adopter open→visit→return→quit suites** (criterion 6).
5. **Capability fallback** (criterion 5), the one needing a semantic
   frontend.
6. **Update the lane and handoff**; Arc 7 closes.

Step 1 before step 3 is the point: flipping first and reading the
fallout as it appears makes intended and collateral changes
indistinguishable.
