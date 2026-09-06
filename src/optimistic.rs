//! T M10.10 — Frontend-side optimistic-apply infrastructure.
//!
//! Two pieces of logic live here, deliberately small and testable in
//! isolation:
//!
//! 1. **`classify_key`** — the text-input predicate. Given a keystroke,
//!    decides whether to take the optimistic path (insert / delete-back /
//!    delete-forward) or fall through to the v0.1 `FrontendEvent::Key`
//!    round-trip. Pure function; no state.
//!
//! 2. **`apply_incoming_crdt_op`** — the echo-dedup filter for incoming
//!    `InstanceMessage::CrdtOp` broadcasts. Compares the broadcast's
//!    source `FrontendId` to the local frontend's id and either applies
//!    the op to the `BufferMirror` (remote) or skips (own echo).
//!
//! The attach loop (`attach.rs`) is the consumer: each keystroke runs
//! through `classify_key`; each incoming `CrdtOp` runs through
//! `apply_incoming_crdt_op`. Keeping these as standalone functions in
//! a dedicated module makes them unit-testable without spinning up an
//! attach session.
//!
//! # Semantics note: keymap and the text-input predicate
//!
//! `classify_key` assumes the default keymap's "text-input → self-
//! insert" mapping. Users with Lua keymap rebindings for text characters
//! (e.g., binding 'a' to a non-insert command) will see those bindings
//! lost on replica frontends — text-input chars take the optimistic
//! path and never reach the daemon's keymap layer.
//!
//! This is a v1.0-acceptable simplification: most users don't rebind
//! text characters; users who do can fall back to v0.1 round-trip by
//! advertising `crdt_replica: false`. v0.2+ may expand the predicate
//! to consult a frontend-side keymap mirror.

use crate::buffer::BufferId;
use crate::buffer_mirror::{BufferMirror, BufferMirrorError};
use crate::protocol::{FrontendEvent, FrontendId, Key, KeyEvent, Modifiers, is_builtin_pair_char};
use crate::rope::CrdtOp;
use unicode_width::UnicodeWidthChar;

/// Result of classifying a keystroke for optimistic apply.
///
/// The frontend's keystroke handler matches on this to either take the
/// optimistic path (the concrete actions) or fall through to the v0.1
/// `FrontendEvent::Key` send.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OptimisticAction {
    /// Insert a single character at the cursor.
    Insert(char),
    /// Delete one byte/grapheme behind the cursor (Backspace).
    DeleteBack,
    /// Delete one byte/grapheme at the cursor (Delete-forward).
    DeleteForward,
    /// Undo this frontend's most recent edit on the active buffer
    /// (M10.11 P1). Triggered by single-key undo bindings whose
    /// modifier set is `Ctrl` and whose `Char` is `_` or `/` — the
    /// two terminal-portable spellings of the default-keymap undo
    /// binding (`builtin/keymaps/default.lua`). Multi-key undo
    /// bindings like `C-x u` fall through to `RoundTrip` because
    /// the optimistic layer doesn't track keymap-prefix state.
    Undo,
    /// No optimistic path applies; fall through to round-trip via
    /// `FrontendEvent::Key`. Covers control-char modifiers (Ctrl, Alt,
    /// Meta, Hyper) that aren't bound to an optimistic action,
    /// function keys, navigation keys, and any keystroke whose
    /// semantics aren't text-input or recognized commands.
    RoundTrip,
}

/// True if the modifier set excludes all editor-control modifiers
/// (Ctrl, Alt, Meta, Hyper). Shift is allowed because capital letters
/// arrive as `Char('A')` with `SHIFT` set — Shift is part of how the
/// character was produced, not a command modifier that changes semantics.
const fn is_text_input_modifiers(mods: Modifiers) -> bool {
    !mods.contains(Modifiers::CTRL)
        && !mods.contains(Modifiers::ALT)
        && !mods.contains(Modifiers::META)
        && !mods.contains(Modifiers::HYPER)
}

/// Classify a key event for the optimistic-apply path.
///
/// Returns:
/// - `Insert(c)` for a printable `Char(c)` with no editor-control modifier.
/// - `DeleteBack` for `Backspace` with no editor-control modifier.
/// - `DeleteForward` for `Delete` with no editor-control modifier.
/// - `RoundTrip` for everything else (modified text input, function
///   keys, arrows, escape, etc.).
///
/// `char::is_control()` filters out ASCII control codes (0x00–0x1F,
/// 0x7F) and Unicode control codes. Tab and Enter qualify as control
/// chars and therefore round-trip — they often have non-insert
/// semantics in editor keymaps (indentation, newline-with-indent).
#[must_use]
pub fn classify_key(key: Key, mods: Modifiers) -> OptimisticAction {
    // M10.11 P1 — single-key undo bindings.
    //
    // The default keymap (`builtin/keymaps/default.lua`) binds four
    // forms of undo: `C-/`, `C-_`, `C-4`, and `C-x u`. Crossterm's
    // raw-terminal parser (`crossterm-0.28.1` /
    // `event/sys/unix/parse.rs:106-113`) only delivers some of
    // these as the literal `Char + Modifiers::CTRL` shape:
    //
    // - 0x01..=0x1A (Ctrl-A..Ctrl-Z) → `Char(letter)` + CTRL
    // - 0x1C..=0x1F → `Char('4')..Char('7')` + CTRL (the offset-
    //   from-'4' convention crossterm uses for non-letter Ctrl
    //   bytes; *not* the "Ctrl-_" / "Ctrl-/" naming users
    //   intuitively expect — that mapping requires Kitty Keyboard
    //   Protocol enhanced mode, which pmacs doesn't currently
    //   negotiate).
    //
    // Practical consequence: when a real terminal user presses
    // Ctrl-_, the byte 0x1F arrives, crossterm produces
    // `Char('7')` + CTRL, *no* default-keymap binding matches.
    // The deliverable undo keystrokes for raw-terminal users are
    // C-4 (byte 0x1C) and C-x u (multi-key, falls through to
    // daemon dispatch).
    //
    // We optimistically recognize `Char('4')` + CTRL as Undo
    // because the default keymap binds it, AND it's the form a
    // real terminal can actually deliver. `Char('/')` and
    // `Char('_')` with CTRL are also recognized for symmetry —
    // they'll match when Kitty enhanced mode is negotiated, or
    // when a non-PTY frontend (future GUI) emits them directly.
    // Multi-key bindings like `C-x u` round-trip because the
    // optimistic layer doesn't track keymap-prefix state.
    //
    // The `mods == CTRL` exact-match (rather than `contains(CTRL)`)
    // ensures combos like `C-S-_` round-trip rather than triggering
    // undo unexpectedly. Lua-rebound forms similarly round-trip.
    if mods == Modifiers::CTRL && matches!(key, Key::Char('/' | '_' | '4')) {
        return OptimisticAction::Undo;
    }
    if !is_text_input_modifiers(mods) {
        return OptimisticAction::RoundTrip;
    }
    match key {
        // Auto-pairing Q#AP1: the built-in pair charset always
        // round-trips so the opener and the pairing hook's closer are
        // adjacent daemon-peer undo units (and dispatch-path CUA
        // type-over applies). An optimistic pair char would be a
        // source-peer op whose reaction closer lives on the daemon
        // peer — uncleanly undoable from either frontend.
        Key::Char(c) if !c.is_control() && !is_builtin_pair_char(c) => OptimisticAction::Insert(c),
        Key::Backspace => OptimisticAction::DeleteBack,
        Key::Delete => OptimisticAction::DeleteForward,
        _ => OptimisticAction::RoundTrip,
    }
}

/// Compute the `FrontendEvent` to send upstream for a keystroke,
/// applying the optimistic-apply path locally if the mirror is
/// ready **and** the action is paint-eligible.
///
/// This is the keystroke-handler orchestrator: it classifies the
/// key, consults the mirror's readiness + paint-eligibility for the
/// active buffer, and either returns a `FrontendEvent::CrdtOp`
/// (after applying the edit to the local mirror) or a
/// `FrontendEvent::Key` (round-trip path: original keystroke is
/// forwarded as before).
///
/// # Decision flow
///
/// 1. `classify_key(key, mods)` → `OptimisticAction`.
/// 2. `RoundTrip` → return `FrontendEvent::Key`.
/// 3. `Insert(c)` / `DeleteBack` / `DeleteForward`:
///    - Read `mirror.active_buffer()`. None → fall through to
///      `FrontendEvent::Key` (no active buffer known; Refinement 4
///      graceful degradation).
///    - Check `mirror.is_ready(active_buffer)`. False → fall
///      through (mirror not bootstrapped for this buffer).
///    - Check `mirror.cursor_byte_pos(active_buffer)`. None → fall
///      through (cursor position unknown).
///    - **Paint-eligibility gate (post-audit-round-3 F19):**
///      - `Insert(c)`: require `mirror.cursor_at_end_of_line` →
///        Some(true). Mid-line insert falls through to round-trip.
///      - `DeleteBack`: require
///        `mirror.cursor_at_end_of_line_safe_for_delete_back` →
///        Some(true). Mid-line, line-joining, or width-unsafe
///        delete-back falls through to round-trip.
///      - `DeleteForward`: ALWAYS round-trip (no optimistic paint
///        primitive exists for delete-forward; advancing the
///        mirror without painting de-syncs the mirror cursor from
///        the terminal cursor).
///    - Apply the action to the mirror. Errors fall through.
///    - Return `FrontendEvent::CrdtOp { source: my_fid, buffer_id,
///      op: CrdtOp { peer_id, bytes } }`.
///
/// The caller writes whatever `FrontendEvent` is returned. Non-
/// paint-eligible actions round-trip via the daemon's Key path; the
/// daemon's `apply_active_edit` emits a `DaemonKey`-origin CRDT op
/// (F16) which broadcasts to every replica including the source.
/// The source mirror updates via the broadcast, keeping mirror state
/// and terminal cursor coherent.
///
/// # F19 motivation
///
/// Pre-fix, the orchestrator advanced mirror state for non-paint
/// edits (mid-line insert, mid-line backspace, all delete-forward).
/// The terminal cursor stayed at its pre-edit position (no paint
/// primitive fires) until the daemon's `CellDelta` arrived. A fast
/// next keystroke would then run optimistic logic against the
/// already-advanced mirror — `cursor_at_end_of_line` could report
/// "yes" at the mirror's new cursor while the terminal cursor was
/// at the OLD pre-edit position, causing `paint_optimistic_insert`
/// to write at the wrong column.
#[must_use]
pub fn frontend_event_for_keystroke(
    mirror: &mut BufferMirror,
    my_fid: FrontendId,
    pmacs_key: KeyEvent,
) -> FrontendEvent {
    let action = classify_key(pmacs_key.key, pmacs_key.mods);
    let round_trip = || FrontendEvent::Key(pmacs_key);
    if matches!(action, OptimisticAction::RoundTrip) {
        return round_trip();
    }
    // Need: active buffer + mirror ready for it. The cursor-position
    // and cursor-freshness checks only apply to position-targeted
    // actions (Insert / DeleteBack); Undo reverses the last op by
    // peer regardless of cursor position, so it skips those gates.
    let Some(buffer_id) = mirror.active_buffer() else {
        return round_trip();
    };
    if !mirror.is_ready(buffer_id) {
        return round_trip();
    }

    // M10.11 P1 — undo's optimistic path. Undo doesn't depend on
    // cursor position or paint eligibility (stance α: no visual
    // paint for optimistic undo; daemon's CellDelta drives
    // reconciliation). The undo affects content at arbitrary
    // positions; `apply_local_undo` marks the cursor stale so
    // subsequent optimistic keystrokes round-trip until the daemon's
    // `CursorByte` re-grounds.
    if matches!(action, OptimisticAction::Undo) {
        return match mirror.apply_local_undo(buffer_id) {
            Ok(Some(op_bytes)) => FrontendEvent::CrdtOp {
                frontend_id: my_fid,
                buffer_id,
                op: CrdtOp {
                    peer_id: mirror.peer_id(),
                    bytes: op_bytes,
                },
            },
            // Nothing to undo locally (UndoManager stack empty) or
            // loro error. Round-trip the Key event; the daemon's
            // dispatch_key may have its own daemon-peer ops to undo
            // (Lua-driven daemon-side edits), so the Key path remains
            // the right fallback. If the daemon also has nothing, the
            // path silently no-ops — same as v0.1.
            Ok(None) | Err(_) => round_trip(),
        };
    }

    // Position-targeted actions need authoritative cursor state
    // (post-audit-round-4 F22 + F23 freshness invariant). A stale
    // cursor means the mirror's cursor for this buffer hasn't been
    // re-grounded by the daemon's `CursorByte` since the last
    // potential desync.
    if !mirror.is_cursor_fresh(buffer_id) {
        return round_trip();
    }
    let Some(cursor) = mirror.cursor_byte_pos(buffer_id) else {
        return round_trip();
    };

    let result = match action {
        OptimisticAction::Insert(c) => {
            // F19 — only optimistic-apply when paint will fire.
            // Mid-line insert can't be painted by a single Print
            // (cells to the right of cursor would need to shift),
            // so we round-trip and let the daemon's CellDelta drive
            // both visual and mirror update.
            if mirror.cursor_at_end_of_line(buffer_id) != Some(true) {
                return round_trip();
            }
            // Post-audit-round-4 F24 — `paint_optimistic_insert`
            // calls `queue!(Print(c))` which writes one terminal
            // column. Wide chars (width 2) would only paint the
            // base column without the Continuation cell; zero-
            // width chars (combining marks, ZWJ) write a column
            // for what `TextView` renders as a cluster attached to
            // the previous cell. Either mismatch leaves the
            // terminal out of sync with what the daemon's
            // eventual `CellDelta` will paint. Width-unsafe
            // inserts round-trip; the daemon's `CellDelta` drives
            // both visual + mirror update.
            if UnicodeWidthChar::width(c) != Some(1) {
                return round_trip();
            }
            let mut tmp = [0u8; 4];
            let s: &str = c.encode_utf8(&mut tmp);
            let s_len = s.len();
            mirror
                .apply_local_insert(buffer_id, cursor, s)
                .inspect(|_bytes| {
                    mirror.advance_cursor(buffer_id, s_len);
                })
        }
        OptimisticAction::DeleteBack => {
            // F19 — only optimistic-apply when the strict delete-
            // back predicate holds (end-of-line + prev char not '\n'
            // + prev char width == 1). Round-trip otherwise.
            if mirror.cursor_at_end_of_line_safe_for_delete_back(buffer_id) != Some(true) {
                return round_trip();
            }
            match mirror.prev_char_len(buffer_id) {
                Some(n) if n > 0 => {
                    let new_pos = cursor.saturating_sub(n);
                    mirror
                        .apply_local_delete(buffer_id, new_pos, n)
                        .inspect(|_bytes| {
                            mirror.retreat_cursor(buffer_id, n);
                        })
                }
                _ => return round_trip(),
            }
        }
        OptimisticAction::DeleteForward => {
            // F19 — no optimistic paint primitive exists for
            // delete-forward. Always round-trip. Mirror state stays
            // coherent via the daemon's broadcast (F16 ensures the
            // source receives it).
            return round_trip();
        }
        OptimisticAction::Undo => unreachable!("Undo handled above"),
        OptimisticAction::RoundTrip => unreachable!("RoundTrip handled above"),
    };

    match result {
        Ok(op_bytes) => FrontendEvent::CrdtOp {
            frontend_id: my_fid,
            buffer_id,
            op: CrdtOp {
                peer_id: mirror.peer_id(),
                bytes: op_bytes,
            },
        },
        Err(_) => {
            // Optimistic application failed (e.g., mid-codepoint or
            // out-of-range position — shouldn't happen given the
            // char-aware byte counts above, but defensive). Fall
            // through to v0.1 round-trip.
            round_trip()
        }
    }
}

/// Outcome of routing an incoming `InstanceMessage::CrdtOp` through
/// the echo-dedup filter.
#[derive(Debug, Eq, PartialEq)]
pub enum IncomingCrdtOpOutcome {
    /// Op was applied to the mirror (source frontend was not us).
    Applied,
    /// Op was filtered out as a local-edit echo (source frontend
    /// was us; the mirror has already integrated this op via
    /// `apply_local_insert` / `apply_local_delete`).
    SkippedEcho,
}

/// Handle an incoming `CrdtOp` broadcast: filter own-echoes; apply
/// remote ops to the mirror.
///
/// **Filter rule (matches `BufferMirror::apply_remote_op` docstring):**
/// op is an echo iff `source == local_id`. The filter lives at this
/// call site (not in `BufferMirror`) because the mirror is
/// identity-ignorant by design — it operates on op bytes only.
/// Pushing the filter here keeps the mirror reusable in non-session
/// contexts (tests, future Lua bindings) and concentrates the
/// `FrontendId` comparison in one place.
///
/// # Errors
///
/// Propagates [`BufferMirrorError`] from the underlying mirror call.
/// Echo-skip never errors.
pub fn apply_incoming_crdt_op(
    mirror: &mut BufferMirror,
    local_id: FrontendId,
    source: FrontendId,
    buffer_id: BufferId,
    op_bytes: &[u8],
) -> Result<IncomingCrdtOpOutcome, BufferMirrorError> {
    if source == local_id {
        return Ok(IncomingCrdtOpOutcome::SkippedEcho);
    }
    mirror.apply_remote_op(buffer_id, op_bytes)?;
    Ok(IncomingCrdtOpOutcome::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::CrdtState;

    // -----------------------------------------------------------------
    // classify_key — the text-input predicate.
    // -----------------------------------------------------------------

    #[test]
    fn classify_ascii_char_no_modifiers_is_insert() {
        assert_eq!(
            classify_key(Key::Char('a'), Modifiers::NONE),
            OptimisticAction::Insert('a')
        );
    }

    #[test]
    fn classify_ascii_char_with_shift_is_still_insert() {
        // Capital letters arrive with SHIFT set; the char itself is
        // already the shifted version.
        assert_eq!(
            classify_key(Key::Char('A'), Modifiers::SHIFT),
            OptimisticAction::Insert('A')
        );
    }

    #[test]
    fn classify_ascii_char_with_ctrl_is_round_trip() {
        // Ctrl+a is a command, not text input.
        assert_eq!(
            classify_key(Key::Char('a'), Modifiers::CTRL),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_ascii_char_with_alt_is_round_trip() {
        assert_eq!(
            classify_key(Key::Char('a'), Modifiers::ALT),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_ascii_char_with_meta_is_round_trip() {
        assert_eq!(
            classify_key(Key::Char('a'), Modifiers::META),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_ascii_char_with_hyper_is_round_trip() {
        assert_eq!(
            classify_key(Key::Char('a'), Modifiers::HYPER),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_space_no_modifiers_is_insert() {
        assert_eq!(
            classify_key(Key::Char(' '), Modifiers::NONE),
            OptimisticAction::Insert(' ')
        );
    }

    #[test]
    fn classify_punctuation_no_modifiers_is_insert() {
        for c in ['.', ',', ';', ':', '!', '?', '@', '#', '$', '%'] {
            assert_eq!(
                classify_key(Key::Char(c), Modifiers::NONE),
                OptimisticAction::Insert(c)
            );
        }
    }

    #[test]
    fn classify_builtin_pair_chars_round_trip() {
        // Auto-pairing Q#AP1: the nine built-in pair chars must reach
        // the daemon's dispatch so the opener and the hook's closer are
        // adjacent daemon-peer undo units. Both modifier shapes real
        // keyboards produce are pinned: `[`/`]`/`'`/`` ` `` arrive
        // unshifted, `(`/`)`/`{`/`}`/`"` arrive with SHIFT set — a gate
        // that only caught `Modifiers::NONE` would leak every shifted
        // pair char back onto the optimistic path.
        for c in crate::protocol::BUILTIN_PAIR_CHARS {
            assert_eq!(
                classify_key(Key::Char(c), Modifiers::NONE),
                OptimisticAction::RoundTrip,
                "unshifted {c:?} must round-trip"
            );
            assert_eq!(
                classify_key(Key::Char(c), Modifiers::SHIFT),
                OptimisticAction::RoundTrip,
                "shifted {c:?} must round-trip"
            );
        }
    }

    #[test]
    fn classify_unicode_char_no_modifiers_is_insert() {
        // Non-ASCII printable — multi-byte UTF-8.
        assert_eq!(
            classify_key(Key::Char('é'), Modifiers::NONE),
            OptimisticAction::Insert('é')
        );
        assert_eq!(
            classify_key(Key::Char('中'), Modifiers::NONE),
            OptimisticAction::Insert('中')
        );
    }

    #[test]
    fn classify_backspace_no_modifiers_is_delete_back() {
        assert_eq!(
            classify_key(Key::Backspace, Modifiers::NONE),
            OptimisticAction::DeleteBack
        );
    }

    #[test]
    fn classify_backspace_with_shift_is_still_delete_back() {
        // Shift+Backspace behaves the same as Backspace in default
        // keymap; Shift doesn't change the semantic.
        assert_eq!(
            classify_key(Key::Backspace, Modifiers::SHIFT),
            OptimisticAction::DeleteBack
        );
    }

    #[test]
    fn classify_backspace_with_ctrl_is_round_trip() {
        // Ctrl+Backspace is often "delete previous word" — keymap
        // territory, not optimistic text-input.
        assert_eq!(
            classify_key(Key::Backspace, Modifiers::CTRL),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_delete_no_modifiers_is_delete_forward() {
        assert_eq!(
            classify_key(Key::Delete, Modifiers::NONE),
            OptimisticAction::DeleteForward
        );
    }

    #[test]
    fn classify_delete_with_ctrl_is_round_trip() {
        assert_eq!(
            classify_key(Key::Delete, Modifiers::CTRL),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_function_key_is_round_trip() {
        for n in 1u8..=12 {
            assert_eq!(
                classify_key(Key::F(n), Modifiers::NONE),
                OptimisticAction::RoundTrip
            );
        }
    }

    #[test]
    fn classify_navigation_keys_are_round_trip() {
        for key in [
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
        ] {
            assert_eq!(
                classify_key(key, Modifiers::NONE),
                OptimisticAction::RoundTrip
            );
        }
    }

    #[test]
    fn classify_enter_and_tab_are_round_trip() {
        // Enter and Tab are arguably text input but often have
        // editor-specific semantics (newline-with-indent, indent
        // command). Send via round-trip so keymap can dispatch.
        assert_eq!(
            classify_key(Key::Enter, Modifiers::NONE),
            OptimisticAction::RoundTrip
        );
        assert_eq!(
            classify_key(Key::Tab, Modifiers::NONE),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_escape_is_round_trip() {
        assert_eq!(
            classify_key(Key::Escape, Modifiers::NONE),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_unknown_key_is_round_trip() {
        assert_eq!(
            classify_key(Key::Unknown(0x1234), Modifiers::NONE),
            OptimisticAction::RoundTrip
        );
    }

    #[test]
    fn classify_null_char_is_round_trip() {
        // Char('\0') is control; should not optimistic-insert a NUL.
        assert_eq!(
            classify_key(Key::Char('\0'), Modifiers::NONE),
            OptimisticAction::RoundTrip
        );
    }

    // -----------------------------------------------------------------
    // apply_incoming_crdt_op — echo-dedup composition test.
    // -----------------------------------------------------------------

    /// Helper: build a snapshot of a small CRDT replica seeded with
    /// the given text under `peer_id`.
    fn fresh_snapshot(peer_id: u64, initial: &str) -> Vec<u8> {
        let state = CrdtState::new(peer_id).expect("new");
        state.insert(0, initial).expect("seed");
        state.export_snapshot().expect("export")
    }

    // -----------------------------------------------------------------
    // frontend_event_for_keystroke — Day 3 step 3b orchestrator.
    // -----------------------------------------------------------------

    fn key_event(key: Key, mods: Modifiers, fid: FrontendId) -> KeyEvent {
        KeyEvent {
            frontend_id: fid,
            key,
            mods,
            timestamp_ns: 0,
        }
    }

    fn ready_mirror_with_cursor(initial: &str, cursor: usize) -> (BufferMirror, BufferId) {
        let fid = FrontendId(2);
        let mut m = BufferMirror::new(fid);
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, initial))
            .expect("init");
        m.set_cursor_byte_pos(id, cursor);
        (m, id)
    }

    #[test]
    fn keystroke_round_trips_when_mirror_has_no_active_buffer() {
        let mut m = BufferMirror::new(FrontendId(2));
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('a'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {} // round-trip
            other => panic!("expected Key (graceful fallback), got {other:?}"),
        }
    }

    #[test]
    fn keystroke_round_trips_when_buffer_not_ready() {
        let fid = FrontendId(2);
        let mut m = BufferMirror::new(fid);
        // Set cursor for a buffer that has no snapshot yet — mirror
        // tracks the cursor but is_ready returns false.
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 5);
        assert!(!m.is_ready(id));
        let ev = key_event(Key::Char('a'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {} // round-trip per Refinement 4
            other => panic!("expected Key (graceful fallback), got {other:?}"),
        }
    }

    #[test]
    fn keystroke_round_trips_for_non_text_input() {
        // Ctrl+a — not text input; round-trip regardless of mirror
        // state.
        let (mut m, _id) = ready_mirror_with_cursor("hello", 5);
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('a'), Modifiers::CTRL, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(k) => {
                assert!(k.mods.contains(Modifiers::CTRL));
            }
            other => panic!("expected Key for non-text-input, got {other:?}"),
        }
    }

    #[test]
    fn keystroke_optimistic_insert_produces_crdt_op_and_advances_cursor() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('!'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);

        match result {
            FrontendEvent::CrdtOp {
                frontend_id,
                buffer_id,
                op,
            } => {
                assert_eq!(frontend_id, fid);
                assert_eq!(buffer_id, id);
                assert_eq!(op.peer_id, 2);
                assert!(!op.bytes.is_empty());
            }
            other => panic!("expected CrdtOp, got {other:?}"),
        }
        // Mirror state advanced.
        assert_eq!(m.materialize(id).as_deref(), Some("hello!"));
        assert_eq!(m.cursor_byte_pos(id), Some(6));
    }

    #[test]
    fn keystroke_optimistic_insert_with_multibyte_char_advances_by_byte_length() {
        let (mut m, id) = ready_mirror_with_cursor("ab", 2);
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('é'), Modifiers::NONE, fid); // 2 bytes
        let _ = frontend_event_for_keystroke(&mut m, fid, ev);
        assert_eq!(m.materialize(id).as_deref(), Some("abé"));
        // Cursor advanced by 2 (UTF-8 byte length).
        assert_eq!(m.cursor_byte_pos(id), Some(4));
    }

    #[test]
    fn keystroke_optimistic_delete_back_ascii_removes_one_byte() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        let fid = FrontendId(2);
        let ev = key_event(Key::Backspace, Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::CrdtOp { .. } => {}
            other => panic!("expected CrdtOp, got {other:?}"),
        }
        assert_eq!(m.materialize(id).as_deref(), Some("hell"));
        assert_eq!(m.cursor_byte_pos(id), Some(4));
    }

    #[test]
    fn keystroke_optimistic_delete_back_multibyte_removes_full_char() {
        // Char-boundary-aware: deleting back from after 'é' removes
        // both UTF-8 bytes, not just one (loro rejects mid-codepoint).
        let (mut m, id) = ready_mirror_with_cursor("aé", 3);
        let fid = FrontendId(2);
        let ev = key_event(Key::Backspace, Modifiers::NONE, fid);
        let _ = frontend_event_for_keystroke(&mut m, fid, ev);
        assert_eq!(m.materialize(id).as_deref(), Some("a"));
        assert_eq!(m.cursor_byte_pos(id), Some(1));
    }

    #[test]
    fn keystroke_optimistic_delete_back_at_position_zero_round_trips() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 0);
        let fid = FrontendId(2);
        let ev = key_event(Key::Backspace, Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {} // round-trip — nothing to delete
            other => panic!("expected Key at pos 0, got {other:?}"),
        }
        // Mirror untouched.
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
    }

    #[test]
    fn keystroke_optimistic_delete_forward_at_end_round_trips() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        let fid = FrontendId(2);
        let ev = key_event(Key::Delete, Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {} // round-trip — nothing to delete forward
            other => panic!("expected Key at end, got {other:?}"),
        }
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
    }

    /// Post-audit-round-3 F19: `OptimisticAction::DeleteForward`
    /// **always** round-trips. No paint primitive exists for
    /// forward-delete, so optimistically applying to the mirror
    /// without painting would desync the mirror cursor from the
    /// terminal cursor. The daemon's `CellDelta` + the F16 broadcast
    /// keep both sides in sync via round-trip.
    #[test]
    fn keystroke_optimistic_delete_forward_always_round_trips_f19() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 1);
        let fid = FrontendId(2);
        let ev = key_event(Key::Delete, Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F19: DeleteForward must round-trip; got {other:?}"),
        }
        // Mirror MUST be untouched — F19 narrowing prevents the
        // mirror from advancing on round-tripped edits.
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
        assert_eq!(m.cursor_byte_pos(id), Some(1));
    }

    /// F19: `OptimisticAction::Insert` mid-line round-trips
    /// (paint scope is end-of-line only; advancing the mirror
    /// mid-line desyncs from the terminal cursor).
    #[test]
    fn keystroke_optimistic_insert_mid_line_round_trips_f19() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 2); // mid-line
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('X'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F19: mid-line Insert must round-trip; got {other:?}"),
        }
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
        assert_eq!(m.cursor_byte_pos(id), Some(2));
    }

    /// F24 (post-audit-round-4) — wide char (`UnicodeWidthChar::width
    /// == Some(2)`) Insert round-trips. The optimistic paint
    /// `queue!(Print(c))` writes one column; the daemon's eventual
    /// `CellDelta` paints two cells (base + Continuation).
    #[test]
    fn keystroke_optimistic_insert_wide_char_round_trips_f24() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        let fid = FrontendId(2);
        // CJK ideograph (width 2).
        let ev = key_event(Key::Char('漢'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F24: wide-char Insert must round-trip; got {other:?}"),
        }
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
        assert_eq!(m.cursor_byte_pos(id), Some(5));
    }

    /// F24 — combining mark / zero-width Insert round-trips. The
    /// paint writes a column for what `TextView` renders as a
    /// cluster attached to the previous cell.
    #[test]
    fn keystroke_optimistic_insert_combining_mark_round_trips_f24() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        let fid = FrontendId(2);
        // U+0301 COMBINING ACUTE ACCENT (width 0).
        let ev = key_event(Key::Char('\u{0301}'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F24: zero-width combining-mark Insert must round-trip; got {other:?}"),
        }
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
    }

    /// F22 (post-audit-round-4) — when the mirror cursor is stale
    /// (a prior `FrontendEvent::Key` may have moved the daemon's
    /// cursor in ways the mirror can't predict), the orchestrator
    /// must round-trip subsequent keystrokes until `CursorByte`
    /// re-grounds the mirror cursor. Otherwise a fast `<left>` then
    /// `x` sequence emits an Insert `CrdtOp` for the byte the cursor
    /// was at BEFORE `<left>`.
    #[test]
    fn keystroke_round_trips_when_cursor_is_stale_f22() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        m.mark_cursor_stale(id);
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('x'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F22: stale cursor must round-trip subsequent keys; got {other:?}"),
        }
        // Mirror untouched.
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
        assert_eq!(m.cursor_byte_pos(id), Some(5));
    }

    /// F23 (post-audit-round-4) — `apply_remote_op` marks the cursor
    /// stale. A keystroke between the remote op and the next
    /// `CursorByte` must round-trip.
    #[test]
    fn keystroke_round_trips_after_remote_op_until_cursor_byte_f23() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 5);
        // Remote op: peer inserts "X" at position 0.
        let peer = CrdtState::new(99).expect("peer");
        peer.import_snapshot(&fresh_snapshot(99, "hello"))
            .expect("peer init");
        let v0 = peer.version();
        peer.insert(0, "X").expect("peer insert");
        let op_bytes = peer.export_updates_since(&v0).expect("export");
        m.apply_remote_op(id, &op_bytes).expect("apply remote");

        // Mirror content now "Xhello"; mirror cursor still at 5
        // (no right-gravity adjustment). Without F23 the
        // orchestrator would optimistically insert at byte 5 of
        // "Xhello" — wrong byte for the user's intent.
        let fid = FrontendId(2);
        let ev = key_event(Key::Char('!'), Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F23: post-remote-op cursor staleness must round-trip; got {other:?}"),
        }

        // After daemon's CursorByte re-grounds the mirror, optimistic
        // path reopens.
        m.set_cursor_byte_pos(id, 6); // post-right-gravity position
        let result2 = frontend_event_for_keystroke(&mut m, fid, ev);
        match result2 {
            FrontendEvent::CrdtOp { .. } => {}
            other => panic!("F23: CursorByte must clear staleness; got {other:?}"),
        }
    }

    /// F19: `OptimisticAction::DeleteBack` mid-line round-trips.
    #[test]
    fn keystroke_optimistic_delete_back_mid_line_round_trips_f19() {
        let (mut m, id) = ready_mirror_with_cursor("hello", 3); // mid-line
        let fid = FrontendId(2);
        let ev = key_event(Key::Backspace, Modifiers::NONE, fid);
        let result = frontend_event_for_keystroke(&mut m, fid, ev);
        match result {
            FrontendEvent::Key(_) => {}
            other => panic!("F19: mid-line DeleteBack must round-trip; got {other:?}"),
        }
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
        assert_eq!(m.cursor_byte_pos(id), Some(3));
    }

    /// T M10.10 Day 4 — criterion 1 unit acceptance: keystroke-to-
    /// optimistic-apply completes in sub-frame time regardless of
    /// any daemon state.
    ///
    /// Spec criterion 1: "Local edit visible in less than one frame
    /// regardless of instance latency."
    ///
    /// The load-bearing property under Path β is that the orchestrator
    /// completes synchronously — it doesn't wait for the daemon, doesn't
    /// poll, doesn't block on I/O. Mirror update is in-process; produced
    /// `FrontendEvent` is returned by value. Latency to the daemon
    /// affects when the daemon's `CellDelta` arrives back, but doesn't
    /// affect the orchestrator's completion time.
    ///
    /// This test demonstrates the property directly without involving
    /// a daemon: 100 consecutive keystrokes through the orchestrator
    /// must complete in well under 16ms (one frame at 60Hz). In
    /// practice each call is microseconds; the upper bound is
    /// generous to avoid CI flakiness.
    ///
    /// **Path β scope**: this test exercises end-of-line typing
    /// (cursor at content end after each insert). Mid-line typing
    /// would round-trip and incur daemon-latency for paint per Path
    /// β's documented scope.
    /// Type the pangram end-of-line, asserting every keystroke took the
    /// optimistic path; returns (total, per keystroke).
    fn type_pangram_optimistically() -> (std::time::Duration, std::time::Duration) {
        use std::time::Instant;
        let (mut m, _id) = ready_mirror_with_cursor("", 0);
        let fid = FrontendId(2);

        let start = Instant::now();
        for c in "the quick brown fox jumps over the lazy dog".chars() {
            let ev = key_event(Key::Char(c), Modifiers::NONE, fid);
            let result = frontend_event_for_keystroke(&mut m, fid, ev);
            // Verify each keystroke took the optimistic path —
            // criterion 1 isn't met if the orchestrator falls through
            // to FrontendEvent::Key (which would await daemon round-
            // trip).
            assert!(
                matches!(result, FrontendEvent::CrdtOp { .. }),
                "criterion 1: orchestrator must produce CrdtOp for end-of-line \
                 text input (not round-trip Key)"
            );
        }
        let elapsed = start.elapsed();
        (elapsed, elapsed / 43) // length of the pangram
    }

    #[test]
    fn criterion_1_end_of_line_typing_takes_the_optimistic_path() {
        let (elapsed, per_keystroke) = type_pangram_optimistically();
        eprintln!("criterion 1: {per_keystroke:?} per keystroke, {elapsed:?} total");
    }

    #[test]
    #[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
    fn criterion_1_end_of_line_typing_completes_sub_frame_per_keystroke() {
        let (elapsed, per_keystroke) = type_pangram_optimistically();
        // Upper bound per keystroke: 1ms (60× under frame budget).
        // Loose because CI runners vary; tight enough to catch any
        // synchronous-IO regression that would put criterion 1 at risk.
        assert!(
            per_keystroke < std::time::Duration::from_millis(1),
            "criterion 1: per-keystroke orchestrator time {per_keystroke:?} \
             exceeds 1ms (well below 16ms frame budget). Total: {elapsed:?}"
        );
    }

    /// T M10.10 Day 3 step 6 — bootstrap-window typing test.
    ///
    /// Narrates the bootstrap state machine that the M10.10 frontend
    /// goes through on attach. The "bootstrap window" is the time
    /// between session establishment and the frontend's mirror being
    /// ready for optimistic apply — during this window, keystrokes
    /// must gracefully degrade to v0.1 round-trip (Refinement 4).
    /// After the bootstrap completes (`BufferSnapshot` + `CursorByte`
    /// applied), subsequent keystrokes take the optimistic path.
    ///
    /// This test walks the explicit transitions a real attach would
    /// experience (modulo the inter-thread message delivery the wire
    /// transports do):
    /// 1. Pre-bootstrap: empty mirror, type 'h' → `FrontendEvent::Key`.
    /// 2. `BufferSnapshot` arrives (`init_from_snapshot`): mirror has
    ///    state but no active buffer / cursor yet. Type 'i' → still
    ///    Key (no `active_buffer` until `CursorByte` arrives).
    /// 3. `CursorByte` arrives (`set_cursor_byte_pos`): mirror has
    ///    `active_buffer` + cursor; the optimistic predicate now fires
    ///    for in-scope keystrokes.
    /// 4. Post-bootstrap: type 'j' → `FrontendEvent::CrdtOp`.
    ///
    /// Each transition is the boundary that step 3a's wire variants
    /// (`BufferSnapshot`, `CursorByte`) and step 3b's orchestrator
    /// jointly enforce. The narration matters because the bootstrap-
    /// window race condition isn't deterministic at the daemon-e2e
    /// level (the inter-thread message-drain ordering on the
    /// receiver) — exercising the orchestrator's state machine
    /// directly makes the contract observable.
    #[test]
    fn bootstrap_window_keystrokes_round_trip_until_mirror_ready() {
        let fid = FrontendId(2);
        let id = BufferId::next();
        let mut m = BufferMirror::new(fid);

        // -----------------------------------------------------------
        // State 1: pre-bootstrap. Empty mirror; no buffers; no active
        // buffer; no cursor. Refinement 4 graceful-degradation kicks
        // in: keystrokes that would otherwise be optimistic must
        // fall through to FrontendEvent::Key.
        // -----------------------------------------------------------
        assert!(m.active_buffer().is_none());
        assert!(!m.is_ready(id));

        let ev = key_event(Key::Char('h'), Modifiers::NONE, fid);
        let result_1 = frontend_event_for_keystroke(&mut m, fid, ev);
        match result_1 {
            FrontendEvent::Key(_) => {} // expected — graceful fallback
            other => panic!("pre-bootstrap keystroke must round-trip via Key; got {other:?}"),
        }

        // -----------------------------------------------------------
        // State 2: BufferSnapshot processed. Mirror has CRDT state
        // for `id`, but the daemon hasn't yet emitted CursorByte —
        // active_buffer is still None. The orchestrator must still
        // round-trip because there's no active buffer to target.
        // -----------------------------------------------------------
        m.init_from_snapshot(id, &fresh_snapshot(99, "abc"))
            .expect("init from BufferSnapshot");
        assert!(m.is_ready(id));
        assert!(m.active_buffer().is_none()); // CursorByte hasn't fired yet

        let ev = key_event(Key::Char('i'), Modifiers::NONE, fid);
        let result_2 = frontend_event_for_keystroke(&mut m, fid, ev);
        match result_2 {
            FrontendEvent::Key(_) => {} // expected — no active_buffer
            other => panic!(
                "post-BufferSnapshot but pre-CursorByte keystroke must round-trip; \
                 got {other:?}"
            ),
        }

        // -----------------------------------------------------------
        // State 3: CursorByte processed. Mirror has active_buffer +
        // cursor byte position. The optimistic predicate can now
        // fire for in-scope keystrokes. Bootstrap is complete.
        // -----------------------------------------------------------
        m.set_cursor_byte_pos(id, 3); // cursor at end of "abc"
        assert_eq!(m.active_buffer(), Some(id));
        assert_eq!(m.cursor_byte_pos(id), Some(3));

        // -----------------------------------------------------------
        // State 4: post-bootstrap typing. Optimistic path active;
        // keystroke produces FrontendEvent::CrdtOp.
        // -----------------------------------------------------------
        let ev = key_event(Key::Char('j'), Modifiers::NONE, fid);
        let result_3 = frontend_event_for_keystroke(&mut m, fid, ev);
        match result_3 {
            FrontendEvent::CrdtOp {
                frontend_id,
                buffer_id,
                op,
            } => {
                assert_eq!(frontend_id, fid);
                assert_eq!(buffer_id, id);
                assert_eq!(op.peer_id, fid.0);
                assert!(!op.bytes.is_empty());
            }
            other => {
                panic!("post-bootstrap optimistic keystroke must produce CrdtOp; got {other:?}")
            }
        }

        // Mirror state reflects the optimistic apply.
        assert_eq!(m.materialize(id).as_deref(), Some("abcj"));
        assert_eq!(m.cursor_byte_pos(id), Some(4));
    }

    /// The canonical Day 3 echo-dedup test: a local op echoed back
    /// from the daemon (tagged with our `FrontendId`) must be skipped;
    /// a remote op (tagged with a different `FrontendId`) must apply.
    ///
    /// Symmetric coverage — single test exercises both filter
    /// directions so a one-sided filter bug (filters everything OR
    /// filters nothing) fails the test regardless of which way it
    /// breaks.
    #[test]
    fn echo_dedup_skips_own_op_but_applies_remote() {
        // Frontend A's FrontendId is 2; peer_id_from_frontend(2) == 2.
        let local_id = FrontendId(2);
        let remote_id = FrontendId(7);
        let buffer_id = BufferId::next();

        // Bootstrap: A's mirror initializes from a snapshot containing
        // "abc" (the daemon-side initial state).
        let snap = fresh_snapshot(0xABCD, "abc");
        let mut mirror = BufferMirror::new(local_id);
        mirror.init_from_snapshot(buffer_id, &snap).expect("init");
        assert_eq!(mirror.materialize(buffer_id).as_deref(), Some("abc"));

        // A types 'X' — apply_local_insert produces wire-format op
        // bytes attributable to A's peer_id.
        let local_op_bytes = mirror
            .apply_local_insert(buffer_id, 3, "X")
            .expect("local insert");
        assert_eq!(mirror.materialize(buffer_id).as_deref(), Some("abcX"));

        // Echo arrives: daemon broadcasts A's op back to A (tagged
        // with A's FrontendId). The filter must skip it.
        let outcome = apply_incoming_crdt_op(
            &mut mirror,
            local_id,
            local_id, // source == local → echo
            buffer_id,
            &local_op_bytes,
        )
        .expect("echo filter");
        assert_eq!(outcome, IncomingCrdtOpOutcome::SkippedEcho);

        // After echo: mirror unchanged (op was NOT applied a second
        // time). If the filter were broken and the op double-applied,
        // we'd see "abcXX" here.
        assert_eq!(
            mirror.materialize(buffer_id).as_deref(),
            Some("abcX"),
            "echoed own op must not double-apply"
        );

        // Now a remote edit from B (a different FrontendId) arrives.
        // Build a B-side replica that has integrated A's op + B's
        // own edit; we'll deliver B's op-since-A to A's mirror.
        let b_state = CrdtState::new(0xBEEF).expect("b new");
        b_state.import_snapshot(&snap).expect("b bootstrap");
        b_state
            .import_updates(&local_op_bytes)
            .expect("b sees A's op");
        let v_before_b_edit = b_state.version();
        b_state.insert(0, "Z").expect("b insert");
        let remote_op_bytes = b_state
            .export_updates_since(&v_before_b_edit)
            .expect("b op");

        let outcome = apply_incoming_crdt_op(
            &mut mirror,
            local_id,
            remote_id, // source != local → real remote op
            buffer_id,
            &remote_op_bytes,
        )
        .expect("remote apply");
        assert_eq!(outcome, IncomingCrdtOpOutcome::Applied);

        // After remote apply: B's "Z" prepended to A's mirror content.
        assert_eq!(
            mirror.materialize(buffer_id).as_deref(),
            Some("ZabcX"),
            "remote op must apply when source != local"
        );
    }

    /// **F1 gap pin.** The manual checklist originally told operators
    /// to undo with `C-x u`. That is the *wrong* keystroke for the
    /// per-frontend optimistic-undo path Scenario 2 tests: only the
    /// single-key forms (`Ctrl-4`, and — under Kitty enhanced mode —
    /// `Ctrl-/` / `Ctrl-_`) classify as `OptimisticAction::Undo`
    /// (frontend per-peer undo). `C-x` is a multi-key prefix the
    /// optimistic layer has no state for; it classifies `RoundTrip`
    /// and the sequence `C-x u` round-trips to the *daemon's* undo,
    /// which operates on the daemon's CRDT peer and cannot isolate a
    /// single frontend's edits.
    ///
    /// This pins the gap as a tested invariant rather than prose:
    /// if a future change made `C-x` optimistic, or de-classified
    /// `Ctrl-4`, this fails — and the checklist's `Ctrl-4`
    /// instruction (F1 fix) would silently become wrong again.
    #[test]
    fn f1_undo_keystroke_gap_cx_u_round_trips_only_single_key_is_optimistic() {
        // The keystroke the manual checklist (post-F1) and the PTY
        // test both use — reaches frontend per-peer undo.
        assert_eq!(
            classify_key(Key::Char('4'), Modifiers::CTRL),
            OptimisticAction::Undo,
            "Ctrl-4 must be the frontend per-peer optimistic undo \
             (raw-terminal-deliverable; what the checklist now uses)"
        );
        // Kitty-enhanced-mode forms — also optimistic undo (only
        // delivered when Kitty negotiation lands; v0.2).
        assert_eq!(
            classify_key(Key::Char('/'), Modifiers::CTRL),
            OptimisticAction::Undo
        );
        assert_eq!(
            classify_key(Key::Char('_'), Modifiers::CTRL),
            OptimisticAction::Undo
        );
        // `C-x` — the prefix of the OLD (wrong) checklist instruction
        // `C-x u`. Round-trips; the optimistic layer has no multi-key
        // prefix state, so `C-x u` can NEVER compose to frontend
        // per-peer undo — it reaches daemon undo, which Scenario 2's
        // per-frontend-isolation claim is not about.
        assert_eq!(
            classify_key(Key::Char('x'), Modifiers::CTRL),
            OptimisticAction::RoundTrip,
            "C-x must round-trip — it's the daemon-undo prefix, NOT \
             frontend per-peer undo; this is why the checklist had \
             to switch from C-x u to Ctrl-4 (F1)"
        );
        // The lone `u` after `C-x`, seen in isolation by the
        // stateless optimistic layer, is just text — confirming no
        // prefix-composition path to undo exists.
        assert_eq!(
            classify_key(Key::Char('u'), Modifiers::NONE),
            OptimisticAction::Insert('u'),
            "no multi-key prefix state: the 'u' in C-x u is plain \
             text to the optimistic layer; C-x u cannot be \
             frontend-undo by construction"
        );
    }
}
