// tests/e7_auto_import_acceptance.rs --- E7.4: a completion item's
// `additionalTextEdits` applied on accept.

//! An accepted completion applies the item's `additionalTextEdits` ---
//! the auto-import shape --- inside the accept's own command. The edits
//! are bytes of the text the server answered for, carried across every
//! edit recorded since through the semantic-token store's log (E6b's
//! recorder), never read against the current text. Probed as a user
//! meets it: an import landing above the caret leaves the caret and the
//! just-typed text where the user left them; an edit below the caret
//! lands where the server meant it although the accept's own replace
//! moved it; an accept after the buffer changed since the request still
//! places it; an item whose edits the log cannot place applies nothing
//! and says so; one undo takes the import with the completion.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "common/iso.rs"]
mod iso;

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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_lua_flag(s: &mut EditorState, flag: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        let done: bool = s
            .lua_host
            .lua()
            .load(format!("return ({flag}) == true"))
            .eval()
            .unwrap_or(false);
        if done {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
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

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7imp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// An editor with isolated roots, the rust server pointed at the fake
/// whose every completion item carries one extra edit inserting `text`
/// at `(line, col)`, visiting `dir/a.rs` holding `body`, returning once
/// the fake has initialized.
fn fake_editor(dir: &Path, body: &str, line: u32, col: u32, text: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = fake_lsp_path();
    let spec = format!("{line}:{col}:{}", text.replace('\n', "\\n"));
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{ PMACS_FAKE_LSP_COMPLETION_EXTRA_EDIT = '{spec}' }},
             }}"
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, body).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    s
}

/// Put the caret at `byte`, type `typed`, and pump until the popup is
/// showing with the server's answer (the fake's items all begin with
/// `p`; `printl` narrows them to `println`).
fn type_to_popup(s: &mut EditorState, byte: usize, typed: &str) {
    exec(s, &format!("pmacs.editor.goto_byte({byte})"));
    type_str(s, typed);
    assert!(
        pump_lua_flag(s, "pmacs.completion.popup_visible()", 10),
        "the popup must open on the server's answer; status {:?}",
        status(s)
    );
}

// ---------------------------------------------------------------------------
// Above the caret
// ---------------------------------------------------------------------------

/// The import lands on line 0, the completion replaces the typed
/// prefix, and the caret sits at the end of the inserted text --- moved
/// down by exactly the import's length, so the user's place is kept.
/// One undo takes both.
#[test]
fn e7_4_an_import_above_the_caret_keeps_the_caret_on_the_typed_text() {
    let dir = temp_dir("above");
    let body = "fn main() {\n    \n}\n";
    let mut s = fake_editor(&dir, body, 0, 0, "use std::fmt;\n");
    // Caret inside the indent on line 1.
    type_to_popup(&mut s, "fn main() {\n    ".len(), "printl");
    press(&mut s, KeyCode::Enter);
    let expected = "use std::fmt;\nfn main() {\n    println!\n}\n";
    assert_eq!(buffer_text(&s), expected, "the import and the completion both landed");
    assert!(!popup_visible(&s), "accept closes the popup");
    assert_eq!(
        cursor(&s),
        u64::try_from("use std::fmt;\nfn main() {\n    println!".len()).unwrap(),
        "the caret is at the end of the inserted text, below the import"
    );
    assert!(
        !status(&s).starts_with("completion:"),
        "no refusal was said: {:?}",
        status(&s)
    );
    let this: Option<String> = eval(&s, "return pmacs.editor.this_command()");
    assert_eq!(this.as_deref(), Some("completion.accept"));

    // One undo: the accept and its import are one step.
    exec(&s, "pmacs.command.invoke('buffer.undo')");
    assert_eq!(
        buffer_text(&s),
        "fn main() {\n    printl\n}\n",
        "undo takes the import with the completion"
    );
}

// ---------------------------------------------------------------------------
// Below the caret: carried across the accept's own replace
// ---------------------------------------------------------------------------

/// An edit below the caret was answered for text holding `printl`; the
/// accept replaces that with `println!` first, two bytes longer, and
/// the edit still lands at the start of the line the server named.
/// Applying the server's byte offset against the current text would put
/// it two bytes early, inside the line above.
#[test]
fn e7_4_an_edit_below_the_caret_lands_where_the_server_meant_it() {
    let dir = temp_dir("below");
    let body = "fn main() {\n    \n}\n// tail\n";
    let mut s = fake_editor(&dir, body, 3, 0, "// imported\n");
    type_to_popup(&mut s, "fn main() {\n    ".len(), "printl");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        "fn main() {\n    println!\n}\n// imported\n// tail\n",
        "the edit is carried across the replace that moved its line"
    );
    assert_eq!(
        cursor(&s),
        u64::try_from("fn main() {\n    println!".len()).unwrap(),
        "an edit below the caret does not move it"
    );
}

// ---------------------------------------------------------------------------
// The buffer changed since the request
// ---------------------------------------------------------------------------

/// The popup opened on the answer for `printl`; the user then typed
/// another letter, and the popup re-published from the same answer
/// without waiting for a new one (the request for `println` is still
/// out). The accept replaces `println` with `println!`, and the extra
/// edit --- answered for text one keystroke and one replace ago ---
/// lands on its line. Nothing is applied against the stale offsets.
#[test]
fn e7_4_an_accept_after_the_buffer_changed_since_the_request_still_places_the_edit() {
    let dir = temp_dir("changed");
    let body = "fn main() {\n    \n}\n// tail\n";
    let mut s = fake_editor(&dir, body, 3, 0, "// imported\n");
    type_to_popup(&mut s, "fn main() {\n    ".len(), "printl");
    let base_before: u64 = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment(); \
         for _, c in ipairs(pmacs.completion.collect({ prefix = 'printl', uri = rec.uri })) do \
           if c.label == 'println' then return c.edits_base end \
         end \
         return -1",
    );
    // One more letter, and no tick: the buffer has changed since the
    // answer the popup shows, and the new request is unanswered.
    type_str(&mut s, "n");
    assert!(popup_visible(&s), "the session survives a letter that extends the prefix");
    let base_shown: u64 = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment(); \
         for _, c in ipairs(pmacs.completion.collect({ prefix = 'println', uri = rec.uri })) do \
           if c.label == 'println' then return c.edits_base end \
         end \
         return -1",
    );
    assert_eq!(
        base_shown, base_before,
        "positive control: the popup still carries the earlier answer's edit number"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        "fn main() {\n    println!\n}\n// imported\n// tail\n",
        "carried across the letter typed since the request and the replace"
    );
    assert_eq!(
        cursor(&s),
        u64::try_from("fn main() {\n    println!".len()).unwrap()
    );
}

// ---------------------------------------------------------------------------
// An edit the log cannot place
// ---------------------------------------------------------------------------

/// A candidate whose extra edits name a document the store keeps no log
/// for: the completion itself is inserted, the extra edits are not, and
/// the status line says so. Driven through `popup_show`, the same seam
/// the driver uses, with a carry the accept cannot place.
#[test]
fn e7_4_edits_the_log_cannot_place_apply_nothing_and_say_so() {
    let mut s = EditorState::new_with_roots(&iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    type_str(&mut s, "abc def\n");
    type_str(&mut s, "de");
    exec(
        &s,
        "local b = pmacs.window.buffer()
         pmacs.completion.popup_show {
           buffer = b,
           anchor = b:len() - 2,
           prefix = 'de',
           candidates = {
             { label = 'defined', insert_text = 'defined',
               additional_edits = { { start = 0, stop = 0, text = 'use x;\\n' } },
               edits_uri = 'file:///nowhere/never-opened.rs',
               edits_base = 0 },
           },
         }",
    );
    assert!(popup_visible(&s), "positive control: the synthetic popup is showing");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "abc def\ndefined", "the completion itself is inserted");
    assert!(
        status(&s).starts_with("completion: the item's extra edits were not applied"),
        "the refusal is said: {:?}",
        status(&s)
    );
    assert_eq!(cursor(&s), u64::try_from("abc def\ndefined".len()).unwrap());
}

/// A candidate the server placed outside the text it answered for is
/// marked unresolved on the way in, and the accept applies none of its
/// extra edits rather than a part.
#[test]
fn e7_4_an_unresolvable_edit_applies_none_of_the_item_s_edits() {
    let dir = temp_dir("unresolved");
    let body = "fn main() {\n    \n}\n";
    // Line 40 does not exist in a three-line document.
    let mut s = fake_editor(&dir, body, 40, 0, "// nowhere\n");
    type_to_popup(&mut s, "fn main() {\n    ".len(), "printl");
    let unresolved: bool = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment(); \
         for _, c in ipairs(pmacs.completion.collect({ prefix = 'printl', uri = rec.uri })) do \
           if c.label == 'println' then return c.edits_unresolved == true end \
         end \
         return false",
    );
    assert!(unresolved, "positive control: the item is marked unresolved");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "fn main() {\n    println!\n}\n");
    assert!(
        status(&s).contains("outside the text the server answered for"),
        "status: {:?}",
        status(&s)
    );
}

/// The item's edits survive the trip through Lua unchanged: what
/// `collect` hands out is what `popup_show` takes back.
#[test]
fn e7_4_the_carry_round_trips_through_the_driver() {
    let dir = temp_dir("carry");
    let body = "fn main() {\n    \n}\n";
    let mut s = fake_editor(&dir, body, 0, 0, "use std::fmt;\n");
    type_to_popup(&mut s, "fn main() {\n    ".len(), "printl");
    let (start, stop, text, uri_ok): (u64, u64, String, bool) = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment(); \
         for _, c in ipairs(pmacs.completion.collect({ prefix = 'printl', uri = rec.uri })) do \
           if c.label == 'println' then \
             local e = c.additional_edits[1] \
             return e.start, e.stop, e.text, c.edits_uri == rec.uri \
           end \
         end",
    );
    assert_eq!((start, stop, text.as_str(), uri_ok), (0, 0, "use std::fmt;\n", true));
}
