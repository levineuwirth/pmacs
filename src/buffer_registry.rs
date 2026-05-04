// buffer_registry.rs --- Buffer registry: opaque IDs resolved through Rust.

//! Owns [`Buffer`]s behind opaque [`BufferId`] handles.
//!
//! # Why a registry
//!
//! Spec §3 Checkpoint 6 and the Lua boundary rules (R52, R53) keep raw
//! `Arc<Mutex<Buffer>>` away from Lua. Lua holds `BufferId` values
//! (cheap, `Copy`, no GC entanglement); every method call resolves the
//! ID through this registry. A removed buffer leaves all live IDs as
//! *stale* handles, and the next resolution returns
//! [`RegistryError::Missing`] --- a typed error, not a use-after-free.
//!
//! # Threading
//!
//! Single-threaded by construction. The registry lives next to the
//! [`crate::lua::LuaHost`] on the main thread; workers (M3) communicate
//! via typed messages and snapshots, never by sharing the registry.

use std::collections::HashMap;

use thiserror::Error;

use crate::buffer::{Buffer, BufferId};

/// Errors raised by the registry.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// The supplied [`BufferId`] is not currently registered. The handle
    /// is stale: it referred to a buffer that has since been removed (or
    /// was never registered with this instance).
    #[error("no buffer registered with id {id:?}")]
    Missing {
        /// The offending ID, preserved for structured surfacing across
        /// the Lua boundary (R52).
        id: BufferId,
    },
    /// Removal was requested for a buffer that is currently mid-edit
    /// (T M7.4). The most common path: a Lua intercept body on buffer
    /// `A` called `pmacs.buffer.remove(A)`. Mirrors
    /// [`crate::buffer::BufferError::ConcurrentEdit`] in spirit; the
    /// message names the workaround per project convention.
    #[error(
        "buffer `{name}` (id {id:?}) is already being edited; \
         it cannot be removed while an intercept is running on it. \
         To remove this buffer, return from the intercept first \
         (the registry borrow will release at that point), or remove \
         a different buffer."
    )]
    ConcurrentEdit {
        /// The buffer's identifier.
        id: BufferId,
        /// The buffer's name, for diagnostics.
        name: String,
    },
}

/// Owns [`Buffer`]s behind their [`BufferId`]s.
///
/// Construction goes through [`Self::new`] or [`Self::default`]. Buffers
/// are inserted by [`Self::create`], [`Self::create_from_bytes`], or
/// [`Self::insert`] (which takes a pre-built [`Buffer`] --- useful when a
/// buffer is loaded from disk before being handed off to the registry).
#[derive(Default)]
pub struct BufferRegistry {
    buffers: HashMap<BufferId, Buffer>,
    /// Insertion-order list of IDs. Used for stable iteration / listing
    /// (`HashMap` iteration order is unspecified, which would make
    /// `pmacs.buffer.list()` non-deterministic).
    order: Vec<BufferId>,
}

impl BufferRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an already-constructed [`Buffer`]. Returns its [`BufferId`].
    ///
    /// If a buffer with the same ID is already present (which would
    /// indicate a programming error --- IDs are allocated by the global
    /// counter and unique), the old buffer is overwritten and removed
    /// from the order list before the new one is appended.
    pub fn insert(&mut self, buffer: Buffer) -> BufferId {
        let id = buffer.id();
        if self.buffers.insert(id, buffer).is_some() {
            // Defensive: keep `order` consistent if someone re-inserted.
            self.order.retain(|x| *x != id);
        }
        self.order.push(id);
        id
    }

    /// Create a fresh empty [`Buffer`] under a new [`BufferId`].
    pub fn create(&mut self, name: impl Into<String>) -> BufferId {
        self.insert(Buffer::new(BufferId::next(), name))
    }

    /// Create a [`Buffer`] seeded with `bytes` under a new [`BufferId`].
    pub fn create_from_bytes(&mut self, name: impl Into<String>, bytes: &[u8]) -> BufferId {
        self.insert(Buffer::from_bytes(BufferId::next(), name, bytes))
    }

    /// Resolve an ID to a shared reference. Returns
    /// [`RegistryError::Missing`] for stale handles (R53).
    pub fn get(&self, id: BufferId) -> Result<&Buffer, RegistryError> {
        self.buffers.get(&id).ok_or(RegistryError::Missing { id })
    }

    /// Resolve an ID to an exclusive reference.
    pub fn get_mut(&mut self, id: BufferId) -> Result<&mut Buffer, RegistryError> {
        self.buffers
            .get_mut(&id)
            .ok_or(RegistryError::Missing { id })
    }

    /// Remove and return the buffer behind `id`. Subsequent lookups of
    /// `id` produce [`RegistryError::Missing`].
    ///
    /// Refuses to remove a buffer that is currently mid-edit (T M7.4):
    /// an intercept body running on buffer `A` cannot drop `A` out
    /// from under itself. Returns
    /// [`RegistryError::ConcurrentEdit`] in that case, leaving the
    /// buffer in place.
    pub fn remove(&mut self, id: BufferId) -> Result<Buffer, RegistryError> {
        // Peek without taking ownership: if the buffer is mid-edit we
        // surface a typed error and leave the registry untouched.
        if let Some(buf) = self.buffers.get(&id) {
            if buf.editing_in_progress() {
                return Err(RegistryError::ConcurrentEdit {
                    id,
                    name: buf.name().to_string(),
                });
            }
        }
        let buf = self
            .buffers
            .remove(&id)
            .ok_or(RegistryError::Missing { id })?;
        self.order.retain(|x| *x != id);
        Ok(buf)
    }

    /// True iff `id` currently resolves.
    #[must_use]
    pub fn contains(&self, id: BufferId) -> bool {
        self.buffers.contains_key(&id)
    }

    /// First buffer whose name matches `name` exactly, in insertion
    /// order. Used by host code that addresses well-known buffers
    /// (e.g. `*errors*`) without holding their `BufferId`.
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<BufferId> {
        self.order
            .iter()
            .copied()
            .find(|id| self.buffers.get(id).is_some_and(|b| b.name() == name))
    }

    /// IDs in insertion order. Stable: preserved across non-removing
    /// operations.
    #[must_use]
    pub fn ids(&self) -> &[BufferId] {
        &self.order
    }

    /// Number of registered buffers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    /// True iff no buffers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_get_round_trips() {
        let mut r = BufferRegistry::new();
        let id = r.create("alpha");
        assert_eq!(r.len(), 1);
        let b = r.get(id).unwrap();
        assert_eq!(b.name(), "alpha");
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn create_from_bytes_seeds_content() {
        let mut r = BufferRegistry::new();
        let id = r.create_from_bytes("seed", b"hello");
        assert_eq!(r.get(id).unwrap().len(), 5);
    }

    #[test]
    fn missing_id_returns_typed_error() {
        // Avoids requiring Debug on Buffer (Buffer has trait-object views
        // that don't carry Debug), so we destructure the error directly.
        let r = BufferRegistry::new();
        let stale = BufferId::next();
        let err = r.get(stale).err().expect("expected stale-handle error");
        match err {
            RegistryError::Missing { id } => assert_eq!(id, stale),
            other @ RegistryError::ConcurrentEdit { .. } => {
                panic!("expected Missing, got {other:?}")
            }
        }
    }

    #[test]
    fn remove_invalidates_handle() {
        let mut r = BufferRegistry::new();
        let id = r.create("doomed");
        r.remove(id).unwrap();
        assert!(!r.contains(id));
        assert!(r.get(id).is_err());
        assert!(r.remove(id).is_err());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn find_by_name_returns_first_match() {
        let mut r = BufferRegistry::new();
        let a = r.create("alpha");
        let _b = r.create("beta");
        let alpha2 = r.create("alpha");
        // Insertion order: returns the first.
        assert_eq!(r.find_by_name("alpha"), Some(a));
        // Once removed, the second alpha takes the slot.
        r.remove(a).unwrap();
        assert_eq!(r.find_by_name("alpha"), Some(alpha2));
        assert_eq!(r.find_by_name("nope"), None);
    }

    #[test]
    fn ids_preserved_in_insertion_order() {
        let mut r = BufferRegistry::new();
        let a = r.create("a");
        let b = r.create("b");
        let c = r.create("c");
        assert_eq!(r.ids(), &[a, b, c]);
        r.remove(b).unwrap();
        assert_eq!(r.ids(), &[a, c]);
    }
}
