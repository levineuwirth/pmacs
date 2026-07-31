//! Overlay re-attach on buffer switch (Arc 1b, PR #94 validation
//! finding 2): `switch_active_buffer` clears the window's overlays,
//! and the runtime's dedup tables blocked re-attachment — so plain
//! `C-x b`, buffer-list visits, and panel navigation permanently
//! stripped syntax/LSP styling ("the LSP doesn't activate if I
//! navigate to a reference"). The `buffer.after-switch` hook now
//! re-pushes the views.
//!
//! Hermetic: only the tree-sitter `syntax-highlight` overlay is
//! asserted (always available for `.rs`); the LSP style/diag views
//! ride the same hook but need a live server.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;

fn open_probe_file(_s: &EditorState) -> String {
    let dir = std::env::temp_dir().join(format!("pmacs-ovl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mk tempdir");
    let file = dir.join("probe.rs");
    std::fs::write(&file, "fn main() {}\n").expect("write probe file");
    file.display().to_string()
}

/// `(overlay kinds on the active window, active buffer name)`.
fn kinds(s: &EditorState) -> (Vec<String>, String) {
    s.lua_host
        .lua()
        .load(
            r"
            local d = pmacs.describe.buffer(pmacs.window.buffer())
            return pmacs.window._overlay_kinds(), d.name
            ",
        )
        .eval()
        .expect("probe overlay kinds")
}

fn count_of(kinds: &[String], kind: &str) -> usize {
    kinds.iter().filter(|k| *k == kind).count()
}

#[test]
fn switch_away_and_back_reattaches_syntax_overlay_exactly_once() {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    let path = open_probe_file(&s);
    s.lua_host
        .lua()
        .load(format!(
            r#"
            _G.TARGET = pmacs.buffer.find_or_open("{path}")
            for _, id in ipairs(pmacs.buffer.list()) do
              if pmacs.describe.buffer(id).name == "*scratch*" then _G.SCRATCH = id end
            end
            "#
        ))
        .exec()
        .expect("open probe + find scratch");
    let (before, _) = kinds(&s);
    assert_eq!(
        count_of(&before, "syntax-highlight"),
        1,
        "fresh open attaches the highlight overlay once (got {before:?})"
    );

    // Away and back — twice, so stacking would show as a count > 1.
    s.lua_host
        .lua()
        .load(
            r"
            pmacs.window.switch_buffer(_G.SCRATCH)
            pmacs.window.switch_buffer(_G.TARGET)
            pmacs.window.switch_buffer(_G.SCRATCH)
            pmacs.window.switch_buffer(_G.TARGET)
            ",
        )
        .exec()
        .expect("switch away and back twice");
    let (after, name) = kinds(&s);
    assert!(name.ends_with("probe.rs"), "back on the probe file");
    assert_eq!(
        count_of(&after, "syntax-highlight"),
        1,
        "the switch re-attaches exactly one highlight overlay (got {after:?})"
    );
}

#[test]
fn panel_quit_restores_overlays_on_the_source_buffer() {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    let path = open_probe_file(&s);
    s.lua_host
        .lua()
        .load(format!(
            r#"
            pmacs.buffer.find_or_open("{path}")
            pmacs.listview.open {{
              name = "*ovl-panel*",
              header = "h",
              rows = {{ {{ text = "row", item = 1 }} }},
            }}
            "#
        ))
        .exec()
        .expect("open probe + panel");
    // q leaves the panel back to the source file.
    let mut s = s;
    s.dispatch_key(
        pmacs::protocol::FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        },
    );
    let (after, name) = kinds(&s);
    assert!(name.ends_with("probe.rs"), "q restored the source buffer");
    assert_eq!(
        count_of(&after, "syntax-highlight"),
        1,
        "leaving a panel restores the source buffer's styling (got {after:?})"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
