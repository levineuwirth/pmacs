# Journey Stage 1b-3 — say something when the editor opens

**Status: framing, rev 1 — awaiting approval.**
**Serves `COHERENCE.md` §2 (the golden journey, step 4), §18
(onboarding), §19, §20 Priority 1.**

## 0. Revision history

- rev 1 (2026-07-30) — first framing. Scouted against `githubsucks/main`
  @ `1f290d5` (Journey Stage 1b-1, #203).

## 1. What this stage is

The last of the 1b split. `COHERENCE.md` §20 named three things:
**1b-1** (compile binding + defaults, landed as #203), **1b-2** (LSP
spawn guidance, PR #204), and this — the welcome buffer.

§18 is unusually specific about the size of it:

> the cheap floor — a welcome buffer in `*scratch*` naming `M-x`, the
> keybinding cheat sheet as a help buffer, and a help prefix decision —
> has no prerequisites at all.

**This stage takes the first of those three.** §2's step-4 row lists all
three as what is missing, so §5 is explicit that step 4 does **not**
reach Works here, and says what remains.

## 2. Ground truth

Verified in the tree at `1f290d5`.

### 2.1 A fresh launch says nothing, and the audit's citation is right

`EditorCore::new` sets `status: String::new()` (`src/editor_core.rs:659`)
and creates `*scratch*` (`:629`). A user who runs `pmacs` with no
arguments and no config gets an empty buffer, an empty status line, and
no indication that `M-x` exists.

`builtin/keymaps/default.lua`'s own header comment calls `M-x` the
"command palette" — the sole discovery affordance in the product is
knowing to press it.

### 2.2 The step-2 ratchet pin will collide with any status-line welcome

This is the finding that most shapes the design.
`tests/journey_acceptance.rs`:

```rust
fn journey_step2_launches_unconfigured_into_scratch() {
    let s = EditorState::new();
    assert_eq!(active_name(&s), "*scratch*");
    assert!(
        status(&s).is_empty(),
        "a clean launch reports no error; got {:?}", status(&s)
    );
}
```

The assertion's *message* says "reports no error"; the assertion itself
says **the status is empty**. Those are the same predicate only while
nothing ever writes a non-error status at startup — which is exactly
what a welcome would do.

So a status-line welcome does not merely need a new row, it needs that
existing pin's predicate corrected to what its own message already
claims. §4 treats that as a deliberate, named amendment rather than a
silent edit, because the ratchet's rule is *stages add rows, none
removes them* and an assertion change deserves the same scrutiny.

### 2.3 `C-h` is not free, and the reason is load-bearing

§18 lists "a help prefix decision" as part of the cheap floor. It is
cheap to *decide* and not cheap to get wrong:

```lua
-- builtin/keymaps/default.lua:78-86
-- Why we also bind C-h: most terminals (anything not implementing the
-- kitty keyboard protocol) cannot disambiguate Ctrl+Backspace from
-- Ctrl+H — both legacy paths produce byte 0x08 …
bind("C-h",   "buffer.delete-word-backward")
```

**Rebinding `C-h` to a help prefix would break Ctrl+Backspace on every
non-kitty terminal**, because those terminals cannot tell the two apart.
§2's step-4 row says "`C-h` deletes a word" as though it were an
oversight; it is a deliberate trade with a stated rationale.

This stage therefore **does not touch `C-h`** (§5), and the framing
records the constraint so the discovery arc inherits the reason rather
than rediscovering it.

### 2.4 There is already a `*help*` buffer mechanism

`builtin/commands/default.lua:1226-1260` has `show_help_text(text)`: a
single reused `*help*` buffer, found by name, with a buffer-local `q` →
`buffer.kill-this`. `editor.describe-command` and
`editor.describe-setting` both render through it. `src/help.rs` supplies
`render_command` / `render_key` / `render_buffer` / `render_mode` /
`render_view` plus link resolution (`link_at`, `follow_link_at`).

**Two facts about it that matter to any consumer:**

- It writes with `buf:delete` + `buf:insert`, **not**
  `pmacs.buffer.set_generated_contents`. It is one of the writer
  mechanisms that has not adopted the generated-buffer write invariant,
  so its buffer stays ordinarily editable and keeps its undo history.
  Riding it inherits that; fixing it is not this stage's job, but a
  stage that renders into it should not claim read-only guarantees it
  does not have.
- It is **found by name**, which is the adoption hazard `listview` and
  dired both refused. A foreign `*help*` buffer would be cleared.

### 2.5 A modified `*scratch*` does not block quitting

`editor.quit` runs the `editor.before-quit` hook and then `ed.quit()`
(`builtin/commands/default.lua:250-256`). There is no unsaved-buffer
confirmation, so welcome text left in `*scratch*` cannot strand a user
at exit. That removes the strongest objection to putting text in the
buffer, and it is stated because it is the kind of thing that would
otherwise be assumed in either direction.

## 3. Design

### 3.1 What the welcome says

Three lines, no more. The floor is "this editor is not inert and here is
the one key that opens everything":

```
Welcome to pmacs.   M-x  run a command      C-x C-f  open a file
                    C-c c  build            C-c t    terminal
This buffer is *scratch* — type to edit it, or M-x help for more.
```

Every key named must be **bound in the default keymap and verified by
the acceptance suite**, or the welcome becomes documentation drift with
a user attached. §4 pins that as a property over the message, not a
hardcoded list.

### 3.2 Where it goes — `*scratch*`, not the status line

The status line is the wrong vehicle even though §2's audit cites it:
it is overwritten by the next status write, and a user who blinks loses
the only pointer to `M-x`. §18 says "a welcome buffer in `*scratch*`",
and that is right for the reason the audit itself gives — *the empty
`*scratch*` buffer is what greets the user*.

So: **welcome text is rendered into `*scratch*`**, under three
conditions, all necessary:

1. **No arguments.** `pmacs FILE` and `pmacs DIR` both put something
   else on screen; a welcome would be noise, and for the directory case
   it would fight Stage 1a's dired listing.
2. **`*scratch*` is empty.** Never overwrite content — including a
   restored session's scratch, or anything an `init.lua` wrote.
3. **It leaves the buffer unmodified**, so nothing about the greeting
   looks like unsaved work. §2.5 shows quitting is not blocked either
   way; this is about not lying in the modeline.

### 3.3 What it must not do

- **Not read-only.** Step 5 is "edit immediately"; a read-only greeting
  would break the journey one step after it starts. The user types over
  it, as in Emacs.
- **Not a new buffer.** A `*welcome*` buffer displayed instead of
  `*scratch*` would change what step 2 asserts and give the user a
  buffer whose only purpose is to be closed.
- **Not `set_generated_contents`.** That lifts the rope's read-only,
  discards history, and marks the buffer generated — all wrong for a
  buffer the user is meant to edit immediately. This is the one place
  where *not* adopting the generated-buffer invariant is correct, and it
  is stated so a later audit does not "fix" it.

### 3.4 `M-x help`

The welcome names `M-x help`, so that command has to exist. It renders
the keybinding cheat sheet through the existing `show_help_text`
mechanism (§2.4).

**This is the smallest possible version of §18's second item**, and it
is included only because the welcome would otherwise point at nothing.
The full cheat sheet, `where-is`, `describe-key` and the help-prefix
question belong to §20 Priority 4's discovery arc (§5).

## 4. Acceptance

**N** = new behaviour, must fail on full revert. **P** = preservation,
falsified by a named mutation.

1. **N — journey step 4, through the real entry point.**
   `EditorState::new()` — the same construction `pmacs` with no
   arguments performs — leaves `*scratch*` active **and non-empty**, and
   its text names `M-x`. This is the ratchet row.
2. **N — every key the welcome names is actually bound.** Parse the key
   sequences out of the rendered text and assert each resolves through
   `pmacs.keymap.lookup`. A property over the message, so the pin cannot
   rot when the wording changes — and so the welcome cannot advertise a
   binding that a later stage removes.
3. **N — `M-x help` renders the cheat sheet** into `*help*`, containing
   at least the keys the welcome names. Asserts content produced.
4. **P — the buffer is editable and unmodified.** Typing a character
   into the greeted `*scratch*` inserts it (step 5 still works from the
   first frame), and the buffer reports unmodified *before* that
   keystroke. Targeted mutation: rendering through
   `set_generated_contents`, which would make the buffer read-only and
   fail the insert.
5. **P — a file argument suppresses the welcome.** `EditorState::open`
   on a file leaves that file active with no greeting anywhere.
   Targeted mutation: greeting unconditionally in `EditorCore::new`.
6. **P — a directory argument suppresses it too**, so Stage 1a's dired
   listing is what the user sees. Same mutation; separate pin because
   the directory path reaches scratch differently (the bootstrap
   replaces the buffer rather than never creating it).
7. **P — a non-empty `*scratch*` is never overwritten.** Write to
   scratch, then trigger the greeting path: the content survives.
8. **P (amended pin) — step 2 still reports no error.**
   `journey_step2_launches_unconfigured_into_scratch` currently asserts
   `status.is_empty()` while its message says "reports no error"
   (§2.2). The assertion is corrected to the message's claim — no error
   text on the status line — rather than deleted or weakened. **This is
   the one existing assertion this stage changes**, it is called out
   here rather than buried in the diff, and the row itself is kept.

## 5. Deferred, and why

- **The help prefix / `C-h`.** §2.3: rebinding it breaks Ctrl+Backspace
  on every non-kitty terminal. The decision belongs with §20 Priority
  4's discovery arc, which can weigh a prefix (`F1`? `C-c ?`?) across
  the whole command family instead of one key at a time.
- **The full cheat sheet and the discovery command family** —
  `where-is`, `describe-key`, richer M-x rows: §20 Priority 4, called
  "the best payoff-per-effort in this document".
- **Making `show_help_text` adopt the generated-buffer write
  invariant** (§2.4). A real gap, with its own lane.
- **Onboarding proper** — §18's ten-step teaching sequence. This stage
  is the floor beneath it, not a down payment on it.

**Step 4 therefore stays Partial**, and the PR must say so: §2's row
names a welcome, a cheat sheet *and* `C-h`, and this closes the first
plus a minimal version of the second.

## 6. Coherence impact

- **Journey steps touched:** 4 (Partial → still Partial, with the
  welcome half closed); 2 indirectly, whose pin is amended (§4.8). No
  grade flips on merge, which makes this the first 1b stage with no
  §25 landed-evidence obligation.
- **Interaction islands: none added.** `M-x help` renders through the
  existing `*help*` mechanism rather than inventing a second help
  surface — though §2.4 records that mechanism's own two gaps rather
  than pretending it is exemplary.
- **Config registry:** not adopted. A `welcome.enabled` scalar would be
  expressible, and is deliberately not added — a greeting the user can
  delete by typing does not need a setting, and §11's adoption work
  should not gain a token entry from a stage this small.
- **Background-work attribution:** unchanged; nothing here is
  asynchronous.
- **Docs riding the PR:** `COHERENCE.md` §2's step-4 row and §18's
  ground-truth grade (both stay Partial, with the closed half named);
  `docs/keybindings.md` gains `M-x help`; `docs/agent-handoff.md` §1;
  the ledger.

## 7. Questions

- **Q#W1 — three lines, or one?** One line ("`M-x` runs a command")
  is the true floor and never wraps at 80 columns. Three teaches more
  but risks looking like chrome the user must clear. Recommended:
  three, because the whole complaint in §18 is that the editor teaches
  nothing.
- **Q#W2 — should the welcome name `C-c c` and `C-c t`?** They are
  real, bound, and journey steps 8 and 9 — but naming them means the
  welcome must be updated whenever the default map changes.
  Acceptance 2 turns that from a risk into a caught failure, which is
  the argument for naming them.
- **Q#W3 — is `M-x help` the right name?** `help` is short and
  guessable. The discovery arc may want `help.keys` /
  `help.commands` as a family, and renaming later costs a deprecation.

## 8. Ledger

Branch `journey-stage1b3-welcome`, worktree `../pmacs-journey-1b3`,
based on `githubsucks/main` @ `1f290d5`. Framing only; no code, no PR.

**#204 is open and touches `COHERENCE.md`, `docs/agent-handoff.md` and
`docs/active-work.md`.** This lane will conflict there; integrate late
(at PR time), never by opening a refresh PR.

```sh
git fetch githubsucks
git worktree add ../pmacs-journey-1b3 \
  -b journey-stage1b3-welcome \
  githubsucks/journey-stage1b3-welcome
```
