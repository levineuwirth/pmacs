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
    // Journey Stage 1a (Q#JR14e): the background scope wins.
    //
    // Order is deliberate — scoped override, then interactive origin,
    // then ambient. A `commit_to` callback runs for the frontend that
    // *requested* the work, and it must win over whatever happens to be
    // dispatching when the worker settles. It is a separate slot rather
    // than a reuse of the interactive origin because that origin is
    // authenticated user-command authority (the pre-edit unfold guard,
    // command-boundary rotation, and the terminal surface all key off
    // it), and a background continuation must not acquire it.
    lua.app_data_ref::<crate::editor::ScopedFrontend>()
        .and_then(|scope| scope.current())
        .or_else(|| {
            lua.app_data_ref::<crate::editor::InteractiveCommandOrigin>()
                .and_then(|origin| origin.current())
        })
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
pub(crate) fn selected_window(core: &SharedCore, fid: FrontendId) -> mlua::Result<WindowId> {
    core.borrow()
        .views
        .get(&fid)
        .map(|view| view.active)
        .ok_or_else(|| mlua::Error::runtime("pmacs.window: acting frontend has no layout"))
}

/// Resolve a raw Lua window id, refusing one that is not live in the
/// acting frontend's layout (Q#BP11).
pub(crate) fn lookup_window(
    core: &SharedCore,
    fid: FrontendId,
    raw: u64,
) -> mlua::Result<WindowId> {
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

/// The one rule for the adopter `display` vocabulary: which values are
/// legal, what the error says, and what omission means.
///
/// **Bottom-panel Stage 3 (Q#S3-1).** Before this, four adopters
/// validated the same three-value vocabulary in three places — Rust for
/// the terminal, and hand-written Lua copies in `listview.lua`,
/// `compile.lua` and `dired.lua`, each with its own copy of the error
/// string. Four copies of one rule is how the next adopter gets it
/// subtly wrong, and the next adopter is DAP.
///
/// `default` is a **parameter, not a constant**, because the adopters do
/// not share one. Since Stage 3, listview / compile / terminal resolve
/// omission to the panel; **dired resolves it to `"current"`** and must
/// keep doing so —
/// `pmacs.path.directory_handler` calls it with no `display` at all, so
/// a flipped default would open `pmacs .` in a bottom panel (§1.1a).
/// Passing the default in is what makes dired's exemption visible at its
/// call site instead of hidden in a divergent copy.
///
/// **Non-string values are reported as unknown, not as type errors**,
/// and this is a deliberate normalization (Q#S3-1). Terminal previously
/// read `get::<Option<String>>` and so raised mlua's type error *before*
/// reaching any custom message, while the Lua copies stringified and
/// reported their own. Nothing pinned either behaviour. The custom error
/// wins because it names the legal vocabulary and mlua's does not.
///
/// # Errors
/// Any non-nil value that is not `"current"` or `"panel"`.
pub(crate) fn resolve_adopter_display(
    operation: &str,
    raw: Option<&mlua::Value>,
    default: AdopterDefault,
) -> mlua::Result<AdopterPlacement> {
    let raw = match raw {
        None | Some(mlua::Value::Nil) => return Ok(default.placement()),
        Some(value) => value,
    };
    match raw.as_str().as_deref() {
        Some("current") => Ok(AdopterPlacement::Current),
        Some("panel") => Ok(AdopterPlacement::Panel),
        _ => Err(mlua::Error::runtime(format!(
            "{operation}: unknown display {} (expected \"current\" or \"panel\")",
            display_for_error(raw)
        ))),
    }
}

/// What omission means for one adopter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AdopterDefault {
    /// listview, compile, terminal — Stage 3's flip.
    Panel,
    /// dired, and anything else whose output is a document the user
    /// works in rather than output they consult.
    Current,
}

impl AdopterDefault {
    fn placement(self) -> AdopterPlacement {
        match self {
            Self::Panel => AdopterPlacement::Panel,
            Self::Current => AdopterPlacement::Current,
        }
    }
}

/// Render a rejected `display` value for the error message.
///
/// Strings are quoted so `display = "sideways"` reads as `"sideways"`;
/// anything else is shown **by type alone** — `unknown display
/// (number)` — because quoting a non-string as `"42"` would imply the
/// caller passed a string when they passed a number.
///
/// The value itself is deliberately not interpolated: it is already
/// wrong, `mlua::Value`'s `Display` is not guaranteed useful for tables
/// or userdata, and the type is what tells the caller what to fix.
fn display_for_error(value: &mlua::Value) -> String {
    value.as_str().map_or_else(
        || format!("({})", value.type_name()),
        |s| format!("{:?}", &*s),
    )
}

/// Parse an adopter's placement **before** it creates a buffer, session,
/// process, or wrapper — so an unknown value leaves nothing to roll back.
///
/// Terminal-specific wrapper around [`resolve_adopter_display`]: only
/// the terminal accepts a `window` id, and only it must reject `window`
/// combined with `display = "panel"`. That asymmetry stays here rather
/// than in the shared resolver, because a helper that pretended the four
/// parsers were identical would be its own defect.
///
/// # Errors
/// An unknown `display` value, a `window` combined with
/// `display = "panel"`, or a window id that is not live in the acting
/// frontend's layout.
pub(crate) fn parse_adopter_placement(
    core: &SharedCore,
    fid: FrontendId,
    operation: &str,
    display: Option<&mlua::Value>,
    window: Option<u64>,
    default: AdopterDefault,
) -> mlua::Result<AdopterPlacement> {
    let display = resolve_adopter_display(operation, display, default)?;
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
            "commit_to",
            lua.create_function(
                move |lua,
                      (dest, body): (mlua::Value, mlua::Function)|
                      -> mlua::Result<mlua::MultiValue> {
                    // Journey Stage 1a (Q#JR14). Preflight FIRST, then
                    // scope, then run. The ordering is the whole point:
                    // an async handler mutates real state (dired claims
                    // a buffer, registers a handle, captures `prev`, and
                    // paints) long before it reaches any call that could
                    // refuse. Validating at display time is four
                    // mutations too late and leaves a hidden buffer
                    // behind, so every destination precondition is
                    // checked before the callback is invoked at all.
                    //
                    // Typed as `Value` rather than `AnyUserData` so this
                    // message is REACHABLE: with the narrower type mlua
                    // rejects a table during argument conversion, and a
                    // caller who fabricated one got "error converting Lua
                    // table to userdata" — true, but it names neither the
                    // rule nor how to get a real destination.
                    let dest = match &dest {
                        mlua::Value::UserData(userdata) => {
                            userdata.borrow::<super::DirectoryDestinationLua>().ok()
                        }
                        _ => None,
                    };
                    let dest = dest
                        .ok_or_else(|| {
                            mlua::Error::runtime(
                                "pmacs.window.commit_to: expected a destination captured by \
                                 the editor (it cannot be constructed from Lua)",
                            )
                        })?
                        .0;

                    // 1. The requesting frontend still has a layout.
                    let refusal = {
                        let core = cc.borrow();
                        if !core.views.contains_key(&dest.frontend) {
                            Some("requesting frontend is gone".to_string())
                        } else if !core
                            .views
                            .get(&dest.frontend)
                            .is_some_and(|view| view.layout.iter_ids().contains(&dest.window))
                        {
                            // 2. The destination window is still live in it.
                            Some(format!("window {} is gone", dest.window.raw()))
                        } else if core
                            .windows
                            .get(&dest.window)
                            .is_some_and(|w| w.buffer_id != dest.buffer)
                        {
                            // 3. Stale intent (Q#JR14c): the user
                            //    replaced the buffer while the work was
                            //    in flight. Their action is newer
                            //    information than the request, so the
                            //    request loses.
                            Some(format!(
                                "window {} now shows another buffer",
                                dest.window.raw()
                            ))
                        } else if !core.window_accepts_buffer(dest.window, None) {
                            // 4. Replaceability (Q#JR14f). `None`
                            //    because the replacement does not exist
                            //    yet — passing the captured buffer would
                            //    approve a window dedicated to *it*, and
                            //    the handler's different buffer would be
                            //    refused later, after mutating.
                            Some(format!("window {} is dedicated", dest.window.raw()))
                        } else {
                            None
                        }
                    };
                    if let Some(reason) = refusal {
                        let mut out = mlua::MultiValue::new();
                        out.push_back(mlua::Value::String(lua.create_string(reason.as_bytes())?));
                        out.push_front(mlua::Value::Boolean(false));
                        return Ok(out);
                    }

                    let scope = lua
                        .app_data_ref::<crate::editor::ScopedFrontend>()
                        .ok_or_else(|| {
                            mlua::Error::runtime(
                                "pmacs.window.commit_to: no frontend scope installed",
                            )
                        })?
                        .clone();
                    let commit = lua
                        .app_data_ref::<crate::editor::CommitScopeActive>()
                        .ok_or_else(|| {
                            mlua::Error::runtime(
                                "pmacs.window.commit_to: no commit scope installed",
                            )
                        })?
                        .clone();
                    // Both the override and the core's ambient
                    // `active_frontend` are restored when this guard
                    // drops -- on the normal return AND on a raising
                    // callback, which is why the result is captured
                    // rather than `?`-propagated through the drop.
                    let result = {
                        let _guard = scope.enter(&cc, &commit, dest.frontend);
                        body.call::<mlua::MultiValue>(())
                    };
                    let mut out = result?;
                    out.push_front(mlua::Value::Boolean(true));
                    Ok(out)
                },
            )?,
        )?;
    }

    {
        // Q#S3-1 — the shared adopter-display rule, reachable from Lua.
        //
        // Underscore-prefixed because it is an internal seam between the
        // builtin runtime modules and this one, not user-facing API:
        // `listview.lua`, `compile.lua` and `dired.lua` call it instead
        // of each keeping a hand-written copy of the same three-value
        // check and error string.
        //
        // Returns the resolved `"current"` / `"panel"` rather than an
        // opaque handle, so the Lua callers keep their existing
        // `if display == "panel"` dispatch and this change stays a
        // validation unification rather than a control-flow rewrite.
        win.set(
            "_resolve_display",
            lua.create_function(
                |_,
                 (operation, raw, default): (String, mlua::Value, String)|
                 -> mlua::Result<&'static str> {
                    let default = match default.as_str() {
                        "panel" => AdopterDefault::Panel,
                        "current" => AdopterDefault::Current,
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "window._resolve_display: bad default {other:?}"
                            )));
                        }
                    };
                    match resolve_adopter_display(&operation, Some(&raw), default)? {
                        AdopterPlacement::Panel => Ok("panel"),
                        AdopterPlacement::Current | AdopterPlacement::Window(_) => Ok("current"),
                    }
                },
            )?,
        )?;
    }

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
                    //
                    // Journey Stage 1a (Q#JR13): a DIRECTORY raises here
                    // and does NOT enter the directory resolver chain.
                    // `display_file` is "put this file in a window", not
                    // a CLI router — and `find-file`'s accept arm
                    // (`builtin/commands/default.lua`) wraps this call in
                    // a `pcall` whose comment guarantees that "only a
                    // real failure (a directory, a permission error)
                    // reaches here", pinned by
                    // `find_file_accepting_a_directory_reports_instead_of_raising`.
                    // Routing it into dired would silently change what
                    // `C-x C-f` on a directory does. Opening dired from
                    // find-file is a named deferral, not a side effect of
                    // the CLI work.
                    let (buffer_id, fire) = match cc
                        .borrow_mut()
                        .resolve_target_buffer(&path_buf)
                        .map_err(mlua::Error::runtime)?
                    {
                        crate::editor_core::ResolvedTarget::Buffer { id, fire } => (id, fire),
                        crate::editor_core::ResolvedTarget::Directory { path } => {
                            return Err(mlua::Error::runtime(format!(
                                "pmacs.window.display_file: {} is a directory",
                                path.display()
                            )));
                        }
                    };
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
