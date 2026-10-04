// lua_bindings/lsp_popup.rs --- the hover and signature popup (E8).

//! `pmacs.lsp.popup_*` and the private `pmacs.lsp._popup_*` bindings
//! the runtime's hover and signature commands open the popup through
//! (E8.2). The popup is populated from the LSP hover and signature
//! stores here, in Rust, not from a Lua copy of them: the store holds
//! the server's answer with its positions already in bytes, and the
//! core owns the popup's life ([`crate::lsp_popup`]).
//!
//! A command captures a [`PopupTargetLua`] before it asks the server
//! and hands it back with the answer: the window, buffer, caret and
//! revision the question was about. An answer that lands after the
//! caret moved or the text changed opens nothing and says nothing,
//! since the user has already left what it describes.

use mlua::{FromLua, Lua, Table, UserData, Value};

use super::{LspServerIdLua, SharedCore};
use crate::editor_core::{PopupOutcome, PopupTarget};
use crate::lsp::SharedLspManager;

/// Lua handle for a [`PopupTarget`]: opaque, passed back as given.
#[derive(Clone, Copy)]
pub struct PopupTargetLua(pub PopupTarget);

impl UserData for PopupTargetLua {}

impl FromLua for PopupTargetLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "PopupTargetLua".to_string(),
                message: Some("expected the value pmacs.lsp._popup_target returned".to_string()),
            }),
        }
    }
}

/// `(opened, status)` for Lua: the status line a caller shows, when
/// there is one to show.
fn outcome_to_lua(outcome: PopupOutcome, nothing: &str) -> (bool, Option<String>) {
    match outcome {
        PopupOutcome::Opened => (true, None),
        PopupOutcome::Nothing => (false, Some(nothing.to_owned())),
        PopupOutcome::Stale => (false, None),
    }
}

/// Install the popup bindings onto `pmacs.lsp`.
pub fn install_lsp_popup(
    lua: &Lua,
    core: &SharedCore,
    manager: &SharedLspManager,
) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let lsp: Table = pmacs.get("lsp")?;

    {
        // _popup_target() -> target | nil: what a hover or signature
        // request about to be sent is about.
        let cc = core.clone();
        lsp.set(
            "_popup_target",
            lua.create_function(move |_, ()| {
                Ok(cc.borrow().lsp_popup_target().map(PopupTargetLua))
            })?,
        )?;
    }

    {
        // _popup_hover(server, uri, target) -> opened, status|nil
        let cc = core.clone();
        let mgr = manager.clone();
        lsp.set(
            "_popup_hover",
            lua.create_function(
                move |_, (id, uri, target): (LspServerIdLua, String, PopupTargetLua)| {
                    let hover = {
                        let store = mgr.borrow().hover_store();
                        let guard = store.lock().expect("hover store mutex poisoned");
                        guard
                            .get(&crate::hover::HoverKey::new(id.0.raw().to_string(), uri))
                            .cloned()
                    };
                    let outcome = cc
                        .borrow_mut()
                        .lsp_popup_open_hover(target.0, hover.as_ref());
                    Ok(outcome_to_lua(outcome, "LSP: no hover info"))
                },
            )?,
        )?;
    }

    {
        // _popup_signature(server, uri, target, quiet) -> opened, status|nil
        //
        // `quiet` is the auto-trigger's: an empty answer leaves an open
        // signature popup as it is, where an asked-for one closes it.
        let cc = core.clone();
        let mgr = manager.clone();
        lsp.set(
            "_popup_signature",
            lua.create_function(
                move |_,
                      (id, uri, target, quiet): (
                    LspServerIdLua,
                    String,
                    PopupTargetLua,
                    Option<bool>,
                )| {
                    let help = {
                        let store = mgr.borrow().signature_store();
                        let guard = store.lock().expect("signature store mutex poisoned");
                        guard
                            .get(&crate::signature::SignatureKey::new(
                                id.0.raw().to_string(),
                                uri,
                            ))
                            .cloned()
                    };
                    let outcome = cc.borrow_mut().lsp_popup_open_signature(
                        target.0,
                        help.as_ref(),
                        quiet.unwrap_or(false),
                    );
                    Ok(outcome_to_lua(outcome, "LSP: no signature help"))
                },
            )?,
        )?;
    }

    {
        // popup_close(): close the popup, whatever it shows.
        let cc = core.clone();
        lsp.set(
            "popup_close",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().lsp_popup_close();
                Ok(())
            })?,
        )?;
    }

    {
        // popup_kind() -> "hover" | "signature" | nil
        let cc = core.clone();
        lsp.set(
            "popup_kind",
            lua.create_function(move |_, ()| {
                Ok(cc.borrow().lsp_popup_kind().map(|k| match k {
                    pmacs_protocol::PopupKind::Hover => "hover",
                    pmacs_protocol::PopupKind::Signature => "signature",
                }))
            })?,
        )?;
    }

    Ok(())
}
