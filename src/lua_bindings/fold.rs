// lua_bindings/fold.rs --- pmacs.fold: the code-folding Lua surface (Arc 6).

//! `pmacs.fold.*` --- the Lua surface over [`crate::fold`]. Installed
//! entirely from Rust (like `pmacs.config`), after `make_syntax_registry`
//! so the tree-consuming operations can reach the parse tree via app-data.
//!
//! ```lua
//! -- data API (explicit buffer, no ambient resolution):
//! pmacs.fold.fold(buffer, { start = ..., ["end"] = ... })
//! pmacs.fold.unfold(buffer, { start = ..., ["end"] = ... })
//! pmacs.fold.folds(buffer)          -- -> { {start=,["end"]=}, ... }
//! pmacs.fold.toggle(buffer, pos)
//!
//! -- interactive helpers the fold.lua commands drive (explicit buffer +
//! -- point resolved from the invoking frontend):
//! pmacs.fold.close(buffer, pos)     -- close innermost open
//! pmacs.fold.open(buffer, pos)      -- open outermost closed
//! pmacs.fold.cycle(buffer, pos)     -- org-TAB toggle
//! pmacs.fold.close_all(buffer)      -- top-level regions only
//! pmacs.fold.open_all(buffer)
//! ```
//!
//! Fold *creation* refuses against an absent or stale parse tree (Q#FD10)
//! and validates the buffer kind / UTF-8 boundaries / >= 1-hidden-line
//! rule (Q#FD11); a rejection reports on the status line and returns
//! `false`. Folding a range containing the invoking frontend's point moves
//! that point to the head line (Q#FD3).

use std::sync::{Arc, Mutex};

use mlua::{Lua, Table};
use pmacs_protocol::{BufferId, ByteRange};

use super::{
    BufferIdLua, SharedCore, resolve, resolve_mut, u64_from_lua, with_registry, with_registry_mut,
};
use crate::buffer::Buffer;
use crate::fold::{self, FoldStore, SharedFoldRegistry};
use crate::syntax::{ParseTreeBundle, SharedSyntaxRegistry};

/// Install `pmacs.fold` over `fold_registry` (the same `Rc` the core owns).
#[allow(
    clippy::too_many_lines,
    reason = "linear per-function registration of the pmacs.fold surface, \
              mirroring install_config; splitting fragments the wiring"
)]
pub fn install_fold(lua: &Lua, fold_registry: &SharedFoldRegistry) -> mlua::Result<()> {
    let fold_mod = lua.create_table()?;

    // ---- data API ---------------------------------------------------------

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "fold",
            lua.create_function(
                move |lua, (buf, range): (BufferIdLua, Table)| -> mlua::Result<bool> {
                    let id = buf.id();
                    let requested = range_from_table(&range)?;
                    let Some(bytes) = document_bytes(lua, id)? else {
                        set_status(lua, "fold rejected: not a document buffer");
                        return Ok(false);
                    };
                    if requested.start > bytes.len() as u64
                        || requested.end > bytes.len() as u64
                        || !is_char_boundary(&bytes, requested.start)
                        || !is_char_boundary(&bytes, requested.end)
                    {
                        set_status(lua, "fold rejected: out of bounds or not a char boundary");
                        return Ok(false);
                    }
                    let Some(normalized) = fold::normalize_arbitrary_range(&bytes, requested)
                    else {
                        set_status(lua, "fold rejected: range hides no full line");
                        return Ok(false);
                    };
                    let store = store_for(lua, &reg, id)?;
                    let added = lock(&store).insert(normalized);
                    Ok(added)
                },
            )?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "unfold",
            lua.create_function(
                move |lua, (buf, range): (BufferIdLua, Table)| -> mlua::Result<bool> {
                    let id = buf.id();
                    let requested = range_from_table(&range)?;
                    let Some(store) = reg.store(id) else {
                        return Ok(false);
                    };
                    // Accept an exact stored range (the `folds()` round-trip)
                    // or an arbitrary range that normalizes to a stored one.
                    if lock(&store).remove(requested) {
                        return Ok(true);
                    }
                    if let Ok(Some(bytes)) = document_bytes(lua, id)
                        && let Some(normalized) =
                            fold::normalize_arbitrary_range(&bytes, requested)
                    {
                        return Ok(lock(&store).remove(normalized));
                    }
                    Ok(false)
                },
            )?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "folds",
            lua.create_function(move |lua, buf: BufferIdLua| -> mlua::Result<Table> {
                let out = lua.create_table()?;
                for (i, r) in reg.folds(buf.id()).into_iter().enumerate() {
                    out.set(i + 1, range_to_table(lua, r)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "toggle",
            lua.create_function(
                move |lua, (buf, pos): (BufferIdLua, i64)| -> mlua::Result<bool> {
                    let id = buf.id();
                    let p = u64_from_lua(pos)?;
                    // A stored fold at the point unfolds without needing a tree.
                    if let Some(store) = reg.store(id) {
                        let mut s = lock(&store);
                        if !s.containing(p).is_empty() {
                            s.unfold_containing(p);
                            return Ok(true);
                        }
                    }
                    let Some(bundle) = bundle_or_status(lua, id) else {
                        return Ok(false);
                    };
                    let store = store_for(lua, &reg, id)?;
                    match fold::toggle_at(&mut lock(&store), &bundle, p) {
                        fold::ToggleOutcome::Folded(r) => {
                            maybe_move_point(lua, id, r);
                            Ok(true)
                        }
                        fold::ToggleOutcome::Unfolded(_) => Ok(true),
                        fold::ToggleOutcome::Nothing => {
                            set_status(lua, "nothing foldable here");
                            Ok(false)
                        }
                    }
                },
            )?,
        )?;
    }

    // ---- interactive helpers (state-aware; driven by fold.lua) ------------

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "close",
            lua.create_function(
                move |lua, (buf, pos): (BufferIdLua, i64)| -> mlua::Result<bool> {
                    let id = buf.id();
                    let p = u64_from_lua(pos)?;
                    let Some(bundle) = bundle_or_status(lua, id) else {
                        return Ok(false);
                    };
                    let store = store_for(lua, &reg, id)?;
                    if let Some(r) = fold::close_at(&mut lock(&store), &bundle, p) {
                        maybe_move_point(lua, id, r);
                        Ok(true)
                    } else {
                        set_status(lua, "no more folds to close here");
                        Ok(false)
                    }
                },
            )?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "open",
            lua.create_function(
                move |_, (buf, pos): (BufferIdLua, i64)| -> mlua::Result<bool> {
                    let id = buf.id();
                    let p = u64_from_lua(pos)?;
                    let Some(store) = reg.store(id) else {
                        return Ok(false);
                    };
                    Ok(fold::open_at(&mut lock(&store), p).is_some())
                },
            )?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "cycle",
            lua.create_function(
                move |lua, (buf, pos): (BufferIdLua, i64)| -> mlua::Result<bool> {
                    let id = buf.id();
                    let p = u64_from_lua(pos)?;
                    let Some(bundle) = bundle_or_status(lua, id) else {
                        return Ok(false);
                    };
                    let store = store_for(lua, &reg, id)?;
                    match fold::cycle_at(&mut lock(&store), &bundle, p) {
                        fold::CycleOutcome::Closed(r) => {
                            maybe_move_point(lua, id, r);
                            Ok(true)
                        }
                        fold::CycleOutcome::OpenedAll(_) => Ok(true),
                        fold::CycleOutcome::Nothing => {
                            set_status(lua, "nothing foldable here");
                            Ok(false)
                        }
                    }
                },
            )?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "close_all",
            lua.create_function(move |lua, buf: BufferIdLua| -> mlua::Result<i64> {
                let id = buf.id();
                let Some(bundle) = bundle_or_status(lua, id) else {
                    return Ok(0);
                };
                let targets = fold::top_level_fold_targets(&bundle);
                let store = store_for(lua, &reg, id)?;
                let mut s = lock(&store);
                let mut n = 0i64;
                for t in targets {
                    if s.insert(t) {
                        n += 1;
                    }
                }
                Ok(n)
            })?,
        )?;
    }

    {
        let reg = fold_registry.clone();
        fold_mod.set(
            "open_all",
            lua.create_function(move |_, buf: BufferIdLua| -> mlua::Result<bool> {
                match reg.store(buf.id()) {
                    Some(store) => Ok(lock(&store).clear()),
                    None => Ok(false),
                }
            })?,
        )?;
    }

    let pmacs: Table = lua.globals().get("pmacs")?;
    pmacs.set("fold", fold_mod)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock(store: &Arc<Mutex<FoldStore>>) -> std::sync::MutexGuard<'_, FoldStore> {
    store.lock().expect("fold store mutex poisoned")
}

fn range_from_table(t: &Table) -> mlua::Result<ByteRange> {
    let start = u64_from_lua(t.raw_get::<i64>("start")?)?;
    let end = u64_from_lua(t.raw_get::<i64>("end")?)?;
    Ok(ByteRange { start, end })
}

fn range_to_table(lua: &Lua, r: ByteRange) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    // Byte offsets are always well within `i64` range; the Lua integer
    // type is `i64`.
    t.set("start", r.start.cast_signed())?;
    t.set("end", r.end.cast_signed())?;
    Ok(t)
}

/// The buffer's bytes if it is a normal document buffer, or `None` if it is
/// read-only (a terminal identity buffer or other non-document buffer —
/// the Q#FD11 "normal document buffer" guard).
fn document_bytes(lua: &Lua, buf: BufferId) -> mlua::Result<Option<Vec<u8>>> {
    with_registry(lua, |r| {
        let buffer = resolve(r, buf)?;
        if buffer.is_read_only() {
            return Ok(None);
        }
        Ok(Some(buffer_bytes(buffer)))
    })
}

fn buffer_bytes(buf: &Buffer) -> Vec<u8> {
    let len = buf.len();
    let mut bytes = vec![0u8; len as usize];
    buf.snapshot_rope().slice(0, len, &mut bytes);
    bytes
}

fn is_char_boundary(bytes: &[u8], pos: u64) -> bool {
    let p = pos as usize;
    p == 0 || p == bytes.len() || (p < bytes.len() && (bytes[p] & 0xC0) != 0x80)
}

/// The get-or-attach store handle for `buf`, materializing the store and
/// attaching its translator view on first use.
fn store_for(
    lua: &Lua,
    reg: &SharedFoldRegistry,
    buf: BufferId,
) -> mlua::Result<Arc<Mutex<FoldStore>>> {
    with_registry_mut(lua, |r| {
        let buffer = resolve_mut(r, buf)?;
        Ok(reg.store_or_attach(buffer))
    })
}

/// The settled parse bundle for `buf`, or `None` after reporting the
/// stale/absent-tree rejection on the status line (Q#FD10).
fn bundle_or_status(lua: &Lua, buf: BufferId) -> Option<Arc<ParseTreeBundle>> {
    match settled_bundle(lua, buf) {
        Ok(bundle) => Some(bundle),
        Err(reason) => {
            set_status(lua, reason);
            None
        }
    }
}

fn settled_bundle(lua: &Lua, buf: BufferId) -> Result<Arc<ParseTreeBundle>, &'static str> {
    let syntax = lua
        .app_data_ref::<SharedSyntaxRegistry>()
        .ok_or("fold: no syntax registry")?;
    let handle = syntax.view(buf).ok_or("fold: no parse for this buffer")?;
    if handle.pending_edit_count() > 0 {
        return Err("fold: parse is stale (edits pending); try again");
    }
    handle.current().ok_or("fold: no parse yet; try again")
}

/// Set the editor status line (rejection reporting).
fn set_status(lua: &Lua, msg: &str) {
    if let Some(core) = lua.app_data_ref::<SharedCore>() {
        core.borrow_mut().status = msg.to_string();
    }
}

/// Move the invoking frontend's point to the head line when a just-folded
/// range `r` contains it (Q#FD3). No-op if the folded buffer is not the
/// active one or the point is outside the fold.
fn maybe_move_point(lua: &Lua, buf: BufferId, r: ByteRange) {
    if let Some(core) = lua.app_data_ref::<SharedCore>() {
        let mut c = core.borrow_mut();
        if c.active_buffer_id() == buf {
            let point = c.active_window().cursor;
            // `(start, end]` containment: a point strictly inside the fold
            // moves to `start` (the end of the visible head line).
            if r.start < point && point <= r.end {
                c.set_cursor_byte(r.start);
            }
        }
    }
}
