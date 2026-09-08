// autosave.rs --- periodic recovery copies + crash recovery (Arc 3 phase 3).

//! Every modified file buffer is periodically written to a private
//! recovery file under `$XDG_STATE_HOME/pmacs/autosave/`. If pmacs dies,
//! the next session notices the copy and offers `M-x recover-file`.
//! Emacs's `auto-save-mode` + `recover-file`.
//!
//! This module owns the parts Lua cannot do: enumerating **all** buffers'
//! paths (Lua has no per-buffer path getter), and the `FileMeta`
//! external-change guard (`FileMeta` is neither Lua-visible nor serde).
//! `builtin/runtime/autosave.lua` owns the cadence, the configurable
//! interval, and the recovery UX.
//!
//! Recovery files are written `0600` under a `0700` directory
//! (Q#AS11) — they hold *unsaved file contents*, a different class of
//! secret from saveplace's cursor offsets.
//!
//! Framing: docs/archive/framings/autosave-recovery-framing.md.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use mlua::Lua;
use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
use crate::file_io::FileMeta;
use crate::hash::sha256_hex;
use crate::lua_bindings::{SharedCore, StateDir};

/// Bump when the envelope shape changes incompatibly. A recovery file
/// with an unrecognized version reads as [`RecoveryStatus::Corrupt`] —
/// never silently applied.
pub const AUTOSAVE_VERSION: u32 = 1;

/// The header of a recovery file: one line of JSON, then `\n`, then the
/// raw buffer bytes (Q#AS4). One atomic write, so a crash can never leave
/// a torn header/contents pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Header {
    version: u32,
    /// The buffer's path, for provenance and orphan inspection.
    path: String,
    /// The origin file's identity when this copy was taken.
    ///
    /// **Nullable**: a `[new file]` buffer (a path that does not exist on
    /// disk yet) has no `file_meta`, and its unsaved contents are exactly
    /// the work most worth recovering. `None` means "there was no file on
    /// disk when this was autosaved".
    origin: Option<Origin>,
}

/// `FileMeta` hand-serialized — it is not serde, and `SystemTime` has no
/// stable wire form. Stored as an offset from the Unix epoch so the
/// comparison is exact; we never reconstruct a `SystemTime`, only compare
/// these parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Origin {
    mtime_secs: i64,
    mtime_nanos: u32,
    size: u64,
}

impl Origin {
    fn from_meta(m: &FileMeta) -> Self {
        let (mtime_secs, mtime_nanos) = match m.mtime.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => (
                i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
                d.subsec_nanos(),
            ),
            // Pre-epoch mtimes are exotic but representable.
            Err(e) => {
                let d = e.duration();
                (
                    i64::try_from(d.as_secs()).map_or(i64::MIN, |s| -s),
                    d.subsec_nanos(),
                )
            }
        };
        Self {
            mtime_secs,
            mtime_nanos,
            size: m.size,
        }
    }
}

/// What a recovery file means for a given path (Q#AS5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStatus {
    /// No recovery file.
    None,
    /// The on-disk file is unchanged since the copy was taken (or is
    /// still absent, for a `[new file]`), so the recovery is strictly
    /// newer. The only status that is announced.
    Fresh,
    /// The file changed underneath us — externally edited, deleted, or
    /// (for a `[new file]`) created by someone else. Never auto-offered:
    /// silently clobbering it is the one unrecoverable mistake here.
    Stale,
    /// Unparseable or unrecognized version. Never offered, never errors;
    /// `discard-recovery` removes it.
    Corrupt,
}

impl RecoveryStatus {
    /// The lowercase name Lua sees.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RecoveryStatus::None => "none",
            RecoveryStatus::Fresh => "fresh",
            RecoveryStatus::Stale => "stale",
            RecoveryStatus::Corrupt => "corrupt",
        }
    }
}

/// The `pmacs.state` key a path's recovery file lives under.
#[must_use]
pub fn key_for(path: &Path) -> String {
    format!("autosave/{}", sha256_hex(&path.display().to_string()))
}

/// Per-session autosave bookkeeping.
#[derive(Default)]
pub struct AutosaveCache(RefCell<CacheInner>);

#[derive(Default)]
struct CacheInner {
    /// Skip cache: `BufferId → (path_hash, revision)` (Q#AS8).
    ///
    /// Keyed on the **path hash as well as the revision**, not the
    /// revision alone: a buffer keeps its `BufferId` across a path change
    /// (an LSP `WorkspaceEdit` rename calls `set_buffer_path`), so a
    /// revision-only cache would skip the write, never create the
    /// recovery file under the new key, and orphan the old one. It also
    /// remembers *where a buffer's recovery currently lives*, which is
    /// what makes cleanup work after a rename.
    written: HashMap<BufferId, (String, u64)>,
    /// Which buffer owns each recovery slot: `path_hash → BufferId`
    /// (Q#AS12, Q#AS13).
    ///
    /// Two roles in one map:
    ///
    /// * **Absent** = the recovery file at that hash (if any) is
    ///   *unclaimed crash data* — this session did not write it. Sweeping
    ///   would overwrite the crash copy with the current buffer,
    ///   destroying exactly what autosave protects. So it blocks the
    ///   sweep until `recover-file` adopts it or `discard-recovery`
    ///   removes it, and neither save nor kill may delete it.
    /// * **Present** = the slot belongs to exactly *one* buffer. A
    ///   recovery file is keyed by path (a later session knows only
    ///   paths, never old `BufferId`s), but `pmacs.buffer.from_file` can
    ///   open a *second* buffer on the same path. Both cannot be
    ///   protected under one key: the later write would win on disk while
    ///   both buffers believed themselves saved. So the first modified
    ///   buffer claims the slot and any other buffer on that path is
    ///   reported as conflicted, not silently mis-protected.
    owner: HashMap<String, BufferId>,
}

/// Encode a header + contents into the one-file envelope.
fn encode(header: &Header, contents: &[u8]) -> Result<Vec<u8>, String> {
    // serde_json's compact form never contains a raw newline, so the
    // first `\n` unambiguously ends the header.
    let mut out = serde_json::to_vec(header).map_err(|e| e.to_string())?;
    debug_assert!(!out.contains(&b'\n'));
    out.push(b'\n');
    out.extend_from_slice(contents);
    Ok(out)
}

/// Split an envelope at its **first** newline. Contents may contain
/// newlines and arbitrary non-UTF-8 bytes, so only the first one counts.
/// Returns `None` for anything malformed — the caller maps that to
/// [`RecoveryStatus::Corrupt`].
fn decode(bytes: &[u8]) -> Option<(Header, &[u8])> {
    let nl = bytes.iter().position(|&b| b == b'\n')?;
    let header: Header = serde_json::from_slice(&bytes[..nl]).ok()?;
    if header.version != AUTOSAVE_VERSION {
        return None;
    }
    Some((header, &bytes[nl + 1..]))
}

/// The configured state dir, if any (absent under tests / no HOME).
fn base_dir(lua: &Lua) -> Option<std::path::PathBuf> {
    lua.app_data_ref::<StateDir>().map(|d| d.0.clone())
}

/// Classify the recovery file for `path` (Q#AS5's table).
#[must_use]
pub fn status(base: &Path, path: &Path) -> RecoveryStatus {
    let key = key_for(path);
    let Ok(Some(bytes)) = crate::state::read_bytes(base, &key) else {
        return RecoveryStatus::None;
    };
    let Some((header, _)) = decode(&bytes) else {
        return RecoveryStatus::Corrupt;
    };
    let on_disk = crate::file_io::current_meta(path).ok();
    match (header.origin, on_disk) {
        // Had an origin, file still there: fresh iff identity matches.
        (Some(o), Some(cur)) if o == Origin::from_meta(&cur) => RecoveryStatus::Fresh,
        // `[new file]`: fresh while it is still absent.
        (None, None) => RecoveryStatus::Fresh,
        // Everything else changed underneath us: the file was edited
        // externally, deleted, or (for a `[new file]`) created by someone
        // else. Never auto-offered.
        _ => RecoveryStatus::Stale,
    }
}

/// The recovered contents for `path`, if a parseable recovery exists.
/// Returns bytes for `Stale` too — the command warns and confirms.
#[must_use]
pub fn recover_bytes(base: &Path, path: &Path) -> Option<Vec<u8>> {
    let bytes = crate::state::read_bytes(base, &key_for(path))
        .ok()
        .flatten()?;
    let (_, contents) = decode(&bytes)?;
    Some(contents.to_vec())
}

/// Delete the recovery file for `path` (idempotent).
pub fn discard(base: &Path, path: &Path) -> bool {
    crate::state::remove(base, &key_for(path)).is_ok()
}

/// A buffer that needs a recovery copy written, gathered under the core
/// borrow so all IO happens after it is released.
struct Pending {
    id: BufferId,
    path: String,
    path_hash: String,
    revision: u64,
    origin: Option<Origin>,
    contents: Vec<u8>,
}

/// One autosave pass: write a recovery copy of every modified file
/// buffer whose contents changed since its last copy. Returns how many
/// were written.
///
/// Runs on the main thread; the two skips in Q#AS8 keep that bounded.
/// Unlike desktop-save this is **not** daemon-gated — autosave is
/// per-buffer, not per-frontend, and a daemon holds the unsaved work.
///
/// Returns `(written, blocked, conflicted)`:
///
/// * `blocked` — an **unclaimed** recovery file already sits at the
///   buffer's key (Q#AS12): crash data this session did not write.
/// * `conflicted` — another buffer already owns that path's recovery
///   slot (Q#AS13): two buffers visit the same file and only one can be
///   protected under a path-keyed recovery file.
///
/// # Errors
/// A state-write failure. Individual buffers never abort the pass.
pub fn sweep(lua: &Lua) -> Result<(usize, usize, usize), String> {
    let Some(base) = base_dir(lua) else {
        return Ok((0, 0, 0));
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let gathered = gather(lua, &core, &base)?;
    let Gathered {
        writes,
        orphans,
        live,
        blocked,
        conflicted,
    } = gathered;

    let mut written = 0usize;
    {
        let cache = lua
            .app_data_ref::<AutosaveCache>()
            .ok_or("no autosave cache")?;
        let mut cache = cache.0.borrow_mut();
        // A buffer whose path moved leaves its old recovery behind.
        for old in orphans {
            let _ = crate::state::remove(&base, &format!("autosave/{old}"));
            cache.owner.remove(&old);
        }
        // GC: a buffer that left the registry (killed) takes its recovery
        // copy with it. This is the backstop that covers `[new file]`
        // buffers, which fire no `after-load` and so never get a
        // per-buffer removal callback registered. Only the slot's owner
        // may retire it.
        let dead: Vec<(BufferId, String)> = cache
            .written
            .iter()
            .filter(|(id, _)| !live.contains(id))
            .map(|(id, (hash, _))| (*id, hash.clone()))
            .collect();
        for (id, hash) in dead {
            if cache.owner.get(&hash) == Some(&id) {
                let _ = crate::state::remove(&base, &format!("autosave/{hash}"));
                cache.owner.remove(&hash);
            }
            cache.written.remove(&id);
        }
        for p in writes {
            let header = Header {
                version: AUTOSAVE_VERSION,
                path: p.path,
                origin: p.origin,
            };
            let bytes = encode(&header, &p.contents)?;
            crate::state::write_private(&base, &format!("autosave/{}", p.path_hash), &bytes)
                .map_err(|e| e.to_string())?;
            cache.owner.insert(p.path_hash.clone(), p.id);
            cache.written.insert(p.id, (p.path_hash, p.revision));
            written += 1;
        }
    }
    Ok((written, blocked, conflicted))
}

/// What one pass of the registry decided, before any IO.
struct Gathered {
    writes: Vec<Pending>,
    /// Recovery keys left behind by buffers whose path moved.
    orphans: Vec<String>,
    /// Every buffer still in the registry (drives the dead-buffer GC).
    live: Vec<BufferId>,
    blocked: usize,
    conflicted: usize,
}

/// Walk the registry under a single borrow and decide what to write.
/// All IO happens in [`sweep`] after this returns, because a recovery
/// write must not run while the core is borrowed.
fn gather(lua: &Lua, core: &SharedCore, base: &Path) -> Result<Gathered, String> {
    let mut writes: Vec<Pending> = Vec::new();
    let mut orphans: Vec<String> = Vec::new();
    let mut live: Vec<BufferId> = Vec::new();
    let mut blocked = 0usize;
    let mut conflicted = 0usize;
    // Slots claimed earlier in *this* pass. `owner` is only updated in the
    // write loop, so without this two dirty duplicates of one path would
    // both queue a write to the same key.
    let mut queued: HashMap<String, BufferId> = HashMap::new();
    {
        let cache = lua
            .app_data_ref::<AutosaveCache>()
            .ok_or("no autosave cache")?;
        let cache = cache.0.borrow();
        let c = core.borrow();
        let reg = c.registry.borrow();
        for &id in reg.ids() {
            let Ok(buf) = reg.get(id) else { continue };
            live.push(id);
            // Skips scratch / *special* (no path). Includes `[new file]`
            // buffers: path set, `file_meta` absent.
            let Some(path) = buf.file_path() else {
                continue;
            };
            if !buf.is_modified() {
                continue;
            }
            let path_s = path.display().to_string();
            let path_hash = sha256_hex(&path_s);
            let revision = buf.revision();
            // Exactly one buffer may own a path's recovery slot (Q#AS13):
            // the file is keyed by path, so a second buffer on the same
            // path cannot also be protected — the later write would win on
            // disk while both believed themselves saved.
            let slot_owner = cache
                .owner
                .get(&path_hash)
                .or_else(|| queued.get(&path_hash));
            match slot_owner {
                Some(&owner_id) if owner_id != id => {
                    conflicted += 1;
                    continue;
                }
                Some(_) => {} // we already own the slot
                None => {
                    // Unowned. Never clobber unclaimed crash data (Q#AS12):
                    // a recovery file this session did not write is the
                    // crash copy the user has not recovered yet.
                    if crate::state::exists(base, &format!("autosave/{path_hash}")).unwrap_or(false)
                    {
                        blocked += 1;
                        continue;
                    }
                }
            }
            if let Some((prev_hash, prev_rev)) = cache.written.get(&id) {
                if prev_hash == &path_hash && *prev_rev == revision {
                    continue; // unchanged since its last copy
                }
                if prev_hash != &path_hash {
                    // The path moved: the old key is now an orphan.
                    orphans.push(prev_hash.clone());
                }
            }
            queued.insert(path_hash.clone(), id);
            let len = buf.len();
            let mut contents = vec![0u8; usize::try_from(len).unwrap_or(0)];
            if len > 0 {
                buf.snapshot_rope().slice(0, len, &mut contents);
            }
            writes.push(Pending {
                id,
                path: path_s,
                path_hash,
                revision,
                origin: buf.file_meta().map(Origin::from_meta),
                contents,
            });
        }
    }

    Ok(Gathered {
        writes,
        orphans,
        live,
        blocked,
        conflicted,
    })
}

/// Claim `buffer`'s recovery file for this session (Q#AS12). Called by
/// `recover-file` once the contents are installed in the buffer — the
/// crash data now lives in the buffer, so the copy is no longer
/// irreplaceable.
///
/// It records a `written` entry as well as the ownership, because after a
/// recover the file's contents *are* the buffer's contents. That makes
/// two things right at once: the skip cache correctly declines to rewrite
/// it, and `discard_buffer` can find and retire it — including from a
/// removal callback that fires *after* the buffer is gone, when there is
/// no path left to read (finding).
pub fn adopt(lua: &Lua, id: BufferId) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    let entry = {
        let c = core.borrow();
        let reg = c.registry.borrow();
        let Ok(buf) = reg.get(id) else { return };
        let Some(p) = buf.file_path() else { return };
        (sha256_hex(&p.display().to_string()), buf.revision())
    };
    drop(core);
    if let Some(cache) = lua.app_data_ref::<AutosaveCache>() {
        let mut cache = cache.0.borrow_mut();
        let (hash, revision) = entry;
        // Recovering into this buffer makes it the slot's owner — its
        // contents are now what the file holds. Any previous owner of the
        // slot (a duplicate buffer on the same path) loses the claim and
        // will report as conflicted on the next sweep, which is truthful:
        // the file no longer corresponds to it.
        //
        // Dropping the old owner's skip-cache entry maintains the
        // invariant `written[id] ⟹ owner[hash] == id` (finding). Without
        // it: adopt into B, then kill B without saving. `discard_buffer`
        // frees the slot and deletes the file, but A's stale
        // `written[A] = (hash, revA)` survives — so the next sweep sees A
        // dirty at an unchanged revision, calls it "unchanged since its
        // last copy", and leaves it unprotected until its next edit.
        cache
            .written
            .retain(|&other, (h, _)| other == id || h != &hash);
        cache.owner.insert(hash.clone(), id);
        cache.written.insert(id, (hash, revision));
    }
}

/// Delete the recovery file for `path` and drop every claim and skip-cache
/// entry pointing at it.
///
/// This is the **explicit** release path (`discard-recovery`), so it
/// ignores ownership — the user asked. Clearing the matching `written`
/// entries matters (finding): otherwise a still-dirty buffer would hit the
/// unchanged-`(path_hash, revision)` fast path on the next sweep and go
/// unprotected until its next edit.
pub fn discard_path(lua: &Lua, path: &Path) -> bool {
    let Some(base) = base_dir(lua) else {
        return false;
    };
    let hash = sha256_hex(&path.display().to_string());
    if let Some(cache) = lua.app_data_ref::<AutosaveCache>() {
        let mut cache = cache.0.borrow_mut();
        cache.owner.remove(&hash);
        cache.written.retain(|_, (h, _)| h != &hash);
    }
    discard(&base, path)
}

/// Retire the recovery copy of a specific **buffer** (Q#AS12).
///
/// Keyed by `BufferId`, not by the path captured when the buffer loaded:
/// it considers both the buffer's *current* path key (if it is still
/// live) and the key its last recovery was actually **written** under.
/// Those differ after a rename — an LSP `WorkspaceEdit` changes the path
/// while the `BufferId` stays — and a path-captured callback would leave
/// the real recovery file behind.
///
/// **Only keys this session owns are removed** (Q#AS12, finding). Saving
/// or killing a buffer you reopened after a crash must *not* destroy the
/// unclaimed recovery copy sitting at its path — you never recovered it.
/// Only `recover-file` (which adopts) or an explicit `discard-recovery`
/// releases unclaimed crash data.
pub fn discard_buffer(lua: &Lua, id: BufferId) {
    let Some(base) = base_dir(lua) else {
        return;
    };
    let mut keys: Vec<String> = Vec::new();
    // The key the last sweep (or an adopt) recorded for this buffer. This
    // is the only source that still works once the buffer is gone — a
    // removal callback fires after it has left the registry.
    if let Some(cache) = lua.app_data_ref::<AutosaveCache>()
        && let Some((hash, _)) = cache.0.borrow().written.get(&id)
    {
        keys.push(hash.clone());
    }
    // The buffer's current path, which may have moved since that write.
    if let Some(core) = lua.app_data_ref::<SharedCore>() {
        let c = core.borrow();
        let reg = c.registry.borrow();
        if let Ok(buf) = reg.get(id)
            && let Some(p) = buf.file_path()
        {
            keys.push(sha256_hex(&p.display().to_string()));
        }
    }
    let Some(cache) = lua.app_data_ref::<AutosaveCache>() else {
        return;
    };
    let mut cache = cache.0.borrow_mut();
    // Retire only slots **this buffer** owns. Two guards in one check:
    //   * an unowned slot is unclaimed crash data — saving or killing the
    //     buffer you reopened after a crash must not destroy it (Q#AS12);
    //   * a slot owned by a *different* buffer belongs to that buffer's
    //     recovery — a duplicate buffer on the same path must not retire
    //     it (Q#AS13).
    keys.retain(|h| cache.owner.get(h) == Some(&id));
    for hash in &keys {
        let _ = crate::state::remove(&base, &format!("autosave/{hash}"));
        cache.owner.remove(hash);
    }
    // This buffer's own bookkeeping goes regardless: it is being saved or
    // killed, so any skip-cache entry for it is spent.
    cache.written.remove(&id);
}

/// Every open file buffer that has a recovery file, with its status
/// (Q#AS6). Enumerating in Rust is what makes this cover argv
/// `[new file]` buffers, which fire no hook at all — the Lua reporter
/// never has to know they exist.
///
/// Returns `(fresh_paths, corrupt_count)`.
#[must_use]
pub fn pending(lua: &Lua) -> (Vec<String>, usize) {
    let mut fresh = Vec::new();
    let mut corrupt = 0usize;
    let (Some(base), Some(core)) = (base_dir(lua), lua.app_data_ref::<SharedCore>()) else {
        return (fresh, corrupt);
    };
    // Collect paths first so the guard drops before any IO re-entrancy.
    let paths: Vec<std::path::PathBuf> = {
        let c = core.borrow();
        let reg = c.registry.borrow();
        reg.ids()
            .iter()
            .filter_map(|&id| reg.get(id).ok()?.file_path().map(Path::to_path_buf))
            .collect()
    };
    for p in paths {
        match status(&base, &p) {
            RecoveryStatus::Fresh => fresh.push(p.display().to_string()),
            RecoveryStatus::Corrupt => corrupt += 1,
            RecoveryStatus::None | RecoveryStatus::Stale => {}
        }
    }
    (fresh, corrupt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(origin: Option<Origin>) -> Header {
        Header {
            version: AUTOSAVE_VERSION,
            path: "/tmp/a.rs".into(),
            origin,
        }
    }

    #[test]
    fn envelope_round_trips_arbitrary_bytes() {
        // Contents with newlines AND invalid UTF-8 — the reason we split
        // at the first newline and read bytes, not a String.
        let contents = [0xffu8, b'\n', b'a', 0x00, b'\n'];
        let h = header(Some(Origin {
            mtime_secs: 5,
            mtime_nanos: 7,
            size: 5,
        }));
        let bytes = encode(&h, &contents).unwrap();
        let (got_h, got_c) = decode(&bytes).unwrap();
        assert_eq!(got_h, h);
        assert_eq!(got_c, &contents[..]);
    }

    #[test]
    fn envelope_round_trips_null_origin() {
        // A `[new file]` buffer: no origin meta.
        let h = header(None);
        let bytes = encode(&h, b"draft").unwrap();
        let (got_h, got_c) = decode(&bytes).unwrap();
        assert!(got_h.origin.is_none());
        assert_eq!(got_c, b"draft");
    }

    #[test]
    fn decode_rejects_malformed_and_wrong_version() {
        assert!(decode(b"no newline at all").is_none());
        assert!(decode(b"{not json}\nbody").is_none());
        assert!(decode(b"\nbody").is_none(), "empty header");
        let bad_version = br#"{"version":999,"path":"/x","origin":null}"#;
        let mut bytes = bad_version.to_vec();
        bytes.push(b'\n');
        assert!(decode(&bytes).is_none(), "unrecognized version → corrupt");
    }

    #[test]
    fn key_is_a_valid_state_key() {
        let k = key_for(Path::new("/home/u/a b.rs"));
        assert!(k.starts_with("autosave/"));
        assert!(crate::state::validate_name(&k).is_ok());
    }

    #[test]
    fn status_none_when_no_recovery_file() {
        let dir = std::env::temp_dir().join(format!("pmacs-as-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            status(&dir, Path::new("/tmp/nonexistent.rs")),
            RecoveryStatus::None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn status_corrupt_for_garbage_envelope() {
        let dir = std::env::temp_dir().join(format!("pmacs-as-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = Path::new("/tmp/whatever.rs");
        crate::state::write_private(&dir, &key_for(target), b"garbage, no newline").unwrap();
        assert_eq!(status(&dir, target), RecoveryStatus::Corrupt);
        // And it is discardable.
        assert!(discard(&dir, target));
        assert_eq!(status(&dir, target), RecoveryStatus::None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn new_file_status_is_fresh_until_the_file_appears() {
        let dir = std::env::temp_dir().join(format!("pmacs-as-newfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("draft.rs");
        // origin: null — a `[new file]` buffer.
        let bytes = encode(
            &Header {
                version: AUTOSAVE_VERSION,
                path: target.display().to_string(),
                origin: None,
            },
            b"unsaved draft",
        )
        .unwrap();
        crate::state::write_private(&dir, &key_for(&target), &bytes).unwrap();

        assert_eq!(status(&dir, &target), RecoveryStatus::Fresh, "file absent");
        assert_eq!(
            recover_bytes(&dir, &target).as_deref(),
            Some(&b"unsaved draft"[..])
        );

        // Someone created the file meanwhile → stale, never auto-offered.
        std::fs::write(&target, b"someone else's content").unwrap();
        assert_eq!(status(&dir, &target), RecoveryStatus::Stale);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn existing_file_fresh_until_it_changes_on_disk() {
        let dir = std::env::temp_dir().join(format!("pmacs-as-exist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("a.rs");
        std::fs::write(&target, b"on disk").unwrap();
        let meta = crate::file_io::current_meta(&target).unwrap();
        let bytes = encode(
            &Header {
                version: AUTOSAVE_VERSION,
                path: target.display().to_string(),
                origin: Some(Origin::from_meta(&meta)),
            },
            b"unsaved edits",
        )
        .unwrap();
        crate::state::write_private(&dir, &key_for(&target), &bytes).unwrap();
        assert_eq!(status(&dir, &target), RecoveryStatus::Fresh);

        // Touch the file (different size ⇒ different identity).
        std::fs::write(&target, b"changed underneath us").unwrap();
        assert_eq!(status(&dir, &target), RecoveryStatus::Stale);

        // Delete it entirely → still stale (the base is gone).
        std::fs::remove_file(&target).unwrap();
        assert_eq!(status(&dir, &target), RecoveryStatus::Stale);
        std::fs::remove_dir_all(&dir).ok();
    }
}
