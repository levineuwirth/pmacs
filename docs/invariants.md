# pmacs invariants

The rules a change must not break. Each names the mechanism that makes
it true, so a session can check the rule against the code rather than
trust this file. Symbols are authoritative; nothing here is dated, and a
rule that stops being true is removed rather than annotated. Workflow
rules (branches, gates, commits, the vault) live in `CLAUDE.md`.

## Generated buffers

`Buffer::set_generated_contents` is the one authorized write into a
buffer the editor generates (`*help*`, listviews, dired, compilation
output, and the like). It lifts `read_only`, replaces the whole buffer
with a single `Replace` that skips intercepts, discards history,
re-asserts `read_only`, and returns the `Edit`. The pairing, not the
setter, is the primitive, for three reasons:

- An intercept is not read-only. `Buffer::undo` reaches the rope through
  `ensure_writable` and never consults the intercept chain, so an
  intercept-only "read-only" buffer is emptied by `M-x buffer.undo`.
  Rebinding the undo chords buffer-locally does not close this; only
  rope-level `read_only` does.
- A bare `set_read_only` would refuse the owner's own refresh, which is
  the operation such buffers exist for. There is deliberately no Lua
  `set_read_only`.
- A rope write is half of an edit. The returned `Edit` must be fanned out
  through `notify_buffer_edit_to_windows`, which also queues the
  daemon-origin CRDT op. Skipping it leaves a displaying window with a
  `TextView` line index for the previous contents, and replica mirrors
  that never import the write.

History clearing must clear whichever history the buffer has. In CRDT
mode the undo history lives in loro's `UndoManager`, which has no
`clear`; `CrdtState::clear_undo_history` rebinds a fresh manager to the
same document, which is equivalent because a manager records only what
happens after its construction.

The protection is layered and both layers are needed: rope-level
`read_only` refuses the op at the daemon, while `set_round_trip_input`
stops a semantic frontend applying the op optimistically to its own
mirror. A daemon-side refusal arrives after the frontend has painted,
so on its own it buys divergence, not prevention.

## Command boundaries

`EditorCore.command_history` holds a `CommandBoundary { this, last }` per
frontend. It rotates on a keybound command, self-insert, menu invoke,
and `pmacs.command.invoke_interactive` (the `M-x` path). It breaks on an
unbound key, GPU optimistic CRDT edits, pointer gestures (wheel scroll
deliberately does not break), and unified paste. Plain
`pmacs.command.invoke` stamps nothing; it is the programmatic API.
A single-codepoint optimistic CRDT insert classifies as
`buffer.self-insert` by exact decode; a longer insert breaks. Lua reads
the boundary through `ed.this_command()` and `ed.last_command()`.

## Effective-edit returns

`buf:insert`, `buf:delete` and `buf:replace` return the post-intercept
`(start, end, inserted_len)`. A caller that cares (the kill ring, comment
toggling) compares exactly against what it requested; length-delta and
text-at-position checks are documented defeated patterns. Every mutator
call is wrapped in `pcall`: a rejecting intercept must report, not throw
through, and a failed op must leave no state behind (kill chains, yank
sessions).

## Kill ring

Entries are `{id, text}` with stable monotonic ids. Chains and yank
sessions are per frontend and id-checked, because an index is not stable
under another frontend's pushes. The OS clipboard mirrors to the acting
frontend only. Paste is a unified daemon arm keyed by the dispatcher's
authenticated source, never by a `frontend_id` carried in the payload.

## LSP

Every `Position` and `Range` builder in `src/lsp.rs` routes through
`outbound_position`, which converts byte offsets to the negotiated
encoding; a new request builder must too, because UTF-16 servers reject
raw byte columns on non-ASCII text. Semantic tokens `full`, `full.delta`
and `range` are three independent capabilities and each is gated on its
own.

LaTeX is served by `texlab`, and its root is not the repository root.
`pmacs.lsp.config.latex` resolves the document root by an upward marker
walk over texlab's own markers (`.texlabroot`, `texlabroot`), and `.git`
is deliberately excluded from that walk: a repository root is the wrong
answer for a multi-file document, which is the reason the resolver
exists.

The fake server `src/bin/pmacs_fake_lsp.rs` is selected by
`PMACS_FAKE_LSP_MODE`. Capability modes: `fullonly`, `rangeonly`,
`rangeonly16` (UTF-16 with fail-closed bounds validation), `sighelp`,
`prepare`, `preprefuse`, `rename`, `inlaybounds`, `inlayrefresh`,
`semantictokensrefresh`, `applyeditplan`, `resourceops`, `posecho`,
`defenv`, `wsconfig`, `rooturi`, `leanprogress`. Failure shapes:
`crash`, `error`, `garbage`, `silent`. File watchers: `filewatch`
(a `RelativePattern` `**/*.txt`), `filewatchabs` (an absolute plain
glob), `filewatchflat` (a `RelativePattern` with no leading `**/`),
`filewatchbare` (a bare relative string), `filewatchrereg` (the same id
twice with no unregister), `filewatchjoin`, `filewatchretire`. Use these
for capability-matrix tests, never a real server; the list is
enumerated from the binary and a stale copy is how a test ends up
covering the shape next to the defect.

## Persistence

State-directory wiring lives in `install_state_dirs()` on the real entry
points only, never in `EditorState::new()`, so tests stay hermetic;
`PMACS_STATE_HOME` overrides the XDG state root and outranks it. Autosave:
one buffer owns a path's recovery slot; only recover or discard releases
unclaimed crash data; adopting clears the old owner's skip cache.

## Protocol

- `ADVERTISED_PROTOCOL_VERSION` is never edited. The handshake is
  server-first: the daemon writes `Hello` before the frontend has said
  anything, and a frontend rejects an unrecognized version before it
  can send `AttachRequest`. Advertising `PROTOCOL_VERSION` would lock
  out every shipped frontend whose range ends lower. The baseline stays
  at the highest version every shipped frontend accepts; the session's
  version is settled by the frontend's counter-offer in `AttachRequest`
  and `negotiated_session_version`. Moving the baseline is a deliberately
  incompatible act reserved for a change that cannot be additive.
- A new wire message is an appended variant, bumping `PROTOCOL_VERSION`
  and extending `SUPPORTED_PROTOCOL_VERSIONS`, guarded by a byte pin on
  the previous final variant: an appended variant's own round-trip
  cannot detect a discriminant shift, only a literal fixture of the
  neighbor can.
- A widened field is a break. postcard encodes positionally, so every
  older peer mis-decodes rather than ignores. A superseded variant is
  frozen, kept unchanged and still sent to the versions that know only
  it, and the richer variant is appended; the daemon gates with a range
  on both sides so exactly one variant reaches any peer. A frozen
  variant needs a literal byte fixture, because a round-trip with the
  same types freezes nothing.
- The close message of a surface uses the same variant family as its
  open, or a session closed by the other family leaves the surface on
  screen.
- New wire surface means a version bump, support in both frontends, and
  an acceptance test. A wire-bearing change runs alone in its own phase.
- `pmacs-gpu` depends on `pmacs-protocol` and never on `pmacs`. The one
  existing dev-dependency is a one-off scheduled for removal; no new
  test in `pmacs-gpu` links `pmacs`, and no new `#[doc(hidden)]`
  `*_for_test` method lands on `EditorState`.
- A message that aligns state cannot be gated on the state it names.
  `FrontendEvent::Viewport` both declares a range and switches the
  window to the buffer it names; the gate keys on the authenticated
  source's active buffer, and when two messages declare competing views
  the arbiter is the daemon's own state, never a claim inside either.
- A provisional session that fails mid-bootstrap calls
  `shutdown(Shutdown::Both)` on the socket. Dropping a cloned write half
  shares the descriptor and leaves the reader thread alive with no
  installed session; the session registry is checked before any
  `FrontendEvent` touches editor state.
- An upgrade decision is tracked independently of the outcome that
  triggered it: a helper that may upgrade a buffer to CRDT reports
  whether it did, and the publish decision ORs that in rather than
  inferring it from the caller's load-or-create branch.

## Daemon and dispatch

- Two operations that must be alternatives are not made alternatives by
  being adjacent. Per-frontend-kind work that is mutually exclusive is
  one `if`/`else` keyed on the same fact session establishment uses,
  with the loop body extracted so a test can drive it; two arms with
  individually correct idempotence guards were jointly useless.
- A pass that sets a mode flag clears it on every exit, including the
  `?` early returns; `terminal_active` suppresses `CursorByte` and the
  presence sweep, and was once left set by an early return.
- Knowledge about a buffer belongs in shared stores (the
  `DiagnosticStore` severity totals), never in per-session baselines;
  a field that doubles as an emission baseline and a freeze count is a
  reset-contract trap.
- The no-argument arm of `pmacs.window.buffer()` resolves through the
  ambient view and its fallback is what makes the function total. The
  acting frontend can name a frontend with no registered view, and no
  runtime caller `pcall`s this function.
- Tab width is a rendering semantic, not a configuration gap: the width
  is fixed at eight columns, shared through `pmacs-protocol`, and
  expanded only in each display projection. A configurable width needs
  a buffer-effective frontend fact and cache invalidation, not a scalar
  setting.
- A daemon-side change is not deployed until the daemon restarts from a
  binary that contains it; `pmacs --gpu` attaches to whatever owns the
  socket.

## Lua runtime

- An optional field that a data structure's shape depends on is not
  optional. Listview rows may omit `item`, which leaves `line_to_item`
  sparse, and `#` of a sparse table is not its size. When a field is
  optional, one test omits it.
- A contract two mechanisms must honor is as strong as the weaker one.
  Listview ids are compared with `==` by selection and used as raw table
  keys by collapse state; the contract is narrowed to what both honor
  and enforced where the data enters, never generalized to the stronger
  half.
- Acceptance fixtures that open `.rs` or `.py` files empty
  `pmacs.lsp.config` first, or the after-load hook spawns real servers.
  A scratch buffer has no path and therefore no language.

## Tests and evidence

- A test asserts through the outermost user-reachable seam and is
  falsified by revert or by `scripts/bite`. A guard with no production
  caller passes every direct-call test; a test that shares a file with
  the code it pins is bitten by hand-breaking the production line.
- A test that skips on a missing precondition reports `ok`. Tool-gated
  suites are armed with the matching `PMACS_REQUIRE_*` variable where
  the tool is installed, and otherwise judged by elapsed time.
- A wait predicate weaker than the assertion it guards is a race on
  every platform that happens to lose it. Wait for the exact unit the
  assertion reads (a whole record, a trailing newline), not a prefix.
- A reproduction is a measurement and needs its own positive control:
  assert the precondition the reproduction depends on, in the test,
  before exercising the thing under test.
- An observation helper that advances the system is not an observation.
  Draining an event stream ticks, and a tick reaps; read state from the
  record that does not tick.
- A quiet child is an instrument. Storms and oscillations are invisible
  against a fixture that legitimately emits hundreds of frames; assert
  upper bounds over a fixed window against a child that produces
  nothing, and have the child self-report with fresh distinct markers.
- `TerminalMode::Raw` has no `ICRNL`, so `sh -c 'read -r'` fixtures wait
  forever; use `exec cat`. A PTY in default mode does not translate LF to
  CRLF, so `echo` fixtures staircase and clip to blanks; emit `\r\n` and
  guard every text comparison with a non-empty assertion.
- A `Drop` body runs before its fields. If a `Drop` body waits on
  anything, check what the waited-on party needs that only a field drop
  releases, and move that release into the body.
- A cancel flag beside a blocking syscall is documentation, not a
  mechanism; a thread blocked in `read` never polls it.
- A red is matched by its required fragments, never by test name or
  resemblance; two rows can share a test and differ only in one
  fragment. A green rerun after a red establishes non-reproduction and
  nothing more; the same fragments again are a second occurrence.
- A gate summary assembled through a pipe reports the last command's
  status. Each stage is redirected to its own file and the file is read.
- Scripted edits in files with repeated similar blocks anchor on a
  unique line, or they silently edit the wrong block.

## Process mechanisms

- `git stash` is repository-global across worktrees and shared with the
  owner; it is never used. `scripts/bite` is the tool for running tests
  against an older version of one file. A fix is committed before it is
  bitten, so the gate describes the pushed tree.
- A conflicting pull request runs no CI at all and reports nothing about
  the absence; check that a run exists for the current head sha.
- A stacked pull request is retargeted to `main` before its parent
  merges; a PR whose base branch is deleted is closed and cannot reopen.
- A frozen, reviewed branch does not absorb overlapping work that lands
  first; the integration surface is `git diff <base>..main`, not the
  other branch's file list.

## Declared divergences

A frontend-only capability, or a deliberate difference between the two
frontends, ships with an entry here stating what diverges, why it was
accepted, and what removes it. An entry is removed when its condition is
met.

- **Line wrap.** Under `ui.line-wrap = "wrap"` both frontends wrap at
  the character. The GPU frontend could wrap by word through
  cosmic-text's Unicode line breaking (UAX #14) and the grid frontend
  cannot match those breaks without a UAX #14 dependency, so a grid
  whitespace wrap would give approximate parity, which is worse than an
  honest difference; the ruling chose character wrap on both and
  accepted that GPU users lose word wrap. Under `"truncate"`, text past
  the edge is reachable by moving the cursor in the grid frontend and
  not yet reachable in the GPU frontend. Removed when a word-wrap mode
  ships on both frontends with the difference in breaking rules stated,
  and horizontal reach of truncated text exists on the GPU.
