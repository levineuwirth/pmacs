// keymap_tree.rs --- Keymap trie with bind/unbind and conflict detection.

//! A keymap is a trie of [`Chord`]s.
//!
//! Each non-terminal node holds a `HashMap<Chord, Branch>` where each
//! branch is either a [`Branch::Leaf`] (a complete binding to a
//! command) or a [`Branch::Submap`] (a prefix that must be followed by
//! more chords to reach a binding).
//!
//! # Conflict detection (T M2.4 acceptance)
//!
//! Every [`Keymap::bind`] runs through `prepare_bind`, which walks the
//! existing tree along the new sequence's prefix:
//!
//! * If a sequence's prefix already terminates as a leaf, binding the
//!   longer sequence would require turning that leaf into a submap.
//!   We refuse with [`KeymapError::WouldExtendLeaf`].
//! * If the sequence itself is shorter than (or terminates inside) an
//!   existing submap path, binding it would shadow already-bound
//!   suffixes. We refuse with [`KeymapError::WouldShadowSubmap`].
//! * If the leaf already exists (same exact sequence), we refuse with
//!   [`KeymapError::DuplicateBinding`] rather than silently overwriting.
//!
//! [`Keymap::lookup`] walks the same tree and returns
//! [`Resolution::Bound`] (complete), [`Resolution::Pending`] (a known
//! prefix), or [`Resolution::Unbound`] (no match).

use std::collections::HashMap;

use thiserror::Error;

use crate::command::SourceLocation;
use crate::key::{Chord, Sequence, display_sequence};

// ---------------------------------------------------------------------------
// Binding metadata + branches
// ---------------------------------------------------------------------------

/// Metadata for a single bound key sequence.
///
/// Cheap to clone --- just a few `String`s and a `SourceLocation`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    /// Name of the command this sequence resolves to. Resolution at
    /// dispatch time looks the name up in the [`crate::command::CommandRegistry`].
    pub command: String,
    /// File + line where the bind was registered. Reported by
    /// `pmacs.describe.key`.
    pub source: SourceLocation,
}

/// A node in the keymap trie: either a complete binding or a submap.
#[derive(Clone, Debug)]
enum Branch {
    Leaf(Binding),
    Submap(Box<Keymap>),
}

// ---------------------------------------------------------------------------
// Keymap (trie)
// ---------------------------------------------------------------------------

/// A keymap: a trie keyed by [`Chord`]s.
///
/// Construct via [`Self::new`] / [`Self::default`]. Bind via
/// [`Self::bind`]. Walk a sequence with [`Self::lookup`]. Iterate
/// every (sequence, binding) pair via [`Self::iter`].
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    branches: HashMap<Chord, Branch>,
}

/// Errors raised by [`Keymap::bind`] and [`Keymap::unbind`].
#[derive(Debug, Error, Eq, PartialEq)]
pub enum KeymapError {
    /// The exact sequence is already bound. Refuse rather than silently
    /// overwrite (matches the command registry's policy).
    #[error("key sequence `{sequence}` is already bound to command \"{existing}\"")]
    DuplicateBinding {
        /// The conflicting sequence (canonical display form).
        sequence: String,
        /// The currently-bound command name.
        existing: String,
    },

    /// Binding would require turning an existing leaf into a submap
    /// (e.g. binding `C-x f` when `C-x` is already a complete binding).
    #[error(
        "binding `{sequence}` to \"{command}\" would extend leaf `{existing_sequence}` (bound to \"{existing}\")"
    )]
    WouldExtendLeaf {
        /// The new sequence the user tried to bind.
        sequence: String,
        /// The new command name.
        command: String,
        /// The shorter prefix sequence that's already a leaf.
        existing_sequence: String,
        /// The command currently bound to that leaf.
        existing: String,
    },

    /// Binding the prefix would shadow already-bound suffixes
    /// (e.g. binding `C-x` when `C-x f` is already a complete binding).
    #[error(
        "binding `{sequence}` to \"{command}\" would shadow existing prefix submap (next-key options exist)"
    )]
    WouldShadowSubmap {
        /// The new sequence the user tried to bind.
        sequence: String,
        /// The new command name.
        command: String,
    },

    /// Unbind targeted a sequence that's not currently bound.
    #[error("key sequence `{sequence}` is not bound")]
    NotBound {
        /// The sequence that didn't resolve.
        sequence: String,
    },

    /// An empty sequence was given to bind/unbind/lookup.
    #[error("key sequence is empty")]
    EmptySequence,
}

/// Result of walking a sequence through a keymap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resolution {
    /// The full sequence resolves to a binding.
    Bound(Binding),
    /// The sequence is a known prefix; more chords may complete it.
    Pending,
    /// No binding under this sequence.
    Unbound,
}

impl Keymap {
    /// An empty keymap.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `sequence` to `command` with the given source location.
    ///
    /// Conflict detection is strict (see module docs). The keymap is
    /// only mutated on success: every error path leaves the trie
    /// untouched.
    ///
    /// # Errors
    ///
    /// Returns a [`KeymapError`] for empty sequences, duplicates, or
    /// any of the leaf/submap shadowing cases.
    pub fn bind(
        &mut self,
        sequence: &[Chord],
        command: impl Into<String>,
        source: SourceLocation,
    ) -> Result<(), KeymapError> {
        if sequence.is_empty() {
            return Err(KeymapError::EmptySequence);
        }
        let command = command.into();
        // Pre-check the whole path so we never partially mutate on error.
        self.check_bind_path(sequence, &command)?;
        // Apply.
        bind_recursive(self, sequence, command, source);
        Ok(())
    }

    /// Remove the binding at `sequence`. If removing it leaves an
    /// empty submap, the parent's entry is also pruned.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError::NotBound`] if `sequence` doesn't
    /// resolve to a leaf.
    pub fn unbind(&mut self, sequence: &[Chord]) -> Result<Binding, KeymapError> {
        if sequence.is_empty() {
            return Err(KeymapError::EmptySequence);
        }
        unbind_recursive(self, sequence).ok_or_else(|| KeymapError::NotBound {
            sequence: display_sequence(sequence),
        })
    }

    /// Walk `sequence` through the trie.
    #[must_use]
    pub fn lookup(&self, sequence: &[Chord]) -> Resolution {
        if sequence.is_empty() {
            return Resolution::Unbound;
        }
        let (head, tail) = sequence.split_first().unwrap();
        // The four arms with distinct intent are clearer than the
        // collapsed two-arm form clippy prefers; the duplicated
        // `Resolution::Unbound` body is intentional.
        #[allow(
            clippy::match_same_arms,
            reason = "two distinct cases happen to share a Resolution"
        )]
        match self.branches.get(head) {
            None => Resolution::Unbound,
            Some(Branch::Leaf(b)) if tail.is_empty() => Resolution::Bound(b.clone()),
            Some(Branch::Leaf(_)) => Resolution::Unbound, // sequence longer than the leaf
            Some(Branch::Submap(_)) if tail.is_empty() => Resolution::Pending,
            Some(Branch::Submap(sub)) => sub.lookup(tail),
        }
    }

    /// Iterate over every `(sequence, binding)` pair in this keymap.
    /// Order is unspecified (the underlying `HashMap` is unordered);
    /// callers that need a stable display sort the result themselves.
    pub fn iter(&self) -> impl Iterator<Item = (Sequence, Binding)> + '_ {
        let mut out: Vec<(Sequence, Binding)> = Vec::new();
        collect_pairs(self, &mut Vec::new(), &mut out);
        out.into_iter()
    }

    /// Number of distinct bindings in this map (including suffixes of
    /// any submaps).
    #[must_use]
    pub fn len(&self) -> usize {
        self.branches
            .values()
            .map(|b| match b {
                Branch::Leaf(_) => 1,
                Branch::Submap(sub) => sub.len(),
            })
            .sum()
    }

    /// True iff no bindings are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    fn check_bind_path(&self, sequence: &[Chord], command: &str) -> Result<(), KeymapError> {
        let (head, tail) = sequence.split_first().expect("non-empty checked by caller");
        match self.branches.get(head) {
            None => Ok(()),
            Some(Branch::Leaf(existing)) if tail.is_empty() => Err(KeymapError::DuplicateBinding {
                sequence: display_sequence(sequence),
                existing: existing.command.clone(),
            }),
            Some(Branch::Leaf(existing)) => Err(KeymapError::WouldExtendLeaf {
                sequence: display_sequence(sequence),
                command: command.to_owned(),
                existing_sequence: display_sequence(&[*head]),
                existing: existing.command.clone(),
            }),
            Some(Branch::Submap(_)) if tail.is_empty() => Err(KeymapError::WouldShadowSubmap {
                sequence: display_sequence(sequence),
                command: command.to_owned(),
            }),
            Some(Branch::Submap(sub)) => sub.check_bind_path(tail, command),
        }
    }
}

fn bind_recursive(map: &mut Keymap, sequence: &[Chord], command: String, source: SourceLocation) {
    let (head, tail) = sequence.split_first().expect("non-empty checked by caller");
    if tail.is_empty() {
        map.branches
            .insert(*head, Branch::Leaf(Binding { command, source }));
        return;
    }
    match map.branches.get_mut(head) {
        Some(Branch::Submap(sub)) => bind_recursive(sub, tail, command, source),
        None => {
            let mut sub = Box::new(Keymap::new());
            bind_recursive(&mut sub, tail, command, source);
            map.branches.insert(*head, Branch::Submap(sub));
        }
        Some(Branch::Leaf(_)) => {
            // Pre-check rejected this path; reachable only on bug.
            unreachable!("conflict pre-check missed a leaf at intermediate node");
        }
    }
}

fn unbind_recursive(map: &mut Keymap, sequence: &[Chord]) -> Option<Binding> {
    let (head, tail) = sequence.split_first()?;
    if tail.is_empty() {
        if let Some(Branch::Leaf(_)) = map.branches.get(head) {
            if let Some(Branch::Leaf(b)) = map.branches.remove(head) {
                return Some(b);
            }
        }
        return None;
    }
    let removed = match map.branches.get_mut(head) {
        Some(Branch::Submap(sub)) => unbind_recursive(sub, tail)?,
        _ => return None,
    };
    // Prune empty submaps so the tree doesn't grow stalactites.
    if let Some(Branch::Submap(sub)) = map.branches.get(head) {
        if sub.is_empty() {
            map.branches.remove(head);
        }
    }
    Some(removed)
}

fn collect_pairs(map: &Keymap, prefix: &mut Sequence, out: &mut Vec<(Sequence, Binding)>) {
    for (chord, branch) in &map.branches {
        prefix.push(*chord);
        match branch {
            Branch::Leaf(b) => out.push((prefix.clone(), b.clone())),
            Branch::Submap(sub) => collect_pairs(sub, prefix, out),
        }
        prefix.pop();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn bind_and_lookup_single_chord() {
        let mut m = Keymap::new();
        m.bind(&seq("C-s"), "buffer.save", src(1)).unwrap();
        match m.lookup(&seq("C-s")) {
            Resolution::Bound(b) => assert_eq!(b.command, "buffer.save"),
            other => panic!("expected Bound, got {other:?}"),
        }
    }

    #[test]
    fn bind_and_lookup_multi_chord() {
        let mut m = Keymap::new();
        m.bind(&seq("C-x C-s"), "buffer.save", src(1)).unwrap();
        // Prefix is pending.
        assert_eq!(m.lookup(&seq("C-x")), Resolution::Pending);
        // Full sequence is bound.
        match m.lookup(&seq("C-x C-s")) {
            Resolution::Bound(b) => assert_eq!(b.command, "buffer.save"),
            other => panic!("expected Bound, got {other:?}"),
        }
        // Unrelated chord is unbound.
        assert_eq!(m.lookup(&seq("C-y")), Resolution::Unbound);
    }

    #[test]
    fn duplicate_binding_is_rejected() {
        let mut m = Keymap::new();
        m.bind(&seq("C-s"), "buffer.save", src(1)).unwrap();
        let err = m.bind(&seq("C-s"), "buffer.write", src(2)).unwrap_err();
        match err {
            KeymapError::DuplicateBinding { existing, .. } => {
                assert_eq!(existing, "buffer.save");
            }
            other => panic!("expected DuplicateBinding, got {other:?}"),
        }
        // Original binding still in place.
        match m.lookup(&seq("C-s")) {
            Resolution::Bound(b) => assert_eq!(b.command, "buffer.save"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn would_extend_leaf_is_rejected() {
        // Bind C-x first as a complete binding, then try C-x f --- the
        // longer sequence would need C-x to become a submap.
        let mut m = Keymap::new();
        m.bind(&seq("C-x"), "scratch", src(1)).unwrap();
        let err = m.bind(&seq("C-x f"), "find_file", src(2)).unwrap_err();
        assert!(matches!(err, KeymapError::WouldExtendLeaf { .. }));
        // Original C-x still bound.
        assert!(matches!(m.lookup(&seq("C-x")), Resolution::Bound(_)));
    }

    #[test]
    fn would_shadow_submap_is_rejected() {
        // Bind C-x f first, then try to bind plain C-x --- the leaf
        // would shadow the existing submap (C-x f, etc).
        let mut m = Keymap::new();
        m.bind(&seq("C-x f"), "find_file", src(1)).unwrap();
        let err = m.bind(&seq("C-x"), "scratch", src(2)).unwrap_err();
        assert!(matches!(err, KeymapError::WouldShadowSubmap { .. }));
        // Submap still intact.
        assert_eq!(m.lookup(&seq("C-x")), Resolution::Pending);
        assert!(matches!(m.lookup(&seq("C-x f")), Resolution::Bound(_)));
    }

    #[test]
    fn empty_sequence_is_rejected() {
        let mut m = Keymap::new();
        assert_eq!(m.bind(&[], "x", src(1)), Err(KeymapError::EmptySequence));
        assert_eq!(m.unbind(&[]), Err(KeymapError::EmptySequence));
    }

    #[test]
    fn unbind_reverses_bind_and_prunes_empty_submaps() {
        let mut m = Keymap::new();
        m.bind(&seq("C-x C-s"), "save", src(1)).unwrap();
        m.bind(&seq("C-x C-f"), "find", src(2)).unwrap();
        assert_eq!(m.len(), 2);

        let b = m.unbind(&seq("C-x C-s")).unwrap();
        assert_eq!(b.command, "save");
        assert_eq!(m.len(), 1);

        // Removing the last suffix prunes the C-x submap entirely.
        let _ = m.unbind(&seq("C-x C-f")).unwrap();
        assert!(m.is_empty());
        // C-x is now unbound (the submap has been pruned).
        assert_eq!(m.lookup(&seq("C-x")), Resolution::Unbound);
    }

    #[test]
    fn unbind_unknown_sequence_errors() {
        let mut m = Keymap::new();
        match m.unbind(&seq("C-x")) {
            Err(KeymapError::NotBound { .. }) => {}
            other => panic!("expected NotBound, got {other:?}"),
        }
    }

    #[test]
    fn iter_visits_every_binding() {
        let mut m = Keymap::new();
        m.bind(&seq("C-s"), "save", src(1)).unwrap();
        m.bind(&seq("C-x C-f"), "find", src(2)).unwrap();
        m.bind(&seq("C-x C-w"), "write", src(3)).unwrap();
        let mut got: Vec<String> = m
            .iter()
            .map(|(seq, b)| format!("{} -> {}", display_sequence(&seq), b.command))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec!["C-s -> save", "C-x C-f -> find", "C-x C-w -> write",]
        );
    }

    #[test]
    fn deeply_nested_sequence_round_trips() {
        let mut m = Keymap::new();
        m.bind(&seq("C-x 4 C-f"), "open_other_window", src(1))
            .unwrap();
        assert_eq!(m.lookup(&seq("C-x")), Resolution::Pending);
        assert_eq!(m.lookup(&seq("C-x 4")), Resolution::Pending);
        match m.lookup(&seq("C-x 4 C-f")) {
            Resolution::Bound(b) => assert_eq!(b.command, "open_other_window"),
            other => panic!("expected Bound, got {other:?}"),
        }
    }
}
