// keymap_stack.rs --- Scoped keymaps + per-key resolver.

//! Scope ordering and the per-key dispatcher state machine.
//!
//! The Pmacs editor maintains three categories of keymaps:
//!
//! 1. **Buffer-local**: bindings that apply only when a specific
//!    buffer is the active one. Most-specific scope.
//! 2. **Mode**: bindings that apply when a mode is active. Modes
//!    aren't a real concept until T M2.5+ but the stack accepts them
//!    today so the resolver doesn't grow a dimension when we add them.
//! 3. **Global**: the universal fallback. Last-resort scope.
//!
//! [`KeymapStack::resolve`] walks them in order and returns the
//! most-specific [`Resolution`] for the given chord sequence. Scopes
//! that are partial-match (Pending) cooperate: if buffer-local is
//! Unbound but global has a Pending prefix, the resolver returns
//! Pending so the dispatcher waits for more chords.
//!
//! [`KeyDispatcher`] carries the in-flight prefix between key events.
//! Dispatch returns one of [`Action::Run`] (a complete binding fired),
//! [`Action::Pending`] (more keys expected), or [`Action::Unbound`]
//! (the sequence dead-ended; reset).

use std::collections::HashMap;

use crate::buffer::BufferId;
use crate::key::{Chord, Sequence, display_sequence};
use crate::keymap_tree::{Binding, Keymap, KeymapError, Resolution};

// ---------------------------------------------------------------------------
// Scope identifier
// ---------------------------------------------------------------------------

/// The scope a binding lives in. Reported by `pmacs.describe.key` so
/// users can see *why* their key resolved the way it did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scope {
    /// Buffer-local --- specific to one [`BufferId`].
    Buffer(BufferId),
    /// Mode-active --- one of the active mode keymaps.
    Mode(String),
    /// The global fallback.
    Global,
}

impl Scope {
    /// Render as a stable user-facing string.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Buffer(_) => "buffer".to_owned(),
            Self::Mode(name) => format!("mode:{name}"),
            Self::Global => "global".to_owned(),
        }
    }
}

// ---------------------------------------------------------------------------
// Resolved binding (binding + which scope)
// ---------------------------------------------------------------------------

/// A complete resolution: the binding plus the scope that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedBinding {
    /// The binding metadata (command + source).
    pub binding: Binding,
    /// Which scope held the binding.
    pub scope: Scope,
}

/// Result of [`KeymapStack::resolve`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StackResolution {
    /// At least one scope returned a complete binding; the
    /// most-specific one wins.
    Bound(ResolvedBinding),
    /// No scope had a complete match, but at least one returned
    /// [`Resolution::Pending`] for this prefix.
    Pending,
    /// No scope has any match (complete or partial).
    Unbound,
}

// ---------------------------------------------------------------------------
// KeymapStack
// ---------------------------------------------------------------------------

/// The complete keymap state for an editor session.
#[derive(Default)]
pub struct KeymapStack {
    /// The global keymap (always consulted last).
    pub global: Keymap,
    /// Per-mode keymaps. Mode activation order is preserved by `Vec`;
    /// the resolver consults them after buffer-local but before
    /// global. The top of the vector is the most recently activated
    /// mode and wins ties.
    pub modes: Vec<(String, Keymap)>,
    /// Per-buffer keymaps. Buffer-local always beats mode and global.
    pub buffers: HashMap<BufferId, Keymap>,
}

impl KeymapStack {
    /// An empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind into the global keymap.
    ///
    /// # Errors
    ///
    /// Surfaces [`KeymapError`]s from the underlying [`Keymap::bind`].
    pub fn bind_global(
        &mut self,
        sequence: &[Chord],
        command: impl Into<String>,
        source: crate::command::SourceLocation,
    ) -> Result<(), KeymapError> {
        self.global.bind(sequence, command, source)
    }

    /// Bind into a buffer-local keymap, creating it if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Surfaces [`KeymapError`]s from the underlying [`Keymap::bind`].
    pub fn bind_buffer(
        &mut self,
        buffer: BufferId,
        sequence: &[Chord],
        command: impl Into<String>,
        source: crate::command::SourceLocation,
    ) -> Result<(), KeymapError> {
        self.buffers
            .entry(buffer)
            .or_default()
            .bind(sequence, command, source)
    }

    /// Bind into a mode keymap, creating the mode entry if needed.
    ///
    /// # Errors
    ///
    /// Surfaces [`KeymapError`]s from the underlying [`Keymap::bind`].
    pub fn bind_mode(
        &mut self,
        mode: &str,
        sequence: &[Chord],
        command: impl Into<String>,
        source: crate::command::SourceLocation,
    ) -> Result<(), KeymapError> {
        if !self.modes.iter().any(|(n, _)| n == mode) {
            self.modes.push((mode.to_owned(), Keymap::default()));
        }
        let map = &mut self
            .modes
            .iter_mut()
            .find(|(n, _)| n == mode)
            .expect("just ensured present")
            .1;
        map.bind(sequence, command, source)
    }

    /// Unbind from the global keymap.
    ///
    /// # Errors
    ///
    /// Surfaces [`KeymapError::NotBound`] if the sequence wasn't
    /// bound globally.
    pub fn unbind_global(&mut self, sequence: &[Chord]) -> Result<Binding, KeymapError> {
        self.global.unbind(sequence)
    }

    /// Unbind a buffer-local sequence.
    ///
    /// # Errors
    ///
    /// Surfaces [`KeymapError::NotBound`] if the sequence isn't bound
    /// in the buffer's local map (or if the buffer has no local map).
    pub fn unbind_buffer(
        &mut self,
        buffer: BufferId,
        sequence: &[Chord],
    ) -> Result<Binding, KeymapError> {
        let map = self
            .buffers
            .get_mut(&buffer)
            .ok_or_else(|| KeymapError::NotBound {
                sequence: display_sequence(sequence),
            })?;
        let removed = map.unbind(sequence)?;
        if map.is_empty() {
            self.buffers.remove(&buffer);
        }
        Ok(removed)
    }

    /// Drop every buffer-local binding for a buffer that just left
    /// the registry.
    pub fn remove_buffer(&mut self, buffer: BufferId) -> bool {
        self.buffers.remove(&buffer).is_some()
    }

    /// Unbind a sequence from a mode keymap.
    ///
    /// # Errors
    ///
    /// Surfaces [`KeymapError::NotBound`] if the mode doesn't exist
    /// or the sequence isn't bound there.
    pub fn unbind_mode(&mut self, mode: &str, sequence: &[Chord]) -> Result<Binding, KeymapError> {
        let idx = self
            .modes
            .iter()
            .position(|(n, _)| n == mode)
            .ok_or_else(|| KeymapError::NotBound {
                sequence: display_sequence(sequence),
            })?;
        let removed = self.modes[idx].1.unbind(sequence)?;
        if self.modes[idx].1.is_empty() {
            self.modes.remove(idx);
        }
        Ok(removed)
    }

    /// Resolve `sequence` in scope priority order.
    ///
    /// `active_buffer` is the [`BufferId`] currently in focus (if any).
    /// `active_modes` lists the active mode names in
    /// most-recent-first order; the first match in that order wins
    /// among modes.
    ///
    /// Resolution semantics: the resolver returns the *most-specific*
    /// complete binding it finds. If no scope has a complete match
    /// but at least one has a partial (Pending) match, returns
    /// [`StackResolution::Pending`].
    #[must_use]
    pub fn resolve(
        &self,
        sequence: &[Chord],
        active_buffer: Option<BufferId>,
        active_modes: &[String],
    ) -> StackResolution {
        let mut any_pending = false;

        // 1) Buffer-local --- highest priority.
        if let Some(id) = active_buffer {
            if let Some(map) = self.buffers.get(&id) {
                match map.lookup(sequence) {
                    Resolution::Bound(b) => {
                        return StackResolution::Bound(ResolvedBinding {
                            binding: b,
                            scope: Scope::Buffer(id),
                        });
                    }
                    Resolution::Pending => any_pending = true,
                    Resolution::Unbound => {}
                }
            }
        }

        // 2) Modes --- ordered by `active_modes`.
        for mode_name in active_modes {
            if let Some((_, map)) = self.modes.iter().find(|(n, _)| n == mode_name) {
                match map.lookup(sequence) {
                    Resolution::Bound(b) => {
                        return StackResolution::Bound(ResolvedBinding {
                            binding: b,
                            scope: Scope::Mode(mode_name.clone()),
                        });
                    }
                    Resolution::Pending => any_pending = true,
                    Resolution::Unbound => {}
                }
            }
        }

        // 3) Global --- fallback.
        match self.global.lookup(sequence) {
            Resolution::Bound(b) => StackResolution::Bound(ResolvedBinding {
                binding: b,
                scope: Scope::Global,
            }),
            Resolution::Pending => StackResolution::Pending,
            Resolution::Unbound => {
                if any_pending {
                    StackResolution::Pending
                } else {
                    StackResolution::Unbound
                }
            }
        }
    }

    /// Iterate over every (scope, sequence, binding) triple. Used by
    /// `pmacs.describe.command` to populate `key_bindings` and by
    /// `pmacs.keymap.list`.
    pub fn iter_all(&self) -> Vec<(Scope, Sequence, Binding)> {
        let mut out = Vec::new();
        for (id, map) in &self.buffers {
            for (seq, b) in map.iter() {
                out.push((Scope::Buffer(*id), seq, b));
            }
        }
        for (name, map) in &self.modes {
            for (seq, b) in map.iter() {
                out.push((Scope::Mode(name.clone()), seq, b));
            }
        }
        for (seq, b) in self.global.iter() {
            out.push((Scope::Global, seq, b));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// KeyDispatcher: the multi-key state machine
// ---------------------------------------------------------------------------

/// What the dispatcher decided to do with the latest key.
#[derive(Clone, Debug)]
pub enum Action {
    /// Run a command. The full sequence that fired is included for
    /// `describe-key`-style logging.
    Run {
        /// The command name to invoke.
        command: String,
        /// The complete sequence that fired (just-typed chords).
        sequence: Sequence,
        /// Where the binding came from.
        scope: Scope,
    },
    /// More keys expected; the dispatcher is waiting on a longer
    /// prefix. Includes a copy of the sequence so the UI can echo it
    /// (e.g. "C-x -" in the modeline).
    Pending {
        /// Chord sequence accumulated so far.
        sequence: Sequence,
    },
    /// No binding for this sequence. The accumulated prefix is
    /// returned so callers can report it; the dispatcher's internal
    /// state is now reset.
    Unbound {
        /// The dead-end sequence.
        sequence: Sequence,
    },
}

/// Per-session multi-key dispatch state machine.
#[derive(Default)]
pub struct KeyDispatcher {
    pending: Sequence,
}

impl KeyDispatcher {
    /// An empty dispatcher with no pending prefix.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently-pending prefix (immutable view).
    #[must_use]
    pub fn pending(&self) -> &[Chord] {
        &self.pending
    }

    /// Reset the dispatcher's pending prefix.
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    /// Consume one chord and return the resulting [`Action`].
    pub fn dispatch(
        &mut self,
        chord: Chord,
        stack: &KeymapStack,
        active_buffer: Option<BufferId>,
        active_modes: &[String],
    ) -> Action {
        self.pending.push(chord);
        match stack.resolve(&self.pending, active_buffer, active_modes) {
            StackResolution::Bound(rb) => {
                let sequence = std::mem::take(&mut self.pending);
                Action::Run {
                    command: rb.binding.command,
                    sequence,
                    scope: rb.scope,
                }
            }
            StackResolution::Pending => Action::Pending {
                sequence: self.pending.clone(),
            },
            StackResolution::Unbound => {
                let sequence = std::mem::take(&mut self.pending);
                Action::Unbound { sequence }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::SourceLocation;
    use crate::key::parse_sequence;

    fn src(line: i32) -> SourceLocation {
        SourceLocation {
            file: "test.lua".into(),
            line,
        }
    }

    fn seq(s: &str) -> Sequence {
        parse_sequence(s).unwrap()
    }

    fn buf() -> BufferId {
        BufferId::next()
    }

    #[test]
    fn global_resolves() {
        let mut s = KeymapStack::new();
        s.bind_global(&seq("C-s"), "save", src(1)).unwrap();
        let r = s.resolve(&seq("C-s"), None, &[]);
        match r {
            StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "save");
                assert_eq!(rb.scope, Scope::Global);
            }
            other => panic!("expected Bound, got {other:?}"),
        }
    }

    #[test]
    fn buffer_local_overrides_global() {
        let mut s = KeymapStack::new();
        let id = buf();
        s.bind_global(&seq("C-s"), "global.save", src(1)).unwrap();
        s.bind_buffer(id, &seq("C-s"), "buffer.save", src(2))
            .unwrap();
        match s.resolve(&seq("C-s"), Some(id), &[]) {
            StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "buffer.save");
                assert_eq!(rb.scope, Scope::Buffer(id));
            }
            other => panic!("expected buffer-local Bound, got {other:?}"),
        }
        // Different buffer: falls through to global.
        match s.resolve(&seq("C-s"), Some(buf()), &[]) {
            StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "global.save");
                assert_eq!(rb.scope, Scope::Global);
            }
            other => panic!("expected Global Bound, got {other:?}"),
        }
    }

    #[test]
    fn mode_overrides_global_but_not_buffer() {
        let mut s = KeymapStack::new();
        let id = buf();
        s.bind_global(&seq("C-s"), "global.save", src(1)).unwrap();
        s.bind_mode("normal", &seq("C-s"), "mode.save", src(2))
            .unwrap();
        s.bind_buffer(id, &seq("C-s"), "buffer.save", src(3))
            .unwrap();
        // Buffer wins.
        match s.resolve(&seq("C-s"), Some(id), &["normal".into()]) {
            StackResolution::Bound(rb) => assert_eq!(rb.binding.command, "buffer.save"),
            other => panic!("got {other:?}"),
        }
        // No buffer: mode wins.
        match s.resolve(&seq("C-s"), None, &["normal".into()]) {
            StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "mode.save");
                assert_eq!(rb.scope, Scope::Mode("normal".into()));
            }
            other => panic!("got {other:?}"),
        }
        // No mode: global wins.
        match s.resolve(&seq("C-s"), None, &[]) {
            StackResolution::Bound(rb) => assert_eq!(rb.binding.command, "global.save"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn pending_in_global_yields_pending_overall() {
        let mut s = KeymapStack::new();
        s.bind_global(&seq("C-x C-s"), "save", src(1)).unwrap();
        assert_eq!(s.resolve(&seq("C-x"), None, &[]), StackResolution::Pending);
    }

    #[test]
    fn unbound_when_no_scope_matches() {
        let s = KeymapStack::new();
        assert_eq!(s.resolve(&seq("C-q"), None, &[]), StackResolution::Unbound);
    }

    #[test]
    fn pending_in_one_scope_propagates_when_others_unbound() {
        // If buffer-local has a pending prefix and global has nothing,
        // the overall result is Pending.
        let mut s = KeymapStack::new();
        let id = buf();
        s.bind_buffer(id, &seq("C-x C-s"), "buf.save", src(1))
            .unwrap();
        assert_eq!(
            s.resolve(&seq("C-x"), Some(id), &[]),
            StackResolution::Pending
        );
    }

    #[test]
    fn dispatcher_state_machine_runs_on_complete_sequence() {
        let mut s = KeymapStack::new();
        s.bind_global(&seq("C-x C-s"), "save", src(1)).unwrap();
        let mut d = KeyDispatcher::new();
        let chords = seq("C-x C-s");

        // First chord: pending.
        match d.dispatch(chords[0], &s, None, &[]) {
            Action::Pending { sequence } => assert_eq!(sequence.len(), 1),
            other => panic!("expected Pending, got {other:?}"),
        }
        // Second chord: fires the command.
        match d.dispatch(chords[1], &s, None, &[]) {
            Action::Run {
                command, sequence, ..
            } => {
                assert_eq!(command, "save");
                assert_eq!(sequence.len(), 2);
            }
            other => panic!("expected Run, got {other:?}"),
        }
        // Pending is reset.
        assert!(d.pending().is_empty());
    }

    #[test]
    fn dispatcher_resets_on_unbound_sequence() {
        let mut s = KeymapStack::new();
        s.bind_global(&seq("C-x C-s"), "save", src(1)).unwrap();
        let mut d = KeyDispatcher::new();
        let cx = parse_sequence("C-x").unwrap()[0];
        let cq = parse_sequence("C-q").unwrap()[0];

        let _ = d.dispatch(cx, &s, None, &[]);
        match d.dispatch(cq, &s, None, &[]) {
            Action::Unbound { sequence } => {
                assert_eq!(sequence.len(), 2);
                assert_eq!(sequence[0], cx);
                assert_eq!(sequence[1], cq);
            }
            other => panic!("expected Unbound, got {other:?}"),
        }
        assert!(d.pending().is_empty());
    }

    #[test]
    fn unbind_buffer_prunes_empty_map() {
        let mut s = KeymapStack::new();
        let id = buf();
        s.bind_buffer(id, &seq("C-s"), "save", src(1)).unwrap();
        s.unbind_buffer(id, &seq("C-s")).unwrap();
        assert!(!s.buffers.contains_key(&id));
    }

    #[test]
    fn unbind_mode_prunes_empty_map() {
        let mut s = KeymapStack::new();
        s.bind_mode("normal", &seq("C-s"), "save", src(1)).unwrap();
        s.unbind_mode("normal", &seq("C-s")).unwrap();
        assert!(!s.modes.iter().any(|(n, _)| n == "normal"));
    }

    #[test]
    fn iter_all_visits_every_binding_with_scope() {
        let mut s = KeymapStack::new();
        let id = buf();
        s.bind_global(&seq("C-q"), "quit", src(1)).unwrap();
        s.bind_mode("normal", &seq("C-s"), "save", src(2)).unwrap();
        s.bind_buffer(id, &seq("C-z"), "undo", src(3)).unwrap();
        let mut got: Vec<String> = s
            .iter_all()
            .into_iter()
            .map(|(scope, seq, b)| {
                format!(
                    "{}:{} -> {}",
                    scope.render(),
                    display_sequence(&seq),
                    b.command
                )
            })
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                "buffer:C-z -> undo",
                "global:C-q -> quit",
                "mode:normal:C-s -> save",
            ]
        );
    }

    // ---- proptest: dispatcher invariants -----------------------------------
    //
    // The auto-repeat regression we shipped earlier was a chord-event
    // edge case the unit tests didn't cover. These properties throw
    // random chord soup at the dispatcher under a plausible keymap
    // and check the universal invariants the dispatcher promises.

    use crossterm::event::{KeyCode, KeyModifiers};
    use proptest::prelude::*;

    fn small_chord_strategy() -> impl Strategy<Value = Chord> {
        // Restrict to a small alphabet so prefix interactions actually
        // collide instead of every chord being unique. ASCII letter +
        // optional Control modifier is the realistic dispatcher input.
        (any::<u8>(), any::<bool>()).prop_map(|(b, ctrl)| {
            let ch = (b'a' + (b % 8)) as char;
            let modifiers = if ctrl {
                KeyModifiers::CONTROL
            } else {
                KeyModifiers::NONE
            };
            Chord::new(KeyCode::Char(ch), modifiers)
        })
    }

    fn small_keymap() -> KeymapStack {
        // A keymap with both single-chord bindings and multi-chord
        // prefixes, so dispatching random sequences exercises every
        // arm of the resolver.
        let mut s = KeymapStack::new();
        s.bind_global(&seq("C-a"), "go-line-start", src(1)).unwrap();
        s.bind_global(&seq("C-e"), "go-line-end", src(2)).unwrap();
        s.bind_global(&seq("C-x C-s"), "save", src(3)).unwrap();
        s.bind_global(&seq("C-x C-c"), "quit", src(4)).unwrap();
        s.bind_global(&seq("C-x b"), "switch-buffer", src(5))
            .unwrap();
        s.bind_global(&seq("C-c C-c"), "compile", src(6)).unwrap();
        s
    }

    proptest! {
        /// Universal invariant: after each dispatch, `pending` is
        /// either empty (the last action was Run or Unbound) or
        /// non-empty (the action was Pending). Never panics.
        #[test]
        fn pending_state_matches_last_action(chords in proptest::collection::vec(small_chord_strategy(), 1..30)) {
            let stack = small_keymap();
            let mut d = KeyDispatcher::new();
            for chord in chords {
                let action = d.dispatch(chord, &stack, None, &[]);
                match action {
                    Action::Run { .. } | Action::Unbound { .. } => {
                        prop_assert!(d.pending().is_empty(), "pending should be empty after Run/Unbound");
                    }
                    Action::Pending { ref sequence } => {
                        prop_assert!(!d.pending().is_empty(), "pending must be non-empty after Pending");
                        prop_assert_eq!(d.pending(), sequence.as_slice(), "echoed sequence must match dispatcher pending");
                    }
                }
            }
        }

        /// A bound single-chord sequence always fires on first
        /// dispatch when nothing else is pending.
        #[test]
        fn single_chord_binding_fires_immediately(_unused in 0..10u8) {
            let stack = small_keymap();
            let mut d = KeyDispatcher::new();
            let chord = seq("C-a")[0];
            let action = d.dispatch(chord, &stack, None, &[]);
            match action {
                Action::Run { command, .. } => prop_assert_eq!(command, "go-line-start".to_string()),
                other => prop_assert!(false, "expected Run, got {:?}", other),
            }
            prop_assert!(d.pending().is_empty());
        }

        /// Pending depth never exceeds the longest binding's length.
        /// This caught the auto-repeat regression in spirit:
        /// pending should not grow without bound.
        #[test]
        fn pending_never_exceeds_max_binding_length(chords in proptest::collection::vec(small_chord_strategy(), 1..30)) {
            // Longest binding in `small_keymap` is 2 chords.
            const MAX_BINDING_LEN: usize = 2;
            let stack = small_keymap();
            let mut d = KeyDispatcher::new();
            for chord in chords {
                d.dispatch(chord, &stack, None, &[]);
                prop_assert!(
                    d.pending().len() <= MAX_BINDING_LEN,
                    "pending depth {} exceeded max binding length {MAX_BINDING_LEN}",
                    d.pending().len()
                );
            }
        }

        /// Forming a known multi-chord binding by typing each chord
        /// in sequence resolves to that binding on the final chord.
        #[test]
        fn typing_a_bound_sequence_produces_run(_unused in 0..10u8) {
            let stack = small_keymap();
            let target = seq("C-x C-s");
            let mut d = KeyDispatcher::new();
            // First chord: Pending (C-x is a known prefix).
            let a = d.dispatch(target[0], &stack, None, &[]);
            let is_pending = matches!(a, Action::Pending { .. });
            prop_assert!(is_pending, "C-x should be Pending");
            // Second chord: Run "save".
            let a = d.dispatch(target[1], &stack, None, &[]);
            match a {
                Action::Run { command, .. } => prop_assert_eq!(command, "save".to_string()),
                _ => prop_assert!(false, "expected Run save"),
            }
            prop_assert!(d.pending().is_empty());
        }

        /// Unrelated chord while a prefix is pending: the dispatcher
        /// reports Unbound for the accumulated sequence and resets
        /// state. This is what saves the user from a stuck prefix.
        #[test]
        fn unrelated_chord_after_prefix_reports_unbound_and_resets(byte in 0u8..6) {
            let stack = small_keymap();
            let mut d = KeyDispatcher::new();
            // Start a known prefix.
            let pending = d.dispatch(seq("C-x")[0], &stack, None, &[]);
            let is_pending = matches!(pending, Action::Pending { .. });
            prop_assert!(is_pending);
            // A chord that doesn't extend C-x: pick `q` (bound nowhere
            // in the keymap, including not after C-x).
            let unbound_chord = Chord::new(KeyCode::Char((b'q' + byte) as char), KeyModifiers::NONE);
            let action = d.dispatch(unbound_chord, &stack, None, &[]);
            let is_unbound = matches!(action, Action::Unbound { .. });
            prop_assert!(is_unbound, "expected Unbound");
            prop_assert!(d.pending().is_empty());
        }
    }
}
