// tests/e8b_own_name_acceptance.rs --- E8b fix round 1: a buffer reached
// by its own name, through `C-x b` and `C-x k`.

//! E8b review 1's Medium 1 and Medium 2, through the keys a user presses
//! (`EditorState::dispatch_key`, the TUI's path). The `buffers` source
//! ranks a match within a buffer's own name above one spelled through
//! the directories it sits in, and a buffer's own name is the basename
//! of one that holds a file path and the whole name of any other. A
//! dired buffer is named `*dired:<path>*` and holds no file, so its own
//! name is all of it: `C-x b dired RET` reaches it again, where the
//! first form of the rule read its `/` as directories and reached
//! `dired.lua`.
//!
//! `C-x k` reads its target from the same source, so the same rule
//! decides what it kills, and a kill now says which buffer went, in the
//! `kill-buffer: …` idiom `builtin/commands/default.lua` already uses.
//!
//! One session layout serves every row: `bin-tests` holds `a.rs`, `b.rs`
//! and `notes.txt`, so `b.rs` is a subsequence of `a.rs`'s path through
//! the `b` of `bin`; `proj` holds `dired.lua`, `editor.rs` and
//! `src/lsp.rs`, and a dired buffer lists it; `*scratch*` is kept. Both
//! directories sit in `nightowls`, which spells `nts` loosely, so that
//! every name in the session holds D18's `nts` through its directories
//! on every machine, the dired buffer's included: the letters a dired
//! buffer's own name holds are its whole name's, and here they are
//! spelled worse than `notes.txt` spells them.

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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

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

/// Visit `path`; return the buffer's name as pmacs stores it (macOS's
/// temporary root is a symlink, and pmacs keeps the path as given).
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
    // `C-x d`, the prefill replaced by the directory, RET: dired as a
    // user opens it.
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('d'));
    assert!(active(&s), "C-x d opens the prompt");
    clear_field(&mut s);
    type_str(&mut s, &proj.display().to_string());
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    let dired = active_name(&s);
    assert!(
        dired.starts_with("*dired:") && dired.ends_with("/proj*"),
        "fixture: C-x d opened a dired buffer; showing {dired}"
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
    }
}

/// Review 1's Medium 1 and the owner's rule, in one session with the
/// bare name and D18's four (as `tests/minibuffer_accept_acceptance.rs`
/// pins them).
///
/// `C-x b dired RET` and `C-x b dir RET` reach the dired buffer. Its name
/// holds no file, so its own name is all of it, and `dired.lua`, whose
/// own name holds the same letters, ties it on place and score; the
/// lexical order has always put a `*` name before a `/` one. At
/// `0001946` the place rule read the path inside `*dired:<path>*` as
/// directories, put the dired buffer below every file holding the
/// letters, and both reached `dired.lua`.
///
/// `C-x b nts RET` still reaches `notes.txt`: the dired buffer's name
/// holds `nts` too (through `nightowls`), so both are placed alike, and
/// each is scored within its own name. Scored on the whole name, as the
/// rule was first written for this round, both names spell `nts`
/// through the same directories alike, and the lexical order took the
/// dired buffer. `C-x b lsp RET` reaches `*lsp*` beside `src/lsp.rs`,
/// whose own name holds `lsp` as tightly: their scores tie, since
/// neither begins its name, and the lexical order decides.
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
    } = session(td.path());
    assert!(
        fuzzy_score("nts", &dired).is_some_and(|d| d < fuzzy_score("nts", "/notes.txt").unwrap()),
        "premise: the dired buffer's name holds `nts`, and more loosely than notes.txt's own \
         name; {dired}"
    );

    // The bare name, from `a.rs`, and each full path.
    show(&s, &a);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "b.rs");
    let offered = candidates(&s);
    assert!(
        offered.contains(&a) && offered.contains(&b),
        "premise: `b.rs` is a subsequence of both paths; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), b, "C-x b b.rs RET reaches b.rs");
    assert_eq!(status(&s), format!("switch-buffer: showing {b}"));
    switch_by_typing(&mut s, &a);
    assert_eq!(active_name(&s), a, "a.rs's full path reaches it");
    switch_by_typing(&mut s, &b);
    assert_eq!(active_name(&s), b, "b.rs's full path reaches it");

    // The dired buffer by its own word, from `editor.rs`.
    for typed in ["dired", "dir"] {
        show(&s, &editor);
        ctrl(&mut s, 'x');
        press(&mut s, KeyCode::Char('b'));
        type_str(&mut s, typed);
        let offered = candidates(&s);
        assert!(
            offered.contains(&dired) && offered.contains(&lua),
            "premise: `{typed}` is held by the dired buffer and by dired.lua; got {offered:?}"
        );
        press(&mut s, KeyCode::Enter);
        assert_eq!(
            (active_name(&s), status(&s)),
            (dired.clone(), format!("switch-buffer: showing {dired}")),
            "C-x b {typed} RET reaches the dired buffer; offered {offered:?}"
        );
    }

    // D18's four, in this session.
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "nts");
    let offered = candidates(&s);
    assert!(
        [&a, &b, &notes, &dired].iter().all(|n| offered.contains(n)),
        "premise: `nts` is spelled through the directories of every name; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        notes,
        "C-x b nts RET reaches notes.txt; offered {offered:?}"
    );
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
        offered.contains(&lsp_rs),
        "premise: src/lsp.rs holds `lsp` in its own name; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        "*lsp*",
        "C-x b lsp RET reaches *lsp*; offered {offered:?}"
    );
    show(&s, &scratch);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "zzz");
    assert!(candidates(&s).is_empty(), "premise: nothing matches `zzz`");
    press(&mut s, KeyCode::Enter);
    assert_eq!(status(&s), "no buffer: zzz", "zzz is refused as typed");
    assert_eq!(active_name(&s), scratch, "nothing switched");
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
/// keys killed `a.rs`, whose path also holds `b.rs` through the `b` of
/// `bin` and which the lexical order put first, with no question (it is
/// unmodified) and nothing said. Now the band names what went, and a
/// full path kills its own buffer.
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
    assert!(
        offered.contains(&a) && offered.contains(&b),
        "premise: `b.rs` is a subsequence of both paths; got {offered:?}"
    );
    assert!(!active(&s), "an unmodified buffer goes without a question");
    let left = buffer_names(&s);
    assert!(
        left.contains(&a) && !left.contains(&b),
        "C-x k b.rs RET kills b.rs and keeps a.rs; left {left:?}, band {:?}",
        status(&s)
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {b}"));

    kill_by_typing(&mut s, &a);
    let left = buffer_names(&s);
    assert!(
        !left.contains(&a),
        "a.rs's full path kills it; left {left:?}"
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {a}"));
}

/// Review 1's Medium 2, the second: `C-x k dired RET` acts on the dired
/// buffer, not `dired.lua`. At `0001946` it killed `dired.lua`, an
/// unmodified file buffer, silently, and kept the dired buffer. The
/// dired buffer's generated text reads as modified (review 1's aside,
/// the owner's to rule on, asserted here as a premise), so the kill asks
/// first and names it: `n` keeps both buffers and says so, and `y` kills
/// the dired buffer and names it on the band. `dir` selects it as
/// `dired` does.
#[test]
fn c_x_k_dired_kills_the_dired_buffer_and_says_so() {
    let td = tempfile::tempdir().expect("tempdir");
    let Session {
        mut s, lua, dired, ..
    } = session(td.path());
    assert!(
        modified(&s, &dired) && !modified(&s, &lua),
        "premise: the dired buffer reads as modified and dired.lua does not"
    );
    for typed in ["dir", "dired"] {
        let offered = kill_by_typing(&mut s, typed);
        assert!(
            offered.contains(&dired) && offered.contains(&lua),
            "premise: `{typed}` is held by the dired buffer and by dired.lua; got {offered:?}"
        );
        let asked = prompt(&s);
        assert!(
            asked.contains(&dired) && asked.contains("kill anyway"),
            "C-x k {typed} RET asks about the dired buffer; asked {asked:?}, left {:?}, \
             band {:?}",
            buffer_names(&s),
            status(&s)
        );
        if typed == "dir" {
            press(&mut s, KeyCode::Char('n'));
            assert_eq!(status(&s), "kill-buffer cancelled");
            let left = buffer_names(&s);
            assert!(
                left.contains(&dired) && left.contains(&lua),
                "n keeps both; left {left:?}"
            );
        }
    }
    press(&mut s, KeyCode::Char('y'));
    let left = buffer_names(&s);
    assert!(
        !left.contains(&dired) && left.contains(&lua),
        "y kills the dired buffer and keeps dired.lua; left {left:?}"
    );
    assert_eq!(status(&s), format!("kill-buffer: killed {dired}"));
}

/// D18's four through `C-x k`, which shares `C-x b`'s source and its
/// `candidate` policy: `nts` kills `notes.txt` and `scr` `*scratch*`,
/// both unmodified and so without a question; `lsp` selects `*lsp*`
/// beside `src/lsp.rs`, whose generated text reads as modified, so the
/// kill asks about `*lsp*` and `y` kills it; each is named on the band.
/// `zzz` matches nothing, is looked up as typed and refused, and nothing
/// goes.
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
    assert!(
        offered.contains(&notes) && offered.len() > 1,
        "premise: `nts` matches notes.txt among others; got {offered:?}"
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
    assert!(modified(&s, "*lsp*"), "premise: *lsp* reads as modified");
    let offered = kill_by_typing(&mut s, "lsp");
    assert!(
        offered.contains(&lsp_rs),
        "premise: src/lsp.rs holds `lsp` in its own name; got {offered:?}"
    );
    let asked = prompt(&s);
    assert!(
        asked.contains("*lsp*") && asked.contains("kill anyway"),
        "C-x k lsp RET asks about *lsp*; asked {asked:?}"
    );
    press(&mut s, KeyCode::Char('y'));
    let left = buffer_names(&s);
    assert!(
        !left.contains(&"*lsp*".to_owned()) && left.contains(&lsp_rs),
        "lsp kills *lsp* and keeps src/lsp.rs; left {left:?}"
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
