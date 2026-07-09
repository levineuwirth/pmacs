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
//! Framing: docs/autosave-recovery-framing.md.

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

/// Skip cache: `BufferId → (path_hash, revision)` (Q#AS8).
///
/// Keyed on the **path hash as well as the revision**, not the revision
/// alone: a buffer keeps its `BufferId` across a path change (an LSP
/// `WorkspaceEdit` rename calls `set_buffer_path`), so a revision-only
/// cache would skip the write, never create the recovery file under the
/// new key, and orphan the old one.
#[derive(Default)]
pub struct AutosaveCache(RefCell<HashMap<BufferId, (String, u64)>>);

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
/// # Errors
/// A state-write failure. Individual buffers never abort the pass.
pub fn sweep(lua: &Lua) -> Result<usize, String> {
    let Some(base) = base_dir(lua) else {
        return Ok(0);
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();

    // Gather under a borrow; do all IO after releasing it.
    let mut writes: Vec<Pending> = Vec::new();
    let mut orphans: Vec<String> = Vec::new();
    let mut live: Vec<BufferId> = Vec::new();
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
            if let Some((prev_hash, prev_rev)) = cache.get(&id) {
                if prev_hash == &path_hash && *prev_rev == revision {
                    continue; // unchanged since its last copy
                }
                if prev_hash != &path_hash {
                    // The path moved: the old key is now an orphan.
                    orphans.push(prev_hash.clone());
                }
            }
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

    for old in orphans {
        let _ = crate::state::remove(&base, &format!("autosave/{old}"));
    }

    let mut written = 0usize;
    {
        let cache = lua
            .app_data_ref::<AutosaveCache>()
            .ok_or("no autosave cache")?;
        let mut cache = cache.0.borrow_mut();
        // Drop entries for buffers that no longer exist.
        cache.retain(|id, _| live.contains(id));
        for p in writes {
            let header = Header {
                version: AUTOSAVE_VERSION,
                path: p.path,
                origin: p.origin,
            };
            let bytes = encode(&header, &p.contents)?;
            crate::state::write_private(&base, &format!("autosave/{}", p.path_hash), &bytes)
                .map_err(|e| e.to_string())?;
            cache.insert(p.id, (p.path_hash, p.revision));
            written += 1;
        }
    }
    Ok(written)
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
