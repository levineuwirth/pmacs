# Journey Stage 1b-3 — say something when the editor opens

**Status: framing, rev 2 — awaiting review round 2.**
**Serves `COHERENCE.md` §2 (the golden journey, step 4), §18
(onboarding), §19, §20 Priority 1.**

## 0. Revision history

- rev 2 (2026-07-31) — review round 1. Four findings, all accepted, all
  verified in the tree first. §7's three questions are answered and
  folded into the design.
  - **The startup seam was wrong.** `EditorState::new()` is not the
    no-argument entry point: `EditorState::open` calls it
    (`src/editor.rs:944`) *before* handling the file or directory, the
    daemon constructs it too, user config runs *inside* it, and desktop
    restore happens later still (`:3637`). Greeting from `new()` would
    greet a daemon, greet before a file argument replaced the buffer,
    and run before config or restore could put anything in `*scratch*`.
    §3.2 now defines a **launch-finalization seam** that runs after
    config, after attach dispatch resolves to local, and after desktop
    restore — and §4 pins the three paths that must *not* greet.
  - **The status-pin amendment was unnecessary and weakened a stronger
    ratchet.** Rev 1 analysed a status-line welcome in §2.2 and then
    chose `*scratch*` in §3.2, but kept the amendment — an internal
    contradiction. With the chosen design
    `journey_step2_launches_unconfigured_into_scratch` stays correct as
    written, and "no error text" has no defined predicate over an
    unstructured status string anyway. **The pin is untouched**; the
    welcome gets its own row.
  - **"No §25 landed-evidence obligation" was false.** The scorecard
    row 18 reads **Missing** and §18's ground truth says "missing
    entirely" — both are audited claims that a landed welcome plus
    interactive help changes. §6 now records the obligation: §18 and the
    scorecard move **Missing → Partial** on merge, while §2's step-4 row
    stays Partial.
  - **Acceptance 2 could not be implemented as written.** "Parse the key
    sequences out of the rendered text" is ambiguous: `M-x help` mixes a
    chord with a command name, and multi-chord sequences like `C-c c`
    cannot be scraped unambiguously from prose. §3.1 now specifies a
    **structured entry list** that both renders the text and drives the
    binding checks.
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

### 2.2 `EditorState::new()` is not the no-argument entry point

Rev 1 assumed it was, and every acceptance criterion rested on that.
It is wrong four ways, all in `src/editor.rs`:

- **`EditorState::open` calls it first** (`:944`), *then* resolves the
  target. So a greeting inside `new()` fires before the file or
  directory is handled, and the `*scratch*` buffer survives that
  handling — `replace_active_buffer` reassigns the window's buffer and
  removes nothing.
- **The daemon constructs one too**, so a greeting there is written into
  a session no human is looking at.
- **User `init.lua` runs inside `new()`**, so a greeting written there
  precedes anything config might put in `*scratch*`.
- **Desktop restore happens much later** — `restore_desktop_if_armed`
  at `:3637`, inside the `RunLocal` arm of `run()`, after
  `install_state_dirs` and after attach dispatch. A restored session can
  populate `*scratch*`, and a greeting from `new()` would already have
  written into it.

`run()`'s shape is what defines the correct seam:

```rust
let mut state = match target {
    Some(path) => EditorState::open(path)?,   // config runs inside
    None       => EditorState::new(),         // …and here
};
state.install_state_dirs();
let requested = state.lua_host.take_requested_attach();
match dispatch_attach(requested) {
    RunLocal => {
        state.restore_desktop_if_armed(had_file);   // :3637
        // ← the only correct place to greet
```

Note `had_file` is already threaded to exactly this point for exactly
this kind of question ("a positional argument means *open this*, not
*restore my desktop*"), so the no-target signal does not need inventing.

### 2.2a The step-2 pin stays as it is

Rev 1 proposed amending
`journey_step2_launches_unconfigured_into_scratch`'s
`assert!(status(&s).is_empty())`. **That was left over from a
status-line design rev 1 then rejected**, and keeping both was an
internal contradiction.

The welcome goes into `*scratch*` (§3.2), so the status line stays empty
and the pin stays true as written. It is also the stronger assertion:
"no *error* text" has no defined predicate over an unstructured status
string, so replacing an exact check with a fuzzy one would weaken the
ratchet to buy nothing. **The pin is untouched**; the welcome gets its
own row (§4).

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

### 3.1 What the welcome says, and the shape it is built from

**Three lines** (Q#W1), naming `C-c c` and `C-c t` (Q#W2):

```
Welcome to pmacs.  M-x runs any command; M-x help lists the keys.
  C-x C-f  open a file      C-c t  terminal
  C-c c    build            C-x b  switch buffer
```

**The text is rendered from a structured list, never scraped back out
of it.** Rev 1 said the acceptance would "parse the key sequences out of
the rendered text"; that cannot be implemented reliably — `M-x help`
puts a chord and a command name in one phrase, and `C-c c` is two chords
whose boundary prose does not mark.

One list is the single source for both the rendering and the checks:

```lua
-- Each entry is { keys = "<sequence>", label = "<what it does>" }.
-- `keys` is EXACTLY what `pmacs.keymap.lookup` accepts, so a binding
-- check is a lookup, not a guess about where a chord ends.
pmacs.welcome.entries = {
  { keys = "C-x C-f", label = "open a file"    },
  { keys = "C-c t",   label = "terminal"       },
  { keys = "C-c c",   label = "build"          },
  { keys = "C-x b",   label = "switch buffer"  },
}
```

`M-x` and `M-x help` are prose in the first line rather than entries:
`M-x` is the palette itself and `help` is a command name, so neither is
a keymap lookup. The acceptance checks the command exists instead (§4.3).

**Every entry must resolve through `pmacs.keymap.lookup`**, asserted as
a property over the list (§4.2). That is what stops the welcome becoming
documentation drift with a user attached — and it is why the list is
public: a user who rebinds can rebuild it.

### 3.1a `M-x help`

Named `help` (Q#W3). It renders the cheat sheet through the existing
`show_help_text` mechanism (§2.4) and is the **root of the eventual
family**: when the discovery arc adds `help.keys`, `help.commands` and
friends, `help` remains the index they are reached from, so no
deprecation is owed.

### 3.2 Where it goes — `*scratch*`, not the status line

The status line is the wrong vehicle even though §2's audit cites it:
it is overwritten by the next status write, and a user who blinks loses
the only pointer to `M-x`. §18 says "a welcome buffer in `*scratch*`",
and that is right for the reason the audit itself gives — *the empty
`*scratch*` buffer is what greets the user*.

### 3.2a **When** it goes — a launch-finalization seam

Per §2.2, no existing constructor is the right hook. The stage adds one
named seam:

```rust
/// Final step of a LOCAL, no-target launch, after config, attach
/// dispatch and desktop restore have all had their say. The only
/// caller is `run()`'s `RunLocal` arm; the only thing it does is
/// conditionally greet an untouched `*scratch*`.
pub fn finalize_local_launch(&mut self, had_file: bool)
```

called from `run()` immediately after `restore_desktop_if_armed(had_file)`.

It greets only when **all four** hold, and each excludes a case §2.2
showed rev 1 would have got wrong:

1. **`had_file` is false** — a positional argument means "open this".
2. **The session is local** — it is inside the `RunLocal` arm, so a
   daemon or an attach hand-off never reaches it.
3. **`*scratch*` is the active buffer** — desktop restore may have put
   something else in front.
4. **`*scratch*` is empty** — never overwrite config's or a restored
   session's content.

And it leaves the buffer **unmodified**, so the greeting does not look
like unsaved work in the modeline. §2.5 shows quitting is not blocked
either way; this is about not lying.

**Honest limit on the seam's testability.** `run()` takes over the
terminal, so no test drives it end to end. The acceptance therefore pins
(a) the seam's own behaviour under each condition, and (b) that the
three constructors — `new()`, `open(file)`, `open(dir)` — greet
**nothing** on their own, which is what makes the seam the only writer.
What remains unpinned is `run()` actually calling it; that is stated
here rather than papered over, and it is the reason the seam is a single
named function with one call site rather than logic inlined into the
arm.

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

### 3.4 `M-x help` is deliberately minimal

It exists only because the welcome would otherwise point at nothing.
**This is the smallest possible version of §18's second item**: the full
cheat sheet, `where-is`, `describe-key` and the help-prefix question all
belong to §20 Priority 4's discovery arc (§5).

## 4. Acceptance

**N** = new behaviour, must fail on full revert. **P** = preservation,
falsified by a named mutation.

1. **N — journey step 4: a no-target local launch greets.**
   Construct through `EditorState::new()`, then call
   `finalize_local_launch(false)` — the seam `run()` calls. `*scratch*`
   is active and **non-empty**, and its text names `M-x`. This is the
   ratchet row.
   *Rev 1 claimed `EditorState::new()` was itself the entry point; §2.2
   shows it is shared with `open()` and the daemon, so the pin drives the
   seam instead.*
2. **N — every entry the welcome names is bound.** For each
   `pmacs.welcome.entries` item, `pmacs.keymap.lookup(entry.keys)`
   resolves. A property over the **structured list** (§3.1), not a scrape
   of prose — so it cannot rot when the wording changes, and it fails
   loudly if a later stage unbinds something the welcome advertises.
   Includes a precondition that the list is non-empty, or the loop is
   vacuous.
3. **N — the rendered text contains every entry's `keys` and `label`.**
   This is what ties the list to the thing the user actually sees; pin 2
   alone would pass if rendering dropped an entry.
4. **N — `M-x help` renders the cheat sheet** into `*help*`, containing
   at least the entries' key sequences. **Invoked through the real
   palette path** (`pmacs.command.invoke`), not by calling the render
   helper, so the command is proven reachable the way a user reaches it.
5. **P — the buffer is editable and unmodified.** After greeting,
   `*scratch*` reports unmodified; typing a character inserts it, so
   step 5 works from the first frame. Targeted mutation: rendering
   through `set_generated_contents`, which would lift read-only, discard
   history, and make the insert fail.
6. **P — a file target does not greet.** `EditorState::open(file)` then
   `finalize_local_launch(true)`: no welcome anywhere, and the file is
   active. Targeted mutation: dropping the `had_file` guard.
7. **P — a directory target does not greet.** Same with a directory, so
   Stage 1a's dired listing is untouched. Separate pin because the
   directory path reaches `*scratch*` differently — the bootstrap
   replaces the window's buffer and `replace_active_buffer` removes
   nothing, so the scratch buffer still exists to be wrongly greeted.
8. **P — a non-empty `*scratch*` is never overwritten.** Write to
   scratch (standing in for config or a restored desktop), then run the
   seam: the content survives byte for byte. Targeted mutation: dropping
   the emptiness guard.
9. **P — a non-active `*scratch*` is not greeted.** Switch the active
   buffer away, then run the seam: scratch stays empty. Stands in for
   desktop restore having put something else in front. Targeted
   mutation: dropping the active-buffer guard.
10. **P — the three constructors greet nothing on their own.**
    `EditorState::new()`, `open(file)` and `open(dir)`, each with no
    seam call, leave `*scratch*` empty. **This is what makes the seam the
    only writer**, and it is the pin that would catch a greeting
    smuggled back into a constructor — including the daemon's.
11. **P — the step-2 pin is unchanged and still passes.**
    `journey_step2_launches_unconfigured_into_scratch` keeps asserting
    an empty status, because the welcome goes to the buffer (§2.2a). Not
    a new test — a stated requirement that this stage does not touch it.

**What is not pinned, stated rather than implied:** that `run()` calls
`finalize_local_launch`. `run()` takes over the terminal and no test
drives it (§3.2a). Pins 1 and 10 bracket the risk — the seam works, and
nothing else greets — but the wiring itself is reviewed, not tested.

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
plus a minimal version of the second. **§18 and the scorecard do move**,
Missing → Partial (§6) — a stage can be too small to flip its journey
step while still falsifying a "missing entirely" grade.

## 6. Coherence impact

- **Journey steps touched:** 4 — **and it stays Partial**, because §2's
  row names a welcome, a cheat sheet *and* `C-h`, and this closes the
  first plus a minimal second. Step 2 is not touched at all (§2.2a).
- **§25 landed-evidence obligation: yes, and rev 1 said otherwise.**
  Two audited claims change on merge and both must move
  **Missing → Partial**:
  - the **scorecard** row 18, "Onboarding | **Missing** | No welcome, no
    tutorial; `C-h` deletes a word; `M-x` is the only door in";
  - **§18's ground truth**, "Grade: missing entirely. No welcome buffer,
    … no cheat sheet reachable from inside the editor".

  Both become false the moment this lands — a welcome buffer exists and
  a cheat sheet is reachable by `M-x help` — while `C-h` and the tutorial
  stay untrue, which is what makes the new grade Partial rather than
  Works. §25 requires the update to ride the landing PR. Rev 1's claim
  that this stage had no such obligation was simply wrong, and would
  have left the document asserting "missing entirely" about a feature
  the same PR shipped.
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
- **Docs riding the PR:** `COHERENCE.md`'s **scorecard row 18** and
  **§18's ground truth** (Missing → **Partial**), §2's **step-4 row**
  (stays Partial, with the closed half named), and §18's own note that
  the cheap floor's first two items are done and the help-prefix
  decision is not; `docs/keybindings.md` gains `M-x help`;
  `docs/agent-handoff.md` §1; the ledger.

## 7. Questions — answered in review round 1

- **Q#W1 — three lines, or one? → three.** The whole complaint in §18 is
  that the editor teaches nothing, so the floor is not one line.
- **Q#W2 — name `C-c c` and `C-c t`? → yes**, and verify them from the
  structured entries (§3.1). Naming real bindings is the point; pin 2
  turns the maintenance risk into a caught failure rather than drift.
- **Q#W3 — is `help` the right name? → yes.** It stays the **root/index**
  when `help.keys` and friends arrive under the discovery arc, so no
  future rename or deprecation is owed.

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
