// lua_bindings/diag.rs --- pmacs.diag: diagnostics surface (T M4.6).

//! `pmacs.diag.*` — reads the LSP diagnostic store and pushes diagnostic
//! overlays onto windows. Split out of `lua_bindings.rs` verbatim (audit
//! F-016); behavior unchanged.

use mlua::{Lua, Table, Value};

use super::{BufferIdLua, SharedCore};
use crate::diag::{Diagnostic, DiagnosticSeverity};
use crate::lsp::SharedLspManager;

/// The line starts of the buffer that holds `uri`'s file, when one is
/// open (E7c.3): what turns a diagnostic's current byte span back into
/// the line and column the Lua surface speaks. `None` when no buffer
/// holds the path, and then the published position is what there is.
fn line_starts_for_uri(lua: &Lua, uri: &str) -> Option<Vec<u64>> {
    let path = crate::project_index::uri_to_path(uri)?;
    let core = lua.app_data_ref::<SharedCore>()?;
    let core = core.borrow();
    let registry = core.registry.clone();
    let reg = registry.borrow();
    let id = reg.find_by_path(&path)?;
    let buf = reg.get(id).ok()?;
    let len = buf.len();
    let mut bytes = vec![0u8; len as usize];
    if !bytes.is_empty() {
        buf.snapshot_rope().slice(0, len, &mut bytes);
    }
    let mut starts = vec![0u64];
    starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter_map(|(i, b)| (*b == b'\n').then_some(i as u64 + 1)),
    );
    Some(starts)
}

/// `(line, col)` of `byte` against `starts`.
fn line_col_of(starts: &[u64], byte: u64) -> (u32, u32) {
    let line = starts.partition_point(|&s| s <= byte).saturating_sub(1);
    (line as u32, (byte - starts[line]) as u32)
}

/// The byte of `(line, col)` against `starts`, clamped to the line.
fn byte_of(starts: &[u64], line: u32, col: u32) -> u64 {
    let Some(&start) = starts.get(line as usize) else {
        return u64::MAX;
    };
    start + u64::from(col)
}

/// The position a diagnostic has now: its current span turned into
/// lines and columns of the buffer (`starts`), or the published one.
fn current_position(d: &Diagnostic, starts: Option<&[u64]>) -> (u32, u32, u32, u32) {
    match (d.span, starts) {
        (Some((lo, hi)), Some(starts)) => {
            let (sl, sc) = line_col_of(starts, lo);
            let (el, ec) = line_col_of(starts, hi.max(lo));
            (sl, sc, el, ec)
        }
        _ => (d.start_line, d.start_col, d.end_line, d.end_col),
    }
}

fn diagnostic_to_lua(lua: &Lua, d: &Diagnostic, starts: Option<&[u64]>) -> mlua::Result<Table> {
    let (start_line, start_col, end_line, end_col) = current_position(d, starts);
    let t = lua.create_table_with_capacity(0, 8)?;
    t.set("severity", d.severity.label())?;
    t.set("severity_code", d.severity as i64)?;
    t.set("message", d.message.as_str())?;
    if let Some(s) = &d.source {
        t.set("source", s.as_str())?;
    }
    if let Some(c) = &d.code {
        t.set("code", c.as_str())?;
    }
    let range = lua.create_table_with_capacity(0, 4)?;
    let start = lua.create_table_with_capacity(0, 2)?;
    start.set("line", start_line)?;
    start.set("character", start_col)?;
    let end = lua.create_table_with_capacity(0, 2)?;
    end.set("line", end_line)?;
    end.set("character", end_col)?;
    range.set("start", start)?;
    range.set("end", end)?;
    t.set("range", range)?;
    t.set("start_line", start_line)?;
    t.set("start_col", start_col)?;
    t.set("end_line", end_line)?;
    t.set("end_col", end_col)?;
    Ok(t)
}

/// Install `pmacs.diag.*` (T M4.6).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings; splitting fragments a coherent surface"
)]
pub fn install_diag(
    lua: &Lua,
    manager: &SharedLspManager,
    theme: &crate::highlight::ThemeHandle,
) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let diag_mod = lua.create_table()?;

    {
        let m = manager.clone();
        diag_mod.set(
            "list",
            lua.create_function(move |lua, uri: String| {
                let starts = line_starts_for_uri(lua, &uri);
                let store_handle = m.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let diags = guard.for_uri(&uri);
                let out = lua.create_table_with_capacity(diags.len(), 0)?;
                for (i, d) in diags.iter().enumerate() {
                    out.set(i + 1, diagnostic_to_lua(lua, d, starts.as_deref())?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "count",
            lua.create_function(move |_, uri: Option<String>| {
                let store_handle = m.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let n = match uri {
                    Some(u) => guard.count_for(&u),
                    None => {
                        guard.totals().0 + guard.totals().1 + guard.totals().2 + guard.totals().3
                    }
                };
                Ok(n)
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        diag_mod.set(
            "totals",
            lua.create_function(move |lua, ()| {
                let store_handle = mgr.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let (errs, warns, infos, hints) = guard.totals();
                let table = lua.create_table_with_capacity(0, 4)?;
                table.set("error", errs)?;
                table.set("warning", warns)?;
                table.set("info", infos)?;
                table.set("hint", hints)?;
                Ok(table)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "next",
            lua.create_function(
                move |lua, (uri, line, col, wrap): (String, u32, u32, Option<bool>)| {
                    let starts = line_starts_for_uri(lua, &uri);
                    let store_handle = m.borrow().diag_store();
                    let guard = store_handle.lock().expect("diag store mutex poisoned");
                    // E7c.3: by current bytes when the document's
                    // diagnostics carry them, else by published position.
                    let carried = starts.is_some()
                        && !guard.for_uri(&uri).is_empty()
                        && guard.for_uri(&uri).iter().all(|d| d.span.is_some());
                    let found = if carried {
                        let byte = byte_of(starts.as_deref().unwrap_or(&[]), line, col);
                        guard.next_after_byte(&uri, byte)
                    } else {
                        guard.next_after(&uri, line, col)
                    }
                    .or_else(|| {
                        if wrap.unwrap_or(true) {
                            if carried {
                                guard.for_uri(&uri).iter().min_by_key(|d| d.span)
                            } else {
                                guard.first_for(&uri)
                            }
                        } else {
                            None
                        }
                    });
                    match found {
                        Some(d) => Ok(Value::Table(diagnostic_to_lua(lua, d, starts.as_deref())?)),
                        None => Ok(Value::Nil),
                    }
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "previous",
            lua.create_function(
                move |lua, (uri, line, col, wrap): (String, u32, u32, Option<bool>)| {
                    let starts = line_starts_for_uri(lua, &uri);
                    let store_handle = m.borrow().diag_store();
                    let guard = store_handle.lock().expect("diag store mutex poisoned");
                    let carried = starts.is_some()
                        && !guard.for_uri(&uri).is_empty()
                        && guard.for_uri(&uri).iter().all(|d| d.span.is_some());
                    let found = if carried {
                        let byte = byte_of(starts.as_deref().unwrap_or(&[]), line, col);
                        guard.previous_before_byte(&uri, byte)
                    } else {
                        guard.previous_before(&uri, line, col)
                    }
                    .or_else(|| {
                        if wrap.unwrap_or(true) {
                            if carried {
                                guard.for_uri(&uri).iter().max_by_key(|d| d.span)
                            } else {
                                guard.last_for(&uri)
                            }
                        } else {
                            None
                        }
                    });
                    match found {
                        Some(d) => Ok(Value::Table(diagnostic_to_lua(lua, d, starts.as_deref())?)),
                        None => Ok(Value::Nil),
                    }
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "uris",
            lua.create_function(move |lua, ()| {
                let store_handle = m.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let uris: Vec<String> = guard.uris().map(str::to_owned).collect();
                let out = lua.create_table_with_capacity(uris.len(), 0)?;
                for (i, u) in uris.iter().enumerate() {
                    out.set(i + 1, u.as_str())?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // Helper for tests / Lua-driven flows: clear the
        // diagnostics for a URI.
        let m = manager.clone();
        diag_mod.set(
            "clear",
            lua.create_function(move |_, uri: String| {
                let store_handle = m.borrow().diag_store();
                let mut guard = store_handle.lock().expect("diag store mutex poisoned");
                guard.clear(&uri);
                Ok(())
            })?,
        )?;
    }

    {
        // Look up the severity table-of-strings the rest of the
        // surface uses; returned as a constant table for callers
        // that prefer a tagged value.
        diag_mod.set("severity", {
            let t = lua.create_table_with_capacity(0, 4)?;
            t.set("error", DiagnosticSeverity::Error as i64)?;
            t.set("warning", DiagnosticSeverity::Warning as i64)?;
            t.set("info", DiagnosticSeverity::Information as i64)?;
            t.set("hint", DiagnosticSeverity::Hint as i64)?;
            t
        })?;
    }

    // Sibling of `pmacs.lsp._attach_style` and
    // `pmacs.parse._attach_highlight`: pushes a `DiagnosticView`
    // overlay on the active window keyed under `uri`, so the TUI
    // grid renderer paints diagnostic underlines for buffers that
    // have an LSP server publishing diagnostics. Lua callers dedup
    // per buffer; double-attach stacks duplicate overlays.
    {
        let m = manager.clone();
        let th = theme.clone();
        diag_mod.set(
            "_attach_view",
            lua.create_function(move |lua, (id, uri): (BufferIdLua, String)| {
                let store_handle = m.borrow().diag_store();
                let core = lua
                    .app_data_ref::<SharedCore>()
                    .ok_or_else(|| mlua::Error::external("editor core not yet installed"))?;
                let mut core_borrow = core.borrow_mut();
                let win = core_borrow.active_window_mut();
                if win.buffer_id != id.0 {
                    return Err(mlua::Error::external(format!(
                        "active window's buffer is not {:?}",
                        id.0
                    )));
                }
                // Themes Q#TH9: the Lua attachment path threads the
                // shared theme so ui.diag.* faces reach the squiggles
                // and gutter signs.
                let overlay = crate::diag::DiagnosticView::new(uri, store_handle, Some(th.clone()));
                win.push_overlay(Box::new(overlay));
                Ok(true)
            })?,
        )?;
    }

    // dired Stage 2a §5 step 6 — re-root every attached
    // `DiagnosticView` from `old_uri` to `new_uri` after a rename.
    //
    // `DiagnosticView.uri` is set once at construction and is private,
    // and `View` has no downcast, so nothing outside `diag.rs` can
    // reach it; the `View::rename_resource` hook is the seam. The sweep
    // walks EVERY window, which is what `_attach_view` above cannot do
    // — it takes the active window and errors otherwise — so a passive
    // split that already holds the overlay is re-rooted too. It mutates
    // in place, so each overlay keeps its position in the window's
    // composition order; a remove-and-re-push would move the underline
    // to the end of the stack and pass a one-window test anyway.
    {
        diag_mod.set(
            "_rename_resource",
            lua.create_function(move |lua, (old_uri, new_uri): (String, String)| {
                let Some(core) = lua.app_data_ref::<SharedCore>() else {
                    return Ok(false);
                };
                core.borrow_mut()
                    .rename_resource_in_views(&old_uri, &new_uri);
                Ok(true)
            })?,
        )?;
    }

    pmacs.set("diag", diag_mod)?;
    Ok(())
}
