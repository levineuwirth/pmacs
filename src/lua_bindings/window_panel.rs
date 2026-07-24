// window_panel.rs --- `pmacs.window` display policy + side windows.

//! The Lua surface of the bottom-panel arc (Q#BP11): `display`,
//! `display_file`, `quit`, `panel`, `params` / `set_params`, `resize`,
//! and `display_target`.
//!
//! # Where the transaction lives
//!
//! [`crate::editor_core::EditorCore::display_buffer`] is **Phase 1**: it
//! picks a target under Q#BP3, installs the buffer, and reports what must
//! happen next. It contains no Lua. This module is **Phase 2** (Q#BP4):
//! activate the target, fire the lifecycle hook so overlays reattach and
//! saveplace / recentf / syntax / LSP observe the right active window,
//! run panel reconciliation (a hook may resize, close, or replace the
//! target), then **revalidate both window ids** and apply the final-focus
//! matrix.
//!
//! Two corrections that matrix encodes, both of which an earlier revision
//! of the framing got wrong:
//!
//! * `select = true` **keeps the target selected** — restoring the saved
//!   window unconditionally would erase the request outright;
//! * `select = false` restores a saved window **even when it is the
//!   panel** — a passive display invoked from a focused panel must not
//!   blur it.
//!
//! # What Lua may not write
//!
//! `side` is immutable after placement (Q#BP2a), and `quit_action` /
//! `origin_document` are implementation-owned (Q#BP2c): `params` reports
//! them for diagnostics, `set_params` refuses them. Lua therefore cannot
//! forge a window id, a buffer restore chain, or stale cursor state.

use mlua::{Lua, Table, Value};

use super::{BufferIdLua, SharedCore, config_u32, run_hook_if_defined};
use crate::editor_core::{DisplayOutcome, DisplayRequest, HookKind, QuitOutcome};
use crate::protocol::FrontendId;
use crate::window::{DEFAULT_PANEL_ROWS, MIN_WINDOW_OUTER_ROWS, Side, WindowId};

/// The frontend a `pmacs.window.*` call acts for.
///
/// An interactive command carries authenticated origin; a programmatic
/// call falls back to the ambient active frontend, exactly as the
/// terminal surface does.
pub(crate) fn acting_frontend(lua: &Lua, core: &SharedCore) -> FrontendId {
    lua.app_data_ref::<crate::editor::InteractiveCommandOrigin>()
        .and_then(|origin| origin.current())
        .unwrap_or_else(|| core.borrow().active_frontend_key())
}

/// Run the panel-reconciliation transaction from a Lua-owning context
/// (Q#BP2b).
///
/// The core half is pure; releasing a terminal controller needs the
/// manager, which the terminal module publishes as Lua app data for
/// exactly this reason. A bare core without one still reconciles — it
/// simply has no controller to release.
pub(crate) fn reconcile_panel_layout(lua: &Lua, core: &SharedCore, fid: FrontendId) {
    let outcome = core.borrow_mut().reconcile_panel_layout_core(fid);
    let Some(window_id) = outcome.released_terminal else {
        return;
    };
    let Some(manager) = lua.app_data_ref::<crate::terminal::SharedTerminalManager>() else {
        return;
    };
    let buffer_id = core
        .borrow()
        .windows
        .get(&window_id)
        .map(|window| window.buffer_id);
    if let Some(buffer_id) = buffer_id {
        let _ = manager
            .borrow_mut()
            .release_controller(crate::terminal::TerminalViewKey::new(
                fid, window_id, buffer_id,
            ));
    }
}

/// A window is "visible" for the final-focus matrix when it is live in
/// this frontend's layout and not a derived-hidden panel (Q#BP2b).
fn visible(core: &SharedCore, fid: FrontendId, win: WindowId) -> bool {
    let core = core.borrow();
    let Some(view) = core.views.get(&fid) else {
        return false;
    };
    if !view.layout.iter_ids().contains(&win) {
        return false;
    }
    !(view.panel_hidden
        && core
            .windows
            .get(&win)
            .is_some_and(crate::window::Window::is_side))
}

/// Phase 2 of the display transaction (Q#BP4).
fn complete_display(
    lua: &Lua,
    core: &SharedCore,
    fid: FrontendId,
    outcome: DisplayOutcome,
    fire: HookKind,
) -> mlua::Result<()> {
    core.borrow_mut().focus_window(fid, outcome.target);
    match fire {
        HookKind::AfterSwitch => {
            run_hook_if_defined(lua, "buffer.after-switch", mlua::MultiValue::new());
        }
        HookKind::AfterLoad => {
            run_hook_if_defined(lua, "buffer.after-load", mlua::MultiValue::new());
        }
        HookKind::None => {}
    }
    // A hook may have resized, closed, or replaced the target, so
    // reconcile BEFORE the final-focus decision reads visibility.
    reconcile_panel_layout(lua, core, fid);

    let target_ok = visible(core, fid, outcome.target);
    let saved_ok = visible(core, fid, outcome.saved_active);
    let final_focus = match (outcome.select, target_ok, saved_ok) {
        // `select = true` KEEPS the target selected.
        (true, true, _) | (false, true, false) => Some(outcome.target),
        // `select = false` restores the saved window even when it is the
        // panel — a passive display from a focused panel must not blur it.
        (true, false, true) | (false, _, true) => Some(outcome.saved_active),
        // Both ids died with the hook: fall back to the non-side target
        // rule rather than leaving focus on a dead window.
        _ => None,
    };
    let resolved = match final_focus {
        Some(win) => win,
        None => core
            .borrow()
            .non_side_target(fid)
            .map_err(mlua::Error::runtime)?,
    };
    core.borrow_mut().focus_window(fid, resolved);
    Ok(())
}

/// Parse the shared `{side, window, height, dedicated, select}` option
/// table.
fn parse_request(
    lua: &Lua,
    core: &SharedCore,
    fid: FrontendId,
    buffer_id: crate::buffer::BufferId,
    opts: Option<Table>,
) -> mlua::Result<DisplayRequest> {
    let mut request = DisplayRequest::new(buffer_id);
    let Some(opts) = opts else {
        return Ok(request);
    };
    if let Some(side) = opts.get::<Option<String>>("side")? {
        request.side = Some(Side::from_name(&side).ok_or_else(|| {
            mlua::Error::runtime(format!(
                "pmacs.window.display: unsupported side {side:?} (only \"bottom\" ships)"
            ))
        })?);
    }
    if let Some(raw) = opts.get::<Option<u64>>("window")? {
        request.window = Some(lookup_window(core, fid, raw)?);
    }
    if let Some(height) = opts.get::<Option<u32>>("height")? {
        request.height = Some(height);
    }
    if let Some(dedicated) = opts.get::<Option<bool>>("dedicated")? {
        request.dedicated = Some(dedicated);
    }
    if let Some(select) = opts.get::<Option<bool>>("select")? {
        request.select = Some(select);
    }
    // The setting is resolved against the buffer being displayed, and
    // only consumed when the slot is actually CREATED (Q#BP3).
    request.default_panel_rows = config_u32(
        lua,
        "window.panel-height",
        Some(buffer_id),
        DEFAULT_PANEL_ROWS,
    )
    .max(MIN_WINDOW_OUTER_ROWS);
    Ok(request)
}

/// The ACTING frontend's selected window.
///
/// Not `active_window_id()`, which resolves through the ambient active
/// frontend: every other id in this module is `fid`-scoped, and the two
/// only coincide because dispatch happens to set `active_frontend` first.
fn selected_window(core: &SharedCore, fid: FrontendId) -> mlua::Result<WindowId> {
    core.borrow()
        .views
        .get(&fid)
        .map(|view| view.active)
        .ok_or_else(|| mlua::Error::runtime("pmacs.window: acting frontend has no layout"))
}

/// Resolve a raw Lua window id, refusing one that is not live in the
/// acting frontend's layout (Q#BP11).
fn lookup_window(core: &SharedCore, fid: FrontendId, raw: u64) -> mlua::Result<WindowId> {
    let core = core.borrow();
    let view = core
        .views
        .get(&fid)
        .ok_or_else(|| mlua::Error::runtime("pmacs.window: acting frontend has no layout"))?;
    view.layout
        .iter_ids()
        .into_iter()
        .find(|id| id.raw() == raw)
        .ok_or_else(|| {
            mlua::Error::runtime(format!(
                "pmacs.window: window {raw} is not live in this frontend's layout"
            ))
        })
}

/// A parsed adopter placement request (Q#BP11b).
///
/// `listview`, compile, and terminal all take the same strict
/// `display = "current" | "panel"` value. In Stages 1–2 omission means
/// `"current"`; Stage 3 flips omission to `"panel"`. Explicit
/// `"current"` always preserves the adopter's pre-arc selected-window
/// behavior and is the user-facing opt-out from that flip.
pub(crate) enum AdopterPlacement {
    /// Today's behavior: the raw switch into the frontend's active
    /// window, deliberately bypassing display-policy dedication.
    Current,
    /// The bottom panel.
    Panel,
    /// An exact target window.
    Window(WindowId),
}

/// Parse an adopter's placement **before** it creates a buffer, session,
/// process, or wrapper — so an unknown value leaves nothing to roll back.
///
/// # Errors
/// An unknown `display` value, a `window` combined with
/// `display = "panel"`, or a window id that is not live in the acting
/// frontend's layout.
pub(crate) fn parse_adopter_placement(
    core: &SharedCore,
    fid: FrontendId,
    operation: &str,
    display: Option<&str>,
    window: Option<u64>,
) -> mlua::Result<AdopterPlacement> {
    let display = match display {
        None | Some("current") => AdopterPlacement::Current,
        Some("panel") => AdopterPlacement::Panel,
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "{operation}: unknown display {other:?} (expected \"current\" or \"panel\")"
            )));
        }
    };
    match (window, &display) {
        (Some(_), AdopterPlacement::Panel) => Err(mlua::Error::runtime(format!(
            "{operation}: `window` and `display = \"panel\"` are mutually exclusive"
        ))),
        (Some(raw), _) => Ok(AdopterPlacement::Window(lookup_window(core, fid, raw)?)),
        (None, _) => Ok(display),
    }
}

/// Install `buffer_id` per `placement`, returning Phase 1's outcome
/// (Q#BP11b).
///
/// `Current` keeps the pre-arc raw switch: it is the deliberate escape
/// hatch every existing adopter caller already relies on, and it does not
/// consult display-policy dedication.
///
/// # Errors
/// Any placement failure. The caller owns its own session/buffer
/// rollback, and inspects `created_side` to remove a wrapper this
/// transaction created.
pub(crate) fn place_adopter_buffer(
    lua: &Lua,
    core: &SharedCore,
    fid: FrontendId,
    buffer_id: crate::buffer::BufferId,
    placement: &AdopterPlacement,
    select: bool,
) -> mlua::Result<DisplayOutcome> {
    if matches!(placement, AdopterPlacement::Current) {
        let mut borrowed = core.borrow_mut();
        borrowed
            .switch_active_buffer_for(fid, buffer_id)
            .map_err(mlua::Error::runtime)?;
        let target = borrowed
            .views
            .get(&fid)
            .map(|view| view.active)
            .ok_or_else(|| {
                mlua::Error::runtime("adopter placement: acting frontend has no active window")
            })?;
        return Ok(DisplayOutcome {
            target,
            saved_active: target,
            select: true,
            created_side: false,
        });
    }
    let mut request = DisplayRequest::new(buffer_id);
    match placement {
        AdopterPlacement::Panel => request.side = Some(Side::Bottom),
        AdopterPlacement::Window(window) => request.window = Some(*window),
        AdopterPlacement::Current => unreachable!("handled above"),
    }
    request.select = Some(select);
    request.default_panel_rows = config_u32(
        lua,
        "window.panel-height",
        Some(buffer_id),
        DEFAULT_PANEL_ROWS,
    )
    .max(MIN_WINDOW_OUTER_ROWS);
    core.borrow_mut()
        .display_buffer(fid, &request)
        .map_err(mlua::Error::runtime)
}

/// Phase 2 for an adopter that had to interleave its own work (claiming a
/// terminal controller, seating a cursor) between placement and the hook.
///
/// # Errors
/// Propagates the final-focus resolution error when both window ids died
/// inside the hook.
pub(crate) fn finish_adopter_placement(
    lua: &Lua,
    core: &SharedCore,
    fid: FrontendId,
    outcome: DisplayOutcome,
) -> mlua::Result<()> {
    complete_display(lua, core, fid, outcome, HookKind::AfterSwitch)
}

/// Install the bottom-panel surface onto the existing `pmacs.window`
/// table.
#[allow(
    clippy::too_many_lines,
    reason = "one flat list of bindings, each following the same \
              acting-frontend / Rc-borrow shape; splitting them fragments \
              a coherent surface"
)]
pub(crate) fn install(lua: &Lua, core: &SharedCore, win: &Table) -> mlua::Result<()> {
    {
        let cc = core.clone();
        win.set(
            "display",
            lua.create_function(
                move |lua, (buffer, opts): (BufferIdLua, Option<Table>)| -> mlua::Result<u64> {
                    let fid = acting_frontend(lua, &cc);
                    let request = parse_request(lua, &cc, fid, buffer.0, opts)?;
                    let outcome = cc
                        .borrow_mut()
                        .display_buffer(fid, &request)
                        .map_err(mlua::Error::runtime)?;
                    complete_display(lua, &cc, fid, outcome, HookKind::AfterSwitch)?;
                    Ok(outcome.target.raw())
                },
            )?,
        )?;
    }

    {
        // Q#BP11b — the target-aware load transaction. `find_or_open`
        // switches the ACTIVE window in both branches before firing
        // hooks, so a visit to a previously unopened file would replace
        // a focused panel before any display policy could help.
        let cc = core.clone();
        win.set(
            "display_file",
            lua.create_function(
                move |lua, (path, opts): (String, Option<Table>)| -> mlua::Result<u64> {
                    let fid = acting_frontend(lua, &cc);
                    let path_buf = std::path::PathBuf::from(&path);
                    let mut explicit_window = None;
                    let mut select = None;
                    if let Some(opts) = opts.as_ref() {
                        if let Some(raw) = opts.get::<Option<u64>>("window")? {
                            explicit_window = Some(lookup_window(&cc, fid, raw)?);
                        }
                        select = opts.get::<Option<bool>>("select")?;
                    }
                    // 1. Side-effect-free dedup: do NOT read the file yet.
                    let existing = cc.borrow().find_buffer_for_path(&path_buf);
                    // 2. Resolve the destination BEFORE I/O, so a
                    //    dedicated origin cannot force load-before-failure.
                    cc.borrow()
                        .probe_display_target(fid, existing, explicit_window)
                        .map_err(mlua::Error::runtime)?;
                    // 3. Load, dedup, or create the path-backed buffer.
                    let (buffer_id, fire) = cc
                        .borrow_mut()
                        .resolve_target_buffer(&path_buf)
                        .map_err(mlua::Error::runtime)?;
                    // 4. Enter Q#BP4's transaction, so any hook observes
                    //    the DOCUMENT TARGET as active.
                    let mut request = DisplayRequest::new(buffer_id);
                    request.window = explicit_window;
                    request.select = select;
                    let outcome = cc
                        .borrow_mut()
                        .display_buffer(fid, &request)
                        .map_err(mlua::Error::runtime)?;
                    complete_display(lua, &cc, fid, outcome, fire)?;
                    Ok(outcome.target.raw())
                },
            )?,
        )?;
    }

    {
        // Q#BP11a — the non-side target: what an ordinary visit from a
        // panel should address.
        let cc = core.clone();
        win.set(
            "display_target",
            lua.create_function(move |lua, ()| -> mlua::Result<u64> {
                let fid = acting_frontend(lua, &cc);
                let core = cc.borrow();
                core.non_side_target(fid)
                    .map(WindowId::raw)
                    .map_err(mlua::Error::runtime)
            })?,
        )?;
    }

    {
        // The acting frontend's side window, or nil.
        let cc = core.clone();
        win.set(
            "panel",
            lua.create_function(move |lua, ()| -> mlua::Result<Option<u64>> {
                let fid = acting_frontend(lua, &cc);
                Ok(cc.borrow().side_window_for(fid).map(WindowId::raw))
            })?,
        )?;
    }

    {
        // Q#BP2c — `window.quit`. A window with no recorded action gets
        // a pointed error WITHOUT closing or switching anything.
        let cc = core.clone();
        win.set(
            "quit",
            lua.create_function(move |lua, target: Option<u64>| -> mlua::Result<()> {
                let fid = acting_frontend(lua, &cc);
                let target = match target {
                    Some(raw) => lookup_window(&cc, fid, raw)?,
                    None => cc
                        .borrow()
                        .views
                        .get(&fid)
                        .map(|view| view.active)
                        .ok_or_else(|| {
                            mlua::Error::runtime("pmacs.window.quit: no acting frontend view")
                        })?,
                };
                let outcome = cc
                    .borrow_mut()
                    .quit_window(fid, target)
                    .map_err(mlua::Error::runtime)?;
                match outcome {
                    QuitOutcome::Deleted { focus } => {
                        reconcile_panel_layout(lua, &cc, fid);
                        if let Some(focus) = focus {
                            cc.borrow_mut().focus_window(fid, focus);
                        }
                    }
                    QuitOutcome::Restored { target, .. } => {
                        // Restoring is an ordinary presentation change:
                        // fire the switch hook so store-backed overlays
                        // reattach to the reinstated buffer.
                        cc.borrow_mut().focus_window(fid, target);
                        run_hook_if_defined(lua, "buffer.after-switch", mlua::MultiValue::new());
                        reconcile_panel_layout(lua, &cc, fid);
                        if visible(&cc, fid, target) {
                            cc.borrow_mut().focus_window(fid, target);
                        }
                    }
                }
                Ok(())
            })?,
        )?;
    }

    {
        // Read-only diagnostics over `WindowParams` (Q#BP2c).
        let cc = core.clone();
        win.set(
            "params",
            lua.create_function(move |lua, target: Option<u64>| -> mlua::Result<Table> {
                let fid = acting_frontend(lua, &cc);
                let id = match target {
                    Some(raw) => lookup_window(&cc, fid, raw)?,
                    None => selected_window(&cc, fid)?,
                };
                let core = cc.borrow();
                let window = core
                    .windows
                    .get(&id)
                    .ok_or_else(|| mlua::Error::runtime("pmacs.window.params: window not live"))?;
                let table = lua.create_table()?;
                table.set("window", id.raw())?;
                table.set("side", window.params.side.map(Side::name))?;
                table.set("fixed_rows", window.params.fixed_rows)?;
                table.set("dedicated", window.params.dedicated)?;
                table.set(
                    "origin_document",
                    window.params.origin_document().map(WindowId::raw),
                )?;
                table.set(
                    "quit_action",
                    window.params.quit_action().map(|action| match action {
                        crate::window::QuitAction::Delete => "delete",
                        crate::window::QuitAction::Restore { .. } => "restore",
                    }),
                )?;
                table.set(
                    "quit_depth",
                    window
                        .params
                        .quit_action()
                        .map_or(0, crate::window::QuitAction::depth),
                )?;
                table.set("hidden", window.is_side() && core.panel_hidden_for(fid))?;
                Ok(table)
            })?,
        )?;
    }

    {
        // Only `fixed_rows` and `dedicated` are writable (Q#BP2c).
        let cc = core.clone();
        win.set(
            "set_params",
            lua.create_function(
                move |lua, (target, opts): (u64, Table)| -> mlua::Result<()> {
                    let fid = acting_frontend(lua, &cc);
                    let id = lookup_window(&cc, fid, target)?;
                    for key in ["side", "origin_document", "quit_action"] {
                        if opts.get::<Value>(key)? != Value::Nil {
                            return Err(mlua::Error::runtime(format!(
                                "pmacs.window.set_params: `{key}` is not settable"
                            )));
                        }
                    }
                    let height = match opts.get::<Option<u32>>("fixed_rows")? {
                        Some(rows) => Some(
                            crate::editor_core::EditorCore::clamp_panel_rows(rows)
                                .map_err(mlua::Error::runtime)?,
                        ),
                        None => None,
                    };
                    let dedicated = opts.get::<Option<bool>>("dedicated")?;
                    {
                        let mut core = cc.borrow_mut();
                        let window = core.windows.get_mut(&id).ok_or_else(|| {
                            mlua::Error::runtime("pmacs.window.set_params: window not live")
                        })?;
                        if let Some(rows) = height {
                            // Inert on an ordinary window by construction:
                            // the fixed map is built from side windows only.
                            window.params.fixed_rows = Some(rows);
                        }
                        if let Some(dedicated) = dedicated {
                            window.params.dedicated = dedicated;
                        }
                    }
                    reconcile_panel_layout(lua, &cc, fid);
                    Ok(())
                },
            )?,
        )?;
    }

    {
        // Q#BP5b — `resize(win, delta_rows)` resolves from the SUPPLIED
        // window; the `window.enlarge` / `window.shrink` commands are
        // implicitly active.
        let cc = core.clone();
        win.set(
            "resize",
            lua.create_function(
                move |lua, (target, delta): (Option<u64>, i32)| -> mlua::Result<()> {
                    let fid = acting_frontend(lua, &cc);
                    let id = match target {
                        Some(raw) => lookup_window(&cc, fid, raw)?,
                        None => selected_window(&cc, fid)?,
                    };
                    let area_rows = cc.borrow().frontend_area_rows(fid).ok_or_else(|| {
                        mlua::Error::runtime(
                            "pmacs.window.resize: this frontend has not declared its geometry yet",
                        )
                    })?;
                    let minima: std::collections::HashMap<WindowId, u32> = {
                        let core = cc.borrow();
                        core.views
                            .get(&fid)
                            .map(|view| {
                                view.layout
                                    .iter_ids()
                                    .into_iter()
                                    .map(|id| {
                                        let buffer_id = core.windows.get(&id).map(|w| w.buffer_id);
                                        (
                                            id,
                                            config_u32(
                                                lua,
                                                "window.min-height",
                                                buffer_id,
                                                MIN_WINDOW_OUTER_ROWS,
                                            )
                                            .max(MIN_WINDOW_OUTER_ROWS),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    cc.borrow_mut()
                        .resize_boundary(fid, id, delta, area_rows, &|id| {
                            minima.get(&id).copied().unwrap_or(MIN_WINDOW_OUTER_ROWS)
                        })
                        .map_err(mlua::Error::runtime)?;
                    reconcile_panel_layout(lua, &cc, fid);
                    Ok(())
                },
            )?,
        )?;
    }

    Ok(())
}
