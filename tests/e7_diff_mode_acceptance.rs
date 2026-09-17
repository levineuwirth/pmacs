// tests/e7_diff_mode_acceptance.rs --- E7.2: the `diff` major mode's
// hunk motion on `*git-diff*`.

//! `*git-diff*` carries the `diff` major mode, whose `n` and `p` move
//! between hunks (`@@` lines). Probed the way a user meets it: across
//! hunk boundaries, at the first and the last hunk, in a diff with one
//! hunk, and in an empty diff --- each through the real panel, the
//! real `d`, a real `git diff`, and the keys.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::cell::{Cell, CellGrid};
use pmacs::protocol::{CellSize, FrontendId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Harness (the shape of tests/git_status_stage1_acceptance.rs)
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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

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

fn diff_text(s: &EditorState) -> String {
    named_text(s, "*git-diff*")
}

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(24, 80));
    s
}

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

fn init_repo(root: &Path) {
    git(root, &["init", "-q", "-b", "main", "."]);
    git(root, &["config", "user.email", "gate@example.invalid"]);
    git(root, &["config", "user.name", "Gate"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn write(root: &Path, rel: &str, body: &str) {
    std::fs::write(root.join(rel), body).expect("write fixture file");
}

/// Sixty numbered lines.
fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// A repository with one committed sixty-line file and a `Cargo.toml`.
fn repo_with_long_file(root: &Path) {
    init_repo(root);
    write(root, "Cargo.toml", "[package]\nname = \"fixture\"\n");
    write(root, "long.txt", &numbered(60));
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);
}

fn tempdir() -> (tempfile::TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let canonical = td.path().canonicalize().expect("canonicalize tempdir");
    (td, canonical)
}

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

fn cursor_line(s: &EditorState) -> usize {
    let n: i64 = eval(s, "return pmacs.editor.cursor_line()");
    usize::try_from(n).expect("cursor line fits")
}

fn row_line(s: &EditorState, needle: &str) -> Option<usize> {
    panel_text(s)
        .lines()
        .enumerate()
        .find(|(i, line)| *i > 0 && line.contains(needle))
        .map(|(i, _)| i)
}

fn seat_on(s: &mut EditorState, needle: &str) {
    let target = row_line(s, needle)
        .unwrap_or_else(|| panic!("no row matching {needle:?} in:\n{}", panel_text(s)));
    let current = cursor_line(s);
    assert!(current <= target, "fixture: walk down to {needle:?}");
    for _ in current..target {
        press(s, KeyCode::Char('n'));
    }
    assert_eq!(cursor_line(s), target);
}

/// Press `d` on the seated row, wait for `*git-diff*` to name the
/// path, and assert the diff buffer is the active one with the mode.
fn open_diff(s: &mut EditorState, path_fragment: &str) -> String {
    let before = diff_text(s);
    press(s, KeyCode::Char('d'));
    assert!(
        pump_until(s, 15_000, |s| {
            let now = diff_text(s);
            now != before && now.contains(path_fragment)
        }),
        "d must render a diff naming {path_fragment:?}; buffer was:\n{}\nstatus: {:?}",
        diff_text(s),
        status(s)
    );
    assert_eq!(active_name(s), "*git-diff*", "the diff is the active buffer");
    assert_eq!(
        eval::<Option<String>>(s, "return pmacs.buffer.major_mode(pmacs.window.buffer())")
            .as_deref(),
        Some("diff"),
        "*git-diff* carries the diff major mode"
    );
    diff_text(s)
}

/// The 0-based lines of `text` that begin with `@@`.
fn hunk_lines(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("@@"))
        .map(|(i, _)| i)
        .collect()
}

/// One real TUI paint at 24 x 80: what brings the focused window's
/// cursor into view.
fn paint(s: &EditorState) {
    let (rows, cols) = (24u32, 80u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
}

fn current_line_text(s: &EditorState) -> String {
    diff_text(s)
        .lines()
        .nth(cursor_line(s))
        .unwrap_or_default()
        .to_owned()
}

// ---------------------------------------------------------------------------
// The pure scan
// ---------------------------------------------------------------------------

/// `hunk_lines` finds every `@@` line, including one on the last line
/// without a trailing newline, and nothing in text without one.
#[test]
fn e7_2_hunk_lines_scans_the_boundary_cases() {
    let s = editor();
    let three: Vec<i64> = eval(
        &s,
        "return pmacs.diffmode.hunk_lines('diff --git a b\\n@@ -1 +1 @@\\n-a\\n+b\\n@@ -5 +5 @@\\n x\\n@@ -9 +9 @@')",
    );
    assert_eq!(three, vec![1, 4, 6]);
    let none: Vec<i64> = eval(&s, "return pmacs.diffmode.hunk_lines('(no differences)')");
    assert!(none.is_empty());
    let empty: Vec<i64> = eval(&s, "return pmacs.diffmode.hunk_lines('')");
    assert!(empty.is_empty());
    let not_at_start: Vec<i64> = eval(&s, "return pmacs.diffmode.hunk_lines(' @@ not a hunk\\n+@@ nor this')");
    assert!(not_at_start.is_empty(), "only a line starting with @@ is a hunk");
}

// ---------------------------------------------------------------------------
// Motion
// ---------------------------------------------------------------------------

/// Three edits far apart make three hunks. From the header `n` lands
/// on the first, then the second, then the third; at the third `n`
/// stays and says so. `p` walks back: second, first; at the first `p`
/// stays and says so. Every landing is an `@@` line and the cursor is
/// at its start.
#[test]
fn e7_2_n_and_p_walk_the_hunks_and_stop_at_the_ends() {
    let (_td, root) = tempdir();
    repo_with_long_file(&root);
    let mut body = numbered(60);
    body = body.replace("line 5\n", "line 5 edited\n");
    body = body.replace("line 30\n", "line 30 edited\n");
    body = body.replace("line 55\n", "line 55 edited\n");
    write(&root, "long.txt", &body);

    let mut s = editor();
    open_panel(&mut s, &root, "Cargo.toml");
    seat_on(&mut s, "long.txt");
    let text = open_diff(&mut s, "long.txt");
    let hunks = hunk_lines(&text);
    assert_eq!(hunks.len(), 3, "positive control: three hunks in\n{text}");
    assert_eq!(cursor_line(&s), 0, "the diff opens at its header");

    for (i, expected) in hunks.iter().enumerate() {
        press(&mut s, KeyCode::Char('n'));
        assert_eq!(
            cursor_line(&s),
            *expected,
            "n number {} must land on hunk {} at line {expected}; status {:?}",
            i + 1,
            i + 1,
            status(&s)
        );
        assert!(
            current_line_text(&s).starts_with("@@"),
            "the landing line is a hunk header: {:?}",
            current_line_text(&s)
        );
        let col: i64 = eval(&s, "return pmacs.editor.cursor_col()");
        assert_eq!(col, 0, "at the start of the hunk line");
    }
    // At the last hunk.
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), hunks[2], "n at the last hunk stays");
    assert_eq!(status(&s), "diff: no next hunk");

    // Back up.
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[1]);
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[0]);
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[0], "p at the first hunk stays");
    assert_eq!(status(&s), "diff: no previous hunk");

    // From inside a hunk's body, n crosses to the next boundary and p
    // returns to the enclosing hunk's header.
    exec(&s, &format!("pmacs.editor.move_to_line({})", hunks[1] + 2));
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), hunks[2], "n from a body crosses the boundary");
    exec(&s, &format!("pmacs.editor.move_to_line({})", hunks[1] + 2));
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[1], "p from a body goes to the enclosing header");

    // From the header, p has nothing above it.
    exec(&s, "pmacs.editor.move_to_line(0)");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), 0);
    assert_eq!(status(&s), "diff: no previous hunk");
}

/// A long diff: after a jump to a hunk far below the first screen, the
/// next frame shows it. The diff has to be long in its own lines, not
/// the file's --- `git diff` shows three lines of context around a
/// change, so two small edits four hundred lines apart make a
/// fifteen-line diff. Here every line of a long block is edited, so the
/// second hunk runs for hundreds of diff lines and the third begins far
/// past the first screen. Measured on the way: motion alone leaves
/// `view_top` where it was, and the paint is what brings the cursor
/// into view, which is why the mode sets the line and does not walk.
#[test]
fn e7_2_n_drags_the_viewport_to_a_far_hunk() {
    let (_td, root) = tempdir();
    init_repo(&root);
    write(&root, "Cargo.toml", "[package]\nname = \"fixture\"\n");
    write(&root, "long.txt", &numbered(400));
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    let mut body = numbered(400);
    body = body.replace("line 3\n", "line 3 edited\n");
    for i in 30..=380 {
        body = body.replace(&format!("line {i}\n"), &format!("line {i} edited\n"));
    }
    body = body.replace("line 398\n", "line 398 edited\n");
    write(&root, "long.txt", &body);

    let mut s = editor();
    open_panel(&mut s, &root, "Cargo.toml");
    seat_on(&mut s, "long.txt");
    let text = open_diff(&mut s, "long.txt");
    let hunks = hunk_lines(&text);
    assert_eq!(hunks.len(), 3, "positive control: three hunks in a {}-line diff", text.lines().count());
    assert!(hunks[2] > 24 * 4, "positive control: the third hunk is far past the first screen ({})", hunks[2]);
    paint(&s);
    assert_eq!(eval::<i64>(&s, "return pmacs.editor.view_top()"), 0);
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), hunks[2]);
    // The measurement: motion alone moved nothing.
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.view_top()"),
        0,
        "motion does not scroll; the frame does"
    );
    paint(&s);
    let top: i64 = eval(&s, "return pmacs.editor.view_top()");
    let top = usize::try_from(top).expect("fits");
    assert!(
        top > 0 && top <= hunks[2] && hunks[2] < top + 24,
        "after the paint the viewport (top {top}, at most 24 rows) shows the hunk at {}",
        hunks[2]
    );
    // And back: p from the far hunk, then a frame, shows the second.
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[1]);
    paint(&s);
    let top: i64 = eval(&s, "return pmacs.editor.view_top()");
    let top = usize::try_from(top).expect("fits");
    assert!(
        top <= hunks[1] && hunks[1] < top + 24,
        "after the paint the viewport (top {top}) shows the hunk at {}",
        hunks[1]
    );
}

/// One hunk: `n` from the header lands on it, a second `n` stays and
/// says so, `p` from it says so too.
#[test]
fn e7_2_a_single_hunk_is_reachable_and_is_both_ends() {
    let (_td, root) = tempdir();
    repo_with_long_file(&root);
    write(&root, "long.txt", &numbered(60).replace("line 20\n", "line 20 edited\n"));
    let mut s = editor();
    open_panel(&mut s, &root, "Cargo.toml");
    seat_on(&mut s, "long.txt");
    let text = open_diff(&mut s, "long.txt");
    let hunks = hunk_lines(&text);
    assert_eq!(hunks.len(), 1, "positive control: one hunk");
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), hunks[0]);
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), hunks[0]);
    assert_eq!(status(&s), "diff: no next hunk");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[0]);
    assert_eq!(status(&s), "diff: no previous hunk");
}

/// An empty diff --- a staged file whose worktree edit was undone is
/// still a row, and its total change against HEAD is nothing --- has no
/// hunk; `n` and `p` both say so and move nowhere.
#[test]
fn e7_2_an_empty_diff_has_no_hunk_to_move_to() {
    let (_td, root) = tempdir();
    repo_with_long_file(&root);
    // Stage an edit, then put the worktree back: `M.` in the index,
    // `.M` in the worktree, nothing against HEAD.
    write(&root, "long.txt", &numbered(60).replace("line 20\n", "line 20 edited\n"));
    git(&root, &["add", "long.txt"]);
    write(&root, "long.txt", &numbered(60));
    let mut s = editor();
    open_panel(&mut s, &root, "Cargo.toml");
    seat_on(&mut s, "long.txt");
    let text = open_diff(&mut s, "long.txt");
    assert!(
        text.contains("(no differences)"),
        "positive control: the diff is empty:\n{text}"
    );
    assert!(hunk_lines(&text).is_empty());
    let before = cursor_line(&s);
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), before, "n moves nowhere in an empty diff");
    assert_eq!(status(&s), "diff: no hunks");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), before);
    assert_eq!(status(&s), "diff: no hunks");
}

/// The keys are the mode's, not the buffer's: `M-x diff.next-hunk` in
/// a buffer without the mode refuses, and `n` there is still a letter.
#[test]
fn e7_2_the_motion_refuses_outside_the_mode() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = td.path().join("plain.txt");
    std::fs::write(&path, "@@ looks like a hunk\nbut this is a text file\n").unwrap();
    exec(
        &s,
        &format!("pmacs.buffer.find_or_open({:?})", path.display().to_string()),
    );
    exec(&s, "pmacs.command.invoke('diff.next-hunk')");
    assert_eq!(status(&s), "diff: not a diff buffer");
    assert_eq!(cursor_line(&s), 0);
    press(&mut s, KeyCode::Char('n'));
    let text: mlua::String = eval(&s, "local b = pmacs.window.buffer(); return b:slice(0, b:len())");
    assert!(
        String::from_utf8_lossy(&text.as_bytes()).starts_with("n@@"),
        "without the mode, n is self-insert"
    );
}

#[path = "common/iso.rs"]
mod iso;
