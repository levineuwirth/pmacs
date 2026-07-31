# Journey Stage 1b-2 — say when language intelligence did not start

**Status: framing, rev 1 — awaiting approval.**
**Serves `COHERENCE.md` §1.2 (the silence asymmetry), §2 (the golden
journey, step 6), §19, §20 Priority 1.**

## 0. Revision history

- rev 1 (2026-07-30) — first framing. Scouted against `githubsucks/main`
  @ `fbcf235` (reap-ledger #202).

## 1. Ground truth

Everything below was read in the tree at `fbcf235`. Where
`COHERENCE.md`'s audit (2026-07-25) is now stale, §1.7 says so.

### 1.1 The canonical failure is still completely silent

`ensure_server` (`builtin/runtime/lsp.lua:616`) ends:

```lua
  local ok, sid = pcall(pmacs.lsp.spawn, { … })
  if ok then
    default_servers[tostring(sid)] = language
    return sid
  end
  return nil
```

`return nil` is the whole error path. No status line, no record, no
event. The `buffer.after-load` hook then swallows what is left:

```lua
-- builtin/runtime/lsp.lua:1019-1021
pmacs.hook.add("buffer.after-load", function()
  pcall(attach_buffer, pmacs.window.buffer())
end)
```

This is the case a new user hits first: `rust-analyzer` is
preconfigured (`lsp.lua:44-52`) and, on most machines, not installed.

### 1.2 The reporting pattern is already established — in this same file, twice

The fix is not "invent a channel". `lsp.lua` already does exactly the
right thing at two other sites:

| Site | Failure reported |
|---|---|
| `lsp.lua:570-585` | a root resolver that raised or returned a bad type |
| `lsp.lua:1831-1836` (`report_subscriber_error`) | a notification subscriber that raised |

Both use the same shape, and both carry the reasoning in comments:

```lua
pcall(pmacs.editor.set_status, msg)
if pmacs.error then pcall(pmacs.error, msg) end
```

`pmacs.editor.set_status` is the channel that exists; the `pmacs.error`
arm rides along for when that channel is built. **So the canonical case
is not silent for want of a mechanism — it is silent because the two
sites that adopted the rule were the two that a review happened to
touch.** That is worth stating plainly: this stage finishes an adoption,
it does not start one.

### 1.3 The error string does not name the command

The failure text a caller receives is built at
`src/process.rs:2061`:

```rust
let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
```

On a missing binary that renders as:

```
spawn: No such file or directory (os error 2)
```

**It names neither the program nor the language.** `std::io::Error` from
`Command::spawn` carries no program name, and nothing between there and
Lua adds one. So a message built by forwarding the error verbatim would
tell a user nothing actionable — the guidance has to be composed at the
site that still knows `language` and `cfg.command`.

The model to imitate is in this repo already, and `COHERENCE.md` §1.2
calls it "the best missing-tool message in the codebase"
(`src/main.rs:367-379`): it names the sibling path it tried *and* the
PATH fallback, so the reader knows what was attempted and where to look.

### 1.4 There is no negative memo — the failure repeats per file open

`COHERENCE.md` §1.2's frequency note says the failure "fires **once per
project root** rather than once per language per session". **That is not
what the code does, and the difference decides this stage's hardest
question.**

`LspManager::spawn` (`src/lsp.rs:1287-1297`) returns early on failure:

```rust
let id = LspServerId::next();
let mut client = LspClient::new(spec);
self.start_generation(id, &mut client)?;   // <-- returns here on ENOENT
self.status_tracker.ensure(id, Instant::now());
self.clients.insert(id, client);
```

Both the status-tracker entry and the client insert are *after* the `?`.
So a failed spawn leaves **nothing** — no client, no status record — and
`pmacs.lsp.list()` cannot see it. `ensure_server`'s affinity loop scans
exactly that list, finds nothing, and spawns again.

`attach_buffer` runs from `buffer.after-load`, so the real frequency is
**once per file open of a matching language**, which is strictly more
often than the audit recorded. A naive "report the failure" would put a
status-line message on every single file open in a Rust project.

**This is why "when to speak" is the design question and not a detail.**

### 1.5 The modeline cannot distinguish "failed" from "not applicable"

```lua
-- builtin/runtime/lsp.lua:973-983
fn = function(ctx)
  local rec = attachments[tostring(ctx.buffer)]
  if not rec then return nil end
  return "LSP:" .. pmacs.lsp.modeline_label(rec.server)
end,
```

No attachment record means no segment at all. A `.rs` file whose server
failed to spawn and a `.txt` file that never had one render identically,
while tree-sitter highlighting keeps working — §1.2's "highlighting
**actively masks** the failure".

### 1.6 The status surface is already built, and has no caller

**This is dark matter, and it makes half the stage free.**

- `LspManager::status_buffer_text()` (`src/lsp.rs:1257`) — doc comment:
  *"render the contents of the `*lsp*` status buffer"*.
- `LspManager::last_error(sid)` (`src/lsp.rs:1251`).
- Both are exposed to Lua: `pmacs.lsp.status_buffer_text()`
  (`src/lua_bindings/mod.rs:10949`) and `pmacs.lsp.last_error`
  (`:10851`), plus per-server `last_error` inside the status table
  (`:10931-10938`).
- Both are **tested** (`tests/m4_acceptance.rs:2545`, `:2634-2640`).

And there is **no production caller of any of them**, no `*lsp*` buffer,
and no interactive command: the twelve `lsp.*` commands
(`lsp.lua:2771-2840`) are all feature actions — definition, hover,
rename, format — and not one is diagnostic. Several doc comments across
`src/lsp.rs` and `src/project.rs` refer to "the `*lsp*` buffer" as
though it exists.

So `COHERENCE.md` §2 step 6's "No LSP status command exists to
diagnose" is true, but understates the position in the stage's favour:
the renderer exists, is exposed, and is tested. What is missing is a
command and a buffer.

**One thing it will not show, however:** `status_buffer_text` renders
from `self.clients`, and §1.4 established that a failed spawn inserts no
client. **The durable surface cannot, today, display the very failure
this stage exists to surface.** §2.4 decides what to do about that.

### 1.7 Stale citations in `COHERENCE.md` §1.2

Recorded so a later scout does not lose time, and corrected by this
stage (§6):

- "`ensure_server` `pcall`s it and returns nil
  (`builtin/runtime/lsp.lua:614-626`)" — the function starts at `:616`
  and the spawn/`pcall` is at `:658-674`.
- "the `buffer.after-load` hook `pcall`s the whole attach
  (`builtin/runtime/lsp.lua:895-897`)" — it is `:1019-1021`.
- The frequency note is wrong in kind, not just in line number (§1.4).
- "**Net user-visible result: nothing**" remains **true** for this
  failure, but the surrounding claim that no background failure is
  reported is now false — §1.2 above lists two sites that do.

## 2. Design

### 2.1 What to say

Composed where `language` and `cfg.command` are still in scope, in
`src/main.rs`'s spirit — name what was tried, and what to do:

```
LSP: rust-analyzer for rust did not start (spawn: No such file or
directory (os error 2)) — install it or set
pmacs.lsp.config.rust.command in init.lua. M-x lsp.status for detail.
```

Three parts, each load-bearing:

1. **What was attempted** — the command and the language. §1.3 shows the
   underlying error supplies neither.
2. **The underlying error, verbatim** — so a permissions failure or a
   bad interpreter is distinguishable from a missing file. The message
   must not *classify* the errno; §1.3's string is all we get, and
   guessing "it is not installed" would be wrong for `EACCES`.
3. **The two things a user can do** — install it, or repoint the config
   — plus the durable surface.

### 2.2 When to say it — once per (root, language), per session

Per §1.4 the failure recurs on every file open, so the report is
memoized on the **same affinity key `ensure_server` already computes**
(`key_uri` plus `language`).

**Memoize the report, not the failure.** The spawn is still attempted
every time, so a user who installs the binary mid-session gets a working
server on the next file open with no cache to invalidate. Only the
*message* is suppressed after the first.

That asymmetry is the whole rule, and it is the one thing here easy to
get backwards: memoizing the failure would be a behaviour change, would
need invalidation, and would make recovery require a restart.

### 2.3 Where — status line now, `*lsp*` for later

The status line is transient and can be overwritten before it is read;
COHERENCE's rule is that an automatic failure must leave a **trace**,
not a flash. So the same event also lands in a durable record that
`M-x lsp.status` renders.

### 2.4 The durable record lives in Lua, and that is a deliberate limit

§1.6 established that `status_buffer_text` renders from `self.clients`,
which a failed spawn never enters. Two ways to fix that:

- **Record failures in Rust** (`status_tracker`), so `*lsp*` shows them
  natively. Correct long-term, but it changes what a "server" is in the
  status model, touches typed state several consumers read, and turns a
  Lua stage into a Rust one.
- **Record failures in Lua**, in this module, and have `lsp.status`
  render them as a section *above* `status_buffer_text()`'s output.

**This stage takes the second**, for the same reason 1b-1 kept its
defaults in a Lua table: it is the smallest thing that makes the failure
visible, and it does not commit the Rust status model to a shape before
anyone has used the surface. The limitation is explicit — `*lsp*`'s
native section still lists only servers that started — and §5 names
promoting it as follow-on work rather than pretending it is done.

### 2.5 The modeline marker

§1.5 is the sharpest half of §1.2: highlighting masks the failure, so a
user has no reason to *go looking* for a command. The segment therefore
gains one branch — when the buffer's (root, language) has a recorded
failure and no attachment, render a distinct label:

```
LSP:!        (vs LSP:ok / LSP:… / nothing at all)
```

**It must not fabricate an attachment record.** `attachments` is read by
`attachment_for_request`, the completion driver, and the request paths,
all of which treat a record as naming a live server; inventing one would
route requests at a server that does not exist. The segment reads the
failure table directly and returns early, exactly as it does today for
"no record".

### 2.6 What this stage does not do

- It does not classify errnos into causes (§2.1).
- It does not add a retry, back-off, or auto-install.
- It does not touch `pmacs.error`. Fifteen guarded call sites still
  report through a channel that does not exist; building it is its own
  lane (§5). This stage adds a sixteenth *only* in the ride-along form
  the two existing sites already use, so it upgrades for free and works
  today.
- It does not surface `LspEventKind::Crashed`. A server that started and
  then died is a different failure with a different message, and no
  builtin subscriber handles it today. Named in §5.

## 3. Questions

- **Q#L1 — is per-session the right memo lifetime?** A user who
  uninstalls a server mid-session gets no second message. The
  alternative — re-report after N minutes — adds a clock to a path that
  has none. Recommended: per-session, revisit if it is ever a complaint.
- **Q#L2 — should the message name `init.lua` explicitly?** It assumes
  the user has one; a user with no config has nothing to edit. The
  counter is that naming the file is what makes the advice actionable,
  and `pmacs.config` docs already assume it.
- **Q#L3 — should `lsp.status` get a keybinding?** §20 Priority 4 is
  about exactly this class of command, and binding one diagnostic
  command ahead of that arc invites the inversion §2 warns about. This
  framing says **no binding**, reachable by `M-x`, and lets the
  discovery arc bind the family coherently.
- **Q#L4 — does `LSP:!` belong in the modeline, or is it noise for a
  user who has deliberately not installed a server?** The case against
  §2.5. A user who never wants rust-analyzer sees `!` forever with no
  way to dismiss it short of clearing the config.

## 4. Acceptance

Labels per Stage 1a §6.0: **N** new behaviour, must fail on full
revert; **P** preservation, falsified by a named mutation.

1. **N — the failure is reported, through the real path.** Open a file
   whose configured server command does not exist; the status line names
   the command, the language, and the underlying error. Driven through
   `buffer.after-load`, not by calling `ensure_server` directly — the
   hook's `pcall` is part of what is being tested.
2. **N — it is reported once per (root, language).** Open a second file
   in the same project: the spawn is attempted again (observable), the
   message is not repeated. Falsified by dropping the memo.
3. **N — the memo is on the report, not the failure.** After a failed
   attach, make the command resolvable and open another file: a server
   attaches. Falsified by memoizing the failure instead — this is §2.2's
   whole claim and needs its own pin.
4. **N — a different language, or a different root, reports again.**
   Falsified by keying the memo on the language alone.
5. **N — `M-x lsp.status` renders a buffer** containing both the failure
   section and `status_buffer_text()`'s output. Falsified by removing
   either half. Must assert **content produced**, not that a buffer
   exists.
6. **N — the modeline distinguishes failed from not-applicable.** A
   `.rs` file with a failed spawn renders `LSP:!`; a `.txt` file renders
   nothing. Both halves asserted — a pin that only checks the `.rs` case
   passes if the segment renders `!` unconditionally.
7. **P — a working server is unaffected.** Attach, modeline label,
   requests: unchanged. Targeted mutation: making the new branch fire
   whenever an attachment is absent, which would mark every plain-text
   buffer failed.
8. **P — the two existing report sites still report.** Root-resolver and
   subscriber failures keep their messages. Targeted mutation:
   refactoring the three sites onto a shared helper that drops one.
9. **P — a fabricated attachment is never created.** After a failed
   spawn, `attachment_for_request` returns nil and no request is issued.
   Targeted mutation: §2.5's forbidden implementation.

**Fixture note.** The natural fixture points a config at a path that
does not exist, which is reliable and hermetic. It must assert its own
precondition — that the command really is absent — because a fixture
that accidentally names a real binary would make every absence
assertion vacuous. That is the shape that bit both #202's in-drain pin
and 1b-1's nested-project pin.

**Ambient-root note.** These tests construct an editor, so they need the
five bootstrap-storage variables controlled locally (#201 is framing
only). A developer with a real `rust-analyzer` on PATH must not change
the result — hence a configured non-existent command rather than
relying on rust-analyzer's absence.

## 5. Deferred, named rather than implied

- **Building `pmacs.error`.** Fifteen dead call sites; its own lane.
- **Promoting the failure record into Rust's status model**, so `*lsp*`
  shows failed spawns natively (§2.4).
- **Surfacing `LspEventKind::Crashed`** — a started-then-died server
  (§2.6).
- **A keybinding for `lsp.status`**, which belongs to §20 Priority 4's
  discovery family (Q#L3).
- **Journey Stage 1b-3**, the welcome buffer (step 4).

## 6. Coherence impact

- **Journey steps touched:** 6 (Partial → Works for the failure case;
  the success case is already fine). Indirectly 10, which is gated on 6
  or 9 succeeding. 9 was closed by 1b-1.
- **Interaction islands: none added.** The message uses the existing
  status line; `*lsp*` is a generated read-only buffer of the kind
  `COHERENCE.md` §14 already calls the proven "output channel" pattern,
  and it should adopt `listview`'s idiom rather than inventing a third
  read-only buffer shape.
- **Config registry:** not adopted. Nothing here is a user-tunable
  scalar; the memo is session state, not configuration.
- **Background-work attribution:** this *is* the attribution fix for one
  failure — §1.2's rule that anything failing automatically must leave a
  user-visible trace with a named owner.
- **Doc updates riding this PR:** `COHERENCE.md` §1.2's stale citations
  and frequency note (§1.7), its step-6 verdict row, §20 Priority 1's
  remainder line, §24; `docs/keybindings.md` if Q#L3 is overturned;
  `docs/agent-handoff.md` §1; the ledger.

## 7. Ledger

Branch `journey-stage1b2-lsp-guidance`, worktree `../pmacs-journey-1b2`,
based on `githubsucks/main` @ `fbcf235`. Framing only; no code, no PR.

Recovery from a clean checkout — the two-argument form of
`git worktree add` does not work for a remote-only branch:

```sh
git fetch githubsucks
git worktree add ../pmacs-journey-1b2 \
  -b journey-stage1b2-lsp-guidance \
  githubsucks/journey-stage1b2-lsp-guidance
```
