// tests/e10a_1_completion_edit_acceptance.rs --- E10a.1: an accepted
// completion inserts the server's edit.

//! A completion item's `textEdit` says what to insert and what to
//! replace. Until E10a.1 nothing read it: the accept replaced the typed
//! word with `insertText`, or with the label, so rust-analyzer's
//! `iter().len(as ExactSizeIterator)` was typed into the buffer as it
//! reads in the list. These rows drive the accept as a user does, by
//! keys into the editor against the fake server, whose answer each row
//! writes beside its assertions (`PMACS_FAKE_LSP_COMPLETION_PLAN`), and
//! read the whole buffer afterwards, the text outside the edit's range
//! included.
//!
//! The range is in the text the server answered for. The popup goes on
//! showing an answer while the user types, so the range is carried to
//! the current text through the edit log the semantic-token store
//! keeps, as an item's `additionalTextEdits` are (E7.4).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "common/iso.rs"]
mod iso;

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

fn cursor(s: &EditorState) -> usize {
    let c: i64 = eval(s, "return pmacs.editor.cursor()");
    usize::try_from(c).expect("cursor fits")
}

fn popup_visible(s: &EditorState) -> bool {
    eval(s, "return pmacs.completion.popup_visible()")
}

/// The label of the popup's highlighted row: what the user sees
/// selected when they press `RET`.
fn selected_label(s: &EditorState) -> Option<String> {
    let core = s.core.borrow();
    let guard = core.completion_popup.lock().unwrap();
    guard
        .as_ref()
        .and_then(|p| p.selected_candidate().map(|c| c.label.clone()))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e10a1-{tag}-{}", std::process::id()));
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

/// The lines above and below the one every row edits. No word in them
/// begins with `le`, the prefix the rows type, so these lines offer the
/// popup no word of their own.
const HEAD: &str =
    "struct Book { entries: Vec<u8> }\nimpl Book {\n    fn count(&self) -> usize {\n";
const TAIL: &str = "\n    }\n}\n// tail\n";

/// A range on line 3, the edited line, in the server's columns.
fn range(start: u32, end: u32) -> serde_json::Value {
    serde_json::json!({
        "start": { "line": 3, "character": start },
        "end": { "line": 3, "character": end }
    })
}

/// An editor with isolated roots, the rust server pointed at the fake
/// answering every completion request with `plan`, visiting `dir/a.rs`
/// holding `HEAD`, `line` and `TAIL`, with the caret `caret` bytes into
/// `line`, returning once the fake has initialized. The fake negotiates
/// no position encoding, so its columns are UTF-16 units.
fn fake_editor(dir: &Path, line: &str, caret: usize, plan: &serde_json::Value) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let plan_file = dir.join("plan.json");
    std::fs::write(&plan_file, serde_json::to_string(plan).unwrap()).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{}',
               env = {{ PMACS_FAKE_LSP_COMPLETION_PLAN = '{}' }},
             }}",
            fake_lsp_path(),
            plan_file.display()
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, format!("{HEAD}{line}{TAIL}")).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    exec(
        &s,
        &format!("pmacs.editor.goto_byte({})", HEAD.len() + caret),
    );
    s
}

/// Type `typed` and pump until the popup shows the server's answer with
/// `label` highlighted: the row the user accepts is the plan's item.
/// The popup can open before the answer, on the buffer's own words (the
/// rest of the word the caret is in, for one), so its being visible is
/// not the wait.
fn type_to_popup(s: &mut EditorState, typed: &str, label: &str) {
    type_str(s, typed);
    let deadline = Instant::now() + Duration::from_secs(10);
    while selected_label(s).as_deref() != Some(label) {
        assert!(
            Instant::now() < deadline,
            "the popup must show the server's item {label:?} highlighted; it shows {:?}, status {:?}",
            selected_label(s),
            status(s)
        );
        tick(s);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn whole(line: &str) -> String {
    format!("{HEAD}{line}{TAIL}")
}

fn one_item(item: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "isIncomplete": false, "items": [item] })
}

/// No refusal and no fallback was said.
fn assert_nothing_said(s: &EditorState) {
    assert!(
        !status(s).starts_with("completion:"),
        "nothing was said about the accept: {:?}",
        status(s)
    );
}

// ---------------------------------------------------------------------------
// The label is not the text
// ---------------------------------------------------------------------------

/// rust-analyzer's shape, the baseline's defect 1: the label describes
/// the item and its `textEdit` holds the text. The accept inserts the
/// edit's `newText` over the edit's range, and one undo takes it back.
/// At the base the buffer reads `self.entries.iter().len(as
/// ExactSizeIterator)`.
#[test]
fn e10a_1_an_item_whose_label_is_not_its_text_inserts_the_servers_edit() {
    let dir = temp_dir("label");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({
        "label": "iter().len(as ExactSizeIterator)",
        "kind": 2,
        "filterText": "len",
        "textEdit": { "range": range(21, 23), "newText": "iter().len()" }
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "iter().len(as ExactSizeIterator)");
    press(&mut s, KeyCode::Enter);
    let after = "        self.entries.iter().len()";
    assert_eq!(
        buffer_text(&s),
        whole(after),
        "the edit's text is inserted, not the label"
    );
    assert!(!popup_visible(&s), "accept closes the popup");
    assert_eq!(
        cursor(&s),
        HEAD.len() + after.len(),
        "the caret is at the end of the inserted text"
    );
    assert_nothing_said(&s);

    exec(&s, "pmacs.command.invoke('buffer.undo')");
    assert_eq!(
        buffer_text(&s),
        whole("        self.entries.le"),
        "one undo takes the accept back to the typed word"
    );
}

// ---------------------------------------------------------------------------
// The edit starts before the typed prefix
// ---------------------------------------------------------------------------

/// A member list asked for at the `.`: the range is empty at the caret
/// when the request goes out, and the word is typed afterwards against
/// the same answer. The accept replaces exactly what was typed. A range
/// whose start moved past the typed text, as a range's start does
/// everywhere else in the edit log, would leave `le` in front of the
/// completion.
#[test]
fn e10a_1_a_member_list_asked_for_at_the_dot_replaces_the_word_typed_since() {
    let dir = temp_dir("dot");
    let line = "        ";
    let plan = one_item(&serde_json::json!({
        "label": "len (as ExactSizeIterator)",
        "kind": 2,
        "filterText": "len",
        "textEdit": { "range": range(10, 10), "newText": "len()" }
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    // `v` is one letter, too short to ask for anything; the driver opens
    // on a trigger character only after an edit it has seen.
    type_to_popup(&mut s, "v.", "len (as ExactSizeIterator)");
    // Typed against the answer for the text that ended at the dot. The
    // list is complete, so no second request goes out.
    type_str(&mut s, "le");
    assert_eq!(
        selected_label(&s).as_deref(),
        Some("len (as ExactSizeIterator)"),
        "the session survives the word typed into it"
    );
    assert_eq!(
        buffer_text(&s),
        whole("        v.le"),
        "positive control: the typed word is in the buffer before the accept"
    );
    press(&mut s, KeyCode::Enter);
    let after = "        v.len()";
    assert_eq!(
        buffer_text(&s),
        whole(after),
        "the typed word is replaced exactly: nothing of it is left, nothing before it is taken"
    );
    assert_eq!(cursor(&s), HEAD.len() + after.len());
    assert_nothing_said(&s);
}

/// The server's range begins before the word, at the `.`: an edit that
/// rewrites the access itself. The accept replaces from where the
/// server said, not from the start of the typed word.
#[test]
fn e10a_1_a_range_that_begins_before_the_word_is_replaced_from_its_own_start() {
    let dir = temp_dir("before");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({
        "label": "len (through an option)",
        "kind": 2,
        "filterText": "len",
        "textEdit": { "range": range(20, 23), "newText": "?.len()" }
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "len (through an option)");
    press(&mut s, KeyCode::Enter);
    let after = "        self.entries?.len()";
    assert_eq!(
        buffer_text(&s),
        whole(after),
        "the dot the range named is replaced with the word"
    );
    assert_eq!(cursor(&s), HEAD.len() + after.len());
    assert_nothing_said(&s);
}

// ---------------------------------------------------------------------------
// The range ends after the caret, and a letter is typed since
// ---------------------------------------------------------------------------

/// The caret is inside a word and the server's range covers all of it.
/// The popup opens on the answer for `le`; the user types `n`, the list
/// is incomplete so a new request is out and unanswered, and the popup
/// shows the earlier answer's item. The accept replaces the whole word
/// as it now stands, the letter typed since included, and leaves what
/// follows the range alone.
#[test]
fn e10a_1_a_range_past_the_caret_takes_the_rest_of_the_word_and_a_letter_typed_since() {
    let dir = temp_dir("past");
    let line = "        self.entries.ngth() + 1";
    let plan = serde_json::json!({
        "isIncomplete": true,
        "items": [{
            "label": "len (replacing the word)",
            "kind": 2,
            "filterText": "len",
            // `length`, as the text reads when `le` has been typed.
            "textEdit": { "range": range(21, 27), "newText": "len" }
        }]
    });
    let mut s = fake_editor(&dir, line, "        self.entries.".len(), &plan);
    type_to_popup(&mut s, "le", "len (replacing the word)");
    let base_of = |s: &EditorState, prefix: &str| -> i64 {
        eval(
            s,
            &format!(
                "local rec = pmacs.lsp.active_attachment(); \
                 for _, c in ipairs(pmacs.completion.collect({{ prefix = '{prefix}', uri = rec.uri }})) do \
                   if c.text_edit then return c.edits_base end \
                 end \
                 return -1"
            ),
        )
    };
    let base_before = base_of(&s, "le");
    assert!(
        base_before >= 0,
        "positive control: the item carries its range"
    );
    // One more letter, and no tick: the buffer has changed since the
    // answer the popup shows, and the new request is unanswered.
    type_str(&mut s, "n");
    assert_eq!(
        selected_label(&s).as_deref(),
        Some("len (replacing the word)"),
        "the session survives a letter that extends the prefix"
    );
    assert_eq!(
        base_of(&s, "len"),
        base_before,
        "positive control: the popup still carries the earlier answer's edit number"
    );
    assert_eq!(
        buffer_text(&s),
        whole("        self.entries.lenngth() + 1"),
        "positive control: the text the accept meets"
    );
    press(&mut s, KeyCode::Enter);
    let after = "        self.entries.len() + 1";
    assert_eq!(
        buffer_text(&s),
        whole(after),
        "the whole word is replaced and the text after the range is as it was"
    );
    assert_eq!(
        cursor(&s),
        HEAD.len() + "        self.entries.len".len(),
        "the caret is at the end of the inserted text"
    );
    assert_nothing_said(&s);
}

// ---------------------------------------------------------------------------
// additionalTextEdits beside a textEdit
// ---------------------------------------------------------------------------

/// An import beside the item's own edit: both land, the caret stays at
/// the end of the inserted text below the import, and one undo takes
/// both.
#[test]
fn e10a_1_an_items_extra_edits_land_beside_its_own_edit() {
    let dir = temp_dir("extra");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({
        "label": "iter().len(as ExactSizeIterator)",
        "kind": 2,
        "filterText": "len",
        "textEdit": { "range": range(21, 23), "newText": "iter().len()" },
        "additionalTextEdits": [{
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "newText": "use std::iter::ExactSizeIterator;\n"
        }]
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "iter().len(as ExactSizeIterator)");
    press(&mut s, KeyCode::Enter);
    let import = "use std::iter::ExactSizeIterator;\n";
    let after = "        self.entries.iter().len()";
    assert_eq!(
        buffer_text(&s),
        format!("{import}{}", whole(after)),
        "the import and the item's own edit both landed"
    );
    assert_eq!(
        cursor(&s),
        import.len() + HEAD.len() + after.len(),
        "the caret is at the end of the inserted text, below the import"
    );
    assert_nothing_said(&s);

    exec(&s, "pmacs.command.invoke('buffer.undo')");
    assert_eq!(
        buffer_text(&s),
        whole("        self.entries.le"),
        "one undo takes the import with the edit"
    );
}

// ---------------------------------------------------------------------------
// Columns in UTF-16
// ---------------------------------------------------------------------------

/// The fake counts columns in UTF-16 units, the specification's
/// default, and the line holds `é` before the word: one unit, two
/// bytes. The range is placed in the text the server answered for by
/// that encoding, once. Placed from the answer after its columns have
/// already been turned into bytes, the range would begin a byte late
/// and leave the word's first letter.
#[test]
fn e10a_1_a_range_after_a_wide_character_is_placed_by_the_servers_encoding() {
    let dir = temp_dir("utf16");
    let line = "        /* é */ self.entries.";
    assert_eq!(line.len(), 30, "the line is 29 units and 30 bytes");
    let plan = one_item(&serde_json::json!({
        "label": "iter().len(as ExactSizeIterator)",
        "kind": 2,
        "filterText": "len",
        "textEdit": { "range": range(29, 31), "newText": "iter().len()" }
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "iter().len(as ExactSizeIterator)");
    press(&mut s, KeyCode::Enter);
    let after = "        /* é */ self.entries.iter().len()";
    assert_eq!(buffer_text(&s), whole(after));
    assert_eq!(cursor(&s), HEAD.len() + after.len());
    assert_nothing_said(&s);
}

/// The same for an item's extra edit, which E7.4 placed from the
/// converted answer: an insertion the server put before `fn`, eight
/// units into a line whose `é` makes that byte nine, landed inside the
/// word at the base (`/* é */ fpub n first() {}`).
#[test]
fn e10a_1_an_extra_edit_after_a_wide_character_is_placed_by_the_servers_encoding() {
    let dir = temp_dir("utf16x");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({
        "label": "len",
        "kind": 2,
        "insertText": "len()",
        "additionalTextEdits": [{
            "range": {
                "start": { "line": 7, "character": 8 },
                "end": { "line": 7, "character": 8 }
            },
            "newText": "pub "
        }]
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    // A line of the fixture's own with a wide character, below the edit.
    exec(
        &s,
        "local b = pmacs.window.buffer(); b:insert(b:len(), '/* é */ fn first() {}\\n')",
    );
    let lead = "/* é */ fn first() {}\n";
    assert_eq!(
        buffer_text(&s),
        format!("{}{lead}", whole(line)),
        "positive control: line 7 is the wide line"
    );
    exec(
        &s,
        &format!("pmacs.editor.goto_byte({})", HEAD.len() + line.len()),
    );
    type_to_popup(&mut s, "le", "len");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        format!(
            "{}/* é */ pub fn first() {{}}\n",
            whole("        self.entries.len()")
        ),
        "the extra edit lands at the server's column, before `fn`"
    );
    assert_nothing_said(&s);
}

// ---------------------------------------------------------------------------
// Items without a textEdit: unchanged
// ---------------------------------------------------------------------------

/// An item with `insertText` and no `textEdit` replaces the typed word
/// with `insertText`, as before.
#[test]
fn e10a_1_an_item_with_only_insert_text_replaces_the_typed_word_with_it() {
    let dir = temp_dir("insert");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({
        "label": "len (the count)",
        "kind": 2,
        "filterText": "len",
        "insertText": "len()"
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "len (the count)");
    press(&mut s, KeyCode::Enter);
    let after = "        self.entries.len()";
    assert_eq!(buffer_text(&s), whole(after));
    assert_eq!(cursor(&s), HEAD.len() + after.len());
    assert_nothing_said(&s);
}

/// An item with a label alone replaces the typed word with the label,
/// as before.
#[test]
fn e10a_1_an_item_with_only_a_label_replaces_the_typed_word_with_it() {
    let dir = temp_dir("bare");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({ "label": "length", "kind": 5 }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "length");
    press(&mut s, KeyCode::Enter);
    let after = "        self.entries.length";
    assert_eq!(buffer_text(&s), whole(after));
    assert_eq!(cursor(&s), HEAD.len() + after.len());
    assert_nothing_said(&s);
}

// ---------------------------------------------------------------------------
// An InsertReplaceEdit the client did not ask for
// ---------------------------------------------------------------------------

/// The client declares no `insertReplaceSupport`, so a conforming
/// server sends a plain `TextEdit`. One that sends both ranges anyway is
/// read by its `insert` range, the one that ends at the request's
/// position: the typed word is replaced and the rest of the word after
/// the caret stays.
#[test]
fn e10a_1_an_insert_replace_edit_sent_unasked_is_read_by_its_insert_range() {
    let dir = temp_dir("insrep");
    let line = "        self.entries.ngth()";
    let plan = one_item(&serde_json::json!({
        "label": "iter().len(as ExactSizeIterator)",
        "kind": 2,
        "filterText": "len",
        "textEdit": {
            "insert": range(21, 23),
            "replace": range(21, 27),
            "newText": "iter().len()"
        }
    }));
    let mut s = fake_editor(&dir, line, "        self.entries.".len(), &plan);
    type_to_popup(&mut s, "le", "iter().len(as ExactSizeIterator)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        whole("        self.entries.iter().len()ngth()"),
        "the insert range is replaced; the text after the caret is kept"
    );
    assert_nothing_said(&s);
}

// ---------------------------------------------------------------------------
// A range that cannot be placed at the caret
// ---------------------------------------------------------------------------

/// A range that does not hold the caret --- the specification requires
/// that it does, and an item from an answer for another place does not
/// --- is not applied where it points. The typed word is replaced with
/// the edit's text, the line the range named is untouched, and the echo
/// line says so.
#[test]
fn e10a_1_a_range_that_does_not_hold_the_caret_replaces_the_typed_word_and_says_so() {
    let dir = temp_dir("elsewhere");
    let line = "        self.entries.";
    let plan = one_item(&serde_json::json!({
        "label": "iter().len(as ExactSizeIterator)",
        "kind": 2,
        "filterText": "len",
        // `struct`, on line 0.
        "textEdit": {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 6 }
            },
            "newText": "iter().len()"
        }
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_to_popup(&mut s, "le", "iter().len(as ExactSizeIterator)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        whole("        self.entries.iter().len()"),
        "line 0 is as it was and the typed word holds the edit's text"
    );
    assert!(
        status(&s).starts_with("completion: the server's range for this item could not be placed"),
        "the fallback is said: {:?}",
        status(&s)
    );
}

/// The store keeps an answer until the next one lands. The user types a
/// word the server's items do not match, so no popup shows, then a `.`:
/// the popup opens at once on what the store holds, the answer for the
/// word, while the request made at the `.` is unanswered. That answer's
/// range is the word, and a range's end follows text typed at it, so
/// carried to now it reads `entries.` and holds the caret. Accepted in
/// that moment, the item must not take the word and the dot with it:
/// its request was made before the place this session completes, so
/// its range is not used, and the echo line says so.
#[test]
fn e10a_1_an_answer_for_the_word_before_the_dot_does_not_take_that_word() {
    let dir = temp_dir("stale");
    let line = "        self.entri";
    let plan = one_item(&serde_json::json!({
        "label": "entrance",
        "kind": 5,
        // `entries`, as the text reads when `es` has been typed.
        "textEdit": { "range": range(13, 20), "newText": "entrance" }
    }));
    let mut s = fake_editor(&dir, line, line.len(), &plan);
    type_str(&mut s, "es");
    let stored = "(function() \
       local rec = pmacs.lsp.active_attachment(); \
       for _, c in ipairs(pmacs.completion.collect({ prefix = '', uri = rec.uri })) do \
         if c.label == 'entrance' and c.text_edit then return true end \
       end \
       return false \
     end)()";
    assert!(
        pump_lua_flag(&mut s, stored, 10),
        "positive control: the store holds the answer for the word"
    );
    assert!(
        !popup_visible(&s),
        "positive control: the item does not match the word, so nothing shows"
    );
    // The dot, and no tick: the request it sends is unanswered.
    type_str(&mut s, ".");
    assert_eq!(
        selected_label(&s).as_deref(),
        Some("entrance"),
        "positive control: the popup shows the earlier answer at the dot"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        whole("        self.entries.entrance"),
        "the word and the dot are as typed; the item's text is at the caret"
    );
    assert!(
        status(&s).starts_with("completion: the server's range for this item could not be placed"),
        "the fallback is said: {:?}",
        status(&s)
    );
}

/// A range in a document the store keeps no log for cannot be carried
/// to the current text at all. Driven through `popup_show`, the seam the
/// driver uses, as E7.4's row for extra edits is.
#[test]
fn e10a_1_a_range_the_log_cannot_place_replaces_the_typed_word_and_says_so() {
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
             { label = 'defined (somewhere)', insert_text = 'defined',
               text_edit = { start = 0, stop = 3, at = 3 },
               edits_uri = 'file:///nowhere/never-opened.rs',
               edits_base = 0 },
           },
         }",
    );
    assert!(
        popup_visible(&s),
        "positive control: the synthetic popup is showing"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        "abc def\ndefined",
        "the typed word is replaced; `abc`, which the range named, is as it was"
    );
    assert!(
        status(&s).starts_with("completion: the server's range for this item could not be placed"),
        "the fallback is said: {:?}",
        status(&s)
    );
}
