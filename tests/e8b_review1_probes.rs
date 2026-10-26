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
    assert_eq!(
        name, "proj/",
        "fixture: C-x d opened a listing of proj, named by its directory"
    );
    name
}

/// The query that reaches the listing, `proj/`, by its own letters,
/// which no other name here holds.
const LISTING_QUERY: &str = "proj";

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

/// FAILED at `0001946`, where a dired buffer named `*dired:<path>*` was
/// no longer reached by the word its name began with: `name_tier` read
/// the path inside it as directories and `C-x b dired RET` reached
/// `dired.lua`. Read again under D40 (E10c): `C-x b` matches names alone
/// and the listing's own letters reach it, while `dired`, which
/// `dired.lua`'s name begins with and the listing's does not hold first,
/// reaches the file, as the shared ranker puts it, asserted as the
/// premise. The band says where each switch went.
#[test]
fn review1_c_x_b_dired_reaches_the_dired_buffer_not_a_file_its_letters_spell() {
    let td = tempfile::tempdir().expect("tempdir");
    let (mut s, dired, lua, rs) = dired_session(td.path());
    let pool = buffer_names(&s);
    for (typed, first) in [(LISTING_QUERY, &dired), ("dired", &lua), ("dir", &lua)] {
        assert_eq!(
            rank_candidates(typed, &pool, None).first(),
            Some(first),
            "premise: the shared ranker puts {first} first for `{typed}`; pool {pool:?}"
        );
    }
    let mut reached = Vec::new();
    for typed in [LISTING_QUERY, "dired", "dir"] {
        show(&s, &rs);
        switch_by_typing(&mut s, typed);
        reached.push((typed, active_name(&s), status(&s)));
    }
    assert_eq!(
        reached
            .iter()
            .map(|(_, name, _)| name.clone())
            .collect::<Vec<_>>(),
        [dired.clone(), lua.clone(), lua.clone()],
        "C-x b reaches the listing by its own letters and dired.lua by the word its name \
         begins with; reached {reached:?}"
    );
}

/// FAILED at `0001946`: `C-x k` shares the `buffers` source and the
/// default `candidate` policy (`builtin/commands/default.lua`'s
/// `buffer.kill`), so E8b.1's places decided what it killed too, and
/// `kill_buffer_with_prompt` kills an unmodified buffer without asking
/// or saying anything: `C-x k dired RET` killed `dired.lua` and left
/// the listing. Under D40 the listing's own letters select it, so
/// `C-x k` on them acts on the listing and keeps `dired.lua`.
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
    type_str(&mut s, LISTING_QUERY);
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
        "C-x k {LISTING_QUERY} RET acts on the listing {dired} (kills it, or asks first); \
         offered {offered:?}, left {left:?}, asking {asking:?}, dired modified {modified}, \
         band {:?}",
        status(&s)
    );
}

/// PASSES at `0001946`; FAILS against `b374cd8`'s parent. The other
/// side of the row above: in the handoff's crowded fixture (`a.rs` and
/// `b.rs` under `bin-tests`, whose `b` both paths share), `C-x k b.rs
/// RET` kills `b.rs`. Before `b374cd8` the same keys killed `a.rs`, an
/// unmodified buffer, with no question and nothing on the band. Under
/// D40 a name holds no directory, so `b.rs` is held by `b.rs`'s name
/// alone.
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
    assert_eq!(
        offered,
        vec![b.clone()],
        "`b.rs` is held by b.rs's name alone"
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
/// The tie needed the typed name's first letter to be absent from the
/// directories (else the scorer started there and the shorter basename
/// won by its gap), so the row picks a letter its own temporary root
/// does not hold: `q.rs` beside `aq.rs` where the root has no `q`. Under
/// D40 the names hold no directory, and the scorer alone gives the exact
/// basename its start bonus; the row keeps the outcome.
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
    assert!(
        fuzzy_score(&typed, &b) > fuzzy_score(&typed, &ab),
        "premise: on names the scorer itself puts {typed} above a{letter}.rs"
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
