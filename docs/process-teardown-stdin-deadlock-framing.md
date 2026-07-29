# Framing — close child stdin before joining readers (process teardown deadlock)

A pipe-mode child that exits on stdin EOF can deadlock the supervisor's
teardown forever. `RuntimeHandles::drop` joins its reader threads in the
`Drop` body, which runs **before** the `stdin` field drops, so the child
never receives the EOF that would make it close the very pipe write ends
those readers are blocked on. The fix is a two-line reorder that reuses a
mechanism already present in this file.

This is the diagnosed root cause of
`m4_5_basedpyright_initializes_and_negotiates_encoding` hanging
indefinitely — the hazard that has parked `cargo test --workspace` runs
(once for 2h26m) and forced `-- --skip basedpyright` into every gate
recipe.

**Scope: `src/process.rs` only. No protocol change. No Lua surface. No
new primitive.**

---

## Revision history

- **rev 1** — initial framing. Root cause established by live diagnosis
  (gdb stacks + `/proc` fd forensics on a wedged process), reproduced
  5/5 deterministically at `e003b81`.
- **rev 2** — review round 1. rev 1's synthetic child was **itself
  vacuous** (the third in this lane): POSIX assigns `/dev/null` to a
  background job's stdin when job control is off, so `sh -c 'cat &
  exit 0'` EOFs instantly and exits against the *unfixed* tree.
  Q#TD6 now uses the explicit-redirect form and criterion 2 gains a
  positive control. Also: Q#TD3's bound widened to cover a blocked
  stdin writer (a child that read stdin but stopped draining it),
  criterion 5 extended to `docs/agent-handoff.md` §3, Bet 4 marked as
  lane-stopping.

---

## 0. Coherence impact (COHERENCE §20)

This is a defect fix, not coherence work, and it should not claim
otherwise.

- **Journey steps touched:** none directly. It protects the steps that
  depend on a live language server (§2 step 6 onward) from an unbounded
  teardown, but it adds no journey surface.
- **Interaction islands added:** none.
- **Config registry:** no new options.
- **Background-work attribution:** unchanged. The supervisor's process
  model is untouched; only the order of two teardown operations moves.
- **Protocol:** unchanged.

The one genuine coherence connection is indirect and worth stating
plainly: the hang parks `cargo test --workspace`, which is the ratchet
every COHERENCE priority is verified against (§19, §25). A gate that can
hang forever degrades every other lane's evidence. That is the argument
for doing this now rather than parking it — not a claim that it advances
a priority.

---

## 1. Ground truth (scouted @ `e003b81`)

Line numbers are hints; symbols are authoritative.

### 1.1 The reproduction is deterministic, not intermittent

`docs/agent-handoff.md` and the test-improvement audit both describe this
hang as intermittent. On a machine where `basedpyright-langserver`
resolves to a uv-installed shim it is **completely reliable**: 5 runs, 5
hangs, via

```
cargo test --test m4_acceptance -- --exact \
    m4_5_basedpyright_initializes_and_negotiates_encoding
```

§1.7 explains why it looks intermittent across machines. The practical
consequence: this defect is directly testable, and any fix has a
revert-bite.

### 1.2 The cycle, in five links

Observed stack of the wedged test thread (gdb, `sudo` required —
`ptrace_scope=1`):

```
tests/m4_acceptance.rs:1374   Rc<RefCell<ProcessSupervisor>> dropped
→ ProcessSupervisor::drop        src/process.rs:1545
→ ProcessSupervisor::shutdown    src/process.rs:1463
→ ProcessSupervisor::tick        src/process.rs:1188
→ ProcessSupervisor::poll_one    src/process.rs:1268  (drop site: :1337)
→ RuntimeHandles::drop           src/process.rs:631
→ JoinHandle::join                             ← blocked, indefinitely
```

The links:

1. **`RuntimeHandles::drop` (`:631`)** sets `cancel`, then joins every
   handle in `self.readers`.
2. **Rust runs a type's `Drop::drop` body before dropping its fields.**
   `stdin: Option<StdinWriter>` is a *field* (`:505`), so it cannot drop
   until the body returns. The body never returns.
3. **`StdinWriter` (`:556`)** holds the `Sender`; `StdinWriter::spawn`
   (`:568`) moves the `ChildStdin` sink into its thread, which drops the
   sink only once `rx.recv()` errors. Sender alive ⇒ sink alive ⇒ **the
   child's stdin write end never closes.**
4. **The child therefore never sees EOF**, stays alive, and keeps the
   stdout/stderr **write** ends it inherited.
5. **The readers are blocked in `read()`** at `:1886` inside
   `spawn_reader` (`:1874`). `cancel` is consulted only at the loop top
   (`:1883`) and around `send_timeout` — **never while `read` is
   blocked.**

Verified on the live process: the test held fd 4 (child stdin, WRONLY)
and fds 5 and 7 (stdout/stderr, RDONLY); the server process held the
matching opposite ends on fds 0, 1, 2. Two reader threads sat in
`anon_pipe_read`, and the stdin-writer thread sat parked in
`Receiver::recv` at `:577` — alive, still owning the sink.

Confirmation from the other direction: when the wedged test process was
killed, its fd 4 closed, the server immediately saw stdin EOF and
exited. The cycle's load-bearing link is exactly the one the fix cuts.

### 1.3 The existing comment names the false premise

`RuntimeHandles::drop` documents its own reasoning:

> Wake any reader thread blocked in a bounded `send` — dropping the
> master closes the kernel pipe and unblocks `read`, but does nothing
> for a reader stuck on a full channel […]

The premise is true for a **PTY master** and false for **pipe mode**,
where `read` unblocks only when *every* write end closes. `cancel` was
introduced for the full-channel case and is correct for it; the comment
mistakenly treats the `read` case as already handled.

### 1.4 `shutdown()`'s SIGKILL phase is unreachable on this path

`shutdown()` (`:1463`) sends SIGTERM to all ids, then runs a bounded
grace loop (`deadline` at `:1476`) that calls `tick()`, and *then*
escalates to SIGKILL. The stack shows the deadlock occurs **inside that
grace loop's `tick()`**, because `poll_one` drops `RuntimeHandles` the
moment it observes the recorded pid exited. The SIGKILL phase is never
reached.

So "shutdown force-kills everything first" is not true of this path.
(An earlier working assumption of mine said it did; the stack refutes
it.) Even if reached, SIGKILL targets the *recorded* pid, which per
§1.7 is not the surviving process.

### 1.5 Only pipe-mode, non-group spawns are affected

`spawn_pipes` (~`:1712`) chooses per stream:

| `spec.group` | reader | cancellable mid-`read`? |
| --- | --- | --- |
| `true` | `spawn_group_reader` (`:1941`) — `O_NONBLOCK` + `poll` | **yes** |
| `false` | `spawn_reader` (`:1874`) — blocking `read` | **no** |

PTY mode (~`:1724`) also uses `spawn_reader`, but there §1.3's premise
holds: dropping the master genuinely ends the read. The `spawn_ansi_parser`
reader also lives in `readers`, and reads a channel rather than an fd, so
it is unaffected.

Non-group pipe consumers are, per `spawn_reader`'s own doc comment, the
**REPL and LSP** paths. This defect is therefore reachable by every LSP
server and every REPL — not by terminals.

### 1.6 The fix mechanism already exists in this file

`close_stdin` (`:1611`) already does precisely what is needed, and
already documents the semantics and the idempotence:

```rust
// Dropping the writer closes the pipe at the kernel
// level. `take()` is idempotent — second call sees None.
let _ = runtime.stdin.take();
```

The fix is applying an existing, already-reviewed mechanism at the one
site that is missing it. It introduces no new concept.

### 1.7 Why basedpyright wedges and clangd/gopls do not

`basedpyright-langserver` is a uv-installed **Python console script**:

```python
from basedpyright.langserver import main
sys.exit(main())
```

`main()` spawns the bundled `node …/langserver.index.js --stdio` and the
Python process exits, so the real server is an **orphaned grandchild**
(observed `PPid: 1`, reparented to systemd) holding the inherited pipe
fds. The supervisor recorded the shim's pid, which has already exited and
been reaped, so `poll_one` sees a terminated process on its very first
tick and proceeds straight into the deadlock.

`clangd` and `gopls` are real binaries: genuine children, reaped
normally, write ends closed, blocking `read` returns `Ok(0)` cleanly. The
"intermittency" in the handoff is not timing — it is *which server binary
is installed how*.

### 1.8 Limits of the evidence

- The deterministic reproduction is **one machine, one server**. The
  causal chain is verified there link by link; its generality to other
  shim-launched servers is reasoned, not measured.
- The gdb capture is a single sample of a state that was stable across a
  four-minute window and identical across two independent runs. That is
  strong for a deadlock and would be weak for a race.
- Nothing here establishes how often the hang has fired in CI. CI never
  installs basedpyright (`PMACS_REQUIRE_PYRIGHT` is deliberately never
  set, #194), so in CI this test skips and the defect is **dark**. Every
  observation is local.

---

## 2. Decisions

### Q#TD1 — the fix is a reorder inside `Drop`, not a new primitive

```rust
impl Drop for RuntimeHandles {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Close the child's stdin BEFORE joining. A stdio child exits
        // on EOF and closes its stdout/stderr write ends, and that —
        // not `cancel` — is what unblocks a reader parked in `read`
        // (`cancel` is only observed between reads and around `send`).
        // The sink lives in the `stdin` field, which cannot drop until
        // this body returns, so joining first deadlocks against it.
        let _ = self.stdin.take();
        for h in std::mem::take(&mut self.readers) {
            let _ = h.join();
        }
    }
}
```

Rejected alternative: reordering the struct's *fields*. Field order does
not help — the explicit `Drop::drop` body runs before **all** fields
regardless of their declaration order. This is the trap that makes the
bug non-obvious, and it belongs in the comment.

### Q#TD2 — the reorder is unconditional across modes

Applying it only to pipe+non-group would require `RuntimeHandles::drop`
to learn which mode it is in, which it currently does not need to know.
Closing stdin before teardown is correct in both modes, so the reorder is
unconditional.

This is a uniformity change, and uniformity changes in this repo have
made total functions partial before. It is therefore carried as a **bet
with a named falsifier** (§3, Bet 2), not as an assumption: PTY-mode
`stdin` is the pty *writer*, and dropping it while `pair.master` and the
cloned reader still exist must not end the read early.

### Q#TD3 — the fix assumes the child drains stdin to EOF, and covers nothing outside that

Stated up front because it bounds the claim: the fix works by making the
child exit. A child that never reads stdin — or reads it and ignores EOF
— keeps its write ends open and still wedges the join.

There is a third member of that family, and it is not covered by the
wording above because such a child *did* read stdin: **the EOF is only
delivered if the writer thread reaches the end of its queue.** Its body
is a blocking `sink.write_all(&bytes)` (`:578`), so a child that has
stopped draining stdin while queued bytes remain blocks the writer
indefinitely — the sink never drops, EOF never arrives, and the join
re-wedges. This needs only a full stdin pipe buffer at teardown time, not
a misbehaving child. For LSP teardown the queue is near-empty and the
practical risk is nil, but the bound belongs in the claim: **the fix
assumes the child keeps draining stdin until EOF.** A full stdin pipe
with a non-draining child is P1's case as well.

Covering *that* case requires making the blocking `read` itself
cancellable, i.e. moving non-group readers onto `spawn_group_reader`'s
`O_NONBLOCK` + `poll` mechanism. `spawn_reader`'s doc comment already
names this as a deferral from the compile-mode framing. It stays parked
(§5, P1) rather than riding this PR, because it is a behavioural change
to every REPL and LSP ingest path and deserves its own review.

The honest claim for this PR is therefore: **it fixes the observed
deadlock for stdio children that honour EOF, which is what LSP servers
are, and narrows — not eliminates — the class.**

### Q#TD4 — queued stdin writes are not lost, and the writer is not joined

`crossbeam`'s `Receiver::recv` drains buffered items before reporting
disconnection, so dropping the `Sender` still lets the writer thread
write everything already queued. The writer thread is **not** joined
here, so there remains no guarantee the final flush completes before the
process is signalled. That is pre-existing, unchanged by this PR, and
noted rather than fixed (P3, §5).

Draining is also the mechanism by which the fix can fail to deliver EOF
at all when the child has stopped reading — see Q#TD3's third case.

### Q#TD5 — the leaked orphan server is not fixed here

After the fix, the wedge is gone but a shim-launched server is still an
orphaned grandchild that teardown's recorded pid cannot signal. It exits
here only because it honours stdin EOF — by cooperation, not by
enforcement. A server that ignores EOF leaks. Parked (§5, P2).

### Q#TD6 — the synthetic reproduction must model EOF-honouring, not sleeping, and needs an explicit stdin redirect

Two distinct traps here, and this lane has now walked into **three**
vacuous reproductions, so the reasoning is recorded rather than the
conclusion alone.

**Trap 1 — a sleeping child models the wrong defect.**
`sh -c 'sleep 300 & exit 0'` orphans a grandchild that holds the write
ends but **never reads stdin**, so closing stdin does not free it. That
reproduces a hang this fix does *not* address; it belongs to P1 (§5), not
here.

**Trap 2 — a background job does not inherit stdin.** POSIX XCU §2.9.3:

> If job control is disabled, the standard input of an asynchronous
> list, before any explicit redirections, shall be assigned to
> `/dev/null`.

Job control is off in every non-interactive `sh`, so in
`sh -c 'cat & exit 0'` the background `cat` gets **`/dev/null`**, not the
inherited pipe. It EOFs immediately and exits **against the unfixed
tree** — the test would pass either way and Bet 3's revert-bite would
report VACUOUS.

Measured on this machine (`/bin/sh` → `bash`), stdin attached to a
held-open fifo, checking the orphan's `/proc/<pid>/fd/0`:

| form | grandchild | fd 0 |
| --- | --- | --- |
| `sh -c 'cat & exit 0'` | **gone** | — (EOF'd from `/dev/null`) |
| `sh -c 'cat <&0 & exit 0'` | alive | the real pipe |

**The faithful model is therefore `sh -c 'cat <&0 & exit 0'`.** The
explicit redirect is what defeats the `/dev/null` assignment; it is
load-bearing, not incidental, and must not be "simplified" away.

`sh` exits immediately (so `poll_one` observes termination), `cat` is
orphaned holding the real stdin read end plus both write ends, and it
exits on EOF exactly as a stdio language server does. Unfixed, this
deadlocks; fixed, teardown completes.

Which `/bin/sh` applies the rule how varies by machine, so the redirect
alone is not enough of a guarantee — criterion 2 carries a positive
control (§4) so the test cannot silently degrade back into modelling the
wrong thing on someone else's box. This is #192's lesson one level down:
the bite needs a control, and so does the reproduction.

---

## 3. Bets (falsifiable)

1. **The reorder resolves the observed hang.** Falsified if
   `m4_5_basedpyright_initializes_and_negotiates_encoding` still fails to
   terminate after the change.
2. **The reorder is safe for PTY mode.** Falsified by any regression in
   `vterm_stage1/2/3_acceptance`, `terminal_config_acceptance`,
   `terminal_copy_mode_acceptance`, `m6_4/m6_5_repl_acceptance`,
   `m6_7_scrollback_acceptance`, `m6_8_multi_repl_acceptance`, or
   `worker_shutdown_acceptance`.
3. **The synthetic test bites.** Falsified if the new test passes with
   `let _ = self.stdin.take();` removed. This must be checked by actual
   revert, per the standing rule that a new pin needs its own bite.
4. **The basedpyright test passes rather than merely terminating.** The
   hang is at teardown (`m4_acceptance.rs:1374`), *after* the body's
   assertions, so it should now pass outright. Falsified if it terminates
   with a failure — which would mean a second, independent defect.
   **If falsified, stop the lane and frame that defect separately.** Do
   not paper over it: "terminates" was never the goal, and a failing
   assertion here is new information, not a loose end.

---

## 4. Acceptance

1. `RuntimeHandles::drop` takes `stdin` before joining readers, with a
   comment naming the drop-body-before-fields trap.
2. New unit test in `src/process.rs` (so it runs under the standard
   `cargo test --lib` gate, not only an acceptance suite):
   `teardown_closes_stdin_before_joining_readers`.
   - Spawns `sh -c 'cat <&0 & exit 0'` as a **non-group pipe** process.
     The `<&0` is load-bearing (Q#TD6) and gets a comment saying so.
   - **Positive control, before teardown starts:** assert the orphaned
     grandchild is alive *and* that its `/proc/<pid>/fd/0` is not
     `/dev/null`. Without this the test silently degrades into modelling
     the wrong thing wherever `/bin/sh` behaves differently, and reports
     green while doing it.
   - `#[cfg(target_os = "linux")]`: the control reads `/proc`, and the
     reproduction depends on `sh` async-list semantics. Gate it
     explicitly and say why, rather than letting it be incidentally
     Linux-only. (Same reasoning as the APFS gate — `cfg(unix)` would be
     wrong here.)
   - Performs the full reap-and-drop sequence on a helper thread and
     asserts completion via `recv_timeout`, so a regression **fails**
     within a bounded window instead of hanging. A test that hangs on
     regression would reproduce the exact hazard this PR removes.
   - Bound: 10s (default `grace_period` is 2s, `:927`).
   - On the failure path the helper thread stays wedged and the `cat`
     survives until the harness's fds close at process exit. That is
     bounded and acceptable — but the test comment must **say so**, or a
     future reviewer correctly flags a leaked thread as a defect.
3. The bite is demonstrated by revert, and the result recorded in the PR
   body — pass/fail both ways, per Bet 3.
4. `cargo test --test m4_acceptance` runs **without**
   `-- --skip basedpyright` and completes, locally, on the machine where
   it currently hangs 5/5.
5. Docs, in **both** places the superseded cause lives — replacing it,
   not appending to it:
   - `docs/agent-handoff.md` §5 gains the drop-body-before-fields lesson
     and the corrected cause, replacing "no timeout on the initialize
     handshake".
   - `docs/agent-handoff.md` §3's machine caveat currently says the
     desktop's **local binary is broken and hangs**. §1.7 shows the
     binary was never broken: the shim architecture plus this defect
     was. Left alone, §3 keeps steering readers toward a false model —
     and toward keeping the skip forever.

**Deliberately not a criterion:** removing `-- --skip basedpyright` from
`CLAUDE.md`'s standing gate list. It is a separate call that is the
user's to make, and it changes only *local* behaviour — CI skips the test
regardless (§1.8). I will propose it with evidence after the fix has been
green repeatedly, rather than fold a process change into a defect fix.

When that proposal comes it owes two things beyond the green runs: the
`docs/agent-handoff.md` §3 caveat updated (criterion 5 covers it here,
but the *skip* rationale lives with it), and an explicit note that
`PMACS_REQUIRE_PYRIGHT` stays **unarmed** in CI until the per-test
timeout lane (3a) merges — the ordering #194 established, where presence
of the variable decides execution and arming without a timeout would give
CI the same unbounded hang this PR removes locally.

---

## 5. Parked (each needs its own evidence)

- **P1 — cancellable non-group `read`.** Move `spawn_reader` onto
  `spawn_group_reader`'s `O_NONBLOCK` + `poll` mechanism so `cancel` is
  observed within `READER_SEND_POLL_INTERVAL` (`:421`, 50ms) even
  mid-`read`. Bounds teardown unconditionally, including for children
  that ignore EOF (Q#TD3) — **and** the blocked-writer case, where EOF is
  never delivered because `write_all` is stuck on a full pipe. Already
  named as a deferral by `spawn_reader`'s own doc comment. Tests: the
  `sleep 300` shape from Q#TD6 (child never reads stdin), plus a
  fill-the-pipe-then-stop-reading shape for the writer case.
- **P2 — orphaned-grandchild lifecycle (Q#TD5).** Spawn stdio servers in
  their own process group and signal the group, reusing the machinery the
  group path and `reap_ledger` already have. Fixes a real leak: every
  basedpyright-backed session currently leaves a `node` process behind.
- **P3 — join the stdin writer thread** so the final flush is ordered
  against child termination (Q#TD4).
- **P4 — re-audit the "intermittent" label** in `docs/agent-handoff.md`
  and the audit now that §1.7 explains it. Rides this PR's doc update
  only insofar as criterion 5 requires; a broader sweep is separate.

---

## 6. Gates

Per `CLAUDE.md`, each as its own step with a real exit status checked
(never `cmd | tail` — a pipe returns the tail's status and has masked a
real failure here before):

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --lib`
- `cargo test --lib --features crdt`
- `cargo test --test m4_acceptance` — **without** the basedpyright skip
- The PTY/REPL suites named in Bet 2
- `cargo test --test worker_shutdown_acceptance`
- `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`
- `git diff --check`

Commit before gating, so the results describe the pushed tree.

---

## 7. Branch plan

One branch, one PR: `process-teardown-stdin-deadlock`, from `main` @
`e003b81` or later. Worktree `pmacs-hang` (already clean at that SHA).

Small diff — the reorder, one unit test, one comment, the handoff
update. P1–P4 do not ride it.

`docs/active-work.md` is integrated **late**, immediately before pushing,
to avoid the ledger-contention treadmill with the other open lanes.
