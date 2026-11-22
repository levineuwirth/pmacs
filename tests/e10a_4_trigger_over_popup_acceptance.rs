// tests/e10a_4_trigger_over_popup_acceptance.rs --- E10a.4: a trigger
// character typed over an open popup asks for its list.

//! The baseline read this as the GPU drawing no completion popup on the
//! window's last row. It is neither the GPU's nor the row's: with a
//! word's popup open, a `.` closed it and the driver stopped there, so
//! `self.` showed nothing in either frontend until a second letter
//! reopened the popup by the word rule (E10a.4 as the owner re-scoped
//! it, 2026-11-22). The GPU draws a popup above the caret on its last
//! row whenever one is open, at the base too, and the last row here
//! holds that as a guard.
//!
//! These rows type a word until its popup shows, then the `.`, against
//! the fake server answering from a plan each row writes
//! (`PMACS_FAKE_LSP_COMPLETION_PLAN`), and read the popup: in the
//! editor, on a real TUI's screen, and in the GPU's window through the
//! headless probe's production `App`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod common;
#[path = "support/mod.rs"]
mod support;

use common::iso;

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn cursor(s: &EditorState) -> u64 {
    let c: i64 = eval(s, "return pmacs.editor.cursor()");
    u64::try_from(c).expect("cursor fits")
}

fn popup_visible(s: &EditorState) -> bool {
    eval(s, "return pmacs.completion.popup_visible()")
}

/// The open popup's rows, in order, and the byte it is anchored at.
fn popup(s: &EditorState) -> Option<(Vec<String>, u64)> {
    let core = s.core.borrow();
    let guard = core.completion_popup.lock().unwrap();
    guard.as_ref().map(|p| {
        (
            p.candidates.iter().map(|c| c.label.clone()).collect(),
            p.anchor,
        )
    })
}

fn labels(s: &EditorState) -> Vec<String> {
    popup(s).map(|(rows, _)| rows).unwrap_or_default()
}

/// Pump until the popup's rows are exactly `rows`.
fn wait_rows(s: &mut EditorState, rows: &[&str], what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while labels(s) != rows {
        assert!(
            Instant::now() < deadline,
            "{what}: the popup must show {rows:?}; it shows {:?}, status {:?}",
            labels(s),
            status(s)
        );
        tick(s);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Pump for `ms`, as the run loop would, so an answer that is going to
/// open a popup has opened it.
fn pump_for(s: &mut EditorState, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        tick(s);
        std::thread::sleep(Duration::from_millis(2));
    }
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

const HEAD: &str =
    "struct Book { entries: Vec<u8> }\nimpl Book {\n    fn count(&self) -> usize {\n";
const TAIL: &str = "\n    }\n}\n// tail\n";
const INDENT: &str = "        ";

fn whole(line: &str) -> String {
    format!("{HEAD}{line}{TAIL}")
}

fn items(labels: &[&str]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = labels
        .iter()
        .map(|label| serde_json::json!({ "label": label, "kind": 2 }))
        .collect();
    serde_json::json!({ "isIncomplete": false, "items": items })
}

/// The answer for the word: one item, which `va` narrows to.
fn word_plan() -> serde_json::Value {
    items(&["vault"])
}

/// The answer for the place after the dot: the receiver's members.
fn member_plan() -> serde_json::Value {
    items(&["capacity", "len"])
}

struct Fixture {
    dir: PathBuf,
    file: PathBuf,
    plan: PathBuf,
}

fn fixture(tag: &str, text: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10a4-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, text).unwrap();
    let plan = dir.join("plan.json");
    Fixture { dir, file, plan }
}

impl Fixture {
    fn write_plan(&self, plan: &serde_json::Value) {
        std::fs::write(&self.plan, serde_json::to_string(plan).unwrap()).unwrap();
    }

    /// The Lua that points the rust server at the fake answering from
    /// the plan file, read anew at each request.
    fn server_config(&self) -> String {
        format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.lsp.config.rust = {{\n\
               command = '{}',\n\
               env = {{ PMACS_FAKE_LSP_COMPLETION_PLAN = '{}' }},\n\
             }}\n",
            fake_lsp_path(),
            self.plan.display()
        )
    }
}

/// An editor on `HEAD`, an indented empty line and `TAIL`, the caret at
/// the end of that line, the fake initialized and answering `plan`.
fn editor_on(tag: &str, plan: &serde_json::Value) -> (EditorState, Fixture) {
    let fx = fixture(tag, &whole(INDENT));
    fx.write_plan(plan);
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(fx.dir.clone()));
    exec(&s, &fx.server_config());
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            fx.file.display().to_string()
        ),
    );
    let mut s = s;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !eval::<bool>(&s, &format!("return {INITIALIZED}")) {
        assert!(Instant::now() < deadline, "fake server init");
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(2));
    }
    exec(
        &s,
        &format!("pmacs.editor.goto_byte({})", HEAD.len() + INDENT.len()),
    );
    (s, fx)
}

/// Type `va` and wait for the word's popup, then give the fake the
/// answer it has for the place after the dot.
fn word_popup_open(s: &mut EditorState, fx: &Fixture) {
    type_str(s, "va");
    wait_rows(s, &["vault"], "the word's popup");
    fx.write_plan(&member_plan());
}

// ---------------------------------------------------------------------------
// The dot over an open popup
// ---------------------------------------------------------------------------

/// A word's popup is open and the user types the `.`. The word's rows
/// go at once, since they are the answer for another place, and the
/// member list opens when the server's answer for the dot lands,
/// anchored at the caret; typing narrows it and `RET` accepts from it.
/// At the base no popup opened after the dot at all.
#[test]
fn e10a_4_a_dot_typed_over_an_open_word_popup_asks_for_the_member_list() {
    let (mut s, fx) = editor_on("dot", &word_plan());
    word_popup_open(&mut s, &fx);
    type_str(&mut s, ".");
    assert!(
        !labels(&s).contains(&"vault".to_owned()),
        "the word's rows are not shown at the dot: {:?}",
        labels(&s)
    );
    wait_rows(&mut s, &["capacity", "len"], "the member list at the dot");
    let after_dot = cursor(&s);
    assert_eq!(
        popup(&s).map(|(_, anchor)| anchor),
        Some(after_dot),
        "the list is anchored at the caret, after the dot"
    );
    type_str(&mut s, "le");
    assert_eq!(labels(&s), ["len"], "typing narrows the member list");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        whole("        va.len"),
        "RET accepts from the member list"
    );
}

/// The word's popup was dismissed, `Esc`, before the dot: the driver
/// then finds the popup closed under a session it opened, the branch it
/// takes after an accept, and stopped there too. The dot still asks.
#[test]
fn e10a_4_a_dot_after_a_dismissed_popup_asks_for_the_member_list() {
    let (mut s, fx) = editor_on("dismissed", &word_plan());
    word_popup_open(&mut s, &fx);
    press(&mut s, KeyCode::Esc);
    assert!(!popup_visible(&s), "premise: Esc dismissed the popup");
    type_str(&mut s, ".");
    wait_rows(&mut s, &["capacity", "len"], "the member list at the dot");
}

// ---------------------------------------------------------------------------
// What still opens nothing
// ---------------------------------------------------------------------------

/// A character the server does not name as a trigger closes the word's
/// popup and opens none, as before: a space, and a semicolon.
#[test]
fn e10a_4_a_character_that_is_no_trigger_closes_the_popup_and_opens_none() {
    for ch in [" ", ";"] {
        let (mut s, fx) = editor_on("plain", &word_plan());
        word_popup_open(&mut s, &fx);
        type_str(&mut s, ch);
        pump_for(&mut s, 400);
        assert!(
            !popup_visible(&s),
            "{ch:?} is no trigger: no popup; it shows {:?}",
            labels(&s)
        );
    }
}

/// An accepted candidate whose own text ends in the trigger character
/// moves the caret one byte past what was typed, the signature of a
/// typed character. It is an accept and not a typed dot, so no list is
/// asked for and the popup stays closed, as after any accept.
#[test]
fn e10a_4_an_accept_whose_text_ends_in_a_trigger_does_not_raise_the_popup_again() {
    let plan = serde_json::json!({
        "isIncomplete": false,
        "items": [{ "label": "va. (the receiver and its dot)", "kind": 15,
                    "filterText": "va", "insertText": "va." }]
    });
    let (mut s, fx) = editor_on("accept", &plan);
    type_str(&mut s, "va");
    wait_rows(
        &mut s,
        &["va. (the receiver and its dot)"],
        "the word's popup",
    );
    fx.write_plan(&member_plan());
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        whole("        va."),
        "premise: the accept left the caret one byte on, after a dot"
    );
    pump_for(&mut s, 500);
    assert!(
        !popup_visible(&s),
        "an accept raises no popup; it shows {:?}",
        labels(&s)
    );
}

// ---------------------------------------------------------------------------
// The TUI's screen
// ---------------------------------------------------------------------------

/// The baseline's keys on a real TUI in a PTY: a word typed until its
/// popup is on screen, then the dot. The member rows come on screen. At
/// the base the popup went and nothing replaced it.
#[test]
fn e10a_4_the_tui_shows_the_member_list_after_the_dot() {
    use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

    let fx = fixture("tui", &whole(INDENT));
    // One answer for every request: `va` narrows it to the word, and the
    // dot, with no prefix, shows the rest.
    fx.write_plan(&items(&["vault", "capacity", "len"]));
    let config = fx.dir.join("config");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::write(config.join("pmacs/init.lua"), fx.server_config()).unwrap();
    let file = fx.file.display().to_string();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &[&file],
        &[
            ("HOME", fx.dir.as_path()),
            ("XDG_CONFIG_HOME", config.as_path()),
        ],
        24,
        80,
    );
    let mut tui = ScreenTui::new(pty, tap, 24, 80);
    let limit = Duration::from_secs(30);
    tui.wait_for(limit, "the server ready on the mode line", &|t| {
        t.screen_contains("LSP:ready")
    });
    // M-x goto-line RET 4 RET, C-e: the end of the indented line.
    tui.write(b"\x1bx");
    tui.wait_for(limit, "the M-x prompt", &|t| t.screen_contains("M-x"));
    tui.write(b"goto-line\r4\r");
    tui.wait_for(limit, "the caret on line 4", &|t| {
        t.screen_contains("L4:C1 ")
    });
    tui.write(b"\x05");
    tui.wait_for(limit, "the caret at the line's end", &|t| {
        t.screen_contains("L4:C9 ")
    });
    tui.write(b"va");
    tui.wait_for(limit, "the word's popup on screen", &|t| {
        t.screen_contains("vault") && !t.screen_contains("capacity")
    });
    tui.write(b".");
    tui.wait_for(
        Duration::from_secs(10),
        "the member list after the dot",
        &|t| t.screen_contains("capacity") && t.screen_contains("len"),
    );
    assert!(
        tui.screen_contains("va."),
        "the dot is in the text; {}",
        tui.describe()
    );
}

// ---------------------------------------------------------------------------
// The GPU window, on its last row
// ---------------------------------------------------------------------------

#[cfg(feature = "crdt")]
fn floats(text: &str) -> Vec<f32> {
    text.split(',')
        .map(|part| {
            part.parse()
                .unwrap_or_else(|_| panic!("a number in {text:?}"))
        })
        .collect()
}

/// Where the baseline met it: the GPU window with the caret on its last
/// row, a word's popup open, then the dot. The member list opens, and
/// the dropdown is painted above the caret's row and inside the document
/// area, there being no room below. At the base the popup was closed
/// after the dot; the placement above the caret is the base's own and is
/// held here.
#[cfg(feature = "crdt")]
#[test]
fn e10a_4_the_gpu_window_shows_the_member_list_above_the_caret_on_its_last_row() {
    use common::daemon::TestDaemon;
    use std::fmt::Write as _;

    let mut text = String::new();
    for i in 0..80 {
        let _ = writeln!(text, "// line {i}");
    }
    let fx = fixture("gpu", &text);
    fx.write_plan(&items(&["vault", "capacity", "len"]));
    let init = format!(
        "{}pmacs.buffer.find_or_open({:?})\n",
        fx.server_config(),
        fx.file.display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    // A jump lands its line on the window's last row; `C-e`, `RET` opens
    // a line below it, which becomes the last row.
    let steps = "shows:a.rs quiet:1500 key:M-x type:goto-line key:ret type:40 key:ret quiet:500 \
                 key:C-e key:ret quiet:400 type:va quiet:1200 dropdown:word \
                 type:. quiet:1500 dropdown:dot place:dot";
    let Some(facts) = common::gpu_probe::run_script_probe(daemon.socket_path(), "e10a4", steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert_eq!(
        (fact("word.open"), fact("word.first")),
        ("true".to_owned(), "vault".to_owned()),
        "premise: the word's popup is open before the dot; facts {facts:?}"
    );
    assert_eq!(
        (fact("dot.open"), fact("dot.rows"), fact("dot.first")),
        ("true".to_owned(), "3".to_owned(), "capacity".to_owned()),
        "the member list is open after the dot; facts {facts:?}"
    );
    assert_ne!(
        fact("dot.box"),
        "none",
        "the dropdown is paintable; facts {facts:?}"
    );
    let area = floats(&fact("dot.area"));
    let (area_top, area_bottom) = (area[0], area[1]);
    let dropdown = floats(&fact("dot.box"));
    let (top, height) = (dropdown[1], dropdown[3]);
    let anchor = floats(&fact("dot.anchor_row"));
    let (row_top, row_height) = (anchor[0], anchor[1]);
    let shown: f32 = fact("dot.shown").parse().unwrap();
    assert!(
        row_top + row_height + height / shown > area_bottom,
        "premise: the caret is on the last row, with no room for one dropdown row below it; \
         facts {facts:?}"
    );
    assert!(
        top >= area_top && top + height <= row_top + 0.5,
        "the dropdown is above the caret's row and inside the document area; facts {facts:?}"
    );
    assert!(
        (shown - 3.0).abs() < f32::EPSILON,
        "all three rows are shown; facts {facts:?}"
    );
}
