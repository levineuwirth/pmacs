# GPU attach robustness — framing + as-built

Three audit findings on the `pmacs-gpu` attach path, taken as one arc
because they share a surface (the frontend↔daemon connection) and a
theme — the GPU frontend fails *quietly* where it should fail *loudly* or
*bounded*. None touches the wire protocol or the daemon; all three live in
`pmacs-gpu` (`attach.rs` + `main.rs`).

- **F-003** — attach against a daemon built without `--features crdt`
  negotiates "successfully", then sits on `(connecting...)` forever
  because no `BufferSnapshot` ever arrives. Silent, confusing.
- **F-008** — the outbound `FrontendEvent` queue is an unbounded
  `mpsc::channel`. A stalled daemon lets the UI thread enqueue without
  backpressure: unbounded memory, and a backlog of stale viewport/pointer
  traffic replayed on recovery.
- **F-007** — the minibuffer completion dropdown grows upward from the
  status band by `n * row_height` with no clamp; on a very short window
  it renders above `y=0` and the selected row can be off-screen.

## What the recon established

- **F-003 needs no protocol change.** `connect()` (`attach.rs:95`) reads
  `Hello`, checks `protocol_version`, then sends `AttachRequest`
  **fire-and-forget** — it never inspects what the daemon granted. But
  `Hello.instance_capabilities` *already* carries `crdt_replica`,
  `semantic_render`, and `multi_frontend` (the daemon advertises them; on
  a non-CRDT build they default `false`). The GPU can read the field it
  was already sent and reject the attach itself. The call site
  (`main.rs:751`) already renders a fatal on-screen line
  (`set_text("(attach failed; see stderr)")`) — the rendering path exists.
- **F-008's flood sources are `Viewport` and `Pointer{Drag}`.** Viewport
  fires on every wheel/scrub/resize/edge-scroll; `Drag` on every
  cursor-move-with-button. The *other* pointer kinds — `Down`, `Up`,
  `DoubleDown`, `TripleDown`, `Context` — are discrete clicks that are
  causally ordered (a `Down`/`Up` pair *is* a selection gesture; `Context`
  opens a menu). `Key`, `CrdtOp`, `Paste`, `MenuPointer` are all
  order-critical too — `CrdtOp` especially, since the GPU applies it to
  its local Loro replica optimistically, so a dropped one desyncs.
- **F-007 already clips.** The dropdown's `TextArea` sets
  `bounds.bottom = band_top` (`main.rs:3946`); glyphon clips glyphs to the
  rect. So the fix is geometry-only: shape all candidates once, then
  scroll + clamp the visible window at render time — no re-shape on
  resize, and the common (fits) case stays byte-identical.

## The rules

**Q#AR1 — F-003: detect the capability mismatch client-side, fail loud.**
After the version check, read `hello.instance_capabilities`; collect any
of `{multi_frontend, crdt_replica, semantic_render}` the daemon does *not*
advertise. If the set is non-empty, return a new
`AttachClientError::CapabilityMismatch { missing }` instead of proceeding.
`Display` spells out the fix (start the daemon built with `--features
crdt`); the call site renders a concise, actionable on-screen line
(`window_status()` per variant) rather than the generic "see stderr". No
`AttachResponse`, no daemon `Goodbye` — the daemon already told us in
`Hello`; we just have to read it. (This is *better* than the audit's
proposed daemon-side rejection: the daemon needn't know what each frontend
requires; the frontend checks the advertised caps against its own needs.)

**Q#AR2 — F-008: bound the queue and trailing-coalesce the floods.**
Replace the unbounded `mpsc` with an `Arc<(Mutex<Outbox>, Condvar)>` the
writer thread drains. `Outbox` is a `VecDeque<FrontendEvent>` + a `closed`
flag. Enqueue policy:

- **Coalesceable** = `Viewport` and `Pointer{kind: Drag}`. When enqueuing
  one, if the queue's **tail** is the same coalesceable kind, *replace*
  it; else append. This collapses a run of scroll/drag spam to O(1)
  **without reordering** across an intervening click or key — a `Down,
  Drag, Drag, Drag, Up` gesture becomes `Down, Drag(latest), Up`, never
  `Down, Up, Drag`.
- **Everything else is lossless** — appended in order. If appending would
  exceed `OUTBOX_MAX`, set `closed` and return `Err` (fail-fast): a
  daemon so stalled that thousands of keys/ops piled up is dead, and a
  clean disconnect → reconnect → fresh snapshot is more correct than
  silently dropping a `CrdtOp` (which desyncs the optimistic replica).

The writer waits on the condvar, drains the whole batch into a local
`Vec` (releasing the lock before any blocking socket write), writes each,
and exits on write error or `closed`. `send_event` keeps its
`Result<(), TransportError>` signature (callers already log + continue).

**Q#AR3 — F-007: window the dropdown to what fits, keep the selection
visible.** One helper `mb_visible_window() -> Option<(first, count)>`:
`count = min(n, floor(band_top / MB_DROP_ROW_HEIGHT))` (≥1), and `first`
scrolls so `mb.selected` stays in `[first, first+count)`. The three
consumers share it — `mb_dropdown_rect` sizes the box by `count` (so
`top_y ≥ 0` by construction), `mb_dropdown_vertex_bytes` draws `count`
rows and offsets the selection highlight by `first`, and the text
`TextArea` sets `top = top_y - first*row` so line `first` lands at the box
top while `bounds.top = top_y` clips the rows scrolled above. When the
whole list fits (`count == n`, `first == 0`) every value is identical to
today — no regression on the common path.

## Categorical bets

- **Read what you're already sent (F-003).** The daemon advertises its
  capabilities in `Hello`; a client that ignores them and waits for a
  snapshot that structurally can't come is the bug. No new wire state.
- **Coalesce by tail-replacement, not a side slot (F-008).** A separate
  "latest viewport/pointer" slot would reorder those events relative to
  the ordered stream and break the `Down/Drag/Up` causal chain.
  Tail-replacement collapses exactly the consecutive runs that flood,
  and only those, preserving order by construction.
- **Never silently drop a lossless event (F-008).** `CrdtOp` is applied
  optimistically; dropping it desyncs. Under true overflow, fail-fast to a
  reconnect (clean resync) beats a silent divergence.
- **Geometry-only for F-007.** glyphon already clips; shaping all
  candidates once and scrolling at render keeps resize free and the common
  path unchanged.

## Validation implication

F-003 and F-007 are eyeball-confirmable locally (this box has a Vulkan
adapter + a CRDT daemon): attach to a non-CRDT daemon → see the actionable
banner instead of a hang; shrink the window under an open completion →
dropdown stays on-screen with the selection visible. F-008's coalescing
and bound are unit-tested (tail-replacement collapses runs, preserves
order across a click, caps at `OUTBOX_MAX`); its behavior under a real
stalled daemon is not easily reproduced in a test and rides on the logic
proof + the existing reader-driven disconnect path.

## As-built

Landed as framed; all three in `pmacs-gpu`, no protocol/daemon change.

- **F-003** (`attach.rs`): new `AttachClientError::CapabilityMismatch {
  missing }`; a free `missing_capabilities(&InstanceCapabilities)` checks
  `multi_frontend`/`crdt_replica`/`semantic_render` against
  `Hello.instance_capabilities` right after the version check and returns
  the error before sending `AttachRequest`. `window_status()` gives the
  call site (`main.rs`) a concise in-window line ("daemon lacks CRDT
  support — restart it built with `--features crdt`") while `Display`
  keeps the full detail for stderr. 3 unit tests.
- **F-008** (`attach.rs`): the unbounded `mpsc` became
  `Arc<(Mutex<Outbox>, Condvar)>`. `Outbox::enqueue` tail-replaces a
  same-kind `Viewport`/`Pointer{Drag}` (coalesce), appends everything else
  lossless, and on a lossless append past `OUTBOX_MAX` sets `closed` +
  returns `false` (fail-fast). The writer waits on the condvar, `mem::take`s
  the whole batch, and writes with the lock released.
  **Fail-fast is a real teardown, not just a flag** (review follow-up):
  closing the outbox alone left the reader blocked on its still-open
  socket clone, so the optimistic edit whose `send_crdt_op` failed was
  applied locally, logged, and forgotten — silent divergence, the exact
  stalled-daemon case F-008 targets. Now a `shutdown_handle` clone is
  `shutdown(Both)` whenever the outbox closes (overflow in `send_event`,
  or a writer write error): the reader wakes with EOF and fires the
  existing `Disconnected` path (`(daemon disconnected)` in the window),
  and the daemon sees the half-close. 6 unit tests (coalesce-to-latest,
  clicks keep order, same-kind-tail only, overflow closes, coalescing is
  uncapped, and a socketpair test asserting a closed outbox drives the
  peer to EOF).
- **F-007** (`main.rs`): a pure `mb_dropdown_window(n, selected, band_top)
  -> Option<(first, count)>` clamps the row count to what fits above the
  band (hides entirely when not even one row fits, so `top_y` is never
  negative) and scrolls to keep the selection visible. The method
  `mb_visible_window` feeds `mb_dropdown_rect` (box sized by `count`),
  `mb_dropdown_vertex_bytes` (bg + selection offset by `first`), and the
  render `TextArea` (`top = top_y - first*row`, existing `bounds` clip the
  scrolled-out rows). `(0, n)` when the list fits — the common path is
  byte-identical to before. 1 unit test.

Validated: `cargo fmt` clean; `clippy -p pmacs-gpu --all-targets` clean;
52 pmacs-gpu unit tests pass, including the two headless render tests on
this box's Vulkan adapter (`PMACS_REQUIRE_GPU=1`). Divergence from the
framing: the F-008 fail-fast needed an actual socket teardown, added as a
review follow-up (above); the framing's "clean disconnect" was otherwise
aspirational.

**Still needs a human eyeball before merge** (per the validation
implication above): attach to a non-CRDT daemon → the actionable banner;
shrink the window under an open completion → the dropdown stays on-screen
with the selection visible; and a normal attach still renders/resizes
(the queue rewrite is on the live write path).

## Deferred (named)

- **F-008 auto-reconnect / resync.** Overflow-close now actively tears the
  session down and shows `(daemon disconnected)` (the review follow-up
  above). What's still deferred is *recovery*: `pmacs-gpu` has no
  auto-reconnect, so the diverged optimistic replica is reconciled only by
  a fresh `BufferSnapshot` on a manual re-attach. A "daemon not keeping up
  — reconnecting…" banner + automatic re-attach belongs with the reconnect
  thread ([[attach_reconnect]] already exists daemon-side for the TUI).
- **F-003 capability renegotiation.** We reject on missing caps; a
  friendlier flow would offer to relaunch the daemon with `crdt`. Out of
  scope — the frontend can't manage the daemon's lifecycle.
- **F-007 pointer hit-testing.** The minibuffer dropdown is keyboard-only
  (no pointer hit-test today), so "hit testing uses the same window" is
  vacuous now; revisit if the dropdown becomes clickable.
