// tests/e8b_review1_probes.rs --- E8b review round 1's in-process probes.

//! Probes against PR #325's head `0001946`, through the keys a user
//! presses (`EditorState::dispatch_key`), as
//! `tests/minibuffer_accept_acceptance.rs` drives `C-x b`. Rows marked
//! FAILS fail at `0001946` by design, for the reason their doc comment
//! gives; rows marked PASSES pin behavior the handoff states or relies
//! on and no row of its own witnesses.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::minibuffer::{fuzzy_score, rank_candidates};
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

/// Visit `path`; return the buffer's name as pmacs stores it.
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

/// Drive the async runtime until nothing is parked or pending, as
/// `tests/dired_acceptance.rs` does: a dired listing is read on a
/// worker and lands on a later tick.
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

/// `C-x d`, the prefilled directory replaced by `dir`, RET: dired as a
/// user opens it. Returns the dired buffer's name.
fn dired_by_keys(s: &mut EditorState, dir: &Path) -> String {
    ctrl(s, 'x');
    press(s, KeyCode::Char('d'));
    assert!(active(s), "C-x d opens the prompt");
    clear_field(s);
    type_str(s, &dir.display().to_string());
    press(s, KeyCode::Enter);
    pump(s);
    let name = active_name(s);
    assert!(
        name.starts_with("*dired:") && name.ends_with("proj*"),
        "fixture: C-x d opened a dired buffer; showing {name}"
    );
    name
}

/// Empty the minibuffer's prefilled field as a user does, by holding
/// backspace (the minibuffer has no line kill; `dispatch_minibuffer_key`
/// maps `C-a` to the line start and swallows `C-k`).
fn clear_field(s: &mut EditorState) {
    for _ in 0..contents(s).chars().count() {
        press(s, KeyCode::Backspace);
    }
    assert_eq!(contents(s), "", "fixture: backspace empties the field");
}

/// `C-x b`, `text`, RET, as a user types them.
fn switch_by_typing(s: &mut EditorState, text: &str) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('b'));
    assert!(active(s), "C-x b opens the prompt");
    type_str(s, text);
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// A session holding a dired buffer on `proj` beside two files of it,
/// `dired.lua` and `editor.rs`, as pmacs's own checkout has them
/// (`builtin/runtime/dired.lua`, `src/editor.rs`). Returns the editor
/// and the three names: dired, `dired.lua`, `editor.rs`.
fn dired_session(td: &Path) -> (EditorState, String, String, String) {
    let proj = td.join("proj");
    std::fs::create_dir_all(&proj).expect("proj dir");
    std::fs::write(proj.join("dired.lua"), b"-- d\n").expect("write");
    std::fs::write(proj.join("editor.rs"), b"// e\n").expect("write");
    let mut s = fresh();
    let lua = visit(&s, &proj.join("dired.lua"));
    let rs = visit(&s, &proj.join("editor.rs"));
    let dired = dired_by_keys(&mut s, &proj);
    (s, dired, lua, rs)
}

/// FAILS at `0001946`: a dired buffer is no longer reached by the word
/// its name begins with. Its name is `*dired:<path>*`
/// (`builtin/runtime/dired.lua`'s `buffer_name`), so `name_tier` reads
/// the path inside it as directories: the last component is `proj*`,
/// and `dired` typed is "a match that needs the directories", place 4.
/// Any file whose own name holds the letters is place 3 and outranks it:
/// `C-x b dired RET` reaches `dired.lua`, and `C-x b dir RET` reaches a
/// file (here `dired.lua`; `editor.rs` too holds `dir`). The shared
/// ranker the prompt used before `b374cd8` puts the dired buffer first
/// for both, asserted below as the premise, so this is a reversal the
/// change made, in D18's own class: a special buffer reached by a
/// subsequence of its name, as `*lsp*` by `lsp`. The band says where
/// the switch went, which is how a user would find out.
#[test]
fn review1_c_x_b_dired_reaches_the_dired_buffer_not_a_file_its_letters_spell() {
    let td = tempfile::tempdir().expect("tempdir");
    let (mut s, dired, lua, rs) = dired_session(td.path());
    let pool = buffer_names(&s);
    for typed in ["dired", "dir"] {
        assert_eq!(
            rank_candidates(typed, &pool, None).first(),
            Some(&dired),
            "premise: before b374cd8 the prompt ranked the dired buffer first for `{typed}`; \
             pool {pool:?}"
        );
    }
    let mut reached = Vec::new();
    for typed in ["dired", "dir"] {
        show(&s, &rs);
        switch_by_typing(&mut s, typed);
        reached.push((typed, active_name(&s), status(&s)));
    }
    assert!(
        reached.iter().all(|(_, name, _)| *name == dired),
        "C-x b <a subsequence of the dired buffer's own name> RET reaches the dired buffer \
         {dired}; reached {reached:?} (dired.lua is {lua})"
    );
}

/// FAILS at `0001946`: `C-x k` shares the `buffers` source and the
/// default `candidate` policy (`builtin/commands/default.lua`'s
/// `buffer.kill`), so E8b.1's places decide what it kills too, and
/// `kill_buffer_with_prompt` kills an unmodified buffer without asking
/// or saying anything. `C-x k dired RET` kills `dired.lua`, the file,
/// and leaves the dired buffer; before `b374cd8` it killed the dired
/// buffer (the premise in the row above). The handoff names `C-x b`
/// alone and says every other source is unchanged.
#[test]
fn review1_c_x_k_dired_kills_the_dired_buffer_not_a_file() {
    let td = tempfile::tempdir().expect("tempdir");
    let (mut s, dired, lua, rs) = dired_session(td.path());
    show(&s, &rs);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('k'));
    assert!(active(&s), "C-x k opens the prompt");
    assert_eq!(contents(&s), rs, "fixture: C-x k prefills the shown buffer");
    clear_field(&mut s);
    type_str(&mut s, "dired");
    let offered = candidates(&s);
    press(&mut s, KeyCode::Enter);
    let left = buffer_names(&s);
    let asking: String = eval(
        &s,
        "return pmacs.minibuffer.is_active() and pmacs.minibuffer.prompt() or ''",
    );
    let modified: bool = eval(
        &s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do\n\
               local d = pmacs.describe.buffer(id)\n\
               if d.name == {dired:?} then return d.modified end\n\
             end\n\
             return false"
        ),
    );
    assert!(
        left.contains(&lua) && (!left.contains(&dired) || asking.contains(&dired)),
        "C-x k dired RET acts on the dired buffer {dired} (kills it, or asks first); \
         offered {offered:?}, left {left:?}, asking {asking:?}, dired modified {modified}, \
         band {:?}",
        status(&s)
    );
}

/// PASSES at `0001946`; FAILS against `b374cd8`'s parent. The other
/// side of the row above: in the handoff's crowded fixture (`a.rs` and
/// `b.rs` under `bin-tests`, whose `b` both paths share), `C-x k b.rs
/// RET` kills `b.rs`. Before `b374cd8` the same keys killed `a.rs`, an
/// unmodified buffer, with no question and nothing on the band: E8b.1
/// fixed a silent wrong kill that no pass records, and no row of the
/// phase witnesses `C-x k` at all.
#[test]
fn review1_c_x_k_b_rs_kills_b_rs_in_a_crowded_directory() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("bin-tests");
    std::fs::create_dir_all(&dir).expect("dir");
    let mut s = fresh();
    let mut names = Vec::new();
    for file in ["a.rs", "b.rs", "c.txt"] {
        std::fs::write(dir.join(file), b"x\n").expect("write");
        names.push(visit(&s, &dir.join(file)));
    }
    let (a, b, c) = (names[0].clone(), names[1].clone(), names[2].clone());
    show(&s, &c);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('k'));
    clear_field(&mut s);
    type_str(&mut s, "b.rs");
    let offered = candidates(&s);
    assert!(
        offered.contains(&a) && offered.contains(&b),
        "fixture premise: `b.rs` is a subsequence of both paths; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    let left = buffer_names(&s);
    assert!(
        left.contains(&a) && !left.contains(&b),
        "C-x k b.rs RET kills b.rs and keeps a.rs; left {left:?}, band {:?}",
        status(&s)
    );
}

/// PASSES at `0001946`, and is the only row through the keys that does
/// so because of places 1 and 2. Every acceptance row's bare name
/// (`b.rs` beside `a.rs`) is also won by place 3 alone, since `a.rs`'s
/// last component does not hold `b`; so a mutant that drops "ends with
/// it at a path component" passes the handoff's rows. Here the
/// neighbor's own name ends with the typed one (`aq.rs` for `q.rs`, as
/// `domain.rs` for `main.rs` or `stdlib.rs` for `lib.rs`): both hold it
/// in their last component, the scorer gives them one score, and the
/// lexical tie-break picks `aq.rs` unless the exact basename is placed
/// first.
///
/// The tie needs the typed name's first letter to be absent from the
/// directories (else the scorer starts there and the shorter basename
/// wins by its gap), so the row picks a letter its own temporary root
/// does not hold: `q.rs` beside `aq.rs` where the root has no `q`.
#[test]
fn review1_an_exact_basename_beats_a_name_ending_with_it() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("zz");
    std::fs::create_dir_all(&dir).expect("dir");
    let root = dir.display().to_string().to_lowercase();
    let letter = ['q', 'x', 'z', 'w', 'y', 'k', 'f', 'v']
        .into_iter()
        .find(|c| !root.contains(*c))
        .expect("fixture: some letter is absent from the temporary root");
    let typed = format!("{letter}.rs");
    let mut s = fresh();
    let mut names = Vec::new();
    for file in [format!("a{letter}.rs"), typed.clone(), "c.txt".to_owned()] {
        std::fs::write(dir.join(&file), b"x\n").expect("write");
        names.push(visit(&s, &dir.join(&file)));
    }
    let (ab, b, c) = (names[0].clone(), names[1].clone(), names[2].clone());
    assert_eq!(
        fuzzy_score(&typed, &ab),
        fuzzy_score(&typed, &b),
        "premise: the scorer cannot tell them apart"
    );
    assert_eq!(
        rank_candidates(&typed, &[b.clone(), ab.clone()], None)[0],
        ab,
        "premise: the shared ranker puts a{letter}.rs first"
    );
    show(&s, &c);
    switch_by_typing(&mut s, &typed);
    assert_eq!(
        active_name(&s),
        b,
        "C-x b {typed} RET reaches {typed}, not a{letter}.rs"
    );
    assert_eq!(status(&s), format!("switch-buffer: showing {b}"));
}

/// PASSES at `0001946`, on the shipped scorer: the brief's "−37" is
/// what `pmacs::minibuffer::fuzzy_score` gives `b.rs` against both
/// paths of the shape #318 recorded
/// (`/home/jeans/build/pmacs-gate-targets/tmp/fr1a/.tmpXXXXXX/`, with a
/// suffix holding none of `b`, `.`, `r`, `s`), and the shared ranker
/// then puts `a.rs` first. So the number the brief cites is the shipped
/// function's on #318's own fixture, and `b374cd8`'s unit row, which
/// asserts the same tie on a shorter path, measures that function and
/// not a copy of it.
#[test]
fn review1_the_shipped_scorer_gives_318_s_paths_minus_37() {
    let root = "/home/jeans/build/pmacs-gate-targets/tmp/fr1a/.tmpQWXYZK";
    let a = format!("{root}/a.rs");
    let b = format!("{root}/b.rs");
    assert_eq!(fuzzy_score("b.rs", &a), Some(-37));
    assert_eq!(fuzzy_score("b.rs", &b), Some(-37));
    assert_eq!(
        rank_candidates("b.rs", &[b.clone(), a.clone()], None),
        vec![a, b]
    );
}

#[path = "common/iso.rs"]
mod iso;
