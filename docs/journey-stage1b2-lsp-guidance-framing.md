# Journey Stage 1b-2 — say when language intelligence did not start

**Status: framing, rev 3 — awaiting review round 3.**
**Serves `COHERENCE.md` §1.2 (the silence asymmetry), §2 (the golden
journey, step 6), §19, §20 Priority 1.**

## 0. Revision history

- rev 3 (2026-07-30) — review round 2. Two blocking, two cleanups; all
  four accepted, and the two blockers verified by running Lua rather
  than by reading it.
  - **Recovery was inconsistent across buffers that share an affinity.**
    Rev 2 cleared `failures[K]` on success but cleared only the
    *succeeding* buffer's projection. So: buffer A fails, buffer B
    succeeds for the same `(language, key_uri)` — `M-x lsp.status` says
    the failure is gone while **A's modeline still reads `LSP:!`**. Rev
    2's claim that the two tables are "written and cleared at the same
    moment" was simply false for the cross-buffer case, which is the
    normal case for a project with more than one file. Each buffer
    projection now carries its affinity key, and a success **sweeps every
    projection holding that key** (§2.5). Acceptance 9 pins it.
  - **The markerless key had no Lua representation.** `key_uri` is
    deliberately nil, and `t[nil] = v` raises *"table index is nil"* —
    confirmed by running it, in both LuaJIT and 5.4. So rev 2's central
    markerless criterion was literally unimplementable as written, and
    left to implementation would have invited two different ad-hoc
    encodings for the two tables. §2.2 now prescribes **one** key
    function, used by both.
  - **Acceptance 10 could not have observed what it claimed.** Making the
    command resolvable changes no state by itself; `failures` is cleared
    by a *successful spawn*, which needs an attach to occur. The pin now
    reattaches before pressing `g`.
  - The ledger heading still said revision 1.
- rev 2 (2026-07-30) — review round 1. Two blocking, three major, one
  minor; all six accepted, all six verified in the code first.
  - **The affinity key was misstated** (§2.2). The runtime's reuse key
    is `(language, key_uri)`, and `key_uri` is deliberately **nil** for a
    markerless file — `ensure_server` sets it only when the root came
    from config or a marker walk (`lsp.lua:644-648`), so loose files
    across unrelated directories share **one** server per language, on
    purpose. Rev 1 said "(root, language)", which would have given every
    directory of loose files its own memo entry and re-reported the same
    shared failure once per directory. The stage now keys on the real
    affinity key and does not change that behaviour.
  - **Dedupe and current-failure state were conflated** (§2.2, §2.4).
    Rev 1 had one record and said nothing about recovery. One record
    cannot do both jobs: keep it and `*lsp*` shows a failure that has
    since been fixed; clear it and the per-session dedupe is lost, so
    the message returns on the next file open. They are now two records
    with different lifetimes, and **the reported identity includes the
    command**, so repointing config from one missing executable to
    another reports again. Recovery is pinned in both surfaces.
  - **The modeline's pure per-buffer projection had to be preserved**
    (§2.5). The provider reads `attachments[ctx.buffer]` specifically so
    a passive split reports its own buffer, and does no work while
    painting. Rev 1's "read the failure table" would have made the
    segment recompute an affinity key — invoking `project_root_for`,
    user root resolvers, and `pmacs.project.detect` **every frame, for
    every window**. The failure is now projected per buffer at attach
    time, and the segment stays a single map lookup.
  - **"Adopt listview's idiom" was too weak** (§2.3). It now requires
    `pmacs.listview.open` and names the guarantees that come with it,
    including `on_refresh` — without which `listview.refresh` early
    returns (`listview.lua:259`) and `g` is a **dead key** in the new
    panel.
  - **§19's journey ratchet was missing from acceptance** (§4). This
    stage makes step 6 real, and `tests/journey_acceptance.rs`'s stated
    rule is that steps 6–12 join as later stages make them real. The M4
    pins stay; an end-to-end journey row is added.
  - **The ledger's canonical-base anchor was stale** — it named
    `7586905` while `main` is `fbcf235`. Updated with the recovery
    floor, which moves with it.
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

### 2.2 When to say it — and two records, not one

Per §1.4 the failure recurs on every file open, so the report is
memoized. Two things have to be got right, and rev 1 got both wrong.

**The key is `ensure_server`'s own affinity key, which is not the
root.** `ensure_server` computes:

```lua
-- builtin/runtime/lsp.lua:644-648
local root, source = project_root_for(language, path)
local key_uri = nil
if source == "config" or source == "detected" then
  key_uri = file_uri_for(root)
end
```

`key_uri` is **nil** whenever the root came from the fallback (the
file's own directory), and the reuse loop matches `info.root_uri ==
key_uri`, nil matching nil. That is deliberate and documented in place:
loose files in unrelated directories share **one** server per language,
because keying them on their own directories "would give every directory
of loose scratch files its own server, for every language".

So the memo key is `(language, key_uri)` — the same pair, nil included —
and **not** the resolved root. Keying on the root would split what the
runtime deliberately shares and re-report one shared failure once per
directory. This stage does not change that behaviour; it matches it.

**Two records, because one cannot do both jobs.**

| Record | Key | Lifetime | Read by |
|---|---|---|---|
| `reported` | `(language, key_uri, command)` | never cleared, session-scoped | the status-line report only |
| `failures` | `(language, key_uri)` | **cleared when a spawn for that key succeeds** | `*lsp*`, and the per-buffer projection |

**`key_uri` is nil, and Lua cannot index a table by nil.** `t[nil] = v`
raises `table index is nil` — verified by running it under both LuaJIT
and Lua 5.4, not inferred. Since the markerless case *is* the nil case,
the central criterion of §4 acceptance 4 is unimplementable without an
encoding, and leaving it to implementation would let the two tables
diverge on how they spell it.

**One key function, used by both tables:**

```lua
-- Lua strings are 8-bit clean (`#"a\0b" == 3`, checked), so \0 is a
-- separator no path or language id can contain.
--
-- The `u`/`n` discriminator is what makes markerless unambiguous: a
-- bare `key_uri or ""` would collide with a (pathological but legal)
-- empty URI, and a sentinel like "markerless" is a string a URI could
-- in principle equal. The prefix cannot collide with anything.
local function affinity_key(language, key_uri)
  return language .. "\0" .. (key_uri and ("u" .. key_uri) or "n")
end

local function reported_key(language, key_uri, command)
  return affinity_key(language, key_uri) .. "\0" .. tostring(command)
end
```

`false` would also be a legal Lua key, but the keys here are tuples, so
a single encoded string is what both tables want anyway — and one
function is what stops the two encodings drifting apart.

Rev 1 had one record and said nothing about recovery. One record forces
a bad choice: keep it and `*lsp*` reports a failure the user has since
fixed; clear it and the dedupe is gone, so the message returns on the
next file open.

**`reported` includes the command.** A user who repoints
`pmacs.lsp.config.rust.command` from one missing executable to another
has a genuinely new failure and must hear about it; a key without the
command would swallow it as a duplicate.

**Memoize the report, not the failure.** The spawn is still attempted on
every file open, so a user who installs the binary mid-session gets a
working server with no cache to invalidate — and that success is what
clears `failures`. This asymmetry is the whole rule and the easiest
thing here to get backwards: memoizing the *failure* would be a
behaviour change, would need invalidation, and would make recovery
require a restart.

### 2.3 Where — status line now, `*lsp*` for later

The status line is transient and can be overwritten before it is read;
COHERENCE's rule is that an automatic failure must leave a **trace**,
not a flash. So the same event also lands in `failures`, which
`M-x lsp.status` renders.

**`*lsp*` is opened with `pmacs.listview.open`, not merely "in its
idiom".** Naming the primitive is what buys the guarantees, and a
hand-rolled panel would have to re-derive every one of them:

- **Owned-handle identity.** Found-by-name is *not* adoption
  (`listview.lua:157-167`): a foreign buffer already called `*lsp*` is
  never claimed, clobbered, or given an erroring intercept.
- **Collision behaviour.** A taken name disambiguates `*lsp*<2>`…`<99>`
  and **raises** when exhausted rather than adopting.
- **Immutable generated contents** — a read-only intercept with a named
  error, plus round-trip input so a semantic frontend cannot swallow the
  panel's single-key bindings.
- **`q`** quits to the previous buffer.
- **`on_refresh`, which is not optional here.** `listview.refresh`
  early-returns unless the panel has one (`listview.lua:259`), so
  omitting it makes **`g` a dead key** — a bound chord that silently
  does nothing. The panel's `on_refresh` recomputes *both* sections, so
  `g` after installing a server shows the recovery.

`status_buffer_text()` returns one string; the panel splits it into
inert rows (no `on_visit`) beneath the failure section. Rows without a
visit action are an ordinary listview shape, not a special case.

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
gains one branch — when the buffer has a recorded failure and no
attachment, render a distinct label:

```
LSP:!        (vs LSP:ok / LSP:… / nothing at all)
```

**The segment must stay a pure per-buffer projection.** Its comment says
so — *"pure modeline projection … reads the private per-buffer
attachment map directly so passive split windows report their own buffer
… never attaches, flushes didChange, or issues a request"*
(`lsp.lua:969-983`) — and the reason is structural, not stylistic: it
runs for **every window, every paint**.

Rev 1 said the segment should "read the failure table", which is keyed
by `(language, key_uri)`. Deriving that key from a buffer means calling
`project_root_for`, which invokes **user-supplied root resolvers** and
`pmacs.project.detect`. Per frame, per window. A resolver with a side
effect, or one that raises, would then run inside painting.

So the failure is **projected per buffer at attach time**, beside the
existing map, and **each projection carries the affinity key that
produced it**:

```lua
failed_attachments[tostring(buf)] = {
  key = affinity_key(language, key_uri),   -- §2.2
  language = …,
  command = …,
}
```

**Clearing has to sweep, not touch one entry.** Rev 2 said the two
tables were "written and cleared at the same moment"; that is false as
soon as two buffers share an affinity, which is the normal case for a
project with more than one file:

> A `.rs` file fails, so `failures[K]` and `failed_attachments[A]` are
> both written. The user installs the server and opens a *second* `.rs`
> file in the same project. That spawn succeeds for the same `K`, so
> `failures[K]` is cleared and `*lsp*` reports nothing wrong — while
> **A's modeline still reads `LSP:!`**, because only B's projection was
> touched. The two surfaces now contradict each other, and the stale one
> is the one the user is looking at.

So a success for key `K` clears `failures[K]` **and every projection
whose `key == K`**. The sweep is bounded by the number of buffers with a
recorded failure, which is at most the number of open buffers, and it
runs on a spawn success — not on a paint.

The segment becomes one more map lookup and computes nothing:

```lua
local rec = attachments[key]
if rec then return "LSP:" .. pmacs.lsp.modeline_label(rec.server) end
if failed_attachments[key] then return "LSP:!" end
return nil
```

**It must not fabricate an attachment record.** `attachments` is read by
`attachment_for_request`, the completion driver, and every request path,
all of which treat a record as naming a live server; inventing one would
route requests at a server that does not exist. The failure projection is
a **separate** table for exactly that reason.

Two tables with two readers: `failures` (affinity-keyed, for `*lsp*`)
and `failed_attachments` (buffer-keyed, carrying its affinity key, for
the modeline). Neither can serve the other's reader without doing work
in the wrong place — and, per the sweep above, they are cleared by the
same *event* but not by the same *touch*.

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

**The journey row comes first**, because it is the one that says the
stage worked.

1. **N — journey step 6, end to end** (`tests/journey_acceptance.rs`).
   That file's rule is that steps 6–12 join as later stages make them
   real, and 1b-1 added step 9 the same way. Launch on a project whose
   configured server command does not exist, open a source file through
   the real dired `RET`, and assert the user is **told**: the status
   line names the command and the modeline reads `LSP:!` rather than
   nothing. This is the ratchet row; the M4-level pins below stay and
   cover the mechanism.
2. **N — the failure is reported, through the real path.** The status
   line names the command, the language, and the underlying error.
   Driven through `buffer.after-load`, not by calling `ensure_server`
   directly — the hook's `pcall` is part of what is being tested.
3. **N — reported once per `(language, key_uri, command)`.** Open a
   second file **in the same project** (same detected root, so the same
   non-nil `key_uri`): the spawn is attempted again — observable, and
   asserted — and the message is not repeated. Falsified by dropping the
   memo.
4. **N — the markerless case shares one memo, as it shares one server.**
   Two loose files in *different* directories, neither under a project
   marker, both resolve `key_uri = nil` and report **once** between them.
   Falsified by keying the memo on the resolved root, which is rev 1's
   design: that reports twice. This pin exists because the root and the
   affinity key differ exactly here.
5. **N — a different language, or a genuinely different root, reports
   again.** Falsified by keying on the language alone, and by keying on
   the language plus a constant.
6. **N — a changed command reports again.** Repoint the config from one
   missing executable to another and open a file: a second message,
   naming the new command. Falsified by dropping `command` from the
   reported identity.
7. **N — the memo is on the report, not the failure.** After a failed
   attach, make the command resolvable and open another file: a server
   attaches. Falsified by memoizing the failure instead — §2.2's whole
   claim, and it needs its own pin.
8. **N — recovery clears both surfaces.** Continuing from 7 in the same
   session: the modeline for the recovered buffer reads its live label,
   and `M-x lsp.status` no longer lists that failure. Falsified by never
   clearing `failures` / `failed_attachments`, which is what one record
   would have forced. **Asserted in both surfaces**, because they read
   different tables (§2.5).
9. **N — recovery reaches every buffer sharing the affinity.** Buffer A
   fails; the command becomes resolvable; buffer B in the **same
   project** attaches successfully. Assert **A's** modeline no longer
   reads `LSP:!` — not B's. Falsified by clearing only the succeeding
   buffer's projection, which is rev 2's design and leaves `*lsp*` and
   A's modeline contradicting each other.
   *This pin is the reason the projection carries its affinity key at
   all*, so it must assert on A: a version that checks B passes on the
   broken implementation.
10. **N — `M-x lsp.status` renders a buffer** containing both the
    failure section and `status_buffer_text()`'s output. Asserts
    **content produced**, not that a buffer exists.
11. **N — `g` refreshes the panel.** With the panel open: make the
    command resolvable **and then reattach** — open a file in the
    project so a spawn actually succeeds — then press `g`, and the
    failure section is gone. **The reattach is load-bearing**: making
    the command resolvable changes no state by itself, since `failures`
    is cleared by a successful spawn, so a version of this pin that
    only edits the config and presses `g` would assert nothing about
    refresh. Falsified by omitting `on_refresh`, which makes
    `listview.refresh` early-return (`listview.lua:259`) and leaves the
    panel stale while `g` appears bound.
12. **N — a foreign `*lsp*` buffer is not adopted.** Create a buffer
    named `*lsp*` with user content, then run `lsp.status`: the user's
    bytes are untouched and the panel opens as `*lsp*<2>`. This is
    `listview.open`'s guarantee, and it is pinned here rather than
    assumed because "found by name is not adoption" is precisely the
    rule a hand-rolled panel loses.
13. **N — the modeline distinguishes failed from not-applicable.** A
    source file with a failed spawn renders `LSP:!`; a plain-text buffer
    renders nothing. **Both halves asserted** — a pin that checks only
    the failing case passes if the segment renders `!` unconditionally.
14. **P — the segment does no work.** With a failure recorded, painting
    the modeline invokes neither a root resolver nor
    `pmacs.project.detect`. Pinned with a counting resolver installed
    through the real config; assert the count is unchanged across
    repeated renders. Targeted mutation: rev 1's design, which derives
    the affinity key inside the provider.
15. **P — a working server is unaffected.** Attach, modeline label,
    requests: unchanged. Targeted mutation: making the new branch fire
    whenever an attachment is absent, which would mark every plain-text
    buffer failed.
16. **P — the two existing report sites still report.** Root-resolver
    and subscriber failures keep their messages. Targeted mutation:
    refactoring the three sites onto a shared helper that drops one.
17. **P — a fabricated attachment is never created.** After a failed
    spawn, `attachment_for_request` returns nil and no request is
    issued. Targeted mutation: §2.5's forbidden implementation.

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
  or 9 succeeding. 9 was closed by 1b-1. **The ratchet gains a step-6
  row** (§4 acceptance 1) — `tests/journey_acceptance.rs` states that
  steps 6–12 join as later stages make them real, so a stage that makes
  one real and adds no row leaves the ratchet describing a journey the
  editor has outgrown.
- **Interaction islands: none added.** The message uses the existing
  status line; `*lsp*` is opened through **`pmacs.listview.open`**
  (§2.3), the primitive `COHERENCE.md` §14 records as the proven
  listview/panel shape — not a third hand-rolled read-only buffer. §14's
  complaint is precisely that each new panel re-invented ownership,
  read-only enforcement, and quit behaviour; naming the primitive is how
  this stage avoids being the next example.
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
