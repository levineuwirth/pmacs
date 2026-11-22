// tests/e10a_6_panel_close_focus_acceptance.rs --- E10a.6: closing a
// panel gives the keys back to the document.

//! A panel opened over what the band already shows remembers that
//! presentation, and `q` puts it back. Until E10a.6 the focus stayed in
//! the band whatever came back: with a terminal under a result list,
//! `q` handed the next `C-x b`, and every key after it, to the shell.
//! The focus now returns to where the closed panel was opened from: a
//! document window, or the band itself when it was opened from there.
//!
//! The result panel here is a list view opened by a command these rows
//! define, the primitive the references, symbols and git panels open
//! through; its `q` is `listview.quit`, which is `window.quit` in the
//! band. The rows read the focus in the editor, the buffer prompt on a
//! real TUI's screen, and the prompt in the GPU's window through the
//! headless probe.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::CellSize;
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::window::WindowId;
use std::path::PathBuf;
use std::time::Duration;

mod common;
#[path = "support/mod.rs"]
mod support;

use common::iso;

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

/// The command that opens the result panel, and the terminal's shell.
const INIT: &str = r#"
pmacs.lsp.config = {}
pmacs.terminal.profiles.sh = { command = "/bin/sh" }
pmacs.config.set("terminal.default-profile", "sh")
pmacs.command.define {
  name = "e10a6.results",
  description = "Open a result list in the panel.",
  fn = function()
    pmacs.listview.open {
      name = "*results*",
      header = "results   q quit",
      rows = {
        { text = "first-result", item = 1 },
        { text = "second-result", item = 2 },
      },
    }
  end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-c z", command = "e10a6.results" }
"#;

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    exec(&s, INIT);
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(24, 80));
    s
}

fn press(s: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

/// `C-c z`, the test command's chord: the result list.
fn open_results(s: &mut EditorState) {
    press(s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(s, KeyCode::Char('z'), KeyModifiers::NONE);
}

/// `C-x o` until `window` has the focus.
fn focus(s: &EditorState, window: WindowId) {
    for _ in 0..6 {
        if active_window(s) == window {
            return;
        }
        exec(s, "pmacs.window.focus_next()");
    }
    panic!("premise: `C-x o` reaches {window:?}");
}

fn active_window(s: &EditorState) -> WindowId {
    s.core.borrow().active_window_id()
}

fn side_window(s: &EditorState) -> Option<WindowId> {
    s.core.borrow().side_window_for(FrontendId::LOCAL)
}

/// The name of the buffer `window` shows.
fn shown_in(s: &EditorState, window: WindowId) -> String {
    eval(
        s,
        &format!(
            "return pmacs.describe.buffer(pmacs.window.buffer({})).name",
            window.raw()
        ),
    )
}

/// A terminal in the band, opened from the active document window, and
/// the focus back in that window: the band's window.
fn terminal_under(s: &EditorState) -> WindowId {
    let document = active_window(s);
    exec(
        s,
        r#"pmacs.terminal.open { command = "/bin/sh", args = { "-c", "sleep 30" } }"#,
    );
    let band = side_window(s).expect("the terminal opened in the band");
    assert_eq!(
        active_window(s),
        band,
        "premise: a terminal takes the focus"
    );
    exec(s, "pmacs.window.focus_next()");
    assert_eq!(
        active_window(s),
        document,
        "premise: the focus is back in the document"
    );
    band
}

// ---------------------------------------------------------------------------
// In the editor
// ---------------------------------------------------------------------------

/// A terminal in the band, a result list opened over it from the
/// document, `q`: the band shows the terminal again and the focus is in
/// the document. At the base the focus stayed in the band, on the
/// terminal.
#[test]
fn e10a_6_closing_a_panel_over_a_terminal_returns_the_focus_to_the_document() {
    let mut s = editor();
    let document = active_window(&s);
    let band = terminal_under(&s);
    let terminal = shown_in(&s, band);
    open_results(&mut s);
    assert_eq!(
        (active_window(&s), shown_in(&s, band)),
        (band, "*results*".to_owned()),
        "premise: the result list is in the band and has the focus"
    );
    press(&mut s, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(
        shown_in(&s, band),
        terminal,
        "the band shows the terminal again: it stays in the band's history"
    );
    assert_eq!(
        active_window(&s),
        document,
        "the focus is back in the document the list was opened from"
    );
}

/// A second list opened from inside the first, `q`: the first is shown
/// again and keeps the focus, since that is where the second was opened
/// from. A second `q` closes the first, opened from the document, and
/// the focus goes there. The first half is the base's behaviour, held.
#[test]
fn e10a_6_a_panel_opened_from_the_panel_gives_the_focus_back_to_the_panel() {
    let mut s = editor();
    let document = active_window(&s);
    let band = terminal_under(&s);
    open_results(&mut s);
    assert_eq!(
        active_window(&s),
        band,
        "premise: the first list has the focus"
    );
    exec(
        &s,
        r#"pmacs.listview.open { name = "*more*", header = "more", rows = { { text = "m", item = 1 } } }"#,
    );
    assert_eq!(
        (active_window(&s), shown_in(&s, band)),
        (band, "*more*".to_owned()),
        "premise: the second list replaced the first in the band"
    );
    press(&mut s, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(
        (active_window(&s), shown_in(&s, band)),
        (band, "*results*".to_owned()),
        "the first list is back and has the focus"
    );
    press(&mut s, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(
        active_window(&s),
        document,
        "closing the first list returns to the document it was opened from"
    );
}

/// The document window the list was opened from is closed while the
/// list is up. `q` then gives the focus to the document window that is
/// left, not to the terminal in the band.
#[test]
fn e10a_6_with_its_document_window_gone_the_focus_goes_to_the_one_that_is_left() {
    let mut s = editor();
    let first = active_window(&s);
    exec(
        &s,
        "pmacs.window.split_horizontal(); pmacs.window.focus_next()",
    );
    let second = active_window(&s);
    assert_ne!(first, second, "premise: the split made a second window");
    exec(
        &s,
        r#"pmacs.terminal.open { command = "/bin/sh", args = { "-c", "sleep 30" } }"#,
    );
    let band = side_window(&s).expect("the terminal opened in the band");
    focus(&s, second);
    open_results(&mut s);
    assert_eq!(active_window(&s), band, "premise: the list has the focus");
    // Round to the window the list was opened from, and close it.
    focus(&s, second);
    exec(&s, "pmacs.window.close()");
    focus(&s, band);
    assert_eq!(
        (active_window(&s), shown_in(&s, band)),
        (band, "*results*".to_owned()),
        "premise: in the list again, its window of origin closed"
    );
    press(&mut s, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(
        active_window(&s),
        first,
        "the focus goes to the document window that is left"
    );
}

// ---------------------------------------------------------------------------
// The TUI's screen
// ---------------------------------------------------------------------------

struct Fixture {
    dir: PathBuf,
    file: PathBuf,
    config: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10a6-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("config");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::write(config.join("pmacs/init.lua"), INIT).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "a line of the document\n").unwrap();
    Fixture { dir, file, config }
}

/// The baseline's keys on a real TUI in a PTY: `C-c t`, back to the
/// document, the result list, `q`, `C-x b`. The buffer prompt comes on
/// screen. At the base the two keys went to the shell, which showed
/// them, and no prompt opened.
#[test]
fn e10a_6_the_tui_reaches_the_buffer_prompt_after_closing_a_panel_over_a_terminal() {
    use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

    let fx = fixture("tui");
    let file = fx.file.display().to_string();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &[&file],
        &[
            ("HOME", fx.dir.as_path()),
            ("XDG_CONFIG_HOME", fx.config.as_path()),
        ],
        24,
        80,
    );
    let mut tui = ScreenTui::new(pty, tap, 24, 80);
    let limit = Duration::from_secs(30);
    tui.wait_for(limit, "the document", &|t| {
        t.screen_contains("a line of the document")
    });
    tui.write(b"\x03t");
    tui.wait_for(limit, "the terminal in the band", &|t| {
        t.screen_contains("*terminal:")
    });
    // The shell answers, so the band holds a live terminal with the keys.
    tui.write(b"echo in-$((40+2))\r");
    tui.wait_for(limit, "premise: the shell has the keys", &|t| {
        t.screen_contains("in-42")
    });
    // The escape, then `C-x o`: back to the document.
    tui.write(b"\x03\x18o");
    tui.write(b"\x03z");
    tui.wait_for(limit, "the result list in the band", &|t| {
        t.screen_contains("first-result")
    });
    tui.write(b"q");
    tui.wait_for(limit, "the terminal back in the band", &|t| {
        !t.screen_contains("first-result") && t.screen_contains("in-42")
    });
    tui.write(b"\x18b");
    tui.wait_for(
        Duration::from_secs(10),
        "the buffer prompt, the keys having reached the editor",
        &|t| t.screen_contains("Switch to buffer:"),
    );
}

// ---------------------------------------------------------------------------
// The GPU window
// ---------------------------------------------------------------------------

/// The same keys in the GPU's window, through the headless probe's
/// production `App`: after `q` the `C-x b` opens the buffer prompt. At
/// the base no prompt opened; the keys went to the terminal's child.
#[cfg(feature = "crdt")]
#[test]
fn e10a_6_the_gpu_window_reaches_the_buffer_prompt_after_closing_a_panel_over_a_terminal() {
    use common::daemon::TestDaemon;

    let fx = fixture("gpu");
    let init = format!(
        "{INIT}pmacs.buffer.find_or_open({:?})\n",
        fx.file.display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let steps = "shows:notes.txt quiet:800 key:C-c key:t quiet:1200 place:terminal \
                 key:C-c key:C-x key:o quiet:400 place:document \
                 key:C-c key:z quiet:800 place:list \
                 key:q quiet:800 place:closed \
                 key:C-x key:b quiet:800 place:asked";
    let Some(facts) = common::gpu_probe::run_script_probe(daemon.socket_path(), "e10a6", steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert_eq!(
        (fact("terminal.panel"), fact("document.panel")),
        ("focused".to_owned(), "unfocused".to_owned()),
        "premise: the terminal took the keys, and `C-c C-x o` gave them back; facts {facts:?}"
    );
    assert!(
        fact("list.panel") == "focused"
            && fact("list.panel_text").contains("first-result")
            && fact("list.prompt") == "\"\"",
        "premise: the list is up in the band with the focus, and no prompt is open; \
         facts {facts:?}"
    );
    assert!(
        !fact("closed.panel_text").contains("first-result"),
        "premise: `q` closed the list; facts {facts:?}"
    );
    assert_eq!(
        fact("closed.panel"),
        "unfocused",
        "the band, the terminal in it again, does not have the keys; facts {facts:?}"
    );
    assert_eq!(
        fact("asked.prompt"),
        "\"Switch to buffer: \"",
        "`C-x b` after the close opens the buffer prompt; facts {facts:?}"
    );
}
