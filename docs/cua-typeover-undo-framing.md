# CUA type-over undo grouping — framing pass

Date: 2026-06-15. The region-aware type-over (PR #60-era) replaces a
selection by composing two ops in Lua —
`ed.delete_region(); ed.insert_char(cp)` (builtin/commands/default.lua
`buffer.{newline,tab,self-insert}`). That's two `apply_edit` calls,
hence two undo steps: undo once leaves the half-replaced text, undo
twice restores the original. We want one atomic undo step.

## Survey facts

- Undo granularity is **per `apply_edit`** in both modes. v0.1 pushes
  one `UndoEntry` per edit (buffer.rs:1162). CRDT mode bypasses that
  stack and lets loro's `UndoManager` group ops; each `apply_edit`
  commits via `export_updates_since` (buffer.rs:1061), and nothing
  calls `record_checkpoint` (it's dead code) — so the undo unit is
  effectively the commit, i.e. one per `apply_edit`.
- `EditOp::Replace { range, bytes }` already exists end to end:
  `rope.replace` in v0.1 (buffer.rs:1111), delete-then-insert on the
  loro text in CRDT (buffer.rs:1028) — but **one** `apply_edit`, so
  one undo unit either way.
- Type-over always runs daemon-side: with a selection, pmacs-gpu
  round-trips the key rather than optimistically applying (the comment
  at default.lua:104), so the daemon's command does the edit and
  broadcasts the result. No frontend-undo coordination needed.

## Q#U1 — mechanism

**Stance: model type-over as a single `EditOp::Replace`, not a
delete+insert pair.** This is the *correct* representation (a
type-over is one replace, not two edits) and gives one undo step in
both modes by construction — no undo-group / transaction machinery,
no reliance on loro's time-based merge interval (which is unset). A
general `begin/end_undo_group` boundary was considered and rejected
as over-built for the one concrete case; it can be inducted later if a
third multi-edit command needs it.

## Q#U2 — surface

**Stance: a core `insert_char_over_region(ch)` + one Lua binding.**
Region active → one `Replace(region, ch_bytes)`, cursor → region
start + len, selection cleared. No region → delegate to the existing
`insert_char` (unchanged Insert path). The three type-over commands
call the single binding with their codepoint (10 / 9 / arg).
`delete_region` and `insert_char` stay (other callers); only the
type-over composition moves into the core. Region-aware
backspace/delete already emit one `delete_region` op, so they're
already one undo step — untouched.

## Predicted findings (categorical bets)

1. **CRDT undo-unit assumption**: the per-commit grouping reasoning is
   inferred from the code, not loro docs — the dual-mode test is the
   proof; if CRDT still shows two steps, loro is checkpointing between
   the internal delete and insert and an explicit boundary is needed.
2. **Cursor/selection after replace**: an off-by-the-inserted-length
   cursor or a lingering selection in some multi-byte / empty-insert
   edge — pinned by an editor_core test.

## Session plan

One commit: core `insert_char_over_region`, Lua binding, rewire the
three commands, a dual-mode buffer test (Replace = one undo step) and
an editor_core / acceptance test (type-over over a selection = one
undo step, cursor + selection correct). Manual validation deferred to
the user as usual.
