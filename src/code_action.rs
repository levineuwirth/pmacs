// code_action.rs --- T M4.5 L3 LSP code actions / executeCommand.

//! `textDocument/codeAction` response state.
//!
//! The response is `(Command | CodeAction)[] | null`. The two shapes
//! are normalised into one [`CodeActionItem`]:
//!
//!   * a bare `Command` becomes an item with no `edit` and its
//!     `command` populated;
//!   * a `CodeAction` keeps its `title`/`kind`, an optional inline
//!     [`WorkspaceEditResponse`] (`edit`), and an optional `command`
//!     to run after (or instead of) the edit.
//!
//! Applying the chosen item is Lua policy (same division as
//! [`crate::rename`] / [`crate::formatting`]): an inline `edit` is fed
//! to the shared `WorkspaceEdit` applier; a `command` is dispatched via
//! `workspace/executeCommand`, after which the server typically drives
//! the real change with a server→client `workspace/applyEdit` request
//! (handled by the Lua event pump). Nothing here mutates the editor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::rename::WorkspaceEditResponse;

/// A server command to run via `workspace/executeCommand`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandRef {
    /// Human-readable title.
    pub title: String,
    /// The command identifier the server registered.
    pub command: String,
    /// Opaque arguments, passed through verbatim.
    pub arguments: Vec<Value>,
}

/// One normalised code action.
#[derive(Clone, Debug, Default)]
pub struct CodeActionItem {
    /// Display title.
    pub title: String,
    /// LSP `CodeActionKind` (e.g. `quickfix`, `refactor`), if any.
    pub kind: Option<String>,
    /// Inline workspace edit, if the action carries one. Empty when
    /// the action is command-only.
    pub edit: WorkspaceEditResponse,
    /// Command to dispatch, if any.
    pub command: Option<CommandRef>,
}

impl CodeActionItem {
    /// True iff the action carries an inline edit.
    #[must_use]
    pub fn has_edit(&self) -> bool {
        !self.edit.is_empty()
    }
}

/// Parsed `textDocument/codeAction` response.
#[derive(Clone, Debug, Default)]
pub struct CodeActionResponse {
    /// Actions in server order.
    pub actions: Vec<CodeActionItem>,
}

impl CodeActionResponse {
    /// Parse `(Command | CodeAction)[] | null`.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        let Some(arr) = v.as_array() else {
            return Self::default();
        };
        let mut actions = Vec::with_capacity(arr.len());
        for item in arr {
            if let Some(a) = parse_item(item) {
                actions.push(a);
            }
        }
        Self { actions }
    }

    /// True iff the server offered no actions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

fn parse_command(v: &Value) -> Option<CommandRef> {
    // A `Command` always has a string `command`. `arguments` is
    // optional; `title` defaults to the command id when absent.
    let command = v.get("command")?.as_str()?.to_owned();
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&command)
        .to_owned();
    let arguments = v
        .get("arguments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(CommandRef {
        title,
        command,
        arguments,
    })
}

fn parse_item(v: &Value) -> Option<CodeActionItem> {
    // Disambiguate `Command` from `CodeAction`: a `Command`'s
    // `command` is a string; a `CodeAction`'s `command` (when present)
    // is a nested `Command` object. So a top-level *string* `command`
    // means the whole entry is a bare Command.
    if v.get("command").and_then(Value::as_str).is_some() {
        let cmd = parse_command(v)?;
        return Some(CodeActionItem {
            title: cmd.title.clone(),
            kind: None,
            edit: WorkspaceEditResponse::default(),
            command: Some(cmd),
        });
    }
    // Otherwise a CodeAction. `title` is required by spec; tolerate a
    // missing one rather than dropping the action.
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let kind = v.get("kind").and_then(Value::as_str).map(str::to_owned);
    let edit = v
        .get("edit")
        .map(WorkspaceEditResponse::from_lsp_value)
        .unwrap_or_default();
    let command = v.get("command").and_then(parse_command);
    Some(CodeActionItem {
        title,
        kind,
        edit,
        command,
    })
}

/// Per-server, per-uri code-action state.
#[derive(Default)]
pub struct CodeActionStore {
    by_key: HashMap<CodeActionKey, CodeActionResponse>,
}

/// Key into [`CodeActionStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct CodeActionKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI the request was made on.
    pub uri: String,
}

impl CodeActionKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl CodeActionStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: CodeActionKey, response: CodeActionResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &CodeActionKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &CodeActionKey) -> Option<&CodeActionResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedCodeActionStore = Arc<Mutex<CodeActionStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedCodeActionStore {
    Arc::new(Mutex::new(CodeActionStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_bare_command() {
        let v = json!([{
            "title": "Run me",
            "command": "pmacs.do",
            "arguments": [1, "x"]
        }]);
        let r = CodeActionResponse::from_lsp_value(&v);
        assert_eq!(r.actions.len(), 1);
        let a = &r.actions[0];
        assert_eq!(a.title, "Run me");
        assert!(!a.has_edit());
        let c = a.command.as_ref().unwrap();
        assert_eq!(c.command, "pmacs.do");
        assert_eq!(c.arguments.len(), 2);
    }

    #[test]
    fn parses_code_action_with_inline_edit() {
        let v = json!([{
            "title": "Fix it",
            "kind": "quickfix",
            "edit": {
                "changes": {
                    "file:///a.rs": [{
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 3 }
                        },
                        "newText": "ok"
                    }]
                }
            }
        }]);
        let r = CodeActionResponse::from_lsp_value(&v);
        let a = &r.actions[0];
        assert_eq!(a.kind.as_deref(), Some("quickfix"));
        assert!(a.has_edit());
        assert_eq!(a.edit.files[0].uri, "file:///a.rs");
        assert!(a.command.is_none());
    }

    #[test]
    fn parses_code_action_with_nested_command() {
        let v = json!([{
            "title": "Refactor",
            "kind": "refactor",
            "command": { "title": "Apply", "command": "srv.apply", "arguments": [] }
        }]);
        let r = CodeActionResponse::from_lsp_value(&v);
        let a = &r.actions[0];
        assert!(!a.has_edit());
        assert_eq!(a.command.as_ref().unwrap().command, "srv.apply");
    }

    #[test]
    fn null_response_is_empty() {
        assert!(CodeActionResponse::from_lsp_value(&Value::Null).is_empty());
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = CodeActionStore::new();
        let key = CodeActionKey::new("1", "file:///a");
        s.set(
            key.clone(),
            CodeActionResponse {
                actions: vec![CodeActionItem {
                    title: "t".into(),
                    ..Default::default()
                }],
            },
        );
        assert_eq!(s.get(&key).unwrap().actions.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
