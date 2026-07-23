# GPU initial target — session-scoped file opening framing

**Revision 2 — user-review findings resolved. Ground truth: canonical `main`
@ `4daa1b8`, protocol v19, 2026-07-23. No implementation yet.**

Revision 2 pins launcher-owned tilde expansion, requires `after-switch` even
when dedup selects the view's existing buffer, fails bootstrap when a hook
kills the target, and records the deliberate stderr-only wait during slow
pre-window bootstrap. It also sharpens the observed argv panic and negotiated
protocol-version echo.

One-command GPU startup landed in #141:

```sh
pmacs --gpu
pmacs --gpu --socket research
```

The remaining daily-use gap is that the same path cannot open a file. This
stage makes one positional target honest:

```sh
pmacs --gpu README.md
pmacs --gpu --socket research ../notes/today.md
pmacs --gpu -- --name-beginning-with-dash
```

“Honest” is load-bearing. The target belongs to the newly authenticated GPU
session, not to the daemon's ambient/local view or whichever frontend most
recently sent input. Relative paths resolve against the launcher's working
directory even when an existing daemon has a different cwd. A new or reused
file becomes the first buffer content the GPU can draw; scratch never flashes.
Open failures return before a window is shown and make the root command fail.

This is one-file startup only. It does not make GUI the automatic default, add
a general client/server `open` command, widen direct `pmacs-gpu --attach`, or
solve installation, service management, remote GPU attach, or reconnect.

## Ground truth

### Root and GPU CLI

- `src/main.rs` already parses one positional `FILE` for local TUI mode and
  rejects multiple files. It accepts `--` before an option-like filename.
- `Mode::Gpu` currently carries only `socket`; `parse_args` rejects every file
  paired with `--gpu` and points users to `C-x C-f`.
- The root broker invokes the sibling/PATH GPU binary as
  `pmacs-gpu --managed-attach SOCKET DAEMON_EXE`, waits for its status, and
  reflects success/failure.
- Both root and GPU parsers consume `std::env::args()` into `String`; Rust
  panics when an argument is not valid Unicode. They cannot admit a
  non-UTF-8 Unix filename or issue a useful parser error for one.
- The direct GPU CLI is intentionally strict. Public direct attach remains
  `pmacs-gpu --attach RAW_SOCKET`; managed and headless forms are private
  root/acceptance seams.

### Handshake and session bootstrap

- The instance sends `Hello`; the frontend sends `AttachRequest`; then the
  per-attach thread immediately sends `DispatcherEvent::SessionEstablished`
  and becomes the reader for `FrontendEvent`s.
- `AttachRequest` contains protocol version, capabilities, and initial cell
  size. It has no target. `FrontendEvent` has no open-path request.
- The GPU copies `Hello.protocol_version` into `AttachRequest.protocol_version`
  rather than sending its own maximum. Every old-daemon gate therefore keys on
  the negotiated/echoed server version; changing that echo is not cleanup.
- `handle_session_established` creates the authenticated frontend's own
  `FrontendView`, initially sharing the daemon-local active buffer, then sends
  CRDT snapshots and installs grid or semantic render state.
- A semantic frontend follows whichever `BufferSnapshot` it most recently
  applied. The attach-time snapshot sweep currently sends every CRDT-backed
  buffer; a later tick repairs the “last snapshot was not my active buffer”
  ambiguity by re-sending the session's active buffer.
- Managed GPU connect completes before winit creates the window, but normal
  instance messages are read asynchronously after that connect call. Merely
  adding an event after attach would race the first snapshot and first draw.

### File and view semantics already present

- Local `pmacs FILE` loads an existing file, or creates an empty path-backed
  buffer with `[new file]` status on `NotFound`; other I/O errors fail startup
  (`EditorState::open`).
- `EditorCore::get_or_load_buffer` and Lua `pmacs.buffer.find_or_open` already
  establish the important dedup rule: an existing path-backed buffer wins, so
  unsaved edits are not replaced by a disk reload.
- Buffer identity is an absolute, lexically normalized path. It deliberately
  does not canonicalize symlinks, so a not-yet-created file has a stable
  identity and the editor does not silently rewrite the user's spelling to a
  filesystem target.
- `FrontendView`s are per-session. Switching one frontend's active window does
  not switch any sibling frontend.
- Load/switch hooks require `EditorCore::active_frontend` to name the source
  before Lua runs; otherwise `pmacs.window.*` resolves against the wrong view.

### Replica composition constraint

A target loaded during attach may be a brand-new, non-CRDT buffer. The new GPU
needs its snapshot, but every already-attached replica must also learn that
buffer before later CRDT operations can reference it. Calling the current
one-stream `send_buffer_snapshots` helper can upgrade the new buffer for the
new GPU while leaving existing replicas unaware: the later lazy-upgrade sweep
then sees an already-backed buffer and has nothing to broadcast. The initial-
target transaction must therefore use the same all-replica publication
invariant as any other mid-session buffer creation.

## Invariants

1. **Authenticated source owns the target.** No client-supplied frontend id,
   daemon-local active view, or ambient “last frontend” selects the window.
2. **Launcher environment owns path resolution.** Root applies the existing
   leading-tilde rule with the launcher's `$HOME`; any still-relative result
   resolves against the launcher cwd. Daemon cwd/HOME are irrelevant.
3. **Path bytes survive defined resolution.** Apart from that deliberate
   root-side tilde substitution, Unix argv → broker → GPU → wire → daemon file
   I/O preserves exact `OsStr` bytes. Display text may be lossy; backing
   identity and filesystem access may not be.
4. **One target, one buffer identity.** Reuse an already-open normalized path;
   never discard unsaved edits by reloading it.
5. **Target before first draw.** The first buffer content eligible to render is
   the requested target. The explicit terminal launcher may report the wait on
   stderr; no editor window exists yet, and scratch content is never drawable.
6. **Ready means usable.** Success is reported only after the target buffer is
   loaded/created, selected in the authenticated view, CRDT-backed, and its
   matching snapshot has been written to the GPU.
7. **Failure is pre-window and fail-closed.** Non-`NotFound` I/O, malformed
   bootstrap data, CRDT upgrade/export, or target snapshot failure produces no
   successful session and no window.
8. **Existing sessions remain coherent.** A fresh target is published to all
   negotiated replica sessions; another frontend's active window never moves.
9. **No legacy wire drift.** Protocols v6–v19 retain their exact
   `AttachRequest`, `FrontendEvent`, and `InstanceMessage` encodings.
10. **No lifecycle regression.** Existing-daemon reuse, bounded daemon startup,
    process-group isolation, named child reaping, and root exit propagation
    stay owned by #141's managed path.

## Decisions

### Q#GT1 — Public grammar accepts exactly one GPU target

The public shape becomes:

```text
pmacs --gpu [--socket NAME|PATH] [--] [FILE]
```

- `FILE` is optional and may appear before or after `--gpu` / `--socket`, as the
  existing single-pass parser already permits for local mode.
- `--` makes the next and only remaining operand a literal filename, including
  one beginning with `-`.
- A second positional remains `multiple files not yet supported`.
- `--gpu FILE` remains mutually exclusive with `-nw` / `--no-window`,
  `--daemon`, `--attach`, and `--daemon-attach` under the existing mode rules.
- `Mode::Gpu` gains `file: Option<PathBuf>`; no-file behavior is byte-for-byte
  the #141 path.
- README/help examples add `pmacs --gpu FILE`; bare `pmacs FILE` remains TUI.

### Q#GT2 — Root parsing moves to `OsString` without weakening option grammar

The root entry point uses `std::env::args_os`. Parsing distinguishes:

- ASCII option names (`-nw`, `--gpu`, `--socket`, `--`, …), which must match
  exactly;
- socket names/paths, whose existing resolver contract remains UTF-8 and
  receives a targeted error when the operand is not UTF-8;
- local/managed `FILE`, stored as `PathBuf` with exact platform bytes;
- positional attach targets, which remain UTF-8 because their syntax is a
  transport URI/hostname rather than a local filesystem path.

Unit helpers may continue to construct UTF-8 `OsString`s for ordinary cases,
but one Unix-only test must pass an invalid-UTF-8 positional through the real
parser. No lossy conversion is permitted on the file path.

### Q#GT3 — The private broker handoff carries target plus launcher cwd

When `FILE` is present, root invokes the GPU child as:

```text
pmacs-gpu --managed-attach SOCKET DAEMON_EXE \
  --initial-target LAUNCHER_CWD FILE
```

Before transport, root applies the existing identity seam's tilde rule using
the **launcher** environment: a valid-UTF-8 leading whole `~` or `~/…` expands
through `std::env::var_os("HOME")`; `~user`, a non-UTF-8 spelling, or an
unset `$HOME` remains unchanged. Expansion happens exactly once, before any
cwd join. This makes quoted `~/x` dedup with an already-open `$HOME/x` buffer
and prevents a long-lived daemon's different `$HOME` from changing identity.

`LAUNCHER_CWD` is captured by root before spawn and must be absolute. The
post-expansion file and cwd are passed as `OsString`/`Path` operands, not
encoded into UTF-8, environment variables, JSON, or a delimiter-separated
string. The marker makes an option-like `FILE` unambiguous. The no-target
private argv remains unchanged. If `current_dir()` fails, root reports that
error and does not spawn the GPU; falling back to daemon cwd would violate the
target's authority.

The GPU parser also moves to `args_os` for path operands. Public help continues
to advertise only the root command and advanced direct attach; the private
marker is not promoted as a supported standalone workflow. Headless managed
acceptance gets the same optional marker so it exercises production target
transport rather than a test-only side channel.

### Q#GT4 — Protocol v20 adds a semantic-session bootstrap envelope

Protocol increments **v19 → v20** and appends v20 to
`SUPPORTED_PROTOCOL_VERSIONS`; v6 remains the compatibility floor.

`AttachRequest` stays byte-identical. After a v20 semantic frontend sends its
normal `AttachRequest`, it sends one additional framed handshake value:

```rust
pub struct SessionBootstrapRequest {
    pub initial_target: Option<InitialTarget>,
}

pub struct InitialTarget {
    pub cwd: Vec<u8>,
    pub path: Vec<u8>,
}
```

- The extra handshake message is required for `protocol_version >= 20` **and**
  negotiated `semantic_render`; `None` preserves ordinary `pmacs --gpu`.
- v6–v19 peers neither send nor read it. Non-semantic v20 grid/TUI sessions
  keep the existing two-message handshake and do not wait on an irrelevant
  semantic startup envelope.
- The target carries no `FrontendId`; the accepted stream and assigned session
  are the authority.
- `cwd` and `path` are Unix path bytes, not text. This stage is the local Unix-
  socket GPU path; it does not claim a cross-platform/remote path protocol.
- Each field is bounded to 32 KiB before allocation/use. `path` must be
  nonempty, `cwd` must be nonempty and absolute, and embedded NUL is rejected
  with a bootstrap failure.

A second message rather than an appended `AttachRequest` field keeps every
legacy postcard shape mechanically unchanged. A post-handshake
`FrontendEvent::OpenPath` is deliberately not used: it cannot precede session
bootstrap and therefore cannot guarantee first-draw ordering.

### Q#GT5 — The daemon resolves and opens inside one dispatcher transaction

The per-attach thread validates the v20 bootstrap envelope structurally, moves
it into `DispatcherEvent::SessionEstablished`, and starts its reader exactly
where it does today. It performs no filesystem or editor work.

The dispatcher, which exclusively owns `EditorState`, performs this target
transaction without yielding to another event:

1. Register the new frontend's view and set `active_frontend` to the
   authenticated `FrontendId`.
2. Accept root's already tilde-expanded `path`; if it is still relative, join
   it to the supplied launcher `cwd`, then lexically normalize without
   canonicalizing symlinks. The daemon never consults or re-applies `$HOME`.
3. Reuse an existing buffer with that normalized backing path; otherwise load
   the file; on `NotFound`, create an empty path-backed buffer and set
   `[new file]`; on any other error, take the failure path below.
4. Select that buffer in only the new frontend's active window.
5. Fire `buffer.after-switch` exactly once on every dedup, including when the
   fresh view already shares that `BufferId` and selection is a same-buffer
   no-op. Fire `buffer.after-load` on a fresh disk load. Both run with the
   authenticated frontend active; a newly created missing file matches local
   startup and fires neither load nor switch hook.
6. After hooks, verify the target `BufferId` is still live. A listener that
   killed it causes the fail-closed bootstrap path in Q#GT9. Otherwise reassert
   the requested target so configuration can inspect the right session but
   cannot make `pmacs --gpu FILE` acknowledge a different buffer.
7. Establish CRDT/snapshot coherence, then acknowledge readiness.

The target-opening helper must be Rust/editor-core state, not synthetic keys
or a call through the user-facing Lua binding. Lua hooks remain policy
observers; they are not the transport implementation.

### Q#GT6 — Dedup preserves edits; new-file behavior matches local startup

- An already-open normalized path reuses its `BufferId`, current contents,
  modified flag, and metadata. Disk is not read again.
- A disk file not already open is loaded once and receives its normalized
  backing path and `FileMeta`.
- Any `NotFound` from the initial load creates an empty path-backed buffer,
  including when a parent is currently absent; save-time errors remain
  save-time errors, matching local `pmacs FILE`.
- `PermissionDenied`, `IsADirectory`, invalid path bytes at the OS boundary,
  and other non-`NotFound` errors fail startup.
- The buffer display name may use `Path::display()` and therefore replacement
  characters; this must never replace the raw backing path used for dedup,
  load, or save.

### Q#GT7 — Initial-target semantic bootstrap is active-buffer-only

A v20 semantic session with `initial_target = Some` does **not** receive the
legacy “every buffer, then repair active on the next tick” sweep. It receives
exactly the requested active buffer's `BufferSnapshot` before readiness.
Semantic GPU state holds one active rope and already receives a fresh snapshot
when its daemon-side view switches; shipping unrelated buffers only creates a
last-snapshot ambiguity and avoidable work.

If the target was newly created or newly loaded and not yet CRDT-backed:

- upgrade it once using the daemon-owned CRDT peer;
- export one authoritative snapshot;
- publish that buffer/snapshot to every already-attached session that
  negotiated `crdt_replica` before later operations can name it;
- write the same logical snapshot to the new GPU without reloading the file or
  creating a second buffer.

The implementation should avoid cloning snapshot bytes per peer beyond what
framed serialization/write ownership requires. Existing no-target attach and
non-semantic replica bootstrap remain unchanged.

### Q#GT8 — A v20 result is the pre-window readiness barrier

Append a v20-only `InstanceMessage::InitialTargetResult` variant with an
explicit result shape:

```rust
pub enum InitialTargetResult {
    Opened { buffer_id: BufferId },
    Failed { message: String },
}
```

Only a v20 semantic session that requested `Some(initial_target)` may receive
it.

Success ordering on the daemon write stream is:

1. target `BufferSnapshot` written successfully;
2. new session state/render state/stream installed;
3. `InitialTargetResult::Opened` written;
4. normal per-tick messages/events begin.

The GPU connector synchronously reads this bootstrap prefix before returning.
It retains the matching target snapshot as structured bootstrap state,
validates that `Opened.buffer_id` matches it, and only then starts the ordinary
reader thread. The winit path applies that snapshot before its first redraw.
It does not forward unrelated pre-ready buffer state through the event proxy.
Thus no target success can mean “window exists, load may still fail,” and no
scratch snapshot can become drawable.

`Failed` becomes an `AttachClientError` containing the local display path and
daemon detail. Managed startup returns nonzero; root reflects that status.
The daemon itself remains alive, whether reused or newly spawned.

Because the connector waits before winit creates a window, slow dispatcher
work has no graphical “Connecting…” surface. This is deliberate for the
explicit terminal command: before blocking, `pmacs-gpu` writes one bounded,
lossy-display-only `opening …` notice to stderr. There is no second target
timeout beyond #141's bounded daemon-start retry; file I/O and user hooks may
legitimately exceed five seconds, and timing out the client would not cancel
dispatcher work. Ctrl-C remains the escape hatch and still cannot reach the
isolated daemon process group.

### Q#GT9 — Failure cleanup never creates a ghost session

On any target failure before readiness, the dispatcher:

- writes `InitialTargetResult::Failed` when the stream is usable;
- removes the provisional frontend view, render state, session-registry entry,
  size/baseline entries, and stream entry if any were installed;
- shuts down the connection so the per-attach reader wakes and emits at most
  idempotent detach cleanup;
- treats a target `BufferId` killed by `buffer.after-load` or
  `buffer.after-switch` as a bootstrap failure, never reasserts a stale id,
  and performs the same provisional-session cleanup;
- leaves every pre-existing buffer/view/session unchanged, except that a file
  successfully loaded before a later CRDT/export failure may remain as an
  ordinary daemon buffer. It must not become another frontend's active view.

The result error string is bounded (4 KiB) and user-facing. It includes the
operation and OS error but never lossy-converts and then reuses the displayed
path for filesystem access.

### Q#GT10 — Compatibility is directional and tested

- New daemon + v6–v19 client: exact legacy handshake; no bootstrap read, no
  v20 result, no new enum discriminant.
- New GPU without target + supported v6–v19 daemon: exact #141 behavior.
- New GPU **with** target + v6–v19 daemon: fail immediately after `Hello` with
  “initial targets require protocol v20”; do not authorize/spawn a replacement
  for the live daemon.
- New daemon + v20 non-semantic client: legacy handshake; the initial-target
  envelope is a semantic-session contract.
- New daemon + v20 semantic client: bootstrap envelope required, including
  `None`; missing/malformed bootstrap closes only that connection.
- `InitialTargetResult` is appended after all v19 `InstanceMessage` variants
  and independently filtered from negotiated `< 20` streams. Existing
  encoding pins for v6–v19 messages remain unchanged; new pins lock the v20
  discriminant and bootstrap round trip.

No new capability bit is needed. Protocol version and negotiated
`semantic_render` jointly identify the state machine; a bit would add a second
source of truth for a mandatory v20 semantic handshake step.

## Startup state machine

```text
root parses FILE bytes, expands eligible `~` with launcher HOME, captures cwd
  |
  +-- spawn/await pmacs-gpu managed child
        |
        +-- connect existing socket or start/retry daemon (#141 unchanged)
        |
        +-- Hello < 20 and target? ----> fail; never replace live daemon
        |
        +-- Hello >= 20
              send AttachRequest
              send SessionBootstrapRequest { initial_target }
                    |
                    +-- daemon dispatcher resolves/opens in source view
                    |     |
                    |     +-- error --> Failed + connection shutdown
                    |     |
                    |     +-- success
                    |           publish fresh buffer to replicas
                    |           write target BufferSnapshot
                    |           install session
                    |           write Opened { buffer_id }
                    |
              connector validates snapshot/result pair
              create GPU state/window
              apply target snapshot before first redraw
```

## Rejected alternatives

### Send `FrontendEvent::OpenPath` after attach

Rejected for initial startup. The daemon has already established the scratch
view and may have sent snapshots before the event can arrive. Correlation ids,
async results, and live session commands will be appropriate for a future
general client/server open API, but they do not provide a pre-window barrier
without duplicating the bootstrap state machine.

### Put the file only on the spawned daemon command line

Rejected. It cannot affect an already-running daemon and would target the
daemon-local view rather than the authenticated GPU view.

### Open globally, then declare a viewport for that buffer

Rejected. `Viewport` is a rendering declaration whose buffer must already be
known to the replica. Treating it as an open command confuses state alignment
with filesystem authority and can switch the wrong view under races.

### Drive `C-x C-f` / minibuffer with synthetic keys

Rejected. It depends on user keymaps and prompts, loses raw path bytes, cannot
report a structured startup result, and visibly renders intermediate state.

### Resolve relative paths in the daemon cwd

Rejected. A long-lived daemon's cwd describes when it was started, not where a
later launcher invoked `pmacs --gpu FILE`.

### UTF-8 path strings

Rejected for the local Unix contract. They silently exclude valid filenames
and would make GPU startup weaker than `PathBuf`-based local editing.

### General multi-file/open-command protocol now

Rejected. Multiple targets need ordering, active-target choice, per-target
results, and behavior for an already-running client. This stage deliberately
pins the one initial target needed by `pmacs --gpu FILE`.

## Scope and touch map

Expected implementation surface:

- `src/main.rs`
  - `args_os` parser, `Mode::Gpu { file, socket }`, help/grammar, exact private
    child argv, cwd capture, exit propagation.
- `pmacs-gpu/src/main.rs`
  - strict `OsString` path parser, optional private initial-target operands,
    managed/headless plumbing, pre-first-redraw bootstrap application.
- `pmacs-gpu/src/attach.rs`
  - v20 semantic bootstrap send, target-version gate, synchronous
    snapshot/result barrier, structured errors; #141 connector/start/reaper
    policy otherwise unchanged.
- `pmacs-protocol/src/message.rs`, `pmacs-protocol/src/lib.rs`
  - protocol v20, `SessionBootstrapRequest`, `InitialTarget`, result wire type,
    appended `InstanceMessage` variant, bounds and documentation.
- `src/daemon.rs`
  - conditional v20 bootstrap read, dispatcher payload, source-scoped target
    transaction, active-only semantic bootstrap, all-replica publication,
    result ordering, failure cleanup, `< 20` send filter.
- `src/editor_core.rs` and/or a small existing file-open helper
  - cwd-aware lexical normalization, dedup/load/create without ambient-view
    selection. Do not add a parallel path-identity convention.
- `src/protocol.rs`, `pmacs-protocol/src/transport.rs`
  - version ladder, legacy placement pins, v20 round-trip/limit tests.
- `tests/gpu_initial_target_acceptance.rs` (new) and focused existing GPU
  invocation/protocol tests
  - real subprocess/session behavior below.
- `README.md`, this framing's as-built tail, `docs/agent-handoff.md`, and
  `docs/active-work.md` per their update protocols after implementation works.

Explicit non-touch unless evidence forces it:

- no renderer/layout/font/shader changes;
- no `builtin/runtime` keymap or command changes;
- no socket resolver, daemon lock, process-group, retry, or reaper policy
  changes;
- no package/install/service files;
- no remote attach transport changes.

## Acceptance

Behavioral acceptance uses real built subprocesses where CLI, cwd, socket,
handshake, process lifetime, or raw argv is the contract. Pure parser/path/wire
helpers receive unit tests; source-text assertions do not substitute for
process behavior.

1. **Root grammar:** `pmacs --gpu FILE`, `pmacs FILE --gpu`,
   `pmacs --gpu --socket research FILE`, and `pmacs --gpu -- -leading` select
   one managed GPU target. Multiple files and every existing conflicting mode
   exit 2 with the conflicting argument/mode named. No-file `pmacs --gpu`
   remains accepted.
2. **Raw argv forwarding:** a Unix invalid-UTF-8 filename survives the real
   root parser and reaches a fake GPU child byte-for-byte alongside the exact
   launcher cwd. Invalid-UTF-8 option/socket/attach-target text fails with a
   pointed usage error rather than panic or replacement characters.
3. **Private GPU grammar:** managed and headless modes accept either their
   unchanged no-target arity or exactly `--initial-target CWD FILE`; missing,
   trailing, duplicated, or relative-cwd forms exit 2. An option-like `FILE`
   after the marker remains literal.
4. **v20 wire and legacy pins:** bootstrap request/result round trips preserve
   arbitrary Unix bytes and enforce bounds. Every pinned v6–v19 encoding stays
   unchanged; the new result discriminant is appended. The supported ladder is
   exactly `[6, …, 20]`.
5. **No-target compatibility:** rebuilt `pmacs --gpu` attaches to a v19
   fixture/daemon without sending the v20 bootstrap frame and retains #141's
   existing/missing-daemon behavior. A v20 semantic attach sends `None` and
   reaches its normal first snapshot.
6. **Old live daemon refusal:** target mode against a real/fake supported v19
   daemon fails with the protocol-v20 requirement, invokes no daemon spawner,
   leaves the socket/process untouched, and creates no GPU window.
7. **Existing-daemon path authority:** start a real CRDT daemon from cwd/HOME
   A; launch the real headless managed GPU target from cwd/HOME B with
   `sub/file.txt`; require contents from B, not A. Repeat with a shell-quoted
   `~/file.txt` after that `$HOME_B/file.txt` buffer is already open, and
   require the same `BufferId`, proving root-side expansion and dedup.
8. **Missing-daemon target:** from no socket/lock, the production managed path
   starts one daemon, opens the target, reaches ready, and leaves that daemon
   connectable after the probe/window exits. Repeating reuses it and creates no
   replacement.
9. **New file:** a nonexistent target produces an empty snapshot, `[new file]`
   status/path identity, accepts an edit/save through the real session, and
   creates the requested file under the launcher cwd—not the daemon cwd.
10. **Open error:** a directory/permission-denied target returns a specific
    failure before ready/window creation and makes root fail. An existing daemon
    remains connectable; a pre-existing frontend's active buffer and contents
    remain unchanged.
11. **Dedup preserves unsaved edits:** frontend A opens and modifies a file
    without saving; target-launch frontend B opens the same normalized path and
    receives A's authoritative unsaved text with the same `BufferId`, not disk
    contents or a duplicate buffer.
12. **Per-session isolation:** frontend A starts on buffer A while concurrent
    target launches B and C open different files. Each result/snapshot pair
    names its own view; a subsequent input/resize proof shows A, B, and C remain
    independently usable on their original buffers.
13. **Fresh-buffer publication:** keep replica A attached, then target-launch B
    onto a previously unknown file. A receives the new buffer snapshot before
    any CRDT op for it; both replicas accept later operations without unknown-
    buffer fallback or disconnect.
14. **Hook context and count:** fresh disk load fires `buffer.after-load` once;
    dedup fires `buffer.after-switch` once even when the fresh view already
    shares that exact buffer and the select itself is a no-op; missing-file
    creation fires neither hook. Each hook observes the authenticated frontend
    and requested buffer, and the target remains selected afterward. A fixture
    that kills the target inside either hook yields `Failed`, no ready phase,
    and no stale-id reassertion.
15. **Pre-window barrier:** the headless seam records that the only initial
    semantic `BufferSnapshot` is the target and precedes matching `Opened`.
    Injected load/export failure records no ready phase. A window-path unit seam
    proves bootstrap state is applied before the first redraw request. A held
    dispatcher fixture proves the bounded stderr notice appears while no
    window/ready result exists, then succeeds after release without a separate
    target timeout.
16. **Concurrent same-target launch:** two target launchers racing an absent
    daemon and the same file converge on one daemon, one buffer identity, two
    ready sessions, and the existing #141 losing-daemon child is reaped.
17. **Existing direct/probe paths:** `pmacs-gpu --attach RAW_PATH`, the no-target
    managed probe, Vterm Stage 3's headless probe, TUI attach, protocol/capability
    mismatch preservation, Ctrl-C daemon isolation, and post-attach reaping
    remain green.
18. **Visible smoke:** on the real Wayland/Vulkan workstation, run
    `target/release/pmacs --gpu README.md`; the first editor content shown is
    README (never scratch), editing works, closing the window leaves the daemon
    alive, and a second invocation from another cwd opens that cwd's target in
    a new session without moving an already-attached frontend.
19. **Executable documentation:** the coherent workspace build from README
    still produces sibling binaries; all documented target commands parse;
    no command requires spelling the resolved socket pathname.

## Required implementation gates

After the visible/behavioral path works, run the repository gates independently
as required by `AGENTS.md`:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --lib
cargo test --lib --features crdt
cargo test --test gpu_initial_target_acceptance
cargo test --features crdt --test gpu_initial_target_acceptance
cargo test --test m4_acceptance -- --skip basedpyright
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
git diff --check
```

Also rerun the touched GPU invocation suite, protocol/transport tests, and
Vterm Stage 3 acceptance in default and CRDT configurations where the suite
supports both. The final full workspace sweep remains required before PR.

## Deferred (named)

- **Multiple initial files.** Needs result ordering, active choice, and partial
  failure policy; do not turn `Option<InitialTarget>` into a vector casually.
- **General client/server open command.** A live frontend request needs request
  ids, asynchronous per-target results, and clear behavior when no frontend is
  waiting. It may reuse the file transaction, not the bootstrap wire phase.
- **GUI as automatic default.** After this stage and distribution are proven:
  display detection, `PMACS_FRONTEND`, and `-nw` precedence can make bare
  `pmacs FILE` select GPU.
- **Distribution/install bundles.** Ensure root and GPU binaries land together
  before making GPU the default.
- **Automatic reconnect/resync.** Startup target success does not reconcile an
  optimistic replica after a later disconnect.
- **Remote GPU paths.** Raw Unix path bytes and launcher cwd are intentionally
  local. Remote transports need an explicit remote-cwd/path authority model.
- **Daemon services and idle shutdown.** Unchanged from #141's deferral.
- **Direct `pmacs-gpu` target syntax.** Normal users go through root; advanced
  raw-socket attach stays attach-only.

## Approval boundary

Approval of this framing authorizes one implementation branch/PR for
`pmacs --gpu [--socket …] FILE`, protocol v20 semantic bootstrap, exact Unix
path/cwd transport, target-before-first-draw readiness, and the acceptance
matrix above. It does not authorize automatic GUI selection, multiple files,
or a general open-command protocol.
