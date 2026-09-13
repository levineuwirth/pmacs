// tests/minibuffer_tab_acceptance.rs --- E6.3: the minibuffer's TAB.

//! TAB in the minibuffer, through real keys: the candidates' common
//! prefix, then the selection, then --- in the files source --- a
//! second TAB descending into a directory. This is the MINIBUFFER's
//! TAB and nothing else: the buffer's TAB (E1.6's indent clause) is a
//! different key on a different surface and is not touched here.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    s
}

fn contents(s: &EditorState) -> String {
    eval(s, "return pmacs.minibuffer.contents()")
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.minibuffer.candidates()")
}

fn active_path(s: &EditorState) -> Option<String> {
    eval::<Option<String>>(
        s,
        "local b = pmacs.window.buffer()\n\
         if b == nil then return nil end\n\
         local ok, p = pcall(function() return b:path() end)\n\
         if ok then return p end\n\
         return nil",
    )
}

/// An editor whose active buffer is a real file inside `dir`, with the
/// find-file prompt open and its field prefilled with `dir/`.
fn find_file_in(dir: &std::path::Path) -> (EditorState, String) {
    let anchor = dir.join("anchor.txt");
    std::fs::write(&anchor, b"anchor\n").expect("write anchor");
    let mut s = fresh();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            anchor.display().to_string()
        ),
    );
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'f');
    let prefill = contents(&s);
    assert!(
        prefill.ends_with('/'),
        "the field is prefilled with the root directory; got {prefill:?}"
    );
    (s, prefill)
}

/// A custom-source prompt over a fixed list, opened from Lua.
fn open_list_prompt(s: &EditorState, items: &[&str]) {
    let list = items
        .iter()
        .map(|i| format!("{i:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    exec(
        s,
        &format!(
            "pmacs.minibuffer.read {{ prompt = 'P: ', source = function() return {{ {list} }} end, on_accept = function() end }}"
        ),
    );
}

/// The common prefix first: `alp` over two `alpha-` files becomes
/// `alpha-`, and both stay candidates.
#[test]
fn tab_completes_the_common_prefix() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("alpha-one.txt"), b"1").expect("write");
    std::fs::write(td.path().join("alpha-two.txt"), b"2").expect("write");
    let (mut s, prefill) = find_file_in(td.path());
    type_str(&mut s, "alp");
    let mut before = candidates(&s);
    before.sort();
    assert_eq!(before, vec!["alpha-one.txt", "alpha-two.txt"]);

    press(&mut s, KeyCode::Tab);

    assert_eq!(contents(&s), format!("{prefill}alpha-"));
    let mut after = candidates(&s);
    after.sort();
    assert_eq!(
        after, before,
        "both candidates survive the completed prefix"
    );
}

/// A unique match completes whole on the first TAB, and a second TAB
/// descends into it when it is a directory: the listing becomes the
/// directory's entries, and the flow continues inside it to RET.
#[test]
fn tab_completes_a_unique_match_and_a_second_tab_descends() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");
    let inner = td.path().join("sub").join("inner.txt");
    std::fs::write(&inner, b"deep\n").expect("write");
    let (mut s, prefill) = find_file_in(td.path());
    type_str(&mut s, "su");
    assert_eq!(candidates(&s), vec!["sub".to_string()]);

    press(&mut s, KeyCode::Tab);
    assert_eq!(
        contents(&s),
        format!("{prefill}sub"),
        "first TAB: the unique match"
    );

    press(&mut s, KeyCode::Tab);
    assert_eq!(
        contents(&s),
        format!("{prefill}sub/"),
        "second TAB: descends"
    );
    assert_eq!(
        candidates(&s),
        vec!["inner.txt".to_string()],
        "the candidates are the directory's entries"
    );

    type_str(&mut s, "in");
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), format!("{prefill}sub/inner.txt"));
    press(&mut s, KeyCode::Enter);
    let path = active_path(&s).expect("a file must be open");
    assert_eq!(
        std::fs::canonicalize(&path).expect("opened"),
        std::fs::canonicalize(&inner).expect("fixture")
    );
}

/// When no prefix extends what is typed, TAB takes the selection ---
/// and the selection is what `Down` chose.
#[test]
fn tab_falls_back_to_the_selection_when_the_prefix_is_exhausted() {
    let mut s = fresh();
    open_list_prompt(&s, &["apple", "apricot", "banana"]);
    type_str(&mut s, "ap");
    assert_eq!(candidates(&s), vec!["apple", "apricot"]);
    press(&mut s, KeyCode::Tab);
    assert_eq!(
        contents(&s),
        "apple",
        "the prefix `ap` is exhausted; the selection"
    );

    open_list_prompt(&s, &["apple", "apricot", "banana"]);
    type_str(&mut s, "ap");
    press(&mut s, KeyCode::Down);
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), "apricot", "the selection Down chose");
}

/// The prefix extends before the selection is taken: `a` becomes `ap`,
/// not `apple`.
#[test]
fn tab_extends_the_prefix_before_falling_to_the_selection() {
    let mut s = fresh();
    open_list_prompt(&s, &["apple", "apricot"]);
    type_str(&mut s, "a");
    assert_eq!(candidates(&s), vec!["apple", "apricot"]);
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), "ap", "the shared prefix, not the selection");
}

/// Probe: a fuzzy match with no shared prefix completes to the
/// selection, keeping the directory part of the field.
#[test]
fn tab_on_a_fuzzy_match_takes_the_selection_and_keeps_the_directory() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("notes.md"), b"n").expect("write");
    let (mut s, prefill) = find_file_in(td.path());
    type_str(&mut s, "nts");
    assert_eq!(candidates(&s), vec!["notes.md".to_string()]);
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), format!("{prefill}notes.md"));
}

/// Probe: TAB on a completed FILE name is a no-op --- only a directory
/// descends, and a file gains no slash.
#[test]
fn tab_on_a_complete_file_name_changes_nothing() {
    let td = tempfile::tempdir().expect("tempdir");
    let (mut s, prefill) = find_file_in(td.path());
    type_str(&mut s, "anchor.txt");
    assert_eq!(candidates(&s), vec!["anchor.txt".to_string()]);
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), format!("{prefill}anchor.txt"));
    press(&mut s, KeyCode::Tab);
    assert_eq!(
        contents(&s),
        format!("{prefill}anchor.txt"),
        "still nothing"
    );
}

/// Probe: with no candidates TAB leaves the field alone.
#[test]
fn tab_with_no_candidates_leaves_the_field_alone() {
    let td = tempfile::tempdir().expect("tempdir");
    let (mut s, prefill) = find_file_in(td.path());
    type_str(&mut s, "zzz");
    assert!(candidates(&s).is_empty());
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), format!("{prefill}zzz"));
}

/// Probe: the prefix is case-sensitive, so a case split among the
/// candidates yields no prefix and TAB takes the selection.
#[test]
fn tab_with_a_case_split_takes_the_selection() {
    let mut s = fresh();
    open_list_prompt(&s, &["Makefile", "main.rs"]);
    type_str(&mut s, "ma");
    let cands = candidates(&s);
    assert_eq!(
        cands.len(),
        2,
        "both match case-insensitively; got {cands:?}"
    );
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), cands[0], "no shared prefix: the selection");
}

/// Probe: descent works in a directory typed by hand under the root,
/// two levels down.
#[test]
fn tab_descends_two_levels() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(td.path().join("one").join("two")).expect("mkdir");
    std::fs::write(td.path().join("one").join("two").join("leaf.txt"), b"l").expect("write");
    let (mut s, prefill) = find_file_in(td.path());
    type_str(&mut s, "on");
    press(&mut s, KeyCode::Tab);
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), format!("{prefill}one/"));
    assert_eq!(candidates(&s), vec!["two".to_string()]);
    press(&mut s, KeyCode::Tab);
    assert_eq!(
        contents(&s),
        format!("{prefill}one/two/"),
        "a unique directory: one TAB completes, the next descends; here the name was already complete"
    );
    assert_eq!(candidates(&s), vec!["leaf.txt".to_string()]);
}

#[path = "common/iso.rs"]
mod iso;
