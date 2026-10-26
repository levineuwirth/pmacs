// tests/e8b_own_name_acceptance.rs --- E8b fix round 1: a buffer reached
// by its own name, through `C-x b` and `C-x k`.

//! E8b review 1's Medium 1 and Medium 2, through the keys a user presses
//! (`EditorState::dispatch_key`, the TUI's path), read again under D40
//! (E10c): a buffer's name is not its path. E8b ranked a match within a
//! buffer's own name above one spelled through the directories it sat
//! in, because a file buffer was named by its path; a file buffer is now
//! named by its basename, so its name holds no directory to spell
//! through and `C-x b` and `C-x k` match names alone. A name that begins
//! with the query takes the scorer's start bonus: `dired` reaches
//! `dired.lua`, and the listing is reached by its own first letters. A
//! `*name*` is scored inside its stars (the owner's ruling of
//! 2026-10-25), so `lsp` reaches `*lsp*` beside `lsp.rs`, whose own
//! name reaches it.
//!
//! `C-x k` reads its target from the same source, so the same rule
//! decides what it kills, and a kill says which buffer went, in the
//! `kill-buffer: …` idiom `builtin/commands/default.lua` already uses.
//!
//! One session layout serves every row: `bin-tests` holds `a.rs`, `b.rs`
//! and `notes.txt`, so `b.rs` was a subsequence of `a.rs`'s path through
//! the `b` of `bin`, and is not of its name; `proj` holds `dired.lua`,
//! `editor.rs` and `src/lsp.rs`, and a dired buffer lists it; `*scratch*`
//! is kept. Both directories sit in `nightowls`, which spells `nts`
//! loosely, so that every path in the session holds D18's `nts`, and no
//! file's name does but `notes.txt`'s.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::minibuffer::fuzzy_score;
use pmacs::protocol::{CellSize, FrontendId};

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

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(40, 100));
    s
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn active(s: &EditorState) -> bool {
    eval(s, "return pmacs.minibuffer.is_active()")
}

fn prompt(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.minibuffer.is_active() and pmacs.minibuffer.prompt() or ''",
    )
}

fn contents(s: &EditorState) -> String {
    eval(s, "return pmacs.minibuffer.contents()")
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.minibuffer.candidates()")
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn buffer_names(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local t = {}\n\
         for _, id in ipairs(pmacs.buffer.list()) do\n\
           t[#t + 1] = pmacs.describe.buffer(id).name\n\
         end\n\
         return t",
    )
}

/// Point the window at the buffer named `name` (fixture setup, not the
/// path under test).
fn show(s: &EditorState, name: &str) {
    exec(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do\n\
               if pmacs.describe.buffer(id).name == {name:?} then\n\
                 pmacs.window.switch_buffer(id)\n\
               end\n\
             end"
        ),
    );
    assert_eq!(active_name(s), name, "fixture: showing {name}");
}

/// Visit `path`; return the buffer's name as pmacs gives it (its
/// basename, D40).
fn visit(s: &EditorState, path: &Path) -> String {
    exec(
        s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
    active_name(s)
}

/// Drive the async runtime until nothing is parked or pending: a dired
/// listing is read on a worker and lands on a later tick.
fn pump(s: &mut EditorState) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut spins = 0u32;
    loop {
        let idle: bool = eval(
            s,
            "return pmacs._async.parked_count() == 0 and pmacs._async.pending_count() == 0",
        );
        if idle {
            return;
        }
        assert!(Instant::now() < deadline, "async pump deadline exceeded");
        s.tick_async();
        spins += 1;
        if spins > 64 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Empty a prefilled field as a user does, by holding backspace.
fn clear_field(s: &mut EditorState) {
    for _ in 0..contents(s).chars().count() {
        press(s, KeyCode::Backspace);
    }
    assert_eq!(contents(s), "", "fixture: backspace empties the field");
}

/// `C-x b`, `text`, RET.
fn switch_by_typing(s: &mut EditorState, text: &str) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('b'));
    assert!(active(s), "C-x b opens the prompt");
    type_str(s, text);
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// `C-x k`, the prefill cleared, `text`, RET. Returns the candidates the
/// prompt offered for `text`, for a failure to show.
fn kill_by_typing(s: &mut EditorState, text: &str) -> Vec<String> {
    ctrl(s, 'x');
    press(s, KeyCode::Char('k'));
    assert!(active(s), "C-x k opens the prompt");
    clear_field(s);
    type_str(s, text);
    let offered = candidates(s);
    press(s, KeyCode::Enter);
    offered
}

/// The session every row starts from (the module's comment has the
/// layout), with the window on `editor.rs`.
struct Session {
    s: EditorState,
    a: String,
    b: String,
    notes: String,
    lua: String,
    editor: String,
    lsp_rs: String,
    dired: String,
    scratch: String,
    proj: std::path::PathBuf,
    bins: std::path::PathBuf,
}

/// `C-x d`, the prefill replaced by `dir`, RET: dired as a user opens
/// it. Returns the buffer the window then shows.
fn open_dired(s: &mut EditorState, dir: &Path) -> String {
    ctrl(s, 'x');
    press(s, KeyCode::Char('d'));
    assert!(active(s), "C-x d opens the prompt");
    clear_field(s);
    type_str(s, &dir.display().to_string());
    press(s, KeyCode::Enter);
    pump(s);
    active_name(s)
}

fn session(td: &Path) -> Session {
    let root = td.join("nightowls");
    let bins = root.join("bin-tests");
    let proj = root.join("proj");
    std::fs::create_dir_all(&bins).expect("bin-tests");
    std::fs::create_dir_all(proj.join("src")).expect("proj/src");
    let mut s = fresh();
    let scratch: String = eval(
        &s,
        "return pmacs.describe.buffer(pmacs.buffer.list()[1]).name",
    );
    let mut names = Vec::new();
    for path in [
        bins.join("a.rs"),
        bins.join("b.rs"),
        bins.join("notes.txt"),
        proj.join("dired.lua"),
        proj.join("src/lsp.rs"),
        proj.join("editor.rs"),
    ] {
        std::fs::write(&path, b"x\n").expect("write");
        names.push(visit(&s, &path));
    }
    let dired = open_dired(&mut s, &proj);
    assert_eq!(
        dired, "proj/",
        "fixture: C-x d opened a listing of proj, named by its directory"
    );
    let [a, b, notes, lua, lsp_rs, editor] = names.try_into().expect("six visits");
    show(&s, &editor);
    Session {
        s,
        a,
        b,
        notes,
        lua,
        editor,
        lsp_rs,
        dired,
        scratch,
        proj,
        bins,
    }
}

/// Review 1's Medium 1 and the owner's rule, read under D40 (E10c), in
/// one session with the bare name and D18's four (as
/// `tests/minibuffer_accept_acceptance.rs` pins them).
///
/// `C-x b b.rs RET` reaches `b.rs`, whose name alone holds it: `a.rs`'s
/// held it through `bin-tests` while a file was named by its path
/// (#318). A file's full path is refused as typed, no name holding a
/// directory, and its name reaches it. `C-x b dired RET` and `C-x b dir
/// RET` reach `dired.lua`, whose name begins with them, where E8b put the
/// listing first; the listing is reached by its own first letters.
/// `C-x b nts RET` reaches `notes.txt`, whose name alone holds `nts`.
/// `C-x b lsp RET` reaches `*lsp*`, scored inside its stars, and
/// `C-x b lsp.rs RET` reaches `lsp.rs`; `zzz` is refused.
#[test]
fn c_x_b_reaches_a_dired_buffer_by_its_own_name_beside_files_and_d18_s_four() {
    let td = tempfile::tempdir().expect("tempdir");
    let Session {
        mut s,
        a,
        b,
        notes,
        lua,
        editor,
        lsp_rs,
        dired,
        scratch,
        bins,
        ..
    } = session(td.path());

    // The bare name, from `a.rs`; a full path; the name.
    show(&s, &a);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "b.rs");
    let offered = candidates(&s);
    assert!(
        offered.first() == Some(&b) && !offered.contains(&a),
        "`b.rs` selects b.rs, and a.rs's name holds no `b`; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), b, "C-x b b.rs RET reaches b.rs");
    assert_eq!(status(&s), format!("switch-buffer: showing {b}"));
    let a_path = bins.join("a.rs").display().to_string();
    switch_by_typing(&mut s, &a_path);
    assert_eq!(
        (active_name(&s), status(&s)),
        (b.clone(), format!("no buffer: {a_path}")),
        "a full path names no buffer"
    );
    switch_by_typing(&mut s, &a);
    assert_eq!(active_name(&s), a, "a.rs's name reaches it");

    // `dired` names a file; the listing by its own letters.
    for typed in ["dired", "dir"] {
        show(&s, &editor);
        switch_by_typing(&mut s, typed);
        assert_eq!(
            (active_name(&s), status(&s)),
            (lua.clone(), format!("switch-buffer: showing {lua}")),
            "C-x b {typed} RET reaches dired.lua, whose name begins with it"
        );
    }
    show(&s, &editor);
    switch_by_typing(&mut s, LISTING_QUERY);
    assert_eq!(
        (active_name(&s), status(&s)),
        (dired.clone(), format!("switch-buffer: showing {dired}")),
        "C-x b {LISTING_QUERY} RET reaches the listing"
    );

    // D18's four, in this session.
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "nts");
    let offered = candidates(&s);
    assert!(
        offered.first() == Some(&notes) && !offered.contains(&a) && !offered.contains(&b),
        "`nts` selects notes.txt, the names that spelled it through `nightowls` gone; \
         got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), notes, "C-x b nts RET reaches notes.txt");
    switch_by_typing(&mut s, "scr");
    assert_eq!(active_name(&s), scratch, "C-x b scr RET reaches *scratch*");
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('l'));
    assert_eq!(active_name(&s), "*lsp*", "C-c l opens *lsp*");
    show(&s, &scratch);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "lsp");
    let offered = candidates(&s);
    assert!(
        offered.first() == Some(&"*lsp*".to_owned()) && offered.contains(&lsp_rs),
        "`lsp` selects *lsp*, scored inside its stars, before lsp.rs; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), "*lsp*", "C-x b lsp RET reaches *lsp*");
    switch_by_typing(&mut s, "lsp.rs");
    assert_eq!(active_name(&s), lsp_rs, "C-x b lsp.rs RET reaches lsp.rs");
    show(&s, &scratch);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "zzz");
    assert!(candidates(&s).is_empty(), "premise: nothing matches `zzz`");
    press(&mut s, KeyCode::Enter);
    assert_eq!(status(&s), "no buffer: zzz", "zzz is refused as typed");
    assert_eq!(active_name(&s), scratch, "nothing switched");
}

/// The query that reaches the session's listing, `proj/`, by its own
/// name, which no other name in the session holds.
const LISTING_QUERY: &str = "proj";

/// The name as typed in another case comes first (E8b fix round 1's
/// place 2, kept under D40 as the folded name): `AB` reaches `ab` beside
/// `a.b`, where the scorer alone prefers `a.b`, its `b` following a `.`.
/// The pair replaces E8b's `Q.RS` beside `aq.rs`, whose scores tied only
/// while both were read inside their paths.
#[test]
fn a_basename_typed_in_another_case_beats_a_name_ending_with_it() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("zz");
    std::fs::create_dir_all(&dir).expect("dir");
    let mut s = fresh();
    let mut names = Vec::new();
    for file in ["a.b", "ab", "c.txt"] {
        std::fs::write(dir.join(file), b"x\n").expect("write");
        names.push(visit(&s, &dir.join(file)));
    }
    let [near, own, c] = names.try_into().expect("three visits");
    assert!(
        fuzzy_score("AB", &near) > fuzzy_score("AB", &own),
        "premise: the scorer alone ranks {near} above {own}"
    );
    show(&s, &c);
    switch_by_typing(&mut s, "AB");
    assert_eq!(active_name(&s), own, "C-x b AB RET reaches ab, not a.b");
    assert_eq!(status(&s), format!("switch-buffer: showing {own}"));
}

/// Whether the buffer named `name` reads as modified, which decides
/// whether a kill asks first.
fn modified(s: &EditorState, name: &str) -> bool {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do\n\
               local d = pmacs.describe.buffer(id)\n\
               if d.name == {name:?} then return d.modified end\n\
             end\n\
             error('no buffer ' .. {name:?})"
        ),
    )
}

/// Review 1's Medium 2, the first of its two wrong kills, in the review's
/// own crowded fixture: `a.rs`, `b.rs` and `c.txt` under `bin-tests`,
/// `c.txt` shown. `C-x k b.rs RET` kills `b.rs`. Before `b374cd8` the same
/// keys killed `a.rs`, whose path also held `b.rs` through the `b` of
/// `bin` and which the lexical order put first, with no question (it is
/// unmodified) and nothing said. The band names what went; under D40
/// `b.rs` is held by `b.rs`'s name alone, a full path names no buffer,
/// and a name kills its own.
#[test]
fn c_x_k_b_rs_kills_b_rs_and_says_so() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("bin-tests");
    std::fs::create_dir_all(&dir).expect("bin-tests");
    let mut s = fresh();
    let mut names = Vec::new();
    for file in ["a.rs", "b.rs", "c.txt"] {
        std::fs::write(dir.join(file), b"x\n").expect("write");
        names.push(visit(&s, &dir.join(file)));
    }
    let [a, b, c] = names.try_into().expect("three visits");
    show(&s, &c);
    assert!(
        !modified(&s, &a) && !modified(&s, &b),
        "premise: both files are unmodified, so neither kill asks"
    );
    let offered = kill_by_typing(&mut s, "b.rs");
    assert_eq!(
        offered,
        vec![b.clone()],
        "`b.rs` is held by b.rs's name alone"
    );
    assert!(!active(&s), "an unmodified buffer goes without a question");
    let left = buffer_names(&s);
    assert!(
        left.contains(&a) && !left.contains(&b),
        "C-x k b.rs RET kills b.rs and keeps a.rs; left {left:?}, band {:?}",
        status(&s)
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {b}"));

    let a_path = dir.join("a.rs").display().to_string();
    kill_by_typing(&mut s, &a_path);
    assert_eq!(status(&s), format!("kill-buffer: no buffer named {a_path}"));
    assert!(buffer_names(&s).contains(&a), "a full path kills nothing");
    kill_by_typing(&mut s, &a);
    let left = buffer_names(&s);
    assert!(!left.contains(&a), "a.rs's name kills it; left {left:?}");
    assert_eq!(status(&s), format!("kill-buffer: killed {a}"));
}

/// Review 1's Medium 2, the second, read under D40: `C-x k` on the
/// listing's own letters acts on the listing, not on a file. At
/// `0001946` `C-x k dired RET` killed `dired.lua`, silently, and kept
/// the listing; E8b made `dired` select the listing, whose name then
/// held the word; under D40 the listing is named by its directory and
/// `dired` names `dired.lua` (the first row). Until E8c the listing's
/// generated text read as modified (review 1's aside); E8c leaves a
/// buffer the editor writes clean (#299), so the kill goes without a
/// question and names it on the band. Twice, the listing opened again
/// between.
#[test]
fn c_x_k_dired_kills_the_dired_buffer_and_says_so() {
    let td = tempfile::tempdir().expect("tempdir");
    let Session {
        mut s,
        lua,
        dired,
        editor,
        proj,
        ..
    } = session(td.path());
    assert!(
        !modified(&s, &dired) && !modified(&s, &lua),
        "premise: neither the dired buffer nor dired.lua reads as modified"
    );
    for round in 0..2 {
        let typed = LISTING_QUERY;
        if round == 1 {
            assert_eq!(
                open_dired(&mut s, &proj),
                dired,
                "fixture: C-x d lists proj again"
            );
            show(&s, &editor);
        }
        let offered = kill_by_typing(&mut s, typed);
        assert_eq!(
            offered.first(),
            Some(&dired),
            "`{typed}` selects the listing; got {offered:?}"
        );
        assert!(
            !active(&s),
            "C-x k {typed} RET asks nothing of a listing; asked {:?}",
            prompt(&s)
        );
        let left = buffer_names(&s);
        assert!(
            !left.contains(&dired) && left.contains(&lua),
            "C-x k {typed} RET kills the dired buffer and keeps dired.lua; left {left:?}"
        );
        assert_eq!(status(&s), format!("kill-buffer: killed {dired}"));
    }
}

/// D18's four through `C-x k`, which shares `C-x b`'s source and its
/// `candidate` policy: `nts` kills `notes.txt` and `scr` `*scratch*`,
/// both unmodified and so without a question; `*lsp` selects `*lsp*`
/// beside `lsp.rs` (`lsp` selects it too, scored inside its stars, the
/// first row) and kills it, without a question since E8c left the editor's own text
/// clean (before it, `*lsp*` read as modified and this row answered
/// `y`); each is named on the band. `zzz` matches nothing, is looked up
/// as typed and refused, and nothing goes.
#[test]
fn c_x_k_keeps_d18_s_four() {
    let td = tempfile::tempdir().expect("tempdir");
    let Session {
        mut s,
        notes,
        lsp_rs,
        editor,
        scratch,
        ..
    } = session(td.path());
    assert!(
        !modified(&s, &notes) && !modified(&s, &scratch),
        "premise: notes.txt and *scratch* are unmodified"
    );
    let offered = kill_by_typing(&mut s, "nts");
    assert_eq!(
        offered.first(),
        Some(&notes),
        "`nts` selects notes.txt; got {offered:?}"
    );
    assert!(!active(&s), "notes.txt goes without a question");
    assert!(!buffer_names(&s).contains(&notes), "nts kills notes.txt");
    assert_eq!(status(&s), format!("kill-buffer: killed {notes}"));

    kill_by_typing(&mut s, "scr");
    assert!(!active(&s), "*scratch* goes without a question");
    assert!(
        !buffer_names(&s).contains(&scratch),
        "scr kills *scratch*; band {:?}",
        status(&s)
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {scratch}"));

    show(&s, &editor);
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('l'));
    assert_eq!(active_name(&s), "*lsp*", "C-c l opens *lsp*");
    assert!(
        !modified(&s, "*lsp*"),
        "premise: *lsp* is the editor's text and reads as unmodified"
    );
    let offered = kill_by_typing(&mut s, "*lsp");
    assert_eq!(
        offered.first().map(String::as_str),
        Some("*lsp*"),
        "`*lsp` selects *lsp*; got {offered:?}"
    );
    assert!(
        !active(&s),
        "*lsp* goes without a question; asked {:?}",
        prompt(&s)
    );
    let left = buffer_names(&s);
    assert!(
        !left.contains(&"*lsp*".to_owned()) && left.contains(&lsp_rs),
        "*lsp kills *lsp* and keeps lsp.rs; left {left:?}"
    );
    assert_eq!(status(&s), "kill-buffer: killed *lsp*");

    let before = buffer_names(&s);
    let offered = kill_by_typing(&mut s, "zzz");
    assert!(offered.is_empty(), "premise: nothing matches `zzz`");
    assert!(!active(&s), "a refusal asks nothing");
    assert_eq!(status(&s), "kill-buffer: no buffer named zzz");
    assert_eq!(buffer_names(&s), before, "nothing went");
}

/// RET on `C-x k`'s prefill kills the buffer in the window and names it;
/// a modified file buffer is asked about first, and `y` kills it and
/// names it too.
#[test]
fn c_x_k_names_what_it_killed_from_the_prefill_and_after_the_question() {
    let td = tempfile::tempdir().expect("tempdir");
    let Session {
        mut s,
        a,
        b,
        editor,
        ..
    } = session(td.path());
    show(&s, &editor);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('k'));
    assert_eq!(contents(&s), editor, "C-x k prefills the shown buffer");
    press(&mut s, KeyCode::Enter);
    assert!(!active(&s), "an unmodified buffer goes without a question");
    assert!(!buffer_names(&s).contains(&editor), "RET kills editor.rs");
    assert_eq!(status(&s), format!("kill-buffer: killed {editor}"));

    show(&s, &a);
    type_str(&mut s, "edit");
    assert!(modified(&s, &a), "premise: typing modified a.rs");
    kill_by_typing(&mut s, &a);
    let asked = prompt(&s);
    assert!(
        asked.contains(&a) && asked.contains("kill anyway"),
        "a modified buffer is asked about; asked {asked:?}"
    );
    press(&mut s, KeyCode::Char('y'));
    let left = buffer_names(&s);
    assert!(
        !left.contains(&a) && left.contains(&b),
        "y kills a.rs; left {left:?}"
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {a}"));
}

/// `buffer.kill-this` shares `C-x k`'s helper, so it names what it
/// killed too: `M-x buffer.kill-this RET` on `b.rs`, unmodified, kills
/// it without a question and says so.
#[test]
fn kill_this_names_what_it_killed() {
    let td = tempfile::tempdir().expect("tempdir");
    let Session { mut s, a, b, .. } = session(td.path());
    show(&s, &b);
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('x'), KeyModifiers::ALT),
    );
    assert!(active(&s), "M-x opens the prompt");
    type_str(&mut s, "buffer.kill-this");
    press(&mut s, KeyCode::Enter);
    assert!(!active(&s), "an unmodified buffer goes without a question");
    let left = buffer_names(&s);
    assert!(
        !left.contains(&b) && left.contains(&a),
        "kill-this kills the shown b.rs; left {left:?}"
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {b}"));
}

#[path = "common/iso.rs"]
mod iso;
