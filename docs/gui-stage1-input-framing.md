# GUI arc, Stage 1 — input foundation (framing)

**Status: revision 12 — APPROVED.** Revision 12 is §2's ground-truth
re-measurement for Stage 1a and changes no ruling; it carries two
corrections to claims that were wrong at the original anchor too.

**Previously, revision 11 — APPROVED.** Revisions 1–8 rejected; revision 9
is the approved design. Revision 10 recorded a scope correction found
against the 1-pre implementation and **also made a claim about P2 that
review overturned; revision 11 retracts it and P2 is implemented as
written** (§6). **Q#S1-8, Q#S1-9 and Q#S1-10 are RULED.** **1-pre is
IMPLEMENTED**; 1a onward may begin from this document.

**v26, not v25 — corrected by the panel mapping-generation slice.**
That slice (`docs/bottom-panel-framing.md` §5b) takes **v25** for
`PanelFramePayload::PresentMapped` / `FrontendEvent::PanelPointerMapped`,
and it lands ahead of 1e because panel-pointer replay blocks 1b.
Protocol slices stay serialized; one was inserted in front.
`ADVERTISED_PROTOCOL_VERSION` remains pinned at **20**.

**Verification base:** §2 is **re-measured at `4f77491`** (2026-08-12),
the tip after 1-pre; it was originally taken at `a994f37`. Sections
other than §2 were written against `a994f37` and their *rulings* are
unaffected by 1-pre, which changed no behaviour — but **any line number
outside §2 predates 1-pre and should be re-checked before it is relied
on.**

## 1. What this stage closes

Journey **step 5**. **Not step 12** (Stage 4b, P2-gated). Five of nine
§3.1 blockers die here.

## 2. Ground truth — RE-MEASURED at `4f77491` (2026-08-12)

**Originally taken at `a994f37`, before 1-pre.** 1-pre (#237) moved
almost every GPU-side coordinate below, so the section is re-measured
rather than left to rot — **a framing whose ground truth points at the
wrong lines is how an implementation ends up arguing with the tree**.
Two claims were *wrong at both anchors* and are corrected, not merely
renumbered; they are marked **CORRECTION**.

### Still true, re-checked

- **`FrontendEvent`: sixteen variants**, none carrying an open path or
  command invocation. **`PROTOCOL_VERSION = 23`.**
- **`WindowEvent::Ime` is ignored entirely** —
  `set_ime_allowed`/`WindowEvent::Ime` still **zero occurrences**, so
  `Ime::Commit(String)` produces nothing. (1d's D1.)
- **TUI wheel arms unmoved**: `EditorState::dispatch_mouse`
  (`src/editor.rs:3052`), `ScrollUp`/`ScrollDown` at **`:3203`** — 1-pre
  touched only `pmacs-gpu`.
- **The handshake precedes any window**: the client is constructed and
  the handshake done before `run_app` (connect `main.rs:702`, `run_app`
  `main.rs:733`; the old citation `:696` was the enclosing block).
- **1c is producer-side only for Focus/Detach** — those variants exist
  on the wire. **Title, Bell and `Goodbye` are GPU consumer work.**
- **`Outbox::enqueue` returns `false` once closed** and **coalesces by
  kind** (`attach.rs:415`; the old `:414` was off by one at both
  anchors).

### Moved by 1-pre

- **`App::window_event` is `main.rs:4450` and is FOUR lines**, not 655
  at `:2734`. It calls `dispatch_window_event` and performs the exit;
  **routing lives in `route_event`, and the bodies in seven `apply_*`
  methods.**
- **"Eight arms handled, the rest fall to `_`" is now three family
  decision functions** — `route_lifecycle`, `route_keyboard` (plus
  `route_key_action`), `route_pointer` — over **nine** named
  `WindowEvent` variants, with `Route::Unrouted` as the wildcard. **1a
  edits `apply_keyboard` and `translate_key`, not `window_event`.**
- **`translate_key(logical: &Key, …)` is `main.rs:12053`**, not
  `:10975`. It still reads the **logical key** and still truncates via
  `chars().next()`, and `_ => return None` is still there — so **A1's
  witness holds**.

### CORRECTION 1 — `KeyEvent.text` IS read, and always was

The original section said *"`KeyEvent.text` is never read."* **That is
false, and was false at `a994f37` too** (`:2800` there, `main.rs:3251`
now): the AltGr rule reads it, as `is_layout_text(key.text.as_deref(),
pmods)`.

The claim the section meant, and which is true: **`KeyEvent.text` is
never read as the text a keypress INSERTS.** It is consulted only as a
*discriminator* — Ctrl+Alt plus printable text means AltGr rather than a
command chord — and the inserted character always comes from
`translate_key`'s logical key, truncated to one scalar.

**This matters to A5, not just to accuracy.** §5's rule 2 exempts
"printable Ctrl+Alt recognized by the existing AltGr rule", so the
precedence table already depends on the code the section claimed did not
exist. **1a widens `text` from a discriminator to a payload**, and that
is the actual change of kind — stating it as "text is never read" hides
the one place the new payload must not disturb.

### CORRECTION 2 — A4's exit site

A4's witness cited *"exits (`main.rs:2771`)"*. 1-pre moved the mechanism
without changing the behaviour: an idle Escape still exits, but
`apply_keyboard` (`main.rs:3219`) now returns `EventOutcome::Exit` and
**`window_event` (`main.rs:4452`) performs the only executable
`event_loop.exit()` in the crate.**

**A4 therefore deletes a branch in `apply_keyboard` and changes its
return type — it does not touch `window_event`.** And **`EventOutcome`
survives A4**: a native close still returns `Exit`, and
`dispatch_window_event` must still distinguish it from `Continue`.

**Today's two failures are unchanged by any of this:** multi-scalar
keyboard input is truncated to its first scalar, and an IME commit
produces nothing.

## 3. PR topology

`1-pre` → `1a`\* → `1b` → `1c` → `1d` → `1e`\*  (\* `--protocol`)

**1c is NOT protocol-bearing** under Q#S1-8's ruling. Protocol slices
are serialized.

## 4. Q#S1-8 — RULED: (A), preserve pre-window readiness

`AttachRequest.initial_size` for a semantic session is a **named,
provisional `SEMANTIC_BOOTSTRAP_GRID` of 24×80**. It is **not measured
geometry** and **must never become semantic frame or panel authority**.
**`FrontendCellGeometry`, sent after window creation, is the sole real
frame declaration.**

This **codifies current daemon behaviour**, so **1c stays
non-protocol-bearing** — unless implementation changes that behaviour,
which would be a wire-contract change even with no bytes moved.

## 5. Q#S1-9 — RULED: `TextInput` precedence

**A `KeyboardInput` stays `Key` unless a rule below moves it.**

1. **Named keys and control text remain `Key`**, regardless of
   `KeyEvent.text` — so `Enter`'s `"\r"` never becomes text.
2. **Ctrl/Alt chords remain `Key`**, except **printable Ctrl+Alt
   recognized by the existing AltGr rule**.
3. **Meta/Super-only text stays reserved to the OS.**
4. **Plain printable SINGLE-scalar remains `Key`** — preserving mode
   keymaps and today's typed provenance.
5. **Printable MULTI-scalar becomes one `TextInput`.**
6. **Every non-empty `Ime::Commit` becomes one `TextInput`**, even
   single-scalar.
7. **`Key::Dead` is 1d-owned; 1a buffers nothing.**
8. **`Shift` is already reflected in resolved text** and is **not**
   carried on `TextInput`.

**Provenance and chain.** A **single-scalar** `TextInput` rotates to
`buffer.self-insert` and creates **today's one-codepoint typed
provenance**. A **multi-scalar** `TextInput` **breaks the command chain
and creates no typed provenance**. Both are **one edit, one undo unit,
one hook, one eligible CRDT op**.

**Modal precedence is preserved:** terminals take **raw UTF-8**;
search and minibuffer **consume** text; menu and query-replace **retain
their shadow behaviour**; **only the ordinary document path performs the
atomic edit**.

**Payload cap: 64 KiB UTF-8, oversize REJECTED, never truncated.**

## 6. Evidence

**The promise, corrected.** Revision 4 claimed every mutation fails only
its own clause. That is **false and cannot be made true**: clauses have
real dependencies — D1 gates every 1d row, D3's failure surfaces at A6,
and **E0 and E2–E6 all presuppose E1** (no transport, no receiver). The promise is therefore
**dependency-aware, and split by clause kind** — revision 5 still said
"every clause fails today", which C6 falsifies by design:

- **CHANGE clauses** have a witness that **fails today** and a mutation
  that fails **at least** its own clause.
- **PRESERVATION clauses** (marked **[P]**) **pass today**; their
  witness pins behaviour that must not regress, and their mutation is
  the change that would break it.
- Where a mutation necessarily breaks dependents, **the dependency is
  named**.

P3 remains an accepted structural exception: not headlessly testable.

### 1-pre

| # | Contract | Witness (fails today because) | Mutation |
|---|---|---|---|
| P1 | Every handled family routes through an extracted function | no extracted functions exist | misroute one family → that family's row |
| P2 | Harness records outbound events **and local effects** (exit, redraw, resize, state mutation) | no harness | record outbound only → exit/redraw rows |
| P3 | `window_event` is a thin call-through | — | **structural/code-review invariant; not testable headlessly** (no `ActiveEventLoop`) |

**Revision 10 — one finding against the implementation, not the
design.** The 1-pre seam is built and the table above holds, with one
scope correction that could not be seen from the design. *(Revision 10
also argued P2 was satisfied by classification alone. It is not — see
revision 11 at the end of this section.)*

**P1 has a SECOND structural exception, and it is winit's rather than
this seam's.** `KeyEvent` carries a `pub(crate) platform_specific`
field (`winit-0.30.13/src/event.rs:655`), so **no
`WindowEvent::KeyboardInput` can be constructed outside winit** and no
headless test can feed one to the router. P1's mutation — misroute a
family, fail that family's row — is therefore unavailable for the
**keyboard** family alone.

Three things bound it, so it is a measured exception rather than a
blanket one:

- **The exception does not extend to the pointer families.** Winit
  provides `DeviceId::dummy()` for exactly this purpose, and
  `CursorMoved` / `MouseInput` / `MouseWheel` are constructible. Checked
  before the exception was written down; all three are witnessed.
- **What stays unwitnessed is one pattern arm with no logic in it.** The
  family's only decision — a press is acted on, a release is claimed and
  discarded — is factored into `route_key_action(ElementState) ->
  KeyAction`, which takes a constructible argument and is witnessed
  directly, with both misroute mutations failing that row alone.
- **P3 is now measured, not assumed.** Replacing `window_event`'s entire
  body with `let _ = (event_loop, event);` — a GUI that responds to no
  input at all — leaves **every `pmacs-gpu` test green**. That is the
  exception's true extent: no headless test anywhere in the crate
  observes the delegation. **Re-measured at the current shape after
  revision 11: 265/265 under `PMACS_REQUIRE_GPU=1`** (it was 256 before
  the effect rows existed, and the number is re-run rather than carried
  forward). Revision 11 also shrinks what the exception *covers*:
  `window_event` is four lines, so the unwitnessed residue is one `if`
  rather than a 33-line match.

**Revision 11 — P2 IS IMPLEMENTED AS WRITTEN. Revision 10's argument
here was wrong and is retracted.**

Revision 10 claimed a route-classification transcript covered both
halves of P2 because a route "names its local effect". **The wheel
falsifies that.** A wheel route carries a delta; whether that delta
becomes a viewport update, a panel event, a terminal event or nothing at
all depends on `State`. The route names the *family*, and only running
the body names the *effect* — so classification could not have
satisfied P2, and arguing that it did was a narrowing wearing the
costume of a mechanism.

P2 now has a second harness beside the routing one:

- **`EffectHarness` drives production end to end** — a real
  `AttachClient` over a `socketpair` (real handshake, outbox, writer
  thread, encoder, so the transcript is the wire), a real windowless
  `State`, and `App::dispatch_window_event`.
- **`App::dispatch_window_event` is what made this reachable.** Left
  inside `window_event`, the dispatch would force a harness to
  re-implement it, and a harness that re-implements what it tests
  witnesses its own copy. **P3 therefore narrows from a 33-line match to
  a single `if`**: `window_event` is now `call dispatch, exit if it
  asks`.
- **Local effects are read where they land**: exit from the returned
  `EventOutcome`, redraw from a test-only `render_calls`, resize from
  the surface config, modifiers from `App`, scroll from `scroll_top`.
- **Steps are delimited by a non-coalesceable sentinel key, not a
  sleep** — "this step sent nothing" is otherwise undecidable without
  waiting, and a fixed-duration wait against a writer thread is the
  core-count assumption of PR #235's CI red.
- **The rows never skip.** A missing wgpu adapter is an assertion
  failure; mutation M21 confirms all nine effect rows fail loudly while
  the thirteen GPU-free routing rows stay green.

`M22` (blind to outbound) and `M23` (blind to local) fail rows in both
directions, which is P2's contract executable rather than asserted.

The routing rows stay, and the division of labour is deliberate: the
routing harness answers *where did this event go*, the effect harness
answers *what did it do*. **P2 is owned by the effect rows.** The
routing transcript row is only the sole owner of the *routing*
harness's own recording, which is what keeps that one mutation
surgical — revision 10 claimed it owned P2, and it never did.

**One further correction revision 11 carries, and it is the design
consequence 1a inherits.** Revision 10 stated that Stage 1a's A4 would
leave `EventOutcome` with a single variant and that the type should go
with the Escape branch. **Both are wrong.**

`EventOutcome` has **two producers today**: `LifecycleRoute::Exit`, a
native window close that must always exit, and `apply_keyboard`'s idle
Escape, a local quit. **A4 removes the keyboard one**, leaving **exactly
one `Exit` producer** — the native close.

**One producer is not one variant.** The type survives because
`dispatch_window_event` still has to distinguish `Continue` from
`Exit` on every event it handles: the overwhelming majority of
dispatches must *not* exit, and the native close must. What A4 actually
removes is `apply_keyboard`'s need to return an outcome at all, which is
a change to that one signature rather than to this type.

The crate has **exactly one** executable `event_loop.exit()`, in
`window_event`.

### 1a — `TextInput` (v24)

| # | Contract | Witness (fails today because) | Mutation |
|---|---|---|---|
| A1 | `F1`–`F35` → `F(1..=35)` | `_ => return None` | map `F13+` → `None` → F13–F35 rows |
| A2 | Shift+Tab → `BackTab` with `Shift` set | produces `Tab` | drop `Shift` → A2 only |
| A3 | `ContextMenu` → `Menu` | produces nothing | map to `Char('\0')` → A3 only |
| A4 | Idle Escape reaches the daemon, never exits | exits — `apply_keyboard` (`main.rs:3219`) returns `EventOutcome::Exit`, performed at `main.rs:4452` | restore the quit branch → A4 only |
| A5 | Precedence per §5 (1–8) | multi-scalar truncated; IME ignored | move rule 1 (control text → text) → the `Enter`-in-dired row |
| A6 | One commit = one edit, undo unit, hook, eligible CRDT op | commit truncated to one scalar | one edit per scalar → undo-unit row (**and D3 surfaces here**) |
| A7 | Prompts consume scalars **in order** | multi-scalar never arrives | reverse order → A7's prompt transcript |
| A8 | Terminals get **raw UTF-8, never bracketed paste** | multi-scalar never arrives | route via `Paste` → terminal row shows bracket markers |
| A9 | **64 KiB cap; oversize rejected, not truncated** | no cap exists | truncate instead → the oversize row observes silent loss |

### 1b — pointer and scroll

| # | Contract | Witness | Mutation |
|---|---|---|---|
| B1 | Residual per **axis and surface** | deltas discarded | share one accumulator → surface-switch jump |
| B2 | Wheel-right raises leftmost column; wheel-down raises top line | `x` discarded | invert a sign → that axis's row |
| B3 | Clamps at content bounds; never a negative origin | no horizontal scroll to clamp | remove clamp → **at-bounds row: origin goes negative and the view blanks** |
| B4 | Middle-click paste uses **PRIMARY on Linux** | no middle-click path | use `CLIPBOARD` → B4 only |
| B5 | I-beam over text content only | no I-beam | extend over the gutter → B5 only |
| B6 | Wheel over the minimap scrolls the **document viewport** with **its own residual accumulator**; click/drag remains scrub | **a FULL tick already scrolls today** — minimap pixels are `Elsewhere` (`main.rs:2061`) and the wheel falls through to `scroll_by_lines` (`main.rs:3373`). What fails is **fractional accumulation**, and **residual ownership distinct from the document's**: sub-tick minimap deltas are discarded, and a **surface-switch fractional witness** (part-tick over the minimap, then over the document) must not carry residue across | share the document's accumulator → the surface-switch fractional row jumps |
| B7 | **TUI horizontal**: **three columns per wheel tick**, sign per B2; **left origin clamps at 0 and at (widest display-line width − text viewport width), SATURATING AT ZERO**; **wrap pins the origin to 0** | events arrive at `:3203` and are dropped | step of one → the three-column row; clamp at the widest line's **full width** → **the right-bound row blanks the viewport**; drop the wrap guard → the wrap row scrolls a wrapped buffer sideways |

**B7's right bound corrected.** Clamping at the widest line's full width
lets the origin pass every glyph and leave the viewport **entirely
blank**. The bound is *width − viewport*, saturating at zero for buffers
narrower than the viewport, and **the right-bound witness asserts the
final display column is still visible**.

**Why B6 changed — and revision 5's reason was wrong.** Scrubbing on
wheel is not *impossible*: the wheel handler already reads the cached
`state.pointer_pos` for surface routing (`main.rs:3337`), so an absolute
target is available. It is the **wrong semantics**: a wheel is a
**relative** gesture, and mapping relative ticks onto an absolute
position would make one notch jump to wherever the pointer happens to
rest. Click and drag remain scrub because those *are* absolute.

### 1c — session and window signals

| # | Contract | Witness | Mutation |
|---|---|---|---|
| C1 | Title `"<buffer> — pmacs"`, `"pmacs"` when unnamed | title is static | drop the name → C1 only |
| C2 | **Visible bell: 120 ms, WHOLE CLIENT AREA visibly changes**; repeats **neither queue nor extend** the first deadline | `Bell` unconsumed | let repeats extend → the repeat row's flash outlasts 120 ms; flash a sub-region → the headless render witness cannot see it |
| C3 | `Goodbye` names the daemon's reason, else an explicitly **locally-classified** transport/EOF reason; never blank | live-loop reason discarded | blank the fallback → EOF row |
| C4 | `FocusLost` precedes `Detach`; `FocusGained` only after attach completes | `Focused` unhandled | swap → C4 only |
| C5a | **Local DPI correctness**: at scale 1→2 with **unchanged logical size**, logical wrapping, row count and hit testing are **stable** while **physical pixels double** — glyphs, clips, caret, hit tests and overlays all rescale | `scale: 1.0` hardcoded (`main.rs:8950`) | rescale glyphs only → **caret, clip and hit-test rows fail while text still looks right**, which is the bug this splits out |
| C5b | **Geometry declaration**: the epoch advances and `FrontendCellGeometry` is emitted | no `ScaleFactorChanged` arm | suppress the emit → C5b only, **C5a still passing** |
| C6 **[P]** | `initial_size` = `SEMANTIC_BOOTSTRAP_GRID` (24×80), **never** frame or panel authority | **passes today** — this codifies current daemon behaviour; the witness pins that a semantic frame/panel is sized from `FrontendCellGeometry` alone | derive a frame or panel extent from `initial_size` → the panel-authority row |
| C7 | Close contract — §7 | see §7 | see §7 |

**C5 was one row and hid half the defect** — suppressing the wire emit
says nothing about glyphs or hit tests staying at scale 1.

### 1d — IME

| # | Contract | Witness | Mutation |
|---|---|---|---|
| D1 | `set_ime_allowed(true)` | zero occurrences — **no composition arrives at all** | omit → **every 1d row (stated dependency, not a matrix defect)** |
| D2 | Preedit overlay with caret and selection; **indices are BYTE OFFSETS** into the preedit string | no overlay | treat as char indices → multibyte row |
| D3 | `Ime::Commit` emits A5's `TextInput` | commit ignored | emit per-scalar `Key`s → **A6's undo row (named dependency)** |
| D4 | **`set_ime_cursor_area` updated** after caret motion, scroll, resize, font/DPI change, and every preedit change | never called → candidates misplaced | update on caret only → scroll and resize rows |
| D5 | Overlay clears on **empty `Preedit`, `Ime::Disabled`, and focus loss** — all three | no overlay | clear on focus loss only → `Disabled` row leaves stale text |
| D6 | Dead-key state owned here; 1a buffers nothing | dead keys dropped | buffer in 1a → D6 by construction |

### 1e — `OpenTarget` (v26)

| # | Contract | Witness (fails today because) | Mutation |
|---|---|---|---|
| E0 | **SUCCESS**: a **successfully handled valid target** is **resolved, installed, hooked and dispatched through the existing file/directory pipeline**, and the originating frontend receives **exactly one terminal result** — `Opened { request_id, buffer_id }` when a commit lands, **or `Handled` when an extension legitimately claims it** | no receiver — a drop does nothing | dispatch without firing the open hooks → the hook row; settle before the deferred commit lands → the async-directory row (Q#S1-10) |
| E1 | Versioned `OpenTarget { request_id: u64, cwd, path }` carrying `InitialTarget`'s raw shape. **`request_id` is unique among a frontend's OUTSTANDING requests; a duplicate is REJECTED at the protocol boundary** and must never replace or settle the original completion | no variant exists | **omit `request_id`** → the two-concurrent-drops row misattributes; **accept a duplicate id** → the reuse row settles the first drop's completion with the second drop's outcome |
| E2 | Source is **authenticated** | no receiver | accept an unauthenticated sender → E2's forged-source row |
| E3 | Primary document `ViewDestination` captured **immediately on receipt**, before any await | no receiver | capture after the open resolves → **frontend-switch row: the file lands in whichever frontend is ambient** (the #231 defect) |
| E4 | **No window identity trusted from the wire** | no receiver | accept a window id → E4's forged-window row |
| E5 | Failures **before terminal disposition** — including completion-aware `Deferred` work — are **visible to the ORIGINATING frontend** as bounded `Failed { request_id, message }`. After `Handled`, responsibility and any later failure belong to the claiming extension, and no second result is sent — see §8 | no receiver | swallow the error → the permission and embedded-NUL rows |
| E6 | `InitialTarget` limits enforced: **32 KiB per raw path**, **non-empty path**, **absolute non-empty cwd**, **embedded NUL rejected** | no receiver | drop the NUL check → the embedded-NUL row |

**The failure taxonomy was wrong in revision 5.** **A missing path is
NOT a failure**: the resolver deliberately creates an **empty
path-backed buffer** on `NotFound` (`src/editor_core.rs:1177`), which is
how "open a file that does not exist yet" works. **Directories are valid
targets too.** Genuine failures are permission denial, validation
rejection (E6), and open errors that are not `NotFound`. Revision 5
listed "missing-path" and "not-a-directory" rows that would have pinned
the opposite of the intended behaviour.

### Q#S1-10 — RULED: the result is TERMINAL

Directory opening completes **asynchronously**, so `OpenTargetResult`
either waits for the captured-destination commit or merely acknowledges
dispatch. **Ruling: terminal.** The result is sent **when the request
reaches a terminal disposition** — for `Opened`, after the commit
resolves; for `Handled`, at the moment responsibility transfers; for
`Failed`, at the failure, **which for several paths has no commit at
all**. Asynchronous failure is reported through the same result — an acknowledgement-plus-later-channel design would need a
second source-scoped mechanism to carry exactly the failures that matter
most.

```
OpenTargetResult::Opened  { request_id: u64, buffer_id: BufferId }
OpenTargetResult::Handled { request_id: u64 }                  // claimed; no buffer attributable
OpenTargetResult::Failed  { request_id: u64, message: String }  // 4 KiB cap
```

**Terminal completion must be TOTAL over the existing pipeline, and
"after the commit resolves" is not** — three legitimate paths reach
neither a commit nor a result:

- **Claimed**: a `path.open-directory` listener returns `proceed =
  false` and the dispatch **returns without committing**
  (`src/editor.rs:1375`).
- **Disabled**: the `directory_handler` slot is clear, which is a
  supported configuration — the dispatch emits a **status message and no
  commit** (`src/editor.rs:1389`).
- **Asynchronous fallback**: the default handler calls `open_async` and
  **returns immediately** (`builtin/runtime/dired.lua:750`), so the
  commit lands long after the dispatch unwinds.

**Mechanism: a request-scoped ONE-SHOT COMPLETION with an EXPLICIT
STATE MACHINE**, carried through the directory pipeline, settled
**exactly once**, and **discarded on source detach** — a frontend that
left cannot be told anything.

```
Pending ──(defer before scheduling)──▶ Deferred ──▶ Settled
   │                                    │
   └──────────(commit / failure)────────┴──▶ Settled
   │
   └──(source detach, from Pending or Deferred)──▶ Cancelled
```

**`open_async` transitions to `Deferred` BEFORE handing the completion
to scheduled work.** The ownership transfer is explicit and does not
depend on whether today's scheduler starts a coroutine synchronously or
a future scheduler defers its first step. Without the transition the
dispatch unwinds while the completion is still `Pending`, the
end-of-turn fallback fires, and the request is settled *before* the
commit it was waiting for.

**The end-of-turn fallback acts ONLY on `Pending`.** A `Deferred`
completion is owned by the scheduled work. **Any later settlement is
exactly-once**: a second attempt against `Settled` or `Cancelled` is a
no-op, never a second message.

*Mutation:* omit the `Deferred` transition, or delay it until after the
end-of-turn fallback → the async-directory row receives a premature
`Handled`/`Failed`; exactly-once settlement then suppresses the later
`Opened` attempt when the commit lands.

`open_async` carrying the completion to the commit is what makes the
asynchronous path terminal rather than silent.

**Total disposition, so no path can fall through:**

| path | settles as |
|---|---|
| commit lands | `Opened { buffer_id }` |
| listener **claimed** and did not settle | `Handled` |
| **replacement handler** ran and did not settle | `Handled` |
| handler slot **disabled** | `Failed` naming that directory opening is disabled |
| **synchronous error** (listener raised, validation, permission) | `Failed` with the reason |
| pipeline unwinds **still `Pending`** at end of dispatch turn | `Handled` if a listener claimed or a replacement handler ran; **otherwise `Failed`** |
| **source detached** before settlement | discarded — nothing is sent, and the completion is dropped rather than leaked |

**`Handled` exists because `Opened` cannot be honest there.** A claim
means a user listener took responsibility and **no buffer is
attributable**; reporting `Opened` would require inventing a
`buffer_id`, and reporting `Failed` would mislabel a supported
extension point as an error. `Handled` is the terminal responsibility
transfer: any later extension-owned failure uses the extension's own
reporting surface and cannot emit a second `OpenTargetResult`.

*Witness:* one case per **live-source** row, each asserting **exactly
one** result. **The detach row asserts the opposite and must not be
read as "one result":** zero messages sent, **no completion retained**,
and a later settlement attempt **ignored** rather than delivered.
*Mutation:* drop the end-of-turn fallback → the claimed and disabled
rows hang with no result at all, which is the defect this ruling
closes.

`message` uses the **existing 4 KiB error cap**. **Both affected enums
get an independent frozen-byte pin on their own preceding final
variant** — `FrontendEvent` for `OpenTarget`, `InstanceMessage` for
`OpenTargetResult` — because an appended variant's own round-trip cannot
detect a discriminant shift in either.

## 7. The close contract

`send_event` only enqueues (`attach.rs:1145`); the writer takes a batch
and releases the lock before blocking writes (`:671`); **`enqueue`
returns `false` once closed** (`:414`).

**So revision 4's order was literally unexecutable** — closing first
would have rejected the `Detach` it then tried to enqueue.

**Contract — state inspection plus any append/transition is ONE atomic
critical section under a single lock hold, dispatched on the outbox
STATE. The lock is released before notifying the writer, waiting on an
acknowledgement, returning, or shutting down.** Holding it while waiting
would prevent the writer from taking the suffix or transitioning to
`Drained`. The four states are tabulated below; this paragraph is their
normative statement, and revision 6's "append `Detach` /
already-closed → fallback" wording is superseded:

- **Open** → under the lock, append the **complete suffix** (`FocusLost`
  when currently focused, then `Detach`), transition to `Sealed`, and
  capture its acknowledgement handle; release the lock, wake the writer,
  then wait.
- **Sealed** → under the lock, capture the **existing acknowledgement**;
  release the lock, then join it. Do not re-append, do not seal again,
  do not shut down.
- **Drained** → release the lock, then return immediately.
- **Failed** → release the lock, then take the socket-shutdown fallback.

**An ordinary post-seal `send_event` REJECTS and does nothing else** —
it must not invoke shutdown, because a healthy drain is in flight.
**`Detach` is exempt from coalescing.** **Wake the writer after
append-and-seal**, or
a sealed outbox with a pending `Detach` waits on a condvar nobody
signals and the 250 ms bound expires on a daemon that was reading fine.

**The exactly-full case, which revision 5 left undefined.**
`OUTBOX_MAX` is **8192**, and a queue at exactly that length is a
**valid OPEN state**: the *next* ordinary `enqueue` both **sets
`closed`** and **rejects the event** (`attach.rs:427`). So terminal
close must not go through the ordinary path. **Ruling: the terminal operation appends the whole required SUFFIX
atomically** — **`FocusLost` when currently focused, then `Detach`** —
reserving **at most `OUTBOX_MAX + 2`**.

**One slot was not enough, and C4 is why.** C4 requires `FocusLost`
before `Detach`; at exactly `OUTBOX_MAX` an *ordinary* `FocusLost`
enqueue is **rejected and sets `closed`**, so the terminal append would
then find a closed outbox and fall back — losing both events. Revision 6
reserved one slot and so contradicted a contract two sections above it.

**Exact-cap witness:** fill to exactly `OUTBOX_MAX` **while focused**,
then close. The transcript ends **`… FocusLost, Detach`**, the writer is
woken, and acknowledgement precedes exit. *Mutation:* reserve one slot →
`FocusLost` is dropped and C4's ordering row fails at exact cap only.

**Outbox states — clean SEALED is not failed CLOSED.** Revision 6 had
one `closed` flag doing both jobs, so a duplicate close or a post-seal
send would see "closed" and **invoke the socket-shutdown fallback,
aborting the drain it should have joined**. Four states, with distinct
behaviour:

| state | ordinary enqueue | close called again |
|---|---|---|
| **Open** | ordinary policy: accepted below the cap; a cap-crossing lossless append is rejected and transitions to `Failed` | performs the terminal append-and-seal |
| **Sealed** (suffix appended, drain in flight) | rejected | **waits on the existing acknowledgement** — never re-seals, never falls back |
| **Drained** (acknowledged) | rejected | returns immediately |
| **Failed** (overflow, or transport error) | rejected | **takes the socket-shutdown fallback** |

*Mutation:* collapse `Sealed` into `Failed` → the duplicate-close witness
aborts a healthy drain and exits before `Detach` is written.

**Acknowledgement point:** the batch containing `Detach` **fully written
and flushed to the socket**. **Bound: 250 ms.**

**Two witnesses, mutually exclusive:**

- **Responsive reader** — all preceding lossless events **and** `Detach`
  are written, acknowledgement occurs, **then** exit. *Mutation:* drop
  the drain → exit-before-`Detach` is observable.
- **Stalled reader** — the deadline fires, socket shutdown yields EOF
  cleanup, the frontend **exits anyway**. *Mutation:* remove the bound
  → this test hangs.

*Seal mutation:* permit a late enqueue → an event appears after
`Detach` in the responsive transcript.

## 8. Wire contracts

| | **1a — `TextInput`** | **1e — `OpenTarget` + `OpenTargetResult`** |
|---|---|---|
| **floor** | **v24** | **v26**, after v25's mapping generation, serialized |
| **encoding** | **appended variant**; never widen a field in place — postcard is positional | appended variants |
| **byte pin** | frozen-byte fixture on the **previous final variant** | same |
| **gate** | daemon accepts from `>= 24`; producer withholds below | **`>= 26`**; producer withholds below |
| **old peer** | a `< 24` frontend **retains its existing `Key` behaviour and its existing limitations** — it truncates multi-scalar input today and ignores IME, and continues to. **The guarantee is NO REGRESSION, not retroactive correctness** | a **`< 26`** frontend cannot drop-open; nothing it already had degrades |
| **bounds** | **64 KiB** UTF-8; oversize **rejected** | **32 KiB** per raw path; non-empty path; absolute non-empty cwd; **embedded NUL rejected**; `Failed.message` capped at the **existing 4 KiB** error cap |
| **pins** | frozen bytes on `FrontendEvent`'s previous final variant | **two independent pins** — `FrontendEvent` for `OpenTarget`, `InstanceMessage` for `OpenTargetResult` |

**E5's delivery mechanism.** `StatusFacts.message` is **global and can
be cleared before the originating frontend observes it**, so it cannot
carry this. **1e adds a source-scoped `OpenTargetResult`, correlated by
a frontend request ID**, so a failure reaches the frontend that dropped
the file and no other.

## 9. Coherence impact (§20)

- **Journey steps**: **5**; **3** (1e); **6(e)** on the GPU column.
- **Islands**: Escape ceasing to be a local quit **removes** one;
  Q#S1-7 adds none → the census **falls by one**.
- **Config registry**: none. The bell's 120 ms is a constant.
- **Background work**: **1e adds no new worker**, but it **attributes
  the existing asynchronous directory operation to an originating
  frontend and request** until terminal settlement — the one-shot
  completion is that attribution — **and drops it on source detach**.
  That is §9's ownership question appearing in miniature, and it is
  answered here for this one operation rather than in general.

## 10. Rulings

**Q#S1-1** native close detaches, `editor.quit` shuts down the daemon
and its attachments, Escape only cancels/round-trips · **Q#S1-5** A/`1e`
· **Q#S1-6** B/`TextInput` · **Q#S1-7** Meta/Super → Stage 2, arc §2.5
and the backlog amended by 1-pre's first PR · **Q#S1-8** (A),
`SEMANTIC_BOOTSTRAP_GRID` · **Q#S1-9** precedence per §5 · **Q#S1-10** terminal `OpenTargetResult`.

## 11. Gates

`./scripts/gate --acceptance gpu_invocation_acceptance` plus touched
input suites, and `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`.
**`--protocol` for 1a and 1e only.**
