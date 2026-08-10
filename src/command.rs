// command.rs --- Command registry: every editor action is a named, introspectable command.

//! Editor commands.
//!
//! Per spec §4.2 every editor action is a named, introspectable
//! [`Command`]. Commands carry name, description (R42 makes this
//! mandatory), source location (file + line where they were defined),
//! a Lua function body, and an optional availability predicate.
//!
//! # Storage and lookup
//!
//! [`CommandRegistry`] owns the live commands. Insertion is rejected on
//! duplicate names rather than silently overwriting --- silent overwrite
//! makes refactoring bugs invisible. Lookup is `O(1)`; iteration via
//! [`CommandRegistry::names`] preserves insertion order so the command
//! palette renders consistently across launches.
//!
//! # Threading
//!
//! Single-threaded. Lives behind `Rc<RefCell<...>>` next to the Lua
//! state and the buffer registry.

use std::collections::HashMap;

use mlua::Function;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Source location
// ---------------------------------------------------------------------------

/// Where in the source a command was defined.
///
/// Captured at registration time via Lua's debug info. Surfaced
/// verbatim by `pmacs.describe.command`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceLocation {
    /// `short_src` from Lua's debug info: typically a file path or
    /// `[C]` for native code or `=[string "..."]` for an inline chunk.
    pub file: String,
    /// Line where the call was made. `0` if unavailable.
    pub line: i32,
}

impl SourceLocation {
    /// Render as `file:line` (or just `file` if `line == 0`).
    #[must_use]
    pub fn render(&self) -> String {
        if self.line > 0 {
            format!("{}:{}", self.file, self.line)
        } else {
            self.file.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Command + errors
// ---------------------------------------------------------------------------

/// A registered editor command.
///
/// Cloning is cheap: `String`s clone trivially and `mlua::Function` is
/// reference-counted internally.
#[derive(Clone)]
pub struct Command {
    /// Unique name (e.g. `buffer.save`).
    pub name: String,
    /// Human-readable description. Required and non-empty after trim
    /// (R42), but otherwise **free-form, and legitimately multi-line**.
    ///
    /// # Do not add a registration-time one-line guard
    ///
    /// This doc used to read "one-line human-readable description",
    /// which was an aspiration rather than the contract: MCP tool
    /// registration renders a whole schema block in here — the tool's
    /// text, a blank line, `Arguments:`, then one line per argument
    /// (`tests/fixtures/pmacs-mcp-tools/init.lua:272`, a
    /// `table.concat(lines, "\n")`) — and `m9_6_acceptance.rs:583-598`
    /// asserts all four of those lines. Rejecting CR/LF in
    /// [`CommandRegistry::define`] was tried, measured, and abandoned:
    /// it fails 36 tests across `m9_6`/`m9_7`/`m9_8` and could only be
    /// made green by deleting a shipped acceptance criterion.
    ///
    /// The one-line constraint belongs to the **surfaces that have
    /// it**, so a consumer rendering into a single row clips with
    /// [`Self::description_first_line`] — the minibuffer band and the
    /// completion dropdown both do. The full text stays intact for
    /// `describe-command` and `help.list-commands`, which is what keeps
    /// this a rendering decision rather than data loss.
    pub description: String,
    /// Where the command was defined.
    pub source: SourceLocation,
    /// The Lua function body. Invoked by [`crate::lua::LuaHost`] and by
    /// keymap dispatch (T M2.4).
    pub body: Function,
    /// Optional availability predicate. Returns `true` when the command
    /// applies in the current state. The command palette (T M2.7) uses
    /// it to gray out unavailable entries.
    pub predicate: Option<Function>,
}

impl Command {
    /// [`Self::description`] clipped to its first line, for a consumer
    /// rendering into a surface that has exactly one row.
    ///
    /// The description is free-form and may carry a whole schema block
    /// (see that field). Two surfaces cannot show one: the grid TUI
    /// writes the selected candidate into a single-row suffix on the
    /// minibuffer band, and the GPU dropdown derives its height, its
    /// visible window and its selection-highlight offset from
    /// `rows.len()` — **one logical row per candidate** — so a detail
    /// that shapes into more physical lines than that misaligns every
    /// row below it and the highlight with it.
    ///
    /// Clipping here rather than refusing at registration follows the
    /// precedent already in this tree: the MCP fixture's result
    /// delivery keeps only the first line of a tool result because
    /// *"a multi-line `set_status` would corrupt the row layout"*
    /// (`tests/fixtures/pmacs-mcp-tools/init.lua:277-285`), leaving
    /// width clipping to the frontend. Same hazard class, same
    /// resolution.
    ///
    /// **No ellipsis or truncation marker**, matching that precedent
    /// and the minibuffer's own width rule, which rejects stub markers
    /// for the same reason: the full text is one `describe-command`
    /// away, and a marker in a candidate row reads as part of the
    /// candidate.
    #[must_use]
    pub fn description_first_line(&self) -> &str {
        first_line(&self.description)
    }
}

/// The prefix of `text` before its first line break.
///
/// Breaks on CR **or** LF, not LF alone: a lone CR ends a line on
/// classic-Mac-era input and is the leading half of a CRLF, so an
/// LF-only clip would pass a bare `\r` straight through to a
/// single-row surface — and a CR-only clip would do the same for `\n`.
/// Splitting on the first of either handles all three forms with one
/// scan, since CRLF's `\r` comes first.
fn first_line(text: &str) -> &str {
    match text.find(['\n', '\r']) {
        Some(break_at) => &text[..break_at],
        None => text,
    }
}

/// Errors raised by the command registry.
#[derive(Debug, Error)]
pub enum CommandError {
    /// `define` was called with no name or an empty string.
    #[error("command name must be non-empty")]
    EmptyName,

    /// R42: `define` was called without a description, or with one that
    /// is empty after trimming.
    #[error("command \"{name}\" requires a non-empty description (R42)")]
    MissingDescription {
        /// The offending command name.
        name: String,
    },

    /// `define` was called without a `fn` field.
    #[error("command \"{name}\" requires an `fn` field")]
    MissingFn {
        /// The offending command name.
        name: String,
    },

    /// A command with this name is already registered.
    #[error("command \"{name}\" is already defined (refusing to overwrite)")]
    DuplicateName {
        /// The offending command name.
        name: String,
    },

    /// Lookup or invoke targeted a name that has no registered command.
    #[error("command \"{name}\" not found")]
    NotFound {
        /// The offending command name.
        name: String,
    },

    /// R50: the spec table contained a key the registry doesn't know
    /// about. Typo-detection.
    #[error("unknown field `{field}` in command spec; supported: name, description, fn, predicate")]
    UnknownField {
        /// The offending key.
        field: String,
    },
}

// ---------------------------------------------------------------------------
// CommandRegistry
// ---------------------------------------------------------------------------

/// Registry of named commands.
///
/// Construction goes through [`Self::new`] / [`Self::default`]. Insert
/// via [`Self::define`], which validates the metadata and rejects
/// duplicates. Look up via [`Self::get`], iterate names via
/// [`Self::names`], remove via [`Self::remove`].
#[derive(Default)]
pub struct CommandRegistry {
    by_name: HashMap<String, Command>,
    /// Insertion order. Mirrors the buffer registry's design --- lets
    /// `pmacs.command.list()` return a deterministic sequence.
    order: Vec<String>,
}

impl CommandRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `cmd`. Returns [`CommandError::DuplicateName`] if a
    /// command with the same name already exists. Validates metadata
    /// per R42 / R50: the name must be non-empty and the description
    /// must be non-empty after trim.
    pub fn define(&mut self, cmd: Command) -> Result<(), CommandError> {
        if cmd.name.is_empty() {
            return Err(CommandError::EmptyName);
        }
        if cmd.description.trim().is_empty() {
            return Err(CommandError::MissingDescription { name: cmd.name });
        }
        if self.by_name.contains_key(&cmd.name) {
            return Err(CommandError::DuplicateName { name: cmd.name });
        }
        self.order.push(cmd.name.clone());
        self.by_name.insert(cmd.name.clone(), cmd);
        Ok(())
    }

    /// Look up by name. Returns `None` for unknown commands.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Command> {
        self.by_name.get(name)
    }

    /// True iff `name` is registered.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Remove a command by name. Returns the removed command, or
    /// [`CommandError::NotFound`].
    pub fn remove(&mut self, name: &str) -> Result<Command, CommandError> {
        let cmd = self
            .by_name
            .remove(name)
            .ok_or_else(|| CommandError::NotFound {
                name: name.to_owned(),
            })?;
        self.order.retain(|n| n != name);
        Ok(cmd)
    }

    /// Names in insertion order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Number of registered commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// True iff no commands are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn make_command(lua: &Lua, name: &str, description: &str) -> Command {
        Command {
            name: name.to_owned(),
            description: description.to_owned(),
            source: SourceLocation {
                file: "test.lua".into(),
                line: 1,
            },
            body: lua.create_function(|_, ()| Ok(())).unwrap(),
            predicate: None,
        }
    }

    #[test]
    fn define_then_get_round_trips() {
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        r.define(make_command(&lua, "buffer.save", "Save the buffer."))
            .unwrap();
        assert!(r.contains("buffer.save"));
        assert_eq!(
            r.get("buffer.save").unwrap().description,
            "Save the buffer."
        );
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn missing_description_is_a_registration_error() {
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        let mut c = make_command(&lua, "x", "");
        c.description.clear();
        match r.define(c) {
            Err(CommandError::MissingDescription { name }) => assert_eq!(name, "x"),
            other => panic!("expected MissingDescription, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_only_description_is_rejected() {
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        let c = make_command(&lua, "x", "   \n\t  ");
        assert!(matches!(
            r.define(c),
            Err(CommandError::MissingDescription { .. })
        ));
    }

    #[test]
    fn a_multi_line_description_registers_and_clips_to_its_first_line() {
        // Registration accepts it — MCP tool registration renders a
        // whole schema block into `description` and
        // `m9_6_acceptance.rs:583-598` asserts four of its lines, so a
        // one-line guard here would delete a shipped contract. The
        // one-line constraint lives at the single-row surfaces, which
        // read `description_first_line`.
        //
        // All three break forms: a clip that split on `\n` alone would
        // pass a bare `\r` through, and one that split on `\r` alone
        // would pass `\n` through.
        let lua = Lua::new();
        for (label, description) in [
            (
                "LF",
                "Greet someone.\n\nArguments:\n  name (string, required)",
            ),
            (
                "CR",
                "Greet someone.\r\rArguments:\r  name (string, required)",
            ),
            (
                "CRLF",
                "Greet someone.\r\n\r\nArguments:\r\n  name (string, required)",
            ),
        ] {
            let mut r = CommandRegistry::new();
            r.define(make_command(&lua, "mcp.greet", description))
                .unwrap_or_else(|e| panic!("{label}: a schema block must still register: {e}"));
            let cmd = r.get("mcp.greet").expect("registered");
            assert_eq!(
                cmd.description, description,
                "{label}: the registry stores the description verbatim — the clip is a \
                 rendering decision, so `describe-command` must still see every line"
            );
            assert_eq!(
                cmd.description_first_line(),
                "Greet someone.",
                "{label}: a single-row surface gets the first line only"
            );
            assert!(
                !cmd.description_first_line().contains(['\n', '\r']),
                "{label}: the clipped form must carry no break at all"
            );
        }
    }

    #[test]
    fn a_single_line_description_is_byte_identical_after_the_clip() {
        // The other half: the clip must not tighten past its purpose.
        // Interior whitespace, punctuation and non-ASCII all survive,
        // and there is no ellipsis or truncation marker.
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        let description = "Write the buffer to its file — with a dash, and \ttabs.";
        r.define(make_command(&lua, "buffer.save", description))
            .expect("registers");
        assert_eq!(
            r.get("buffer.save").unwrap().description_first_line(),
            description,
            "a description with no break is returned unchanged"
        );
    }

    #[test]
    fn a_description_whose_first_line_is_empty_clips_to_empty() {
        // The case the producer turns into `None` rather than
        // `Some("")`: a leading break leaves nothing to render, and a
        // `Some("")` detail would draw trailing padding after the label.
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        r.define(make_command(&lua, "x", "\nArguments:\n  a (string)"))
            .expect("registers");
        assert_eq!(r.get("x").unwrap().description_first_line(), "");
    }

    #[test]
    fn empty_name_is_rejected() {
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        let c = make_command(&lua, "", "valid desc");
        assert!(matches!(r.define(c), Err(CommandError::EmptyName)));
    }

    #[test]
    fn duplicate_name_is_rejected() {
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        r.define(make_command(&lua, "x", "first")).unwrap();
        let err = r.define(make_command(&lua, "x", "second")).unwrap_err();
        assert!(matches!(err, CommandError::DuplicateName { name } if name == "x"));
    }

    #[test]
    fn names_preserve_insertion_order() {
        let lua = Lua::new();
        let mut r = CommandRegistry::new();
        r.define(make_command(&lua, "a", "a")).unwrap();
        r.define(make_command(&lua, "b", "b")).unwrap();
        r.define(make_command(&lua, "c", "c")).unwrap();
        assert_eq!(r.names(), &["a".to_owned(), "b".into(), "c".into()]);
        r.remove("b").unwrap();
        assert_eq!(r.names(), &["a".to_owned(), "c".into()]);
    }

    #[test]
    fn source_location_renders_file_and_line() {
        let s = SourceLocation {
            file: "@init.lua".into(),
            line: 7,
        };
        assert_eq!(s.render(), "@init.lua:7");
    }

    #[test]
    fn source_location_omits_zero_line() {
        let s = SourceLocation {
            file: "[C]".into(),
            line: 0,
        };
        assert_eq!(s.render(), "[C]");
    }
}
