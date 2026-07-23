# GPU invocation — one-command broker framing

**Revision 6 — second implementation review complete on `gpu-invocation`.
Ground truth: canonical `main` @ `96d0bae`, protocol v19, 2026-07-23.**

The GPU editor works, but reaching it is still a development-session ritual:
build two packages with different feature requirements, keep a foreground
daemon alive in one terminal, reconstruct its resolved Unix-socket path, and
pass that raw path to a second binary in another terminal. This framing makes
the normal local GPU path one explicit command:

```sh
pmacs --gpu
pmacs --gpu --socket research
```

This is an additive first stage. It does **not** yet change bare `pmacs` from
TUI to GUI, and it does not pretend that `pmacs --gpu FILE` works before the
daemon has a real per-frontend initial-file contract. It preserves the
separate `pmacs-gpu` binary and its independent dependency graph.

Revision 2 closes the first review round: the spawned daemon is isolated from
the launcher's foreground process group; the required Vterm headless probe is
retained; retry/error/reaping behavior is complete; a real display-less
managed-attach seam is named; bare `--socket` stops being silently ignored;
and the non-CRDT gate is explicitly justified as default-socket protection.

Revision 3 makes the managed probe deterministic for signal/reaper tests,
keeps `Interrupted` / `WouldBlock` transient inside the post-spawn retry
window, states the process-group signal simulation in CI-executable terms,
and distinguishes the socket type check from liveness inference.

Revision 4 records the as-built cutover: the root broker, strict GPU CLI,
managed connector, process-group isolation, named child reaper, deterministic
managed probe, acceptance suite, coherent workspace build, and one-command
visible smoke are implemented and verified.

Revision 5 closes the first implementation review. Managed attach now buffers
messages that arrive before winit creates application state, spawned-daemon
ownership remains local until the named reaper accepts it, and the daemon
inherits no launcher stdio. Direct GPU help points normal users to the root
broker and labels raw socket attach as advanced. The retry, timeout, socket
type, and concurrent-loser contracts now have deterministic behavioral tests.

Revision 6 closes the remaining non-blocking review findings. The managed
probe throttles after its event channel disconnects; option-like path operands
are rejected; cleanup never signals an already-reaped daemon PID; and the
acceptance suite now proves both pre-I/O non-CRDT gating and post-SIGINT use of
a frontend attached before the launcher exits.

## Ground truth

### Current user path

The daemon is a mode of the root `pmacs` binary; there is no
`pmacs-daemon` executable (`Cargo.toml`, `src/main.rs`). `pmacs-gpu` is a
separate unpublished workspace package and binary
(`pmacs-gpu/Cargo.toml`). The current source-checkout path is:

```sh
cargo build --release --workspace --features pmacs/crdt

# terminal 1
target/release/pmacs --daemon

# terminal 2
runtime="${XDG_RUNTIME_DIR:-/tmp/pmacs-$(id -u)}"
target/release/pmacs-gpu --attach "$runtime/pmacs/default.sock"
```

The unified workspace build above succeeds. The README currently documents
two separate build commands instead. Its `cargo run --release -- <file>`
example is not runnable as written: the root package has no `default-run`,
and Cargo reports that it cannot choose among `pmacs`, `pmacs-audit`,
`pmacs_fake_lsp`, and `pmacs_fake_mcp`.

A live smoke on this base exposed the cost of independent builds: the first
`target/release/pmacs-gpu` was protocol v15 while the daemon was v19. The GPU
opened but rejected the attach. Rebuilding `pmacs-gpu` produced a successful
v19 attach. The handshake caught the mismatch correctly; the invocation path
made it easy to create.

### Existing CLI and process boundaries

- `pmacs [FILE]` runs the in-process TUI. `-nw` / `--no-window` is already
  parsed as an explicit TUI choice, though it is currently equivalent to the
  default (`src/main.rs:94-131,288-318`).
- The source comment at `src/main.rs:28-49` reserves the future shape:
  explicit `-nw` wins, a future `--gui` can select GUI, then an environment /
  display-based default may choose GUI. None of that GUI selection is
  implemented today.
- `pmacs --daemon [--socket NAME|PATH]` runs in the foreground. Bare names
  resolve to `<runtime>/pmacs/NAME.sock`; omission means `default.sock`; a
  value containing `/` is used as a path (`src/socket_path.rs`).
- `<runtime>` is nonempty `$XDG_RUNTIME_DIR`, otherwise
  `/tmp/pmacs-<uid>`. The daemon creates a private parent, locks a sibling
  lockfile, removes a stale socket only after acquiring the lock, and binds
  the socket owner-only (`src/socket_path.rs`, `src/lockfile.rs`,
  `src/daemon.rs:437-530`).
- `pmacs-gpu --attach <socket>` takes a raw pathname. It has no default/name
  resolver and no file positional (`pmacs-gpu/src/main.rs:548-565,802-848`).
- `pmacs-gpu` has three current modes: bare hello-world, direct `--attach`,
  and the test/acceptance seam `--headless-probe <socket> <report>`. The
  required Vterm Stage 3 acceptance invokes that third mode as a real
  subprocess (`tests/vterm_stage3_acceptance.rs:679-691`).
- Bare `pmacs-gpu` still opens the inert Session-2 `hello, pmacs` window. It
  is scaffolding, not an editor session.
- The GPU parser consumes the attach/probe operands but does not reject later
  argv, so trailing values are silently ignored.
- The GPU package intentionally depends on `pmacs-protocol`, not the root
  editor crate. The distribution decision in
  `docs/pmacs-gpu-design.md:195-202` keeps wgpu, winit, font, and
  window-system dependencies out of TUI-only installs. That boundary still
  holds.

### Capability and attach constraints

A usable GPU daemon must advertise all of `multi_frontend`, `crdt_replica`,
and `semantic_render`. The root `crdt` feature enables those capabilities;
`pmacs-gpu` itself has no Cargo features. The GPU validates the daemon's
`Hello` before sending `AttachRequest` and produces an actionable capability
mismatch instead of waiting forever (`pmacs-gpu/src/attach.rs`).

`pmacs-gpu` currently attempts one `UnixStream::connect` from
`ApplicationHandler::resumed`. A connect/handshake failure stays visible in
the window but is not retried. Later disconnect also requires a manual
relaunch. Automatic reconnect is an existing named deferral, separate from
startup (`docs/gpu-attach-robustness-framing.md:175-186`).

The root already has daemon auto-start precedent in `src/daemon_attach.rs`:
try an existing socket, otherwise spawn `current_exe --daemon --socket PATH`,
wait up to five seconds, then enter its byte bridge. That helper must retain
the successful connection because a disposable connect probe makes the
daemon send `Hello` into a stream the probe drops, producing a broken pipe.
The GPU cannot directly reuse the helper: it lives in the root crate and
returns the stream to the stdio bridge, while `pmacs-gpu` owns its own direct
Unix transport.

The SSH-side daemon auto-start precedent does not isolate the daemon into a
new process group: `daemon_attach.rs` relies on the SSH spawn having no
controlling terminal (`src/daemon_attach.rs:53-60`). A local `pmacs --gpu`
launcher does have one. Without an explicit process-group split, root, GPU,
and the auto-started daemon inherit the foreground group; terminal Ctrl-C
reaches all three, and the daemon deliberately treats SIGINT as graceful
shutdown (`src/daemon.rs:617-633`). SIGHUP is already ignored, so foreground
SIGINT is the specific lifecycle gap.

### Initial-file constraint

Every daemon attachment currently receives a fresh scratch view in
`handle_session_established` (`src/daemon.rs:1533-1552`). The code explicitly
names cloning or taking an initial-buffer argument as future work. Opening a
file in the daemon before attach would not put that file in the new GPU
frontend's view. There is no frontend `OpenPath` event and no initial target
in `AttachRequest` (`pmacs-protocol/src/message.rs:2023-2040`).

Therefore a launcher that accepts `FILE` without new daemon/session work
would either ignore it, drive the minibuffer by synthetic keys, or open it in
the wrong view. All three are rejected.

### Distribution gaps

There is no editor installation recipe, desktop entry, user service,
Make/Just target, Cargo alias, or launcher wrapper. The only repository
script is the test-development helper `scripts/bite`. Cargo does not install
repository shell scripts, and `pmacs-gpu` has `publish = false`.

## Decisions

### Q#GI1 — Add the explicit root command `pmacs --gpu [--socket NAME|PATH]`

The first-stage public surface is:

```text
pmacs --gpu [--socket NAME|PATH]
```

It is additive. Bare `pmacs` and `pmacs FILE` continue to run the local TUI;
`pmacs -nw` remains the explicit TUI spelling. `--gpu` is mutually exclusive
with `-nw` / `--no-window`, `--daemon`, `--attach`, and `--daemon-attach`.
It rejects every positional argument with:

```text
pmacs: --gpu does not yet accept FILE; open it from the GPU with C-x C-f
```

The parser also closes the adjacent existing hole: `--socket` without one of
`--daemon`, `--attach`, `--daemon-attach`, or `--gpu` exits 2 instead of being
silently discarded by `Mode::Local`. A flag is used rather than a `pmacs gpu`
subcommand because `gpu` is a valid existing positional filename. No
`PMACS_FRONTEND` environment selection and no display auto-detection land in
this stage.

Rejected alternatives:

- **Bare `pmacs` becomes GUI immediately** — this mixes launcher correctness,
  daemon lifecycle, initial-file semantics, and a default-behavior change in
  one cut. `-nw` reserves that eventual migration; it does not make an
  incomplete migration safe.
- **A shell wrapper** — not installed by Cargo, duplicates readiness and path
  policy, and makes version-skewed binaries easier to combine.
- **Link GPU into the root binary** — violates the deliberate independent
  dependency graphs and adds wgpu/window dependencies to TUI-only builds.

### Q#GI2 — Root owns policy and paths; `pmacs-gpu` owns the successful connection

The root launcher owns:

1. CLI validation;
2. the canonical `resolve_socket_path` call;
3. the CRDT-build gate;
4. discovery of the separate `pmacs-gpu` executable;
5. waiting for that executable and reflecting its outcome.

The GPU child owns the actual connection that becomes the session. For the
managed launch, root passes two hidden/internal arguments: the resolved raw
socket path and `current_exe()` as the daemon executable. The child first
tries the socket itself; the stream that completes `Hello` / `AttachRequest`
is retained as its real `AttachClient`. There is no disposable readiness
connection and therefore no deliberate broken-pipe noise.

The hidden argument shape is not a second user-facing launcher. Direct users
keep `pmacs-gpu --attach PATH`; the documented managed surface is
`pmacs --gpu`.

### Q#GI3 — Managed GPU attach may start the supplied daemon, then retries boundedly

Managed attach follows this state machine before creating the window:

1. Try the resolved Unix socket once.
2. If connection reaches `Hello`, perform the normal version and capability
   validation. Either failure is final and surfaced; never start a
   replacement daemon over a live incompatible instance.
3. `NotFound` authorizes daemon startup. `ConnectionRefused` authorizes it
   only when `metadata(socket)` says the existing entry is a Unix socket, or
   the entry disappeared in the race between connect and metadata. An
   existing non-socket file is a final error and is never handed to
   `pmacs --daemon`, whose established stale-path transaction would otherwise
   unlink it after acquiring the sibling lock.
4. Every other initial connect error (`PermissionDenied`, invalid path shape,
   resource exhaustion, `Interrupted`, `WouldBlock`, and other errno classes)
   is final, surfaced with the socket path, and invokes no daemon spawner.
   During the post-spawn retry window, `NotFound`, authorized
   `ConnectionRefused`, `Interrupted`, and `WouldBlock` / `EAGAIN` continue to
   the deadline; all other errors remain final. This tolerates a signal-
   interrupted `connect(2)` and the just-bound daemon's temporarily full
   AF_UNIX accept backlog without broadening what may trigger daemon startup.
5. Spawn the supplied executable as
   `pmacs --daemon --socket <resolved-path>`, with the process-group isolation
   in Q#GI13.
6. Retry the real GPU connect every 50 ms for up to five seconds, matching
   `AUTO_START_POLL_INTERVAL` / `AUTO_START_TIMEOUT` in
   `daemon_attach.rs:149-160`. The first successful stream becomes the real
   attach; no probe is dropped.
7. If the spawned child exits during startup, reap it, retain its status, and
   keep retrying until the deadline: another concurrent launcher may have won
   the socket lock and be about to listen.
8. At the deadline, exit nonzero with the socket path, timeout, and spawned
   child status when available.

Connection/retry occurs before `run_app`, so the winit event thread never
freezes for five seconds behind startup polling. On success, events sent to
the already-created `EventLoopProxy` may queue until `run_app` starts; the
window then assembles with the connected frontend id. Existing direct
`--attach` behavior may retain its in-window failure banner.

During startup the managed connector owns `Option<Child>` so every early exit
is reaped. After a successful attach, a still-running spawned daemon moves to
a named reaper thread whose only job is blocking `Child::wait`; this prevents
a daemon that later crashes or quits from remaining a zombie throughout a
long GPU session. Closing the GPU process still leaves a live daemon orphaned
and long-lived per Q#GI5.

Do not extract a generic launcher crate for two constants, a short loop, and
one child reaper.

### Q#GI4 — Auto-start is race-safe through the existing daemon lock

Two simultaneous `pmacs --gpu` commands may both observe a missing socket and
spawn a daemon. The sibling lockfile is the arbiter: one daemon binds, the
other exits. Both GPU children continue their bounded connect loop and attach
to the winner. The loser child is reaped; its lock error is not treated as a
launch failure if the socket becomes usable.

A stale socket follows the existing daemon transaction: acquire the lock,
unlink the stale socket, bind the replacement. The launcher may read
filesystem metadata solely to distinguish a socket entry from non-socket data
(Q#GI3); it neither deletes entries nor treats path existence as proof of
liveness. The daemon lock and successful protocol connection remain the
arbiters.

### Q#GI5 — An auto-started daemon remains long-lived

Closing the GPU window detaches that frontend but does not kill a daemon the
launcher started. This matches pmacs's long-lived-instance architecture and
the existing `--daemon-attach` auto-start behavior. A later `pmacs --gpu`
reuses the same default/named daemon and retains buffers, processes, and
language services.

The startup child has null stdin/stdout/stderr, matching the existing
background auto-start precedent. Startup failure is reported through child
status + timeout; detailed daemon diagnostics remain available by running
`pmacs --daemon --socket ...` directly. Q#GI13, not the SSH precedent,
defines the local terminal's signal isolation. The launcher does not
daemonize, install a service, or invent idle shutdown in this stage.

### Q#GI6 — Require a CRDT-capable root build before launching

In a root binary compiled without `feature = "crdt"`, `pmacs --gpu` exits
before executable discovery or any socket connection with an actionable
message:

```text
pmacs: --gpu requires pmacs built with --features crdt
```

This prevents default-socket poisoning: without the gate, a non-CRDT root
could auto-start an incapable daemon that successfully owns `default.sock`;
the GPU would then reject its capabilities, and Q#GI3's correct
never-replace-a-live-instance rule would make later managed launches keep
failing until the user manually stopped that daemon.

The deliberate trade-off is that a non-CRDT root also refuses the managed
fast path when a separate capable daemon is already listening. The broker's
root executable is its daemon-start authority and must be capable for
deterministic behavior. Advanced users may still attach the independent GPU
directly to a known capable daemon with `pmacs-gpu --attach PATH`; that path
retains its own Hello capability validation.

### Q#GI7 — Prefer the sibling GPU binary, then fall back to `PATH`

Discovery order:

1. test-only `PMACS_TEST_GPU_BIN` override;
2. `current_exe().parent()/pmacs-gpu` when that path exists;
3. `pmacs-gpu` through `PATH`.

Sibling-first keeps ordinary source builds and side-by-side installations on
the same release/protocol build. The handshake remains authoritative: path
co-location is not proof of protocol compatibility. Failure names the sibling
candidate and PATH fallback rather than reporting a generic spawn error.

There is no production `PMACS_GPU_BIN` configuration knob in v1. A permanent
override would become distribution policy; tests need substitution, users
need an installation that places the two shipped binaries coherently.

### Q#GI8 — Root waits for the GPU child and reflects failure

`pmacs --gpu` remains the terminal-visible parent while the window is open.
A successful GPU exit returns success; a nonzero exit returns failure and
prints which GPU executable failed. Spawn failure is immediate and
actionable. No shell, `nohup`, or detached launcher process sits between the
user and the frontend.

The daemon is independent after startup (Q#GI5); root waits only for the GPU
child.

### Q#GI9 — Retire bare `pmacs-gpu` hello-world and make argv strict

The hello-world window was Session-2 dependency scaffolding and no longer
serves a user workflow. Bare `pmacs-gpu` exits with usage that points to
`pmacs --gpu` for managed startup and `pmacs-gpu --attach PATH` for direct
debugging.

The existing test-only
`pmacs-gpu --headless-probe <socket> <report>` mode is retained byte-for-byte
as the Vterm Stage 3 real daemon + PTY + wgpu seam. Its parser becomes strict
about exactly those two operands; `tests/vterm_stage3_acceptance.rs` keeps its
current subprocess command and semantics. Q#GI14 adds a separate managed
headless probe rather than overloading this vterm contract.

The GPU parser rejects:

- trailing argv after direct-attach or probe operands;
- a missing attach/probe operand;
- user attempts to invoke the hidden managed modes without every broker
  operand;
- unknown flags.

Add `pmacs-gpu --version` so reports can name both package version and
protocol version without opening a window. `--help` labels direct attach as
an advanced/manual path and does not advertise internal broker/probe
arguments.

### Q#GI10 — No protocol change and no initial file in this stage

The broker changes process orchestration only. It sends the existing
`AttachRequest`, negotiates protocol v19, and uses the existing semantic/CRDT
session. `SUPPORTED` and every wire discriminant remain unchanged.

`pmacs --gpu FILE` is rejected rather than accepted partially. The future
file feature must target the authenticated source frontend's view, preserve
non-UTF-8 local paths, define relative-path resolution, surface open failure,
and avoid a scratch-buffer flash. It receives its own framing and protocol
review.

### Q#GI11 — Fix checkout build/run instructions with the broker

Set root package `default-run = "pmacs"`, making the existing README
`cargo run --release -- ...` family unambiguous. Document one coherent build:

```sh
cargo build --release --workspace --features pmacs/crdt
```

Then document:

```sh
target/release/pmacs --gpu
# explicit TUI remains:
target/release/pmacs -nw [FILE]
```

The build line deliberately compiles both binaries together. Keep the
advanced two-process commands in a troubleshooting/manual-attach subsection,
including the canonical XDG fallback rather than Linux-only `$UID` prose.
Do not claim `cargo install` support until installation is exercised and the
unpublished GPU package has a deliberate distribution story.

### Q#GI12 — Scope stays on invocation, not reconnect or packaging

The managed startup retry ends when the first attach succeeds. A later daemon
disconnect retains today's `(daemon disconnected)` state and manual relaunch.
Desktop files, system services, release bundles, package managers, and remote
GPU transports are not smuggled into this feature.

### Q#GI13 — Put the auto-started daemon in its own process group

Before spawning the managed daemon, call the safe Unix
`std::os::unix::process::CommandExt::process_group(0)`. The daemon becomes the
leader of a new process group while the root broker and GPU child remain in
the terminal's foreground group. Terminal Ctrl-C therefore terminates the
foreground launcher/frontend without delivering SIGINT to the daemon or any
other attached frontend.

This is process-group isolation, not a new session or full daemonization.
Direct `pmacs --daemon` remains foreground and keeps its established
SIGINT/SIGTERM graceful-shutdown contract. Terminal close remains safe through
the daemon's existing SIGHUP no-op. No `unsafe`, `setsid` helper process, or
platform-specific FFI enters the codebase.

### Q#GI14 — Add a real display-less managed-attach acceptance seam

Extract the pre-`run_app` production path as
`connect_managed_with_sink(socket, daemon_exe, sink)`. The normal managed
window calls it with the existing `EventLoopProxy` sink. A new hidden strict
subprocess mode:

```text
pmacs-gpu --headless-managed-probe <socket> <report> <daemon-exe>
```

calls the same function with a channel sink and keeps processing that channel
after the real `BufferSnapshot`. At the snapshot checkpoint it atomically
writes a complete report with `phase=ready` and initial named facts (protocol,
whether this invocation spawned, child status so far), then holds the live
session until stdin reaches EOF. A dedicated stdin-reader thread reports EOF
to the probe loop; the loop itself remains free to receive disconnects and
reaper observations. Each such lifecycle observation atomically refreshes the
`phase=ready` report, so a harness can wait for a named fact such as
`daemon_reaped=true` without sleeping. On EOF the probe atomically replaces
the report with `phase=complete` plus final child/reaping facts and exits
without winit or wgpu.

Tests that need a hold spawn the probe with piped stdin, wait for
`phase=ready`, perform the signal/daemon action, wait for any required named
fact, then close stdin when they want normal probe completion. An ordinary
invocation with null stdin advances immediately from ready to complete. There
is no timing-based linger duration.

This is the acceptance seam for Q#GI3–GI6 and Q#GI13: real binary, real Unix
socket, real Hello/capability negotiation, and real daemon subprocess. A
decoded-message fixture or a second test-only connect implementation is not
accepted evidence.

The existing `--headless-probe <socket> <report>` remains separate and still
drives real offscreen wgpu for Vterm Stage 3 (Q#GI9). Narrow injected-spawner
unit tests pin rare errno/timeout branches, including post-spawn
`Interrupted` / `WouldBlock` followed by success, but they do not replace the
managed subprocess acceptance.

## Categorical bets

1. **An explicit broker is enough to validate lifecycle policy before changing
   the default frontend.** Users gain a one-command GPU path without forcing
   GUI startup into scripts, `$EDITOR`, SSH sessions, or terminals that rely
   on today's bare `pmacs` TUI.
2. **Sibling-first discovery covers source and coherent installed layouts.**
   A fallback to PATH handles split prefixes; the protocol handshake catches
   stale or foreign binaries.
3. **The existing lock is the correct concurrency arbiter.** Launcher-side
   PID files, path-existence checks, or socket deletion would duplicate and
   weaken the daemon's established ownership transaction.
4. **The successful connect must be the session connect.** Disposable probes
   are observably wrong because the daemon speaks first; retrying the actual
   GPU connect avoids false BrokenPipe logs and frontend-id churn.
5. **A five-second pre-window startup bound is acceptable.** Existing remote
   daemon auto-start uses the same bound. A missing/broken daemon fails before
   creating a misleading inert window.
6. **Persistent auto-start matches user expectation.** The instance owns
   buffers and services across frontend lifetime; killing it on window close
   would turn the daemon split into implementation overhead with no persistence
   benefit.
7. **Foreground job control must not own the daemon.** A local launcher's
   terminal process group is not the SSH no-controlling-terminal precedent;
   a safe process-group split preserves the daemon across Ctrl-C without full
   daemonization.
8. **Rejecting FILE is better than synthetic input.** Driving `C-x C-f` or the
   minibuffer from a launcher is timing-dependent, configuration-dependent,
   and cannot provide an atomic initial view.
9. **Non-socket paths are data, not stale sockets.** Managed auto-start never
   feeds a regular file to the daemon's existing unlink-and-bind transaction.
10. **No new shared crate is warranted.** Root already resolves paths and
    hands the result to the GPU; the GPU adds only bounded connect-or-spawn
    behavior around its existing transport.

## Deferred (named)

- **GUI as the automatic default.** Complete the reserved
  `FrontendChoice::Auto` plan after the broker is proven: display detection,
  `PMACS_FRONTEND`, and `pmacs -nw` precedence. This is a user-default change,
  not part of additive startup.
- **Initial file(s) for an attached frontend.** A real per-session target that
  opens/switches in the authenticated source's view, reports errors, handles
  path bytes and relative cwd, and avoids scratch flash. This is prerequisite
  to honest `pmacs --gpu FILE` and eventual bare `pmacs FILE` GUI startup.
- **Automatic reconnect/resync.** Startup retry does not reconcile an
  optimistic replica after a live connection drops; retain the attach-
  robustness deferral.
- **Direct `pmacs-gpu --socket NAME`.** Managed users go through root, which
  already owns canonical resolution. Revisit only if direct frontend use is a
  supported standalone workflow.
- **Remote GPU attach.** The TUI owns SSH/daemon-attach transport today;
  generic GPU stream transports need separate latency, clipboard, and
  frontend-resource semantics.
- **Daemon service management.** systemd/launchd units, socket activation,
  idle shutdown, logs, restart policy, and a `pmacs --stop-daemon` command.
- **Distribution/install bundles.** Cargo install, release archives, desktop
  entries, icons, and ensuring both binaries land together.
- **Multiple files and client/server open commands.** Follow the initial-file
  contract rather than widening this stage's rejected positional grammar.
- **GPU executable override for users.** Keep only the test override until a
  real packaging use case establishes precedence and diagnostics.

## Acceptance

CLI and launcher cases run against the real built binaries where process
behavior is the contract. Pure parsing/discovery helpers receive unit tests;
no source-text assertions substitute for subprocess behavior. Managed
connection cases use Q#GI14's real display-less binary seam; the existing
Vterm probe continues to cover offscreen wgpu.

1. **Root CLI grammar:** `pmacs --gpu` and `pmacs --gpu --socket research`
   select managed GPU mode. Combinations with `-nw`, `--daemon`, `--attach`,
   `--daemon-attach`, or a positional file exit 2 with the conflicting
   argument named. Bare `pmacs --socket research` (with or without a local
   file / `-nw`) exits 2 and says which owning mode is required.
2. **Non-CRDT build fails before socket or spawn:** a default-feature
   `pmacs --gpu` names `--features crdt`, invokes no GPU executable, and leaves
   the default socket absent under a private runtime directory. Repeating the
   command against an already-listening Unix socket neither invokes the GPU
   nor disturbs that socket, proving the gate precedes socket I/O.
3. **Sibling discovery wins:** with executable fixtures at the current-exe
   sibling and on PATH, the sibling regular file receives the managed
   arguments. A directory at the sibling pathname is ignored in favor of
   PATH. With neither executable available, the error names both lookup
   attempts.
4. **Existing daemon fast path:** start a real CRDT daemon on a private socket,
   run `--headless-managed-probe`, and assert a v19 session establishes and a
   real `BufferSnapshot` arrives without invoking the supplied daemon spawner.
5. **Missing daemon auto-start:** from no socket/lock, the managed probe starts
   the supplied real CRDT daemon, completes the real Hello/capability
   handshake, receives the first `BufferSnapshot`, and leaves the daemon
   connectable after the probe exits.
6. **Ctrl-C does not kill the daemon:** spawn the managed probe/broker in its
   own process group, wait for the probe's `phase=ready`, and complete a second
   real frontend's handshake and initial snapshot/grid sync before simulating
   terminal Ctrl-C with `kill(-pgid, SIGINT)`. After the launcher/frontend
   exits, resize the pre-existing second frontend and require its full-grid
   response; the separately grouped daemon and existing session remain usable.
   This does not require a controlling terminal or a foreground-process-group
   claim in CI. Direct foreground `pmacs --daemon` still exits cleanly on
   SIGINT.
7. **Concurrent launchers converge:** hold two daemon wrappers behind a shared
   barrier so both managed probes authorize and spawn before either daemon
   binds the absent named socket. Exactly one daemon owns the lock; both
   clients establish sessions; exactly one probe reports its losing daemon
   child reaped rather than leaving a zombie or aborting its frontend.
8. **Stale socket recovery stays daemon-owned:** leave a stale Unix socket
   with no lock owner, launch managed GPU, and assert the daemon replaces it
   and the frontend attaches. The launcher itself performs no unlink.
9. **Other connect errors fail closed; retry transients survive:** a
   permission-denied initial socket path invokes no daemon spawner and reports
   the path/error immediately. A regular file at the socket path survives
   unchanged, invokes no daemon, and reports that managed startup refuses to
   replace a non-socket entry. A unit connector injects post-spawn
   `Interrupted`, `WouldBlock`, then a successful real `UnixStream::connect`
   inside a private temporary directory; it stays within the deadline,
   establishes the protocol session, and hands the spawned child to the
   reaper.
10. **Live capability mismatch is not replaced:** against a non-CRDT daemon,
    managed launch reports the existing capability mismatch, invokes no
    second daemon, and leaves the live daemon untouched.
11. **Live protocol mismatch is not replaced:** a real/fake-Hello listener
    advertising an unsupported protocol produces the version-mismatch error,
    invokes no daemon spawner, and leaves the listener/path untouched.
12. **Bounded startup failure:** substitute a daemon executable that exits
    nonzero without binding. Managed GPU exits after five seconds (50 ms
    polls) and reports socket + child status. Unit tests with a one-millisecond
    deadline assert the configured duration appears in `StartupTimeout`.
    A concurrent-winner fixture proves an early losing-child exit does not
    abort while another process binds before the deadline.
13. **Post-attach daemon exit is reaped:** let the spawned daemon establish a
    managed session, wait for the probe's `phase=ready`, then terminate the
    daemon while keeping the probe's stdin open. Poll the atomically replaced
    ready report until it records both disconnect and named reaper completion,
    close stdin, and assert the `phase=complete` report retains the wait
    outcome. The daemon never remains a zombie until frontend exit.
14. **Root reflects the GPU outcome:** fake GPU success makes `pmacs --gpu`
    succeed; fake nonzero and spawn failure make it fail with the executable
    named.
15. **GPU argv is strict without breaking probes:** bare invocation points to
    `pmacs --gpu`; `--help` leads with normal root-broker usage and labels
    direct `--attach PATH` as advanced. Existing `--headless-probe SOCKET
    REPORT` and hidden `--headless-managed-probe SOCKET REPORT DAEMON_EXE`
    accept exactly their operands. Missing/trailing/unknown/incomplete args
    exit 2; trailing help/version operands say those flags accept no operands;
    option-like path operands are rejected and require an explicit `./`
    prefix when they name a real relative path. `--version` prints package and
    protocol versions without initializing winit/wgpu.
16. **Existing direct and Vterm paths remain intact:** rebuilt
    `pmacs-gpu --attach RAW_PATH` still renders an existing CRDT daemon, and
    `tests/vterm_stage3_acceptance.rs` still invokes its unchanged
    `--headless-probe` command and passes the real daemon + PTY + wgpu
    criterion.
17. **One-command visible smoke:** on a Vulkan/display-capable machine, build
    the workspace once, run only `target/release/pmacs --gpu`, observe the GPU
    scratch buffer attach at protocol v19, close the window, then invoke the
    same command again and observe reuse of the still-running daemon.
18. **Documentation commands are executable:** the README's unified release
    build succeeds; `cargo run --release -- --version` selects `pmacs` through
    `default-run`; no documented command requires users to spell the resolved
    socket pathname for managed GPU startup.

## As built

- `Cargo.toml` sets `default-run = "pmacs"`. The documented coherent build is
  `cargo build --release --workspace --features pmacs/crdt`.
- `src/main.rs` owns `pmacs --gpu [--socket NAME|PATH]`, the non-CRDT gate,
  socket resolution, test override, sibling-first GPU discovery with PATH
  fallback, child argv, and exit-status propagation.
- `pmacs-gpu/src/main.rs` accepts only explicit direct, managed, and headless
  modes. Managed windowed attach completes before winit creates a window;
  decoded messages arriving first are buffered and drained in order once
  application state exists. Bare invocation is an exit-2 usage error pointing
  users to `pmacs --gpu`; help labels raw-socket attach as advanced.
- `pmacs-gpu/src/attach.rs` owns connect-or-start policy, the five-second /
  50-ms retry window, socket-type protection, daemon process-group isolation,
  and the named child-reaper thread. Every successful spawn is handed to the
  reaper before any later connection or handshake operation can fail, and
  daemon stdio is null. The first successful protocol connection wins;
  protocol/capability failures never authorize replacement.
- `--headless-managed-probe SOCKET REPORT DAEMON_EXE` drives the production
  managed connector, writes atomic `phase=ready` / `phase=complete` reports,
  holds on stdin, exposes disconnect plus daemon-reaper observations, and
  retains its 50-ms cadence after the event channel closes.
- `tests/gpu_invocation_acceptance.rs` covers the root broker, pre-I/O
  non-CRDT gate, existing/missing/stale/racing daemon paths, process-group
  SIGINT isolation with a pre-attached surviving frontend, capability and
  protocol mismatches, bounded startup failure, deterministic losing-child
  reaping without signaling freed PIDs, outcome propagation, and strict
  headless CLI behavior.

Verification on 2026-07-23 after the second implementation review:

- root CLI unit suite: 33 passed;
- required GPU suite: 149 passed;
- managed invocation acceptance: 1 default + 9 CRDT passed;
- Vterm Stage 3: 7 passed with `PMACS_REQUIRE_GPU=1`;
- default / CRDT libraries: 1,768 / 1,944 passed;
- M4 acceptance: 121 passed, 3 ignored, 1 requested skip;
- full workspace sweep: 2,961 passed across 85 suites, 19 ignored, 1 requested
  skip;
- strict workspace Clippy, CRDT invocation-acceptance Clippy, formatting, and
  `git diff --check` passed;
- the documented release workspace build and `cargo run --release --
  --version` passed;
- two real `target/release/pmacs --gpu` launches on Wayland/Vulkan attached at
  protocol v19. The first auto-started daemon remained alive after the GPU
  process closed; the second reused that same daemon and created no replacement.
