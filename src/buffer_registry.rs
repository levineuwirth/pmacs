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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use thiserror::Error;

use crate::buffer::{Buffer, BufferId};
use crate::buffer_names::{NamePath, groups_of_name, name_depth, name_holders, without_suffix};
use crate::rope::Edit;

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

/// True iff `buf`'s whole contents are exactly `bytes`.
fn buffer_holds(buf: &Buffer, bytes: &[u8]) -> bool {
    if buf.len() != bytes.len() as u64 {
        return false;
    }
    let mut held = vec![0u8; bytes.len()];
    if !held.is_empty() {
        buf.snapshot_rope().slice(0, buf.len(), &mut held);
    }
    held == bytes
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
    /// What naming reads (D40), kept as buffers come, go and change, so
    /// an event renames only the paths whose names it can move.
    names: NameIndex,
}

/// The registry's naming index (D40). Buffers named by a path are filed
/// by group (basename) and, within a group, by path; for every name a
/// path can be shown by, the index counts the paths that have it, so a
/// path's name ([`crate::buffer_names::name_depth`]) is read from its
/// own names alone. An event renames the paths whose names it shares,
/// the path a reserved name equals, and the paths whose `<N>` a name can
/// meet, and looks at nothing else (E10c review 1's Medium 1: renaming
/// every buffer on every open made a session's opens quadratic, and
/// renaming a basename's whole group would leave a thousand alike
/// basenames quadratic still).
#[derive(Default)]
struct NameIndex {
    /// How many buffers hold each name.
    held: HashMap<String, usize>,
    /// How many buffers not named by a path hold each name: the names a
    /// derived one must not take.
    reserved: HashMap<String, usize>,
    /// The group and the path, by its whole name, of each buffer named
    /// by a path.
    member: HashMap<BufferId, (String, String)>,
    groups: HashMap<String, Group>,
    /// Each buffer's place in the registry's order. Never reused.
    place: HashMap<BufferId, u64>,
    next_place: u64,
    next_path: u64,
    /// How many buffers the last event named: what the unit rows read to
    /// show an event names what it can move and no more.
    #[cfg(test)]
    last_named: usize,
}

/// One basename's buffers named by a path.
#[derive(Default)]
struct Group {
    /// Each distinct path, by its whole name.
    paths: HashMap<String, GroupPath>,
    /// For each name a path of the group has: how many paths have it,
    /// and the xor of their ids, which is the one left when one is.
    shared: HashMap<String, (usize, u64)>,
    /// The path each name is the name of, by its whole name.
    named: HashMap<String, String>,
    /// Each path's whole name, by its id.
    by_id: HashMap<u64, String>,
}

/// One path of a group and the buffers holding it.
struct GroupPath {
    id: u64,
    path: NamePath,
    /// The buffers holding the path, `(place, id)`, sorted by place: the
    /// order its `<N>` are given in.
    holders: Vec<(u64, BufferId)>,
    /// Which of its names the path is shown by, once named.
    depth: Option<usize>,
}

/// Paths to rename, each `(group, whole name)`.
type Dirty = HashSet<(String, String)>;

fn count_up(counts: &mut HashMap<String, usize>, name: &str) {
    match counts.get_mut(name) {
        Some(count) => *count += 1,
        None => {
            counts.insert(name.to_owned(), 1);
        }
    }
}

fn count_down(counts: &mut HashMap<String, usize>, name: &str) {
    if let Some(count) = counts.get_mut(name) {
        *count -= 1;
        if *count == 0 {
            counts.remove(name);
        }
    }
}

impl NameIndex {
    fn hold(&mut self, name: &str) {
        count_up(&mut self.held, name);
    }

    fn release(&mut self, name: &str) {
        count_down(&mut self.held, name);
    }

    /// File buffer `id` under `path`, marking the paths that can rename:
    /// its own, and any whose name it now shares.
    fn join(&mut self, id: BufferId, path: NamePath, dirty: &mut Dirty) {
        let group_name = path.group().to_owned();
        let whole = path.whole().to_owned();
        let place = self.place[&id];
        let group = self.groups.entry(group_name.clone()).or_default();
        if let Some(entry) = group.paths.get_mut(&whole) {
            let at = entry.holders.partition_point(|&(p, _)| p < place);
            entry.holders.insert(at, (place, id));
        } else {
            let path_id = self.next_path;
            self.next_path += 1;
            for name in path.names() {
                if !group.shared.contains_key(name.as_str()) {
                    group.shared.insert(name.clone(), (0, 0));
                }
                let count = group.shared.get_mut(name.as_str()).expect("just filed");
                count.0 += 1;
                count.1 ^= path_id;
                // A name one path had to itself is shared now: if it was
                // that path's name, the path takes a longer one.
                if count.0 == 2
                    && let Some(other) = group.named.get(name.as_str())
                {
                    dirty.insert((group_name.clone(), other.clone()));
                }
            }
            group.by_id.insert(path_id, whole.clone());
            group.paths.insert(
                whole.clone(),
                GroupPath {
                    id: path_id,
                    path,
                    holders: vec![(place, id)],
                    depth: None,
                },
            );
        }
        self.member.insert(id, (group_name.clone(), whole.clone()));
        dirty.insert((group_name, whole));
    }

    /// Unfile buffer `id` from its path, marking the paths that can
    /// rename: its path's other holders, or, when it held the path
    /// alone, any path left alone with a name the two shared.
    fn leave(&mut self, id: BufferId, dirty: &mut Dirty) {
        let Some((group_name, whole)) = self.member.remove(&id) else {
            return;
        };
        let place = self.place[&id];
        let Some(group) = self.groups.get_mut(&group_name) else {
            return;
        };
        let Some(entry) = group.paths.get_mut(&whole) else {
            return;
        };
        if let Ok(at) = entry.holders.binary_search_by_key(&place, |&(p, _)| p) {
            entry.holders.remove(at);
        }
        if !entry.holders.is_empty() {
            dirty.insert((group_name, whole));
            return;
        }
        let entry = group.paths.remove(&whole).expect("present above");
        group.by_id.remove(&entry.id);
        if let Some(depth) = entry.depth {
            let name = entry.path.names()[depth].as_str();
            if group.named.get(name) == Some(&whole) {
                group.named.remove(name);
            }
        }
        for name in entry.path.names() {
            let Some(count) = group.shared.get_mut(name.as_str()) else {
                continue;
            };
            count.0 -= 1;
            count.1 ^= entry.id;
            match count.0 {
                0 => {
                    group.shared.remove(name.as_str());
                }
                // A name two paths shared is the other's alone now.
                1 => {
                    if let Some(other) = group.by_id.get(&count.1) {
                        dirty.insert((group_name.clone(), other.clone()));
                    }
                }
                _ => {}
            }
        }
        if group.paths.is_empty() {
            self.groups.remove(&group_name);
        }
    }

    /// A reserved name came or went: mark the path it is the name of,
    /// the one path that has it among its names, and the path whose `<N>`
    /// it can be.
    fn reserved_changed(&self, name: &str, dirty: &mut Dirty) {
        for group_name in groups_of_name(name) {
            let Some(group) = self.groups.get(group_name) else {
                continue;
            };
            let mut mark = |whole: &String| {
                dirty.insert((group_name.to_owned(), whole.clone()));
            };
            if let Some(whole) = group.named.get(name) {
                mark(whole);
            }
            if let Some(&(1, path_id)) = group.shared.get(name)
                && let Some(whole) = group.by_id.get(&path_id)
            {
                mark(whole);
            }
            if let Some(base) = without_suffix(name)
                && let Some(whole) = group.named.get(base)
            {
                mark(whole);
            }
        }
    }

    /// A derived name came or went: mark the path whose `<N>` it can
    /// be, in its own group (a buffer moved to another path of its
    /// basename holds its old `<N>` until it is renamed) or another
    /// (`a.rs<2>` beside a second `a.rs`).
    fn derived_changed(&self, name: &str, dirty: &mut Dirty) {
        let Some(base) = without_suffix(name) else {
            return;
        };
        for group_name in groups_of_name(name) {
            if let Some(whole) = self
                .groups
                .get(group_name)
                .and_then(|group| group.named.get(base))
            {
                dirty.insert((group_name.to_owned(), whole.clone()));
            }
        }
    }
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
    ///
    /// The buffer is named as it goes in: a new name, explicit or not,
    /// can be one a derived name held, and a buffer named by its path
    /// ([`Self::insert_for_path`]) takes its name here, once (D40).
    pub fn insert(&mut self, buffer: Buffer) -> BufferId {
        let id = buffer.id();
        let mut dirty = Dirty::new();
        if let Some(old) = self.buffers.insert(id, buffer) {
            // Defensive: keep `order` consistent if someone re-inserted.
            self.order.retain(|x| *x != id);
            self.unfile(id, old.name(), &mut dirty);
        }
        self.order.push(id);
        self.names.place.insert(id, self.names.next_place);
        self.names.next_place += 1;
        self.file(id, &mut dirty);
        self.settle(dirty);
        id
    }

    /// Insert `buffer` as the buffer of the file at `path`, which the
    /// caller has normalized: bound to it and named by it as it goes in,
    /// beside every other name (D40), one naming per open. The open
    /// sites come here: `EditorCore::get_or_load_buffer`, a new file's
    /// `resolve_target_buffer`, `pmacs.buffer.from_file` and
    /// `find_or_open`.
    pub fn insert_for_path(&mut self, mut buffer: Buffer, path: PathBuf) -> BufferId {
        buffer.set_file_path(Some(path));
        buffer.name_from_path();
        self.insert(buffer)
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
    ///
    /// A path changed through it on a buffer named by its path is not
    /// named by the new one until [`Self::refresh_names`]: the paths'
    /// doors (`EditorCore::set_buffer_path`, rename reconciliation, a
    /// save-as through [`Self::name_by_path`]) refresh.
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
        if let Some(buf) = self.buffers.get(&id)
            && buf.editing_in_progress()
        {
            return Err(RegistryError::ConcurrentEdit {
                id,
                name: buf.name().to_string(),
            });
        }
        let buf = self
            .buffers
            .remove(&id)
            .ok_or(RegistryError::Missing { id })?;
        self.order.retain(|x| *x != id);
        // A survivor returns to the shorter name it no longer shares.
        let mut dirty = Dirty::new();
        self.unfile(id, buf.name(), &mut dirty);
        self.names.place.remove(&id);
        self.settle(dirty);
        Ok(buf)
    }

    /// Bind the buffer behind `id` to `path`, which the caller has
    /// normalized, and name it from that path from now on
    /// ([`crate::buffer::BufferNameOrigin::PathDerived`]), renaming every
    /// derived name that can move with it (D40). A save-as
    /// (`EditorCore::write_active_buffer_to`) comes here, whatever named
    /// the buffer before.
    ///
    /// # Errors
    /// [`RegistryError::Missing`] for a stale handle.
    pub fn name_by_path(&mut self, id: BufferId, path: PathBuf) -> Result<(), RegistryError> {
        let buf = self.get_mut(id)?;
        buf.set_file_path(Some(path));
        buf.name_from_path();
        self.refresh_names(&[id]);
        Ok(())
    }

    /// Name the buffer behind `id` by the directory it lists, `proj/`,
    /// through the same function as a file's name (E10c.3, D37): a
    /// dired listing records its directory here and is never given a
    /// name of its own.
    ///
    /// # Errors
    /// [`RegistryError::Missing`] for a stale handle.
    pub fn name_by_directory(
        &mut self,
        id: BufferId,
        directory: std::path::PathBuf,
    ) -> Result<(), RegistryError> {
        self.get_mut(id)?.name_from_directory(directory);
        self.refresh_names(&[id]);
        Ok(())
    }

    /// Give the buffer behind `id` the explicit name `name`, which
    /// nothing rewrites, moving any derived name it equals (D40). The
    /// door for `pmacs.buffer.set_name`.
    ///
    /// # Errors
    /// [`RegistryError::Missing`] for a stale handle.
    pub fn set_name(&mut self, id: BufferId, name: impl Into<String>) -> Result<(), RegistryError> {
        let old = self.get(id)?.name().to_owned();
        let mut dirty = Dirty::new();
        self.unfile(id, &old, &mut dirty);
        self.buffers
            .get_mut(&id)
            .expect("resolved above")
            .set_name(name);
        self.file(id, &mut dirty);
        self.settle(dirty);
        Ok(())
    }

    /// Rename what a change to the paths of `ids` can move, through the
    /// one naming rule ([`crate::buffer_names::name_depth`] and
    /// [`crate::buffer_names::name_holders`], D40, D37). Every site that
    /// binds or moves a path on a buffer already registered calls this
    /// (`EditorCore::set_buffer_path`, rename reconciliation once for
    /// every buffer a rename moved), so an open beside an alike basename
    /// lengthens both, a kill shortens the survivor, and a rename names
    /// what it moved as the whole set decides.
    pub fn refresh_names(&mut self, ids: &[BufferId]) {
        let mut dirty = Dirty::new();
        for &id in ids {
            let Some(buf) = self.buffers.get(&id) else {
                continue;
            };
            let filed = self
                .names
                .member
                .get(&id)
                .and_then(|(group, whole)| self.names.groups.get(group)?.paths.get(whole));
            let unchanged = match (filed, buf.name_path()) {
                (Some(entry), Some((path, directory))) => entry.path.is_of(path, directory),
                (None, None) => true,
                _ => false,
            };
            if unchanged {
                continue;
            }
            let name = buf.name().to_owned();
            self.unfile(id, &name, &mut dirty);
            self.file(id, &mut dirty);
        }
        self.settle(dirty);
    }

    /// File buffer `id`, which holds its name, in the index: under its
    /// path if its name derives from one, else as a reserved name.
    fn file(&mut self, id: BufferId, dirty: &mut Dirty) {
        let buf = &self.buffers[&id];
        self.names.hold(buf.name());
        if let Some((path, directory)) = buf.name_path() {
            let path = NamePath::new(path, directory);
            self.names.join(id, path, dirty);
        } else {
            count_up(&mut self.names.reserved, buf.name());
            self.names.reserved_changed(buf.name(), dirty);
        }
    }

    /// Unfile buffer `id`, which held `name`, from the index.
    fn unfile(&mut self, id: BufferId, name: &str, dirty: &mut Dirty) {
        self.names.release(name);
        if self.names.member.contains_key(&id) {
            self.names.leave(id, dirty);
            self.names.derived_changed(name, dirty);
        } else {
            count_down(&mut self.names.reserved, name);
            self.names.reserved_changed(name, dirty);
        }
    }

    /// Name every path in `dirty`, and then every path the names that
    /// changed can move in turn, until none changes. A path's name reads
    /// only reserved names and which of its names other paths share, so
    /// the names settle in one round; a `<N>` that meets another group's
    /// name takes a second.
    fn settle(&mut self, mut dirty: Dirty) {
        #[cfg(test)]
        {
            self.names.last_named = 0;
        }
        let mut rounds = 0;
        while !dirty.is_empty() {
            rounds += 1;
            debug_assert!(rounds <= 8, "the names settle");
            let mut next = Dirty::new();
            for (group, whole) in dirty {
                for (old, new) in self.name_path(&group, &whole) {
                    self.names.derived_changed(&old, &mut next);
                    self.names.derived_changed(&new, &mut next);
                }
            }
            dirty = next;
        }
    }

    /// Name the buffers holding the path `whole` of `group`, returning
    /// each name that changed, old and new.
    fn name_path(&mut self, group: &str, whole: &str) -> Vec<(String, String)> {
        let (depth, holders, changes) = {
            let index = &self.names;
            let Some(entry) = index.groups.get(group).and_then(|g| g.paths.get(whole)) else {
                return Vec::new();
            };
            let shared = &index.groups[group].shared;
            let depth = name_depth(
                &entry.path,
                |n| shared.get(n).is_some_and(|&(count, _)| count > 1),
                |n| index.reserved.contains_key(n),
            );
            let base = entry.path.names()[depth].as_str();
            let holds: Vec<&str> = entry
                .holders
                .iter()
                .map(|(_, id)| self.buffers[id].name())
                .collect();
            let taken = |n: &str| {
                index.held.get(n).copied().unwrap_or(0) > holds.iter().filter(|&&h| h == n).count()
            };
            let names = name_holders(base, holds.len(), !index.reserved.contains_key(base), taken);
            let changes: Vec<(BufferId, String)> = entry
                .holders
                .iter()
                .zip(&holds)
                .zip(names)
                .filter(|((_, held), name)| **held != name.as_ref())
                .map(|(((_, id), _), name)| (*id, name.into_owned()))
                .collect();
            (depth, holds.len(), changes)
        };
        #[cfg(test)]
        {
            self.names.last_named += holders;
        }
        #[cfg(not(test))]
        let _ = holders;
        let group_entry = self.names.groups.get_mut(group).expect("named above");
        let entry = group_entry.paths.get_mut(whole).expect("named above");
        if entry.depth != Some(depth) {
            if let Some(before) = entry.depth {
                let name = entry.path.names()[before].as_str();
                if group_entry.named.get(name).is_some_and(|w| w == whole) {
                    group_entry.named.remove(name);
                }
            }
            group_entry
                .named
                .insert(entry.path.names()[depth].clone(), whole.to_owned());
            entry.depth = Some(depth);
        }
        let mut renamed = Vec::with_capacity(changes.len());
        for (id, name) in changes {
            let buf = self.buffers.get_mut(&id).expect("a holder is registered");
            let old = buf.name().to_owned();
            self.names.release(&old);
            self.names.hold(&name);
            buf.set_derived_name(name.clone());
            renamed.push((old, name));
        }
        renamed
    }

    /// Replace the contents of the generated buffer named `name`
    /// through the one authorized write, [`Buffer::set_generated_contents`]
    /// (`docs/invariants.md`, "Generated buffers"), creating the buffer
    /// when there is none. Returns the buffer's id and the one `Replace`
    /// edit, which the caller fans out --- every displaying window and
    /// the daemon-origin CRDT op --- or `None` when nothing was written
    /// because the buffer already holds `text` under the primitive's
    /// contract (read-only, so nothing a user typed is being kept).
    ///
    /// E8d.3: `*help*`, `*workers*` and `*pmacs-instance*` each wrote
    /// by a `Delete` then an `Insert` through `apply_edit`, a copy per
    /// buffer of the operation this primitive exists for, and each copy
    /// left its buffer writable with a history that reached the rope
    /// (`M-x buffer.undo` emptied `*help*`), which is the failure the
    /// invariant's first bullet describes. One helper, one write.
    pub fn replace_generated(&mut self, name: &str, text: &str) -> (BufferId, Option<Edit>) {
        let id = self.find_by_name(name).unwrap_or_else(|| self.create(name));
        let buf = self.get_mut(id).expect("just resolved");
        if buf.is_read_only() && buffer_holds(buf, text.as_bytes()) {
            // Regenerated content equal to what is shown: no edit to
            // fan out, and nothing for the modeline to call unsaved.
            buf.mark_clean();
            return (id, None);
        }
        (id, buf.set_generated_contents(text.as_bytes()).ok())
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

    /// First buffer bound to `path` on disk, in insertion order.
    /// T M4.5 L1: matches on the per-buffer `file_path` (set at
    /// load), so cross-file navigation reuses an already-open file
    /// instead of creating a duplicate buffer.
    #[must_use]
    pub fn find_by_path(&self, path: &std::path::Path) -> Option<BufferId> {
        self.order.iter().copied().find(|id| {
            self.buffers
                .get(id)
                .is_some_and(|b| b.file_path() == Some(path))
        })
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
    use crate::buffer_names::{NameRequest, derive_names};

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

    fn open(r: &mut BufferRegistry, path: &str) -> BufferId {
        r.insert_for_path(Buffer::new(BufferId::next(), path), PathBuf::from(path))
    }

    fn name(r: &BufferRegistry, id: BufferId) -> &str {
        r.get(id).unwrap().name()
    }

    /// E10c review 1's Medium 1: an event names the groups it can move
    /// and no others. Among a thousand files of a thousand basenames an
    /// open names one buffer, the one it opened, once; an open beside an
    /// alike basename names the two; a kill names the survivors of its
    /// group, and a kill alone in its group names none.
    #[test]
    fn an_event_names_only_the_basenames_it_can_move() {
        let mut r = BufferRegistry::new();
        r.create("*scratch*");
        for i in 0..1000 {
            open(&mut r, &format!("/p/d{i}/f{i}.txt"));
        }
        let lone = open(&mut r, "/p/src/a.rs");
        assert_eq!(r.names.last_named, 1, "an open beside no alike basename");
        assert_eq!(name(&r, lone), "a.rs");
        let other = open(&mut r, "/p/tests/a.rs");
        assert_eq!(r.names.last_named, 2, "an open beside one alike basename");
        assert_eq!(
            [name(&r, lone), name(&r, other)],
            ["src/a.rs", "tests/a.rs"]
        );
        r.remove(other).unwrap();
        assert_eq!(r.names.last_named, 1, "a kill names its group's survivor");
        assert_eq!(name(&r, lone), "a.rs");
        r.remove(lone).unwrap();
        assert_eq!(r.names.last_named, 0, "a kill alone in its group");
        let mine = r.create("f7.txt");
        assert_eq!(
            r.names.last_named, 1,
            "an explicit name moves only its equal"
        );
        assert_eq!(name(&r, mine), "f7.txt");
        let moved = r.ids()[8];
        assert_eq!(name(&r, moved), "d7/f7.txt");

        // A thousand alike basenames: an open names the paths whose name
        // it shares, and once they are told apart, only itself.
        let first = open(&mut r, "/q/d0/mod.txt");
        assert_eq!(r.names.last_named, 1);
        let second = open(&mut r, "/q/d1/mod.txt");
        assert_eq!(
            r.names.last_named, 2,
            "the open and the one it collides with"
        );
        assert_eq!(
            [name(&r, first), name(&r, second)],
            ["d0/mod.txt", "d1/mod.txt"]
        );
        for i in 2..1000 {
            open(&mut r, &format!("/q/d{i}/mod.txt"));
        }
        let last = open(&mut r, "/q/d1000/mod.txt");
        assert_eq!(r.names.last_named, 1, "beside a thousand alike, one");
        assert_eq!(name(&r, last), "d1000/mod.txt");
        r.remove(last).unwrap();
        assert_eq!(r.names.last_named, 0);
    }

    /// Every derived name, decided at once over the whole session: the
    /// definition the per-group naming must agree with.
    fn from_scratch(r: &BufferRegistry) -> Vec<(BufferId, String)> {
        let derived: Vec<(BufferId, NamePath)> = r
            .ids()
            .iter()
            .filter_map(|&id| {
                let (path, directory) = r.get(id).unwrap().name_path()?;
                Some((id, NamePath::new(path, directory)))
            })
            .collect();
        let explicit: HashSet<&str> = r
            .ids()
            .iter()
            .map(|&id| r.get(id).unwrap())
            .filter(|b| b.name_path().is_none())
            .map(Buffer::name)
            .collect();
        let requests: Vec<NameRequest<'_>> = derived
            .iter()
            .map(|(_, path)| NameRequest { path })
            .collect();
        let names = derive_names(&requests, |name| explicit.contains(name));
        derived
            .iter()
            .zip(names)
            .map(|((id, _), name)| (*id, name.into_owned()))
            .collect()
    }

    /// The names now are [`from_scratch`]'s, and no derived name equals
    /// another name.
    fn agrees_with_scratch(r: &BufferRegistry, at: &str) {
        let expected = from_scratch(r);
        let actual: Vec<(BufferId, String)> = expected
            .iter()
            .map(|(id, _)| (*id, name(r, *id).to_owned()))
            .collect();
        assert_eq!(actual, expected, "{at}");
        let mut seen: HashSet<&str> = HashSet::new();
        for (_, derived) in &actual {
            assert!(seen.insert(derived.as_str()), "{at}: {derived} twice");
        }
        for &id in r.ids() {
            let buf = r.get(id).unwrap();
            if buf.name_path().is_none() {
                assert!(
                    !actual.iter().any(|(_, n)| n == buf.name()),
                    "{at}: a derived name equals the explicit {}",
                    buf.name()
                );
            }
        }
    }

    /// Opens, listings, explicit names, kills, renames (one buffer or
    /// several at once) and save-as in seeded orders over paths that
    /// crowd one another, a file named like a second `a.rs` among them,
    /// the names after every event checked against [`from_scratch`]:
    /// naming the paths an event can move names the session as naming
    /// all of it would, and no derived name equals another name.
    #[test]
    fn naming_by_path_agrees_with_naming_the_whole_session() {
        const FILES: [&str; 10] = [
            "/w/src/a.rs",
            "/w/tests/a.rs",
            "/w/x/src/a.rs",
            "/v/x/src/a.rs",
            "/a.rs",
            "/w/b.rs",
            "/w/src/b.rs",
            "/w/proj",
            "/v/proj",
            "/w/a.rs<2>",
        ];
        const EXPLICIT: [&str; 7] = [
            "a.rs",
            "src/a.rs",
            "*scratch*",
            "proj/",
            "b.rs",
            "a.rs<2>",
            "x",
        ];
        const SEEDS: [u64; 4] = [
            0x9e37_79b9_7f4a_7c15,
            0x2545_f491_4f6c_dd1d,
            0xd1b5_4a32_d192_ed03,
            0x8cb9_2ba7_2f3d_8dd7,
        ];
        for start in SEEDS {
            let mut seed = start;
            let mut next = |n: usize| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed % n as u64) as usize
            };
            let mut r = BufferRegistry::new();
            for step in 0..3000 {
                let ids = r.ids().to_vec();
                let pick = |k: usize| ids[k % ids.len().max(1)];
                // Crowded, never large: past two dozen buffers, kill.
                let event = if ids.len() > 24 { 4 } else { next(8) };
                match event {
                    0 | 1 => {
                        open(&mut r, FILES[next(FILES.len())]);
                    }
                    2 => {
                        let id = r.create("listing");
                        r.name_by_directory(id, PathBuf::from(FILES[next(FILES.len())]))
                            .unwrap();
                    }
                    3 => {
                        r.create(EXPLICIT[next(EXPLICIT.len())]);
                    }
                    4 if !ids.is_empty() => {
                        r.remove(pick(next(64))).unwrap();
                    }
                    5 | 7 if !ids.is_empty() => {
                        let moved: Vec<BufferId> = (0..if event == 7 { 3 } else { 1 })
                            .map(|_| pick(next(64)))
                            .collect();
                        for &id in &moved {
                            r.get_mut(id)
                                .unwrap()
                                .set_file_path(Some(PathBuf::from(FILES[next(FILES.len())])));
                        }
                        r.refresh_names(&moved);
                    }
                    6 if !ids.is_empty() => {
                        let id = pick(next(64));
                        if next(2) == 0 {
                            r.set_name(id, EXPLICIT[next(EXPLICIT.len())]).unwrap();
                        } else {
                            r.name_by_path(id, PathBuf::from(FILES[next(FILES.len())]))
                                .unwrap();
                        }
                    }
                    _ => {}
                }
                agrees_with_scratch(&r, &format!("seed {start:#x} step {step}"));
            }
        }
    }
}
