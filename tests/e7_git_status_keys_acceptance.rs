// tests/e7_git_status_keys_acceptance.rs --- E7.1: the four staging
// gestures on `*git-status*`.

//! `s` stages the row, `u` unstages it, `x` discards it behind a
//! y-or-n question, and `c` commits what is staged with a message typed
//! at the minibuffer; every one refreshes the panel afterwards. Each
//! test drives a real `EditorState` against a real repository in a
//! tempdir through the keys a user presses, and reads the result back
//! from `git status --porcelain=v2` (the fact) and the panel (what the
//! user sees). The destructive gesture's question is probed the way E1.1
//! was: an empty answer re-asks, a wrong word re-asks, `C-g` refuses,
//! and only `y` destroys.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::{CellSize, FrontendId};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Harness (the shape of tests/git_status_stage1_acceptance.rs; each
// file owns its helpers)
// ---------------------------------------------------------------------------

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
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

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn minibuffer_active(s: &EditorState) -> bool {
    eval(s, "return pmacs.minibuffer.is_active()")
}

fn minibuffer_prompt(s: &EditorState) -> String {
    eval::<Option<String>>(s, "return pmacs.minibuffer.prompt()").unwrap_or_default()
}

/// `M-x <name> RET`, asserting the palette had `name` selected.
fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    assert!(minibuffer_active(s), "M-x must open the command palette");
    type_str(s, name);
    assert_eq!(
        eval::<Option<String>>(s, "return pmacs.minibuffer.selected()").as_deref(),
        Some(name),
        "the completion source must have {name} selected"
    );
    press(s, KeyCode::Enter);
}

/// Text of the buffer named `name`, or `""` when absent.
fn named_text(s: &EditorState, name: &str) -> String {
    let b: mlua::String = eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do\n\
               if pmacs.describe.buffer(id).name == {name:?} then\n\
                 return id:slice(0, id:len())\n\
               end\n\
             end\n\
             return \"\""
        ),
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn panel_text(s: &EditorState) -> String {
    named_text(s, "*git-status*")
}

/// Fresh editor with LSP spawning disabled and frame geometry declared
/// (a panel is derived-hidden until the frame size is known).
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(24, 80));
    s
}

/// Drive frames until `pred` holds, pumping the process supervisor.
fn pump_until(
    s: &mut EditorState,
    timeout_ms: u64,
    mut pred: impl FnMut(&EditorState) -> bool,
) -> bool {
    let stop = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if pred(s) {
            return true;
        }
        if Instant::now() >= stop {
            return false;
        }
        s.tick_processes();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pump for `ms` without waiting on anything --- for the assertions
/// whose subject is that NOTHING happens.
fn pump_for(s: &mut EditorState, ms: u64) {
    pump_until(s, ms, |_| false);
}

// ---------------------------------------------------------------------------
// Repository fixtures (real `git`, in a tempdir)
// ---------------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The porcelain-v2 XY code git reports for `rel`, or `""` when the
/// path is clean (absent from status). Read from the whole status, not
/// a pathspec-limited one: a pathspec naming only a rename's destination
/// stops git pairing it, and the row reads `A.` instead of `R.`.
fn xy(root: &Path, rel: &str) -> String {
    let out = git(root, &["status", "--porcelain=v2"]);
    for line in out.lines() {
        let mut fields = line.splitn(2, ' ');
        let tag = fields.next().unwrap_or("");
        let rest = fields.next().unwrap_or("");
        match tag {
            "?" | "!" if rest == rel => return tag.repeat(2),
            "1" | "u" => {
                let code = &rest[..2];
                if rest.rsplit(' ').next() == Some(rel) {
                    return code.to_owned();
                }
            }
            "2" => {
                // `2 <XY> ... <score> <path>\t<origPath>`
                let code = &rest[..2];
                let before_tab = rest.split('\t').next().unwrap_or("");
                if before_tab.rsplit(' ').next() == Some(rel) {
                    return code.to_owned();
                }
            }
            _ => {}
        }
    }
    String::new()
}

fn head(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

fn init_repo(root: &Path) {
    git(root, &["init", "-q", "-b", "main", "."]);
    git(root, &["config", "user.email", "gate@example.invalid"]);
    git(root, &["config", "user.name", "Gate"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir -p");
    }
    std::fs::write(&p, body).expect("write fixture file");
}

/// One row of every class: staged, unstaged, both, deleted, renamed,
/// untracked, plus two clean tracked files.
fn mixed_repo(root: &Path) {
    init_repo(root);
    write(root, "Cargo.toml", "[package]\nname = \"fixture\"\n");
    write(root, "a1.txt", "a1 base\n");
    write(root, "staged.txt", "staged base\n");
    write(root, "unstaged.txt", "unstaged base\n");
    write(root, "both.txt", "both base\n");
    write(root, "deleted.txt", "deleted base\n");
    write(
        root,
        "renamed_from.txt",
        "a line of content long enough for rename detection to score it\n",
    );
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);

    write(root, "staged.txt", "staged base\nstaged edit\n");
    git(root, &["add", "staged.txt"]);
    write(root, "unstaged.txt", "unstaged base\nworktree edit\n");
    write(root, "both.txt", "both base\nstaged edit\n");
    git(root, &["add", "both.txt"]);
    write(root, "both.txt", "both base\nstaged edit\nworktree edit\n");
    std::fs::remove_file(root.join("deleted.txt")).expect("rm deleted.txt");
    git(root, &["mv", "renamed_from.txt", "renamed_to.txt"]);
    write(root, "untracked.txt", "untracked body\n");
}

/// `git init`, stage three files, edit one (`AM`), delete one (`AD`).
fn unborn_repo(root: &Path) {
    init_repo(root);
    write(root, "am.txt", "am base\n");
    write(root, "ad.txt", "ad base\n");
    write(root, "plain.txt", "plain base\n");
    git(root, &["add", "-A"]);
    write(root, "am.txt", "am base\nworktree edit\n");
    std::fs::remove_file(root.join("ad.txt")).expect("rm ad.txt");
}

fn tempdir() -> (tempfile::TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let canonical = td.path().canonicalize().expect("canonicalize tempdir");
    (td, canonical)
}

/// Bound project detection to the fixture, open a file inside it, and
/// run `M-x git.status` to completion.
fn open_panel(s: &mut EditorState, root: &Path, seed_file: &str) {
    let root_str = root.display().to_string();
    let seed = root.join(seed_file).display().to_string();
    exec(
        s,
        &format!(
            "pmacs.project.set_search_boundary({root_str:?})\n\
             pmacs.buffer.find_or_open({seed:?})"
        ),
    );
    exec(s, "pmacs.git.status()");
    assert!(
        pump_until(s, 15_000, |s| !panel_text(s).is_empty()),
        "the status panel must render; status was {:?}",
        status(s)
    );
}

/// The path a panel data line names: the text after the two-letter
/// code and its two spaces, up to a rename's ` <- origin`.
fn row_path(line: &str) -> Option<&str> {
    let rest = line.get(4..)?;
    Some(rest.split("  <- ").next().unwrap_or(rest))
}

/// True when the panel has a data row for exactly `path` (not a row
/// whose path merely contains it: `unstaged.txt` contains `staged.txt`).
fn has_row(text: &str, path: &str) -> bool {
    text.lines()
        .skip(1)
        .any(|line| row_path(line) == Some(path))
}

/// The 1-based data line of the panel row for exactly `path`, or `None`.
fn row_line(s: &EditorState, path: &str) -> Option<usize> {
    panel_text(s)
        .lines()
        .enumerate()
        .find(|(i, line)| *i > 0 && row_path(line) == Some(path))
        .map(|(i, _)| i)
}

fn cursor_line(s: &EditorState) -> usize {
    let n: i64 = eval(s, "return pmacs.editor.cursor_line()");
    usize::try_from(n).expect("cursor line fits")
}

/// Seat the cursor on the row containing `needle` with listview's own
/// `n`/`p`, so a broken panel keymap fails here.
fn seat_on(s: &mut EditorState, needle: &str) {
    let target = row_line(s, needle)
        .unwrap_or_else(|| panic!("no row matching {needle:?} in:\n{}", panel_text(s)));
    let current = cursor_line(s);
    if current <= target {
        for _ in current..target {
            press(s, KeyCode::Char('n'));
        }
    } else {
        for _ in target..current {
            press(s, KeyCode::Char('p'));
        }
    }
    assert_eq!(
        cursor_line(s),
        target,
        "cursor must land on the {needle:?} row"
    );
}

/// The panel line the cursor is on, as text.
fn line_under_cursor(s: &EditorState) -> String {
    panel_text(s)
        .lines()
        .nth(cursor_line(s))
        .unwrap_or_default()
        .to_owned()
}

/// Press `code` and pump until the panel's text changes from what it
/// was (the refresh after a mutation), returning the new text.
fn press_and_wait_refresh(s: &mut EditorState, code: KeyCode) -> String {
    let before = panel_text(s);
    press(s, code);
    assert!(
        pump_until(s, 15_000, |s| {
            let now = panel_text(s);
            now != before && !now.contains("(refreshing...)")
        }),
        "the panel must refresh after the gesture; it reads:\n{}\nstatus: {:?}",
        panel_text(s),
        status(s)
    );
    panel_text(s)
}

/// Answer the standing y-or-n question with `y` and pump until the
/// panel refreshes.
fn answer_yes_and_wait(s: &mut EditorState) -> String {
    assert!(minibuffer_active(s), "a question must be standing");
    let before = panel_text(s);
    press(s, KeyCode::Char('y'));
    press(s, KeyCode::Enter);
    assert!(
        pump_until(s, 15_000, |s| {
            let now = panel_text(s);
            now != before && !now.contains("(refreshing...)")
        }),
        "the panel must refresh after the discard; it reads:\n{}\nstatus: {:?}",
        panel_text(s),
        status(s)
    );
    panel_text(s)
}

fn last_spawn(s: &EditorState) -> (String, String, Vec<String>) {
    let label: String = eval(s, "return pmacs.git._last_spawn.label");
    let purpose: String = eval(s, "return pmacs.git._last_spawn.purpose");
    let args: Vec<String> = eval(s, "return pmacs.git._last_spawn.args");
    (label, purpose, args)
}

// ---------------------------------------------------------------------------
// s / u
// ---------------------------------------------------------------------------

/// `s` on the unstaged row stages it: git says `M.`, the panel shows
/// `M.`, the cursor stays on the row, and the child carried its own
/// purpose naming the path.
#[test]
fn e7_1_s_stages_the_row_under_the_cursor() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "unstaged.txt");
    assert_eq!(
        xy(&root, "unstaged.txt"),
        ".M",
        "positive control: unstaged"
    );

    let text = press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(xy(&root, "unstaged.txt"), "M.", "s must stage the file");
    assert!(
        text.lines().any(|l| l == "M.  unstaged.txt"),
        "the panel must show the staged code; it reads:\n{text}"
    );
    assert!(
        row_path(&line_under_cursor(&s)) == Some("unstaged.txt"),
        "the cursor must stay on the row it staged; it is on {:?}",
        line_under_cursor(&s)
    );
    assert_eq!(status(&s), "git: staged unstaged.txt");
    let (label, purpose, args) = last_spawn(&s);
    // The last child is the refresh's `git status`; the mutation's own
    // spawn precedes it in the ring.
    assert_eq!(label, "git status");
    let ring: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");
    let add = ring
        .iter()
        .rev()
        .find(|a| a.iter().any(|x| x == "add"))
        .expect("an `add` invocation in the spawn ring");
    assert_eq!(add[0], "--no-optional-locks");
    assert_eq!(&add[1..3], &["-C".to_owned(), root.display().to_string()]);
    assert_eq!(&add[3..], &["add", "--", "unstaged.txt"]);
    assert!(purpose.contains("*git-status*"), "purpose: {purpose}");
    assert!(args.contains(&"status".to_owned()));
}

/// `u` on the staged row unstages it: `M.` becomes `.M` in git and in
/// the panel, and the file's bytes are untouched.
#[test]
fn e7_1_u_unstages_the_row_under_the_cursor() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "staged.txt");
    assert_eq!(xy(&root, "staged.txt"), "M.", "positive control: staged");
    let before = read(&root, "staged.txt");

    let text = press_and_wait_refresh(&mut s, KeyCode::Char('u'));
    assert_eq!(xy(&root, "staged.txt"), ".M", "u must unstage the file");
    assert!(
        text.lines().any(|l| l == ".M  staged.txt"),
        "panel:\n{text}"
    );
    assert_eq!(
        read(&root, "staged.txt"),
        before,
        "unstaging edits no bytes"
    );
    assert_eq!(status(&s), "git: unstaged staged.txt");
}

/// Stage, then unstage, the same row: the round trip lands where it
/// started, in git and in the panel, with the file's bytes unchanged.
#[test]
fn e7_1_s_then_u_round_trips_a_row() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    let before = read(&root, "unstaged.txt");
    seat_on(&mut s, "unstaged.txt");
    press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(xy(&root, "unstaged.txt"), "M.");
    seat_on(&mut s, "unstaged.txt");
    let text = press_and_wait_refresh(&mut s, KeyCode::Char('u'));
    assert_eq!(xy(&root, "unstaged.txt"), ".M", "back where it started");
    assert!(
        text.lines().any(|l| l == ".M  unstaged.txt"),
        "panel:\n{text}"
    );
    assert_eq!(read(&root, "unstaged.txt"), before);
}

/// The rows a user meets that are not a plain modification: a deleted
/// file stages its removal, an untracked file stages as an add, and the
/// `MM` row stages the worktree half onto the index half.
#[test]
fn e7_1_s_covers_the_deleted_untracked_and_both_rows() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");

    seat_on(&mut s, "deleted.txt");
    assert_eq!(xy(&root, "deleted.txt"), ".D");
    press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(xy(&root, "deleted.txt"), "D.", "the removal is staged");

    seat_on(&mut s, "untracked.txt");
    assert_eq!(xy(&root, "untracked.txt"), "??");
    press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(xy(&root, "untracked.txt"), "A.", "the add is staged");

    seat_on(&mut s, "both.txt");
    assert_eq!(xy(&root, "both.txt"), "MM");
    press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(
        xy(&root, "both.txt"),
        "M.",
        "the worktree half joins the index"
    );

    // And `u` on the staged add makes it untracked again.
    seat_on(&mut s, "untracked.txt");
    press_and_wait_refresh(&mut s, KeyCode::Char('u'));
    assert_eq!(xy(&root, "untracked.txt"), "??");
    assert_eq!(
        read(&root, "untracked.txt").as_deref(),
        Some("untracked body\n")
    );
}

/// A rename row carries two paths and `s`/`u` move both.
#[test]
fn e7_1_u_and_s_move_both_paths_of_a_rename() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "renamed_to.txt");
    assert_eq!(
        xy(&root, "renamed_to.txt"),
        "R.",
        "positive control: staged rename"
    );

    press_and_wait_refresh(&mut s, KeyCode::Char('u'));
    assert_eq!(
        xy(&root, "renamed_from.txt"),
        ".D",
        "the origin's removal is unstaged"
    );
    assert_eq!(
        xy(&root, "renamed_to.txt"),
        "??",
        "the destination is untracked"
    );

    // Staging the destination alone from its `??` row re-stages the
    // add; the origin's deletion is its own row now.
    seat_on(&mut s, "renamed_to.txt");
    press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(xy(&root, "renamed_to.txt"), "A.");
    seat_on(&mut s, "renamed_from.txt");
    press_and_wait_refresh(&mut s, KeyCode::Char('s'));
    assert_eq!(
        xy(&root, "renamed_to.txt"),
        "R.",
        "with both halves staged git reads the pair as a rename again"
    );
}

// ---------------------------------------------------------------------------
// x
// ---------------------------------------------------------------------------

/// The question cannot be stepped past: RET on an empty answer re-asks,
/// a word that is neither re-asks, and `C-g` refuses. Through all of it
/// the file and the index are untouched and no git child was spawned.
#[test]
fn e7_1_x_asks_a_question_that_cannot_be_skipped() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "unstaged.txt");
    let before = read(&root, "unstaged.txt");
    let ring_before: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");

    press(&mut s, KeyCode::Char('x'));
    assert!(minibuffer_active(&s), "x must ask before it discards");
    assert!(
        minibuffer_prompt(&s).contains("Discard changes to unstaged.txt (back to HEAD)?"),
        "the question names the file and what will happen; prompt: {:?}",
        minibuffer_prompt(&s)
    );

    // An empty RET.
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 150);
    assert!(
        minibuffer_active(&s),
        "an empty answer must leave the question standing"
    );
    assert_eq!(status(&s), "please answer y or n");
    assert_eq!(
        read(&root, "unstaged.txt"),
        before,
        "nothing discarded on RET"
    );

    // A word that is neither.
    type_str(&mut s, "sure");
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 150);
    assert!(minibuffer_active(&s), "an unrecognized answer must re-ask");
    assert_eq!(
        read(&root, "unstaged.txt"),
        before,
        "nothing discarded on a wrong word"
    );

    // C-g.
    ctrl(&mut s, 'g');
    pump_for(&mut s, 150);
    assert!(!minibuffer_active(&s), "C-g must close the question");
    assert_eq!(
        status(&s),
        "Quit",
        "C-g's own message is what the user sees"
    );
    assert_eq!(read(&root, "unstaged.txt"), before, "C-g is a refusal");
    assert_eq!(xy(&root, "unstaged.txt"), ".M");
    let ring_after: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");
    assert_eq!(
        ring_after, ring_before,
        "no git child ran for a refused question"
    );
}

/// E7b.2: `x`'s question is the typed one. A bare `y` --- the key that
/// answers the quit prompt since E7b.2 --- answers nothing here: the
/// question stands with the letter in its field, the file and the
/// index are untouched, and only RET after it discards. Bitten by
/// asking through `pmacs.minibuffer.y_or_n` instead of `yes_or_no`.
#[test]
fn e7b_2_x_s_question_is_not_answered_by_one_key() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "unstaged.txt");
    let before = read(&root, "unstaged.txt");
    let ring_before: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");

    press(&mut s, KeyCode::Char('x'));
    assert!(minibuffer_active(&s), "x must ask before it discards");
    press(&mut s, KeyCode::Char('y'));
    pump_for(&mut s, 150);
    assert!(
        minibuffer_active(&s),
        "a bare y must not answer the typed question"
    );
    assert_eq!(
        eval::<String>(&s, "return pmacs.minibuffer.contents()"),
        "y",
        "the letter is in the field, waiting for RET"
    );
    assert_eq!(
        read(&root, "unstaged.txt"),
        before,
        "nothing discarded on y"
    );
    let ring_mid: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");
    assert_eq!(ring_mid, ring_before, "no git child ran on a bare y");

    let panel_before = panel_text(&s);
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 15_000, |s| {
            let now = panel_text(s);
            now != panel_before && !now.contains("(refreshing...)")
        }),
        "RET after y discards and the panel refreshes"
    );
    assert_ne!(read(&root, "unstaged.txt"), before, "y RET discards");
}

/// `n` is a refusal too, and says so.
#[test]
fn e7_1_x_then_n_keeps_the_file() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "both.txt");
    let before = read(&root, "both.txt");
    press(&mut s, KeyCode::Char('x'));
    assert!(minibuffer_active(&s));
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 200);
    assert!(!minibuffer_active(&s));
    assert_eq!(status(&s), "git: kept both.txt");
    assert_eq!(read(&root, "both.txt"), before);
    assert_eq!(xy(&root, "both.txt"), "MM");
}

/// `y` discards: the modified file goes back to HEAD's bytes in the
/// index and the worktree, the `MM` row loses both halves, and the
/// deleted file comes back. Each row then leaves the panel.
#[test]
fn e7_1_x_then_y_restores_head_for_tracked_rows() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");

    seat_on(&mut s, "unstaged.txt");
    press(&mut s, KeyCode::Char('x'));
    let text = answer_yes_and_wait(&mut s);
    assert_eq!(
        read(&root, "unstaged.txt").as_deref(),
        Some("unstaged base\n")
    );
    assert_eq!(xy(&root, "unstaged.txt"), "", "clean: absent from status");
    assert!(
        !has_row(&text, "unstaged.txt"),
        "the row leaves the panel:\n{text}"
    );
    assert_eq!(status(&s), "git: discarded unstaged.txt");

    seat_on(&mut s, "both.txt");
    press(&mut s, KeyCode::Char('x'));
    assert!(minibuffer_prompt(&s).contains("Discard changes to both.txt (back to HEAD)?"));
    answer_yes_and_wait(&mut s);
    assert_eq!(read(&root, "both.txt").as_deref(), Some("both base\n"));
    assert_eq!(
        xy(&root, "both.txt"),
        "",
        "index and worktree both back at HEAD"
    );

    seat_on(&mut s, "deleted.txt");
    press(&mut s, KeyCode::Char('x'));
    answer_yes_and_wait(&mut s);
    assert_eq!(
        read(&root, "deleted.txt").as_deref(),
        Some("deleted base\n"),
        "discarding a deletion restores the file"
    );
    assert_eq!(xy(&root, "deleted.txt"), "");
}

/// An untracked file is in no commit, so the question says "delete",
/// and `y` removes it.
#[test]
fn e7_1_x_on_an_untracked_row_says_delete_and_deletes() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "untracked.txt");
    press(&mut s, KeyCode::Char('x'));
    assert!(
        minibuffer_prompt(&s).contains("Delete untracked untracked.txt? It is in no commit"),
        "prompt: {:?}",
        minibuffer_prompt(&s)
    );
    let text = answer_yes_and_wait(&mut s);
    assert!(!root.join("untracked.txt").exists(), "the file is gone");
    assert!(!has_row(&text, "untracked.txt"), "panel:\n{text}");
    assert_eq!(status(&s), "git: deleted untracked.txt");
}

/// A staged add is in no commit either: the question says so, and `y`
/// removes it from the index and the worktree.
#[test]
fn e7_1_x_on_a_staged_add_removes_it() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    write(&root, "added.txt", "new file\n");
    git(&root, &["add", "added.txt"]);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "added.txt");
    assert_eq!(xy(&root, "added.txt"), "A.");
    press(&mut s, KeyCode::Char('x'));
    assert!(minibuffer_prompt(&s).contains("Delete added.txt? It is in no commit"));
    answer_yes_and_wait(&mut s);
    assert!(!root.join("added.txt").exists());
    assert_eq!(xy(&root, "added.txt"), "");
}

/// A rename's discard restores the origin from HEAD and removes the
/// destination; the question names both.
#[test]
fn e7_1_x_on_a_rename_restores_the_origin_and_removes_the_destination() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    seat_on(&mut s, "renamed_to.txt");
    press(&mut s, KeyCode::Char('x'));
    assert!(
        minibuffer_prompt(&s).contains("Discard renamed_to.txt (back to HEAD's renamed_from.txt)?"),
        "prompt: {:?}",
        minibuffer_prompt(&s)
    );
    let text = answer_yes_and_wait(&mut s);
    assert!(root.join("renamed_from.txt").exists(), "the origin is back");
    assert!(
        !root.join("renamed_to.txt").exists(),
        "the destination is gone"
    );
    assert_eq!(xy(&root, "renamed_from.txt"), "");
    assert!(
        !has_row(&text, "renamed_to.txt") && !has_row(&text, "renamed_from.txt"),
        "panel:\n{text}"
    );
}

/// On an unborn branch nothing is in any commit: `u` removes the index
/// entry and leaves the file (the only unstaged an unborn tree has), and
/// `x` says delete and deletes.
#[test]
fn e7_1_unborn_u_leaves_the_file_and_x_says_delete() {
    let (_td, root) = tempdir();
    unborn_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "plain.txt");
    assert!(
        panel_text(&s).contains("no commits yet"),
        "positive control: unborn; header {:?}",
        panel_text(&s).lines().next()
    );

    seat_on(&mut s, "am.txt");
    assert_eq!(xy(&root, "am.txt"), "AM");
    press_and_wait_refresh(&mut s, KeyCode::Char('u'));
    assert_eq!(
        xy(&root, "am.txt"),
        "??",
        "unstaged on an unborn branch is untracked"
    );
    assert_eq!(
        read(&root, "am.txt").as_deref(),
        Some("am base\nworktree edit\n"),
        "the worktree file is untouched"
    );

    seat_on(&mut s, "plain.txt");
    assert_eq!(xy(&root, "plain.txt"), "A.");
    press(&mut s, KeyCode::Char('x'));
    assert!(
        minibuffer_prompt(&s).contains("Delete plain.txt? It is in no commit"),
        "prompt: {:?}",
        minibuffer_prompt(&s)
    );
    answer_yes_and_wait(&mut s);
    assert!(!root.join("plain.txt").exists());
    assert_eq!(xy(&root, "plain.txt"), "");
}

// ---------------------------------------------------------------------------
// c
// ---------------------------------------------------------------------------

/// An empty message commits nothing and says so; a whitespace-only one
/// is empty too. HEAD does not move and no git child runs.
#[test]
fn e7_1_c_with_an_empty_message_commits_nothing() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    let before = head(&root);
    let ring_before: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");

    press(&mut s, KeyCode::Char('c'));
    assert!(minibuffer_active(&s), "c must prompt for a message");
    assert_eq!(minibuffer_prompt(&s), "Commit message: ");
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 200);
    assert!(!minibuffer_active(&s));
    assert_eq!(status(&s), "git: empty commit message; nothing committed");
    assert_eq!(head(&root), before, "HEAD must not move");

    press(&mut s, KeyCode::Char('c'));
    type_str(&mut s, "   ");
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 200);
    assert_eq!(status(&s), "git: empty commit message; nothing committed");
    assert_eq!(head(&root), before);
    let ring_after: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");
    assert_eq!(
        ring_after, ring_before,
        "no git child ran for an empty message"
    );
    assert_eq!(
        xy(&root, "staged.txt"),
        "M.",
        "the staged row is still staged"
    );
}

/// A real message commits exactly what is staged: the staged rows leave
/// the panel, the unstaged ones stay, and `git log` carries the message
/// as typed --- RET takes the typed text under D18, with no candidate
/// list to take instead.
#[test]
fn e7_1_c_with_a_message_commits_the_staged_rows() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    let before = head(&root);

    press(&mut s, KeyCode::Char('c'));
    type_str(&mut s, "stage two: the panel's own commit");
    let panel_before = panel_text(&s);
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 15_000, |s| {
            let now = panel_text(s);
            now != panel_before && !now.contains("(refreshing...)") && !has_row(&now, "staged.txt")
        }),
        "the staged rows must leave the panel; it reads:\n{}\nstatus: {:?}",
        panel_text(&s),
        status(&s)
    );
    assert_ne!(head(&root), before, "HEAD moved");
    assert_eq!(
        git(&root, &["log", "-1", "--format=%s"]).trim(),
        "stage two: the panel's own commit"
    );
    assert_eq!(xy(&root, "staged.txt"), "", "committed");
    assert_eq!(
        xy(&root, "renamed_to.txt"),
        "",
        "the staged rename went with it"
    );
    assert_eq!(xy(&root, "both.txt"), ".M", "the worktree half of MM stays");
    assert_eq!(
        xy(&root, "unstaged.txt"),
        ".M",
        "unstaged rows are not committed"
    );
    assert!(has_row(&panel_text(&s), "unstaged.txt"));
    assert_eq!(
        status(&s),
        "git: committed stage two: the panel's own commit"
    );
}

/// With nothing staged git refuses, and the refusal is what the user
/// sees; HEAD does not move.
#[test]
fn e7_1_c_with_nothing_staged_reports_gits_refusal() {
    let (_td, root) = tempdir();
    init_repo(&root);
    write(&root, "a.txt", "a\n");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    write(&root, "a.txt", "a\nb\n");
    let mut s = editor();
    open_panel(&mut s, &root, "a.txt");
    let before = head(&root);
    press(&mut s, KeyCode::Char('c'));
    type_str(&mut s, "nothing here");
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 15_000, |s| status(s).starts_with("git commit: ")),
        "git's refusal must reach the status line; status: {:?}",
        status(&s)
    );
    assert_eq!(head(&root), before, "HEAD must not move");
    assert_eq!(xy(&root, "a.txt"), ".M");
}

/// `C-g` at the message prompt cancels the commit.
#[test]
fn e7_1_c_then_c_g_cancels() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    let before = head(&root);
    press(&mut s, KeyCode::Char('c'));
    type_str(&mut s, "half a mess");
    ctrl(&mut s, 'g');
    pump_for(&mut s, 200);
    assert!(!minibuffer_active(&s));
    assert_eq!(
        status(&s),
        "Quit",
        "C-g's own message is what the user sees"
    );
    assert_eq!(head(&root), before);
    assert_eq!(xy(&root, "staged.txt"), "M.");
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// The keys refuse where there is no row, refuse from another buffer,
/// and refuse under `git.enabled = false`, each with a message and no
/// child spawned.
#[test]
fn e7_1_the_keys_refuse_outside_a_row_and_when_disabled() {
    let (_td, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "a1.txt");
    let ring_before: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");

    // The header line.
    while cursor_line(&s) > 0 {
        press(&mut s, KeyCode::Char('p'));
    }
    exec(&s, "pmacs.editor.move_to_line(0)");
    press(&mut s, KeyCode::Char('s'));
    assert_eq!(status(&s), "git: no file on this line");
    press(&mut s, KeyCode::Char('x'));
    assert_eq!(status(&s), "git: no file on this line");
    assert!(!minibuffer_active(&s), "no question without a row");

    // Another buffer, through M-x.
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            root.join("a1.txt").display().to_string()
        ),
    );
    m_x(&mut s, "git.stage");
    assert_eq!(status(&s), "git: no *git-status* row here");
    m_x(&mut s, "git.unstage");
    assert_eq!(status(&s), "git: no *git-status* row here");
    m_x(&mut s, "git.discard");
    assert_eq!(status(&s), "git: no *git-status* row here");
    assert!(!minibuffer_active(&s));
    m_x(&mut s, "git.commit");
    assert_eq!(status(&s), "git: no *git-status* here");
    assert!(!minibuffer_active(&s));

    // Disabled, from the panel.
    exec(
        &s,
        "for _, id in ipairs(pmacs.buffer.list()) do\n\
           if pmacs.describe.buffer(id).name == '*git-status*' then\n\
             pmacs.window.display(id, { side = 'bottom', select = true })\n\
           end\n\
         end\n\
         pmacs.editor.move_to_line(1)",
    );
    seat_on(&mut s, "unstaged.txt");
    exec(&s, "pmacs.config.set('git.enabled', false)");
    press(&mut s, KeyCode::Char('s'));
    assert_eq!(status(&s), "git: disabled by the `git.enabled` setting");
    press(&mut s, KeyCode::Char('c'));
    assert_eq!(status(&s), "git: disabled by the `git.enabled` setting");
    assert!(!minibuffer_active(&s));
    exec(&s, "pmacs.config.set('git.enabled', true)");

    pump_for(&mut s, 100);
    let ring_after: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");
    assert_eq!(ring_after, ring_before, "no refusal spawned a child");
    assert_eq!(xy(&root, "unstaged.txt"), ".M");
    assert_eq!(xy(&root, "staged.txt"), "M.");
}

#[path = "common/iso.rs"]
mod iso;
