// help.rs --- Help-buffer rendering and cross-reference parsing (T M2.11).

//! Renders human-readable descriptions of commands, keys, buffers,
//! modes, hooks, and views into a buffer named `*help*`. Each render
//! call replaces the buffer's content in full --- the help buffer
//! always shows the most recently described target.
//!
//! # Cross-references
//!
//! Help text uses bracketed `[type: target]` tokens for navigable
//! references. Examples:
//!
//! * `[command: cursor.left]` --- navigate to that command's help.
//! * `[key: C-x C-s]` --- describe the chord.
//! * `[key: s @buffer:3]` --- describe a buffer-local chord.
//! * `[buffer: *errors*]` --- describe a buffer by name.
//! * `[mode: normal]`, `[hook: buffer.before-save]`, `[view: *help*]`.
//!
//! [`follow_link_at`] parses the line under a cursor, finds the
//! enclosing token (if any), and re-renders. This makes the help
//! buffer self-navigable: with a buffer-local RET binding (T M2.11
//! also installs that on first render), the user clicks through
//! related entries the way Emacs's help mode does.

use std::fmt::Write;

use crate::buffer::{Buffer, BufferId, EditOp};
use crate::buffer_registry::BufferRegistry;
use crate::command::{CommandRegistry, SourceLocation};
use crate::hook::{Hook, HookRegistry};
use crate::key::{Sequence, display_sequence, parse_sequence};
use crate::keymap_stack::{KeymapStack, Scope, StackResolution};
use crate::keymap_tree::{Binding, Keymap};

/// Canonical name for the help buffer. Looked up via
/// [`BufferRegistry::find_by_name`].
pub const HELP_BUFFER_NAME: &str = "*help*";

/// Result of a render: the buffer id of `*help*` paired with the
/// Edits produced by the content replacement, or [`None`] if the
/// described target doesn't exist (e.g. unknown command name).
///
/// # Post-audit-round-6 F31 — broadcast queueing
///
/// Returning the Edits (zero, one, or two — Delete for old
/// non-empty content + Insert for new non-empty content) lets the
/// caller queue any `crdt_op` they carry via
/// `EditorCore::queue_daemon_origin_crdt_op`. Without this, replica
/// frontends see the `*help*` repaint as `CellDelta` but never
/// update their `BufferMirror`s for the CRDT-backed `*help*`
/// buffer; subsequent optimistic edits on the replica would run
/// against stale mirror content.
pub type RenderResult = Option<(BufferId, Vec<crate::rope::Edit>)>;

// ---------------------------------------------------------------------------
// Render entry points
// ---------------------------------------------------------------------------

/// Render help for a command name. Returns the help buffer id if the
/// command exists, [`None`] otherwise.
pub fn render_command(
    registry: &mut BufferRegistry,
    commands: &CommandRegistry,
    keymaps: &KeymapStack,
    name: &str,
) -> RenderResult {
    let cmd = commands.get(name)?;
    let mut text = String::new();
    let _ = writeln!(text, "Command: {}", cmd.name);
    let _ = writeln!(text, "  Source: {}", cmd.source.render());
    let _ = writeln!(text);
    let _ = writeln!(text, "{}", cmd.description);
    let _ = writeln!(text);
    write_command_bindings(registry, &mut text, &cmd.name, keymaps);
    if cmd.predicate.is_some() {
        let _ = writeln!(text);
        let _ = writeln!(text, "Predicate: yes (this command can refuse to run).");
    }
    let _ = writeln!(text);
    let _ = writeln!(text, "See also: [keymap: list].");
    Some(replace_help_buffer(registry, &text))
}

/// Render help for a chord sequence. Returns the help buffer id if
/// the sequence resolves to a binding, [`None`] otherwise.
///
/// `active_buffer` is the buffer scope to consult when resolving
/// the chord sequence. Pass `Some(id)` to surface buffer-local
/// bindings (matching what `dispatch_key` would see) and `None`
/// for global-only resolution. Buffer-scope keys (e.g.,
/// `pmacs-magit.stage` bound to `s` on the magit buffer) are
/// invisible without this, which is the M8.7 describe-key gap.
pub fn render_key(
    registry: &mut BufferRegistry,
    commands: &CommandRegistry,
    keymaps: &KeymapStack,
    active_buffer: Option<BufferId>,
    sequence: &str,
) -> RenderResult {
    let chords = parse_sequence(sequence).ok()?;
    let resolution = keymaps.resolve(&chords, active_buffer, &[]);
    let StackResolution::Bound(rb) = resolution else {
        return None;
    };
    let mut text = String::new();
    let _ = writeln!(text, "Key: {}", display_sequence(&chords));
    let _ = writeln!(text, "  Scope: {}", rb.scope.render());
    let _ = writeln!(text, "  Source: {}", rb.binding.source.render());
    let _ = writeln!(text);
    let _ = writeln!(text, "Runs: [command: {}]", rb.binding.command);
    if let Some(cmd) = commands.get(&rb.binding.command) {
        let _ = writeln!(text);
        let _ = writeln!(text, "{}", cmd.description);
    }
    Some(replace_help_buffer(registry, &text))
}

/// Render help for a buffer id. Returns [`None`] if the id is stale.
pub fn render_buffer(registry: &mut BufferRegistry, id: BufferId) -> RenderResult {
    let body = {
        let buf = registry.get(id).ok()?;
        format_buffer_text(buf)
    };
    Some(replace_help_buffer(registry, &body))
}

fn format_buffer_text(buf: &Buffer) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "Buffer: {}", buf.name());
    let _ = writeln!(text, "  Length:   {} bytes", buf.len());
    let _ = writeln!(text, "  Modified: {}", buf.is_modified());
    let _ = writeln!(text, "  Views:    {}", buf.view_count());
    let _ = writeln!(text);
    let _ = writeln!(text, "See also: [view: {}]", buf.name());
    text
}

/// Render help for a mode name. Returns [`None`] if no mode keymap
/// exists.
pub fn render_mode(
    registry: &mut BufferRegistry,
    keymaps: &KeymapStack,
    name: &str,
) -> RenderResult {
    let map = keymaps
        .modes
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, m)| m)?;
    let mut text = String::new();
    let _ = writeln!(text, "Mode: {name}");
    let _ = writeln!(text);
    write_mode_bindings(&mut text, map);
    Some(replace_help_buffer(registry, &text))
}

/// Render help for a hook name. Returns [`None`] if no such hook is
/// defined. Callbacks are listed in registration order, satisfying
/// the M2.11 acceptance bullet about `describe-hook`.
pub fn render_hook(
    registry: &mut BufferRegistry,
    hooks: &HookRegistry,
    name: &str,
) -> RenderResult {
    let hook = hooks.get(name)?;
    let body = format_hook_text(hook);
    Some(replace_help_buffer(registry, &body))
}

fn format_hook_text(hook: &Hook) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "Hook: {}", hook.name);
    let _ = writeln!(text, "  Kind:   {}", hook.kind.as_str());
    let _ = writeln!(text, "  Source: {}", hook.source.render());
    let _ = writeln!(text);
    let _ = writeln!(text, "{}", hook.description);
    let _ = writeln!(text);
    if hook.callbacks.is_empty() {
        let _ = writeln!(text, "No callbacks attached.");
    } else {
        let _ = writeln!(text, "Callbacks (in execution order):");
        for (i, cb) in hook.callbacks.iter().enumerate() {
            let _ = writeln!(text, "  {}. {}", i + 1, cb.source.render());
        }
    }
    text
}

/// Render help for a view target. The "target" is the buffer the
/// views are attached to (looked up by id), since views aren't
/// addressable on their own from Lua. Returns [`None`] if the buffer
/// id is stale.
pub fn render_view(registry: &mut BufferRegistry, id: BufferId) -> RenderResult {
    let body = {
        let buf = registry.get(id).ok()?;
        format_view_text(buf)
    };
    Some(replace_help_buffer(registry, &body))
}

fn format_view_text(buf: &Buffer) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "Views attached to [buffer: {}]", buf.name());
    let _ = writeln!(text, "  Count: {}", buf.view_count());
    let ids: Vec<_> = buf.view_ids().collect();
    if ids.is_empty() {
        let _ = writeln!(text);
        let _ = writeln!(text, "No views currently attached.");
    } else {
        let _ = writeln!(text);
        for (i, vid) in ids.iter().enumerate() {
            let _ = writeln!(text, "  {}. {:?}", i + 1, vid);
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_command_bindings(
    registry: &BufferRegistry,
    out: &mut String,
    command: &str,
    keymaps: &KeymapStack,
) {
    let bindings: Vec<(Scope, Sequence, Binding)> = keymaps
        .iter_all()
        .into_iter()
        .filter(|(_, _, b)| b.command == command)
        .collect();
    if bindings.is_empty() {
        let _ = writeln!(out, "Bound to: (no keys)");
    } else {
        let _ = writeln!(out, "Bound to:");
        for (scope, seq, _) in &bindings {
            match scope {
                Scope::Buffer(id) if registry.contains(*id) => {
                    let _ = writeln!(
                        out,
                        "  [key: {} @buffer:{}]   ({})",
                        display_sequence(seq),
                        id.raw(),
                        scope.render()
                    );
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "  [key: {}]   ({})",
                        display_sequence(seq),
                        scope.render()
                    );
                }
            }
        }
    }
}

fn parse_key_target(registry: &BufferRegistry, target: &str) -> Option<(String, Option<BufferId>)> {
    let Some((sequence, raw)) = target.rsplit_once(" @buffer:") else {
        return Some((target.to_owned(), None));
    };
    let Ok(raw) = raw.trim().parse::<u64>() else {
        return None;
    };
    let id = BufferId::from_raw(raw);
    if registry.contains(id) {
        Some((sequence.trim().to_owned(), Some(id)))
    } else {
        None
    }
}

fn write_mode_bindings(out: &mut String, map: &Keymap) {
    let entries: Vec<_> = map.iter().collect();
    if entries.is_empty() {
        let _ = writeln!(out, "(empty mode keymap)");
        return;
    }
    let _ = writeln!(out, "Bindings:");
    for (seq, binding) in entries {
        let _ = writeln!(
            out,
            "  [key: {}]   -> [command: {}]",
            display_sequence(&seq),
            binding.command
        );
    }
}

fn replace_help_buffer(
    registry: &mut BufferRegistry,
    text: &str,
) -> (BufferId, Vec<crate::rope::Edit>) {
    let id = registry
        .find_by_name(HELP_BUFFER_NAME)
        .unwrap_or_else(|| registry.create(HELP_BUFFER_NAME));
    let buf = registry.get_mut(id).expect("just resolved");
    let mut edits = Vec::new();
    if !buf.is_empty() {
        let len = buf.len();
        if let Ok(edit) = buf.apply_edit(EditOp::Delete {
            range: crate::rope::Range::new(0, len),
        }) {
            edits.push(edit);
        }
    }
    if !text.is_empty() {
        if let Ok(edit) = buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: text.as_bytes(),
        }) {
            edits.push(edit);
        }
    }
    // The help buffer is regenerated content; mark it clean so the
    // modeline doesn't claim it has unsaved changes.
    buf.mark_clean();
    (id, edits)
}

// ---------------------------------------------------------------------------
// Cross-reference parsing
// ---------------------------------------------------------------------------

/// A parsed `[type: target]` token at a specific byte range in the help
/// text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    /// Byte offset of the opening `[` within the buffer.
    pub start: u64,
    /// Byte offset just past the closing `]`.
    pub end: u64,
    /// Token type: `command`, `key`, `buffer`, `mode`, `hook`, `view`,
    /// or `keymap` (catch-all for cross-section pointers).
    pub kind: String,
    /// Token target (everything after the `:` and before the `]`,
    /// trimmed).
    pub target: String,
}

/// Scan `text` (viewed as a single buffer body) and return the link
/// span that contains byte offset `cursor`, if any. Cursor on a
/// non-link character returns [`None`].
#[must_use]
pub fn link_at(text: &str, cursor: u64) -> Option<LinkSpan> {
    let bytes = text.as_bytes();
    let cursor = usize::try_from(cursor).ok()?;
    if cursor > bytes.len() {
        return None;
    }
    // Walk left from cursor to find an opening `[` on the same line.
    let line_start = bytes[..cursor]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    let line_end = bytes[cursor.min(bytes.len())..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| cursor.min(bytes.len()) + p);
    let line = &text[line_start..line_end];
    let cursor_in_line = cursor - line_start;

    // Find every `[...:...]` on the line and return the one containing
    // the cursor (or the closest one to it, biased to the right).
    let mut search_from = 0;
    while let Some(open_rel) = line[search_from..].find('[') {
        let open = search_from + open_rel;
        let close_rel = line[open..].find(']')?;
        let close = open + close_rel;
        let inner = &line[open + 1..close];
        if let Some((kind, target)) = inner.split_once(':') {
            let kind = kind.trim();
            let target = target.trim();
            if !kind.is_empty()
                && !target.is_empty()
                && cursor_in_line >= open
                && cursor_in_line <= close
            {
                let start = u64::try_from(line_start + open).ok()?;
                let end = u64::try_from(line_start + close + 1).ok()?;
                return Some(LinkSpan {
                    start,
                    end,
                    kind: kind.to_owned(),
                    target: target.to_owned(),
                });
            }
        }
        search_from = close + 1;
    }
    None
}

/// Read the help-buffer body as a UTF-8 string. Returns an empty
/// string for an empty buffer.
fn read_buffer_text(buf: &Buffer) -> String {
    let len = buf.len();
    if len == 0 {
        return String::new();
    }
    let mut out = vec![0u8; len as usize];
    buf.snapshot_rope().slice(0, len, &mut out);
    String::from_utf8(out).unwrap_or_default()
}

/// Result of [`follow_link_at`]: the help buffer id paired with the
/// Edits produced by the re-render (zero, one, or two), or [`None`]
/// if the cursor wasn't on a recognized link. Same broadcast-queueing
/// contract as [`RenderResult`].
pub type FollowResult = Option<(BufferId, Vec<crate::rope::Edit>)>;

/// Parse the link under the cursor in the `*help*` buffer and
/// re-render. Returns the help buffer id on success.
pub fn follow_link_at(
    registry: &mut BufferRegistry,
    commands: &CommandRegistry,
    keymaps: &KeymapStack,
    hooks: &HookRegistry,
    cursor: u64,
) -> FollowResult {
    let text = {
        let id = registry.find_by_name(HELP_BUFFER_NAME)?;
        let buf = registry.get(id).ok()?;
        read_buffer_text(buf)
    };
    let link = link_at(&text, cursor)?;
    match link.kind.as_str() {
        "command" => render_command(registry, commands, keymaps, &link.target),
        "key" => {
            let (sequence, active_buffer) = parse_key_target(registry, &link.target)?;
            render_key(registry, commands, keymaps, active_buffer, &sequence)
        }
        "buffer" => {
            let id = registry.find_by_name(&link.target)?;
            render_buffer(registry, id)
        }
        "mode" => render_mode(registry, keymaps, &link.target),
        "hook" => render_hook(registry, hooks, &link.target),
        "view" => {
            let id = registry.find_by_name(&link.target)?;
            render_view(registry, id)
        }
        // `[keymap: list]` could one day render the full keymap; for
        // M2.11 it's a documentation breadcrumb that doesn't navigate.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// SourceLocation helper for tests
// ---------------------------------------------------------------------------

/// Used internally; exposed for the integration test in the editor
/// module that defines a test command.
#[doc(hidden)]
#[must_use]
pub fn test_source_location(file: &str, line: i32) -> SourceLocation {
    SourceLocation {
        file: file.to_owned(),
        line,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
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

    fn read_help(reg: &BufferRegistry) -> String {
        let id = reg.find_by_name(HELP_BUFFER_NAME).expect("help buffer");
        let buf = reg.get(id).unwrap();
        read_buffer_text(buf)
    }

    #[test]
    fn render_command_writes_name_description_and_bindings() {
        let lua = Lua::new();
        let mut reg = BufferRegistry::new();
        let mut cmds = CommandRegistry::new();
        let mut kms = KeymapStack::new();
        cmds.define(make_command(&lua, "cursor.left", "Move cursor left."))
            .unwrap();
        kms.bind_global(
            &parse_sequence("C-b").unwrap(),
            "cursor.left",
            SourceLocation {
                file: "default.lua".into(),
                line: 1,
            },
        )
        .unwrap();
        let (id, _) = render_command(&mut reg, &cmds, &kms, "cursor.left").unwrap();
        let body = read_buffer_text(reg.get(id).unwrap());
        assert!(body.contains("Command: cursor.left"));
        assert!(body.contains("Move cursor left."));
        assert!(body.contains("[key: C-b]"));
    }

    #[test]
    fn render_command_unknown_returns_none() {
        let mut reg = BufferRegistry::new();
        let cmds = CommandRegistry::new();
        let kms = KeymapStack::new();
        assert!(render_command(&mut reg, &cmds, &kms, "nope").is_none());
    }

    #[test]
    fn render_key_for_bound_chord() {
        let lua = Lua::new();
        let mut reg = BufferRegistry::new();
        let mut cmds = CommandRegistry::new();
        let mut kms = KeymapStack::new();
        cmds.define(make_command(&lua, "save", "Save buffer."))
            .unwrap();
        kms.bind_global(
            &parse_sequence("C-x C-s").unwrap(),
            "save",
            SourceLocation {
                file: "default.lua".into(),
                line: 5,
            },
        )
        .unwrap();
        let (id, _) = render_key(&mut reg, &cmds, &kms, None, "C-x C-s").unwrap();
        let body = read_buffer_text(reg.get(id).unwrap());
        assert!(body.contains("Key: C-x C-s"));
        assert!(body.contains("[command: save]"));
        assert!(body.contains("Save buffer."));
    }

    #[test]
    fn render_key_unbound_returns_none() {
        let mut reg = BufferRegistry::new();
        let cmds = CommandRegistry::new();
        let kms = KeymapStack::new();
        assert!(render_key(&mut reg, &cmds, &kms, None, "C-q").is_none());
    }

    #[test]
    fn render_buffer_writes_metadata() {
        let mut reg = BufferRegistry::new();
        let id = reg.create_from_bytes("scratch", b"hello");
        let _ = render_buffer(&mut reg, id).unwrap();
        let body = read_help(&reg);
        assert!(body.contains("Buffer: scratch"));
        assert!(body.contains("Length:   5 bytes"));
    }

    #[test]
    fn render_hook_lists_callbacks_in_order() {
        let lua = Lua::new();
        let mut reg = BufferRegistry::new();
        let mut hooks = HookRegistry::new();
        hooks
            .define(
                "demo".into(),
                "Demo.".into(),
                crate::hook::HookKind::AllMustSucceed,
                SourceLocation {
                    file: "test.lua".into(),
                    line: 1,
                },
            )
            .unwrap();
        for line in [10, 11, 12] {
            let f = lua.create_function(|_, ()| Ok(())).unwrap();
            hooks
                .add(
                    "demo",
                    f,
                    SourceLocation {
                        file: "init.lua".into(),
                        line,
                    },
                )
                .unwrap();
        }
        let _ = render_hook(&mut reg, &hooks, "demo").unwrap();
        let body = read_help(&reg);
        assert!(body.contains("Callbacks (in execution order):"));
        let pos10 = body.find("init.lua:10").unwrap();
        let pos11 = body.find("init.lua:11").unwrap();
        let pos12 = body.find("init.lua:12").unwrap();
        assert!(pos10 < pos11 && pos11 < pos12, "order off: {body}");
    }

    #[test]
    fn render_hook_with_no_callbacks_says_so() {
        let mut reg = BufferRegistry::new();
        let mut hooks = HookRegistry::new();
        hooks
            .define(
                "empty".into(),
                "No subscribers.".into(),
                crate::hook::HookKind::AllMustSucceed,
                SourceLocation::default(),
            )
            .unwrap();
        let _ = render_hook(&mut reg, &hooks, "empty").unwrap();
        let body = read_help(&reg);
        assert!(body.contains("No callbacks attached."), "{body}");
    }

    #[test]
    fn render_mode_lists_bindings() {
        let mut reg = BufferRegistry::new();
        let mut kms = KeymapStack::new();
        kms.bind_mode(
            "demo",
            &parse_sequence("C-x").unwrap(),
            "x",
            SourceLocation::default(),
        )
        .unwrap();
        kms.bind_mode(
            "demo",
            &parse_sequence("C-y").unwrap(),
            "y",
            SourceLocation::default(),
        )
        .unwrap();
        let _ = render_mode(&mut reg, &kms, "demo").unwrap();
        let body = read_help(&reg);
        assert!(body.contains("Mode: demo"));
        assert!(body.contains("[key: C-x]"));
        assert!(body.contains("[command: x]"));
    }

    #[test]
    fn render_view_describes_attached_buffer() {
        let mut reg = BufferRegistry::new();
        let id = reg.create("viewless");
        let _ = render_view(&mut reg, id).unwrap();
        let body = read_help(&reg);
        assert!(
            body.contains("Views attached to [buffer: viewless]"),
            "{body}"
        );
        assert!(body.contains("No views currently attached."));
    }

    #[test]
    fn link_at_finds_command_token() {
        let text = "See [command: cursor.left] for details.\n";
        let cursor = text.find("cursor.left").unwrap() as u64;
        let span = link_at(text, cursor).unwrap();
        assert_eq!(span.kind, "command");
        assert_eq!(span.target, "cursor.left");
    }

    #[test]
    fn link_at_finds_key_token_with_spaces() {
        let text = "Try [key: C-x C-s] to save.\n";
        let cursor = text.find("C-x").unwrap() as u64;
        let span = link_at(text, cursor).unwrap();
        assert_eq!(span.kind, "key");
        assert_eq!(span.target, "C-x C-s");
    }

    #[test]
    fn link_at_off_link_returns_none() {
        let text = "Plain text with no [command: foo] here.\n";
        // Cursor on "Plain"
        assert!(link_at(text, 2).is_none());
    }

    #[test]
    fn follow_link_at_chases_command_to_command() {
        let lua = Lua::new();
        let mut reg = BufferRegistry::new();
        let mut cmds = CommandRegistry::new();
        let kms = KeymapStack::new();
        let hooks = HookRegistry::new();
        cmds.define(make_command(&lua, "alpha", "Alpha cmd."))
            .unwrap();
        cmds.define(make_command(&lua, "beta", "Beta cmd."))
            .unwrap();
        // Render one command, then write a manual cross-ref into the
        // help buffer pointing at another, and follow it.
        render_command(&mut reg, &cmds, &kms, "alpha").unwrap();
        let id = reg.find_by_name(HELP_BUFFER_NAME).unwrap();
        let suffix = "\nrelated: [command: beta]\n";
        let buf = reg.get_mut(id).unwrap();
        let pos = buf.len();
        buf.apply_edit(EditOp::Insert {
            pos,
            bytes: suffix.as_bytes(),
        })
        .unwrap();
        // Find cursor on the cross-ref.
        let body = read_help(&reg);
        let cursor = body.find("beta").unwrap() as u64;
        let (returned, _) = follow_link_at(&mut reg, &cmds, &kms, &hooks, cursor).unwrap();
        assert_eq!(returned, id);
        let body = read_help(&reg);
        assert!(body.contains("Command: beta"), "{body}");
    }

    #[test]
    fn follow_link_at_chases_command_to_buffer_local_key() {
        let lua = Lua::new();
        let mut reg = BufferRegistry::new();
        let target_buffer = reg.create("magit");
        let mut cmds = CommandRegistry::new();
        let mut kms = KeymapStack::new();
        let hooks = HookRegistry::new();
        cmds.define(make_command(&lua, "pmacs-magit.stage", "Stage item."))
            .unwrap();
        kms.bind_buffer(
            target_buffer,
            &parse_sequence("s").unwrap(),
            "pmacs-magit.stage",
            SourceLocation {
                file: "magit.lua".into(),
                line: 12,
            },
        )
        .unwrap();

        render_command(&mut reg, &cmds, &kms, "pmacs-magit.stage").unwrap();
        let body = read_help(&reg);
        assert!(
            body.contains(&format!("[key: s @buffer:{}]", target_buffer.raw())),
            "buffer-local key link must carry its buffer scope: {body}"
        );
        let cursor = body.find("s @buffer").unwrap() as u64;
        let (returned, _) = follow_link_at(&mut reg, &cmds, &kms, &hooks, cursor).unwrap();
        assert_eq!(returned, reg.find_by_name(HELP_BUFFER_NAME).unwrap());
        let body = read_help(&reg);
        assert!(body.contains("Key: s"), "{body}");
        assert!(body.contains("Scope: buffer"), "{body}");
        assert!(body.contains("[command: pmacs-magit.stage]"), "{body}");
    }
}
