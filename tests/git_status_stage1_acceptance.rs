//! Git integration Stage 1 acceptance —
//! `docs/git-integration-framing.md` §6, one test per verification
//! bullet.
//!
//! Dispatch-driven wherever a key is claimed: `RET`, `d`, `g`, `n`/`p`
//! and `q` are exercised through `dispatch_key`, never through
//! `pmacs.command.invoke`, so a dead binding fails these tests instead
//! of passing review.
//!
//! Repository fixtures are built with the REAL `git` in a tempdir, and
//! every one of them **bounds project detection** with
//! `pmacs.project.set_search_boundary`. That is not tidiness: R8 was a
//! fixture whose marker walk escaped into the developer's own
//! environment, retired two commits before this branch's base, and a
//! tempdir under `/tmp` is exactly the shape that reaches it.
//!
//! The pure `parse_*` half runs with **no repository at all** — the
//! separation `tests/fixtures/pmacs-magit/status.lua` already proves
//! works, and the reason this lane ported that separation rather than
//! inventing one (Q#G-0).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::{CellSize, FrontendId};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Harness
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

fn errors_text(s: &EditorState) -> String {
    s.lua_host.errors_buffer_text()
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
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

fn diff_text(s: &EditorState) -> String {
    named_text(s, "*git-diff*")
}

/// Fresh editor with LSP spawning disabled and frame geometry declared.
///
/// Geometry is not optional here: a listview opens into the PANEL by
/// default since bottom-panel Stage 3, and a panel is derived-hidden
/// while the frontend's frame size is unknown — so focus would fall
/// back to the document window and every assertion below would read the
/// wrong buffer.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(24, 80));
    s
}

/// Drive frames until `pred` holds, pumping the process supervisor —
/// the production `process.after-tick` path this module's pump rides.
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

/// `git init` plus the identity every commit needs, with the ambient
/// user's own config kept out of it.
fn init_repo(root: &Path) {
    git(root, &["init", "-q", "-b", "main", "."]);
    git(root, &["config", "user.email", "gate@example.invalid"]);
    git(root, &["config", "user.name", "Gate"]);
    // A developer's `commit.gpgsign = true` would make every fixture
    // commit prompt or fail; a repo-local `false` outranks it.
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir -p");
    }
    std::fs::write(&p, body).expect("write fixture file");
}

/// A repository with **one row of every class `d` must answer**:
/// staged, unstaged, both, deleted, renamed, and untracked. Plus a
/// `Cargo.toml`, so `pmacs.project.detect` reports `rust` and NOT
/// `git` — the case a `kind == "git"` gate would have failed, and the
/// reason this module never asks pmacs whether something is a repo.
fn mixed_repo(root: &Path) {
    init_repo(root);
    write(root, "Cargo.toml", "[package]\nname = \"fixture\"\n");
    // Two tracked files that stay CLEAN, so they are absent from status
    // until a test dirties them. `g6_11` needs to insert **two** rows
    // above its selection: inserting one is satisfied by the accidental
    // off-by-one that a missing re-seat produces, which is how that test
    // was vacuous in its first form.
    write(root, "a1.txt", "a1 base\n");
    write(root, "a2.txt", "a2 base\n");
    write(root, "staged.txt", "staged base\n");
    write(root, "unstaged.txt", "unstaged base\n");
    write(root, "both.txt", "both base\n");
    write(root, "deleted.txt", "deleted base\n");
    // Long enough that rename detection scores it at 100%.
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

/// The unborn fixture, enumerated from a real unborn repository rather
/// than reasoned about: `git init`, stage three files, then edit one
/// (`AM`), delete one (`AD`), and `git mv` one — which produces an
/// ordinary `1 A.` add of the new path, never a `2` record.
fn unborn_repo(root: &Path) {
    init_repo(root);
    write(root, "am.txt", "am base\n");
    write(root, "ad.txt", "ad base\n");
    write(root, "r_orig.txt", "r base\n");
    git(root, &["add", "-A"]);
    write(root, "am.txt", "am base\nworktree edit\n");
    std::fs::remove_file(root.join("ad.txt")).expect("rm ad.txt");
    git(root, &["mv", "r_orig.txt", "r_new.txt"]);
    write(root, "untracked.txt", "untracked body\n");
}

/// Bound project detection to the fixture, open a file inside it so the
/// active buffer has a path there, and run `M-x git.status` to
/// completion.
///
/// `set_search_boundary` is the R8 discipline: without it, detection
/// walks out of the tempdir and picks up whatever markers the developer
/// happens to have above `/tmp`.
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

/// The 1-based data line of the first panel row whose text contains
/// `needle`, or `None`.
fn row_line(s: &EditorState, needle: &str) -> Option<usize> {
    panel_text(s)
        .lines()
        .enumerate()
        .find(|(i, line)| *i > 0 && line.contains(needle))
        .map(|(i, _)| i)
}

/// Seat the cursor on the row containing `needle` by pressing `n`,
/// which is listview's own binding — so a broken panel keymap fails
/// here rather than being stepped around.
fn seat_on(s: &mut EditorState, needle: &str) {
    let target = row_line(s, needle)
        .unwrap_or_else(|| panic!("no row matching {needle:?} in:\n{}", panel_text(s)));
    let current: i64 = eval(s, "return pmacs.editor.cursor_line()");
    let current = usize::try_from(current).expect("cursor line fits");
    assert!(
        current <= target,
        "fixture: expected to walk down to {needle:?} (line {target}, cursor {current})"
    );
    for _ in current..target {
        press(s, KeyCode::Char('n'));
    }
    let now: i64 = eval(s, "return pmacs.editor.cursor_line()");
    assert_eq!(
        usize::try_from(now).expect("cursor line fits"),
        target,
        "cursor must land on the {needle:?} row"
    );
}

/// Pump frames for `ms` without waiting on anything — for the
/// assertions whose subject is that NOTHING happens.
fn pump_for(s: &mut EditorState, ms: u64) {
    pump_until(s, ms, |_| false);
}

/// Press `d` and pump until `*git-diff*` names `path_fragment`.
fn press_d_and_wait(s: &mut EditorState, path_fragment: &str) -> String {
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
    diff_text(s)
}

/// The all-zero object id porcelain v2 prints for an absent side of a
/// change. Module scope because clippy refuses an item after statements.
const H: &str = "0000000000000000000000000000000000000000";

/// A Lua EXPRESSION producing a `-z` status payload from `fields`.
///
/// Deliberately assembled in Lua with `string.char(0)` rather than as a
/// Rust string interpolated with `{:?}`. Rust's `Debug` renders a NUL
/// as `\0`, and Lua's decimal escape then swallows the digits that
/// follow — so a `\0` immediately before a `1` record becomes
/// `string.char(1)` and that record silently merges into its
/// predecessor. The first draft of this suite did exactly that, and one
/// of its tests PASSED while parsing nothing at all: the merged text
/// landed in the `# branch.head` value, which the panel header then
/// rendered, and a `contains` assertion on the panel was satisfied by
/// the header.
fn z_payload(fields: &[&str]) -> String {
    let quoted: Vec<String> = fields.iter().map(|f| format!("{f:?}")).collect();
    format!(
        "(table.concat({{ {} }}, string.char(0)) .. string.char(0))",
        quoted.join(", ")
    )
}

fn tempdir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    // Canonicalize: `git rev-parse --show-toplevel` reports a physical
    // path, and `/tmp` is a symlink on some machines, so an
    // uncanonicalized fixture root would not compare equal to the root
    // the module resolves.
    let root = std::fs::canonicalize(dir.path()).expect("canonicalize tempdir");
    (dir, root)
}

// ---------------------------------------------------------------------------
// §6 — parsing, against a corpus rather than a case (Q#G-6)
// ---------------------------------------------------------------------------

/// The pure parser, with **no repository**, over every row class the
/// framing's witness corpus names: modified, added, deleted, untracked,
/// renamed with BOTH paths, copied, a path with a space, and a path
/// with a **newline**.
///
/// The newline case is what `-z` buys. A parser that passes only the
/// space case is the one that ships broken: without `-z` git C-quotes
/// such a path, and a hand-written unquoter is what this design exists
/// to avoid.
#[test]
fn g6_1_the_parser_covers_the_whole_v2_row_corpus() {
    let s = editor();
    // Field layouts taken from real `git status --porcelain=v2 -z`
    // output, not from the documentation:
    //   1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
    //   2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>\0<orig>
    let fields = [
        "# branch.oid 16fa4d708a09af0c96212f66395c3e204049534a".to_string(),
        "# branch.head main".to_string(),
        format!("1 .M N... 100644 100644 100644 {H} {H} modified.txt"),
        format!("1 A. N... 000000 100644 100644 {H} {H} added.txt"),
        format!("1 .D N... 100644 100644 000000 {H} {H} deleted.txt"),
        format!("2 R. N... 100644 100644 100644 {H} {H} R100 renamed_to.txt"),
        "renamed_from.txt".to_string(),
        format!("2 C. N... 100644 100644 100644 {H} {H} C085 copy_dst.txt"),
        "copy_src.txt".to_string(),
        format!("1 .M N... 100644 100644 100644 {H} {H} has space.txt"),
        format!("1 .M N... 100644 100644 100644 {H} {H} new\nline.txt"),
        "? untracked.txt".to_string(),
    ];
    let refs: Vec<&str> = fields.iter().map(String::as_str).collect();

    exec(
        &s,
        &format!("_G.PARSED = pmacs.git.parse_status({})", z_payload(&refs)),
    );

    // Rows are joined with RS (0x1E), not a newline: one of the paths
    // in this corpus CONTAINS a newline, and splitting on `\n` would
    // tear that very row in half — turning the case `-z` exists for
    // into a test artefact.
    let encoded: String = eval(
        &s,
        "local out = {}\n\
         for _, r in ipairs(_G.PARSED.rows) do\n\
           out[#out + 1] = table.concat({ r.kind, r.xy, r.path, r.orig or '-' }, '|')\n\
         end\n\
         return table.concat(out, string.char(30))",
    );
    let rows: Vec<&str> = encoded.split('\u{1e}').collect();

    assert_eq!(
        rows,
        vec![
            "ordinary|.M|modified.txt|-",
            "ordinary|A.|added.txt|-",
            "ordinary|.D|deleted.txt|-",
            "rename|R.|renamed_to.txt|renamed_from.txt",
            "rename|C.|copy_dst.txt|copy_src.txt",
            "ordinary|.M|has space.txt|-",
            "ordinary|.M|new\nline.txt|-",
            "untracked|??|untracked.txt|-",
        ],
        "every corpus row must parse, and a rename/copy must carry BOTH \
         paths — the origin is the NEXT NUL-terminated field under -z, \
         not a tab-joined suffix"
    );

    let head: String = eval(&s, "return _G.PARSED.branch.head");
    assert_eq!(head, "main", "the --branch header is parsed too");
}

/// A path with a newline must not be able to split a panel row.
///
/// This rides alongside the parse assertion above rather than instead
/// of it: parsing the byte correctly and then writing it raw into a
/// one-row-per-line buffer desynchronizes every line-to-row mapping in
/// the panel, which no parser test can see.
#[test]
fn g6_1b_a_newline_in_a_path_is_escaped_for_display() {
    let s = editor();
    let shown: String = eval(&s, "return pmacs.git.display_path('new\\nline.txt')");
    assert_eq!(shown, "new\\x0Aline.txt");
    assert!(
        !shown.contains('\n'),
        "a rendered row must occupy exactly one line"
    );
}

/// Unborn detection reads `# branch.oid (initial)` from the status
/// output already being parsed. Pinned so nobody later reintroduces a
/// second `rev-parse` process for a fact the first one hands over.
#[test]
fn g6_7_unborn_is_read_from_the_branch_oid_header() {
    let s = editor();
    let unborn_payload = z_payload(&["# branch.oid (initial)", "# branch.head main"]);
    let born_payload = z_payload(&["# branch.oid deadbeef", "# branch.head main"]);
    let (unborn, born): (bool, bool) = eval(
        &s,
        &format!(
            "return pmacs.git.parse_status({unborn_payload}).branch.unborn,\n\
                    pmacs.git.parse_status({born_payload}).branch.unborn"
        ),
    );
    assert!(unborn, "`(initial)` is the unborn marker");
    assert!(!born, "a real oid is not");
}

// ---------------------------------------------------------------------------
// §6 — the non-UTF-8 boundary (Q#G-8)
// ---------------------------------------------------------------------------

/// A non-UTF-8 path is **parsed and displayed**, and its gestures
/// **refuse with a message**.
///
/// Not an end-to-end visit, and the framing does not claim one:
/// `pmacs.process.spawn` takes `args: Vec<String>` and
/// `pmacs.buffer.find_or_open` takes `path: String`, both UTF-8 by
/// construction, and the rope is UTF-8 by project invariant. So the
/// witness is parse-and-display **plus the refusal** — a witnessed
/// refusal, not a stack trace and not a silent no-op.
#[test]
fn g6_2_a_non_utf8_path_parses_displays_and_refuses_its_gestures() {
    use std::os::unix::ffi::OsStrExt;

    let (_dir, root) = tempdir();
    init_repo(&root);
    write(&root, "ok.txt", "ok base\n");
    let bad = root.join(std::ffi::OsStr::from_bytes(b"bad\xff.txt"));
    std::fs::write(&bad, b"bad base\n").expect("write non-utf8 path");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    write(&root, "ok.txt", "ok base\nedit\n");
    std::fs::write(&bad, b"bad base\nedit\n").expect("edit non-utf8 path");

    let mut s = editor();
    open_panel(&mut s, &root, "ok.txt");

    // Displayed: the row is there, escaped, so the user is not lied to
    // about what is modified.
    let text = panel_text(&s);
    assert!(
        text.contains("bad\\xFF.txt"),
        "the unrepresentable path must still appear as a row: {text}"
    );

    // RET refuses.
    seat_on(&mut s, "bad\\xFF.txt");
    let before = active_name(&s);
    press(&mut s, KeyCode::Enter);
    assert!(
        status(&s).contains("not valid UTF-8"),
        "RET must report the refusal; status was {:?}",
        status(&s)
    );
    assert_eq!(
        active_name(&s),
        before,
        "and must not have navigated anywhere"
    );

    // `d` refuses too, and renders no diff.
    exec(&s, "pmacs.editor.set_status('')");
    press(&mut s, KeyCode::Char('d'));
    assert!(
        status(&s).contains("not valid UTF-8"),
        "d must report the refusal; status was {:?}",
        status(&s)
    );
    pump_for(&mut s, 300);
    assert!(
        diff_text(&s).is_empty(),
        "no *git-diff* buffer may appear for an unrepresentable path: {:?}",
        diff_text(&s)
    );

    // The positive control: an ordinary neighbour still works, so the
    // refusal above is about the path and not about the panel.
    seat_on(&mut s, "ok.txt");
    let diff = press_d_and_wait(&mut s, "ok.txt");
    assert!(diff.contains("diff --git"), "control diff: {diff}");
}

// ---------------------------------------------------------------------------
// §6 — the invocation (Q#G-6)
// ---------------------------------------------------------------------------

/// `--no-optional-locks` is asserted **structurally**, on the argv the
/// module really spawned. A lock that was not taken cannot be observed
/// directly, so the invocation is what gets pinned.
///
/// The flag is part of the contract, not a nicety: `git status` may
/// refresh and write the index, and this module runs it asynchronously
/// from an editor while the user may be running git in a terminal.
#[test]
fn g6_3_every_invocation_carries_no_optional_locks() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    let log: Vec<String> = eval(
        &s,
        "local out = {}\n\
         for _, args in ipairs(pmacs.git._spawn_log) do\n\
           out[#out + 1] = table.concat(args, ' ')\n\
         end\n\
         return out",
    );
    assert!(!log.is_empty(), "the open must have spawned git");
    for argv in &log {
        assert!(
            argv.starts_with("--no-optional-locks"),
            "every git argv must lead with --no-optional-locks; got {argv:?}"
        );
    }
    assert!(
        log.iter()
            .any(|a| a.contains("status --porcelain=v2 --branch -z")),
        "the status invocation is the pinned one: {log:?}"
    );

    // And the diff path too, which is a separate call site.
    seat_on(&mut s, "staged.txt");
    press_d_and_wait(&mut s, "staged.txt");
    let last: Vec<String> = eval(&s, "return pmacs.git._last_spawn.args");
    assert_eq!(
        last.first().map(String::as_str),
        Some("--no-optional-locks"),
        "the diff argv carries it as well: {last:?}"
    );
}

// ---------------------------------------------------------------------------
// §6 — `d` on every row class (Q#G-7)
// ---------------------------------------------------------------------------

/// `d` is witnessed on staged, unstaged, both, deleted, renamed **and
/// untracked** rows.
///
/// Untracked is the load-bearing one: a normal `git diff` shows nothing
/// at all for an untracked file, so a missing `--no-index` case makes
/// `d` silently dead exactly where a user is most likely to press it.
#[test]
fn g6_4_d_answers_every_row_class() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    // Each row class, with a fragment only the RIGHT diff can contain.
    // `both.txt` is asserted on BOTH halves: `git diff HEAD` is one view
    // of the total change, so a `--cached`-only or plain-only answer
    // would lose one of them and still look plausible.
    for (needle, expect) in [
        ("staged.txt", &["+staged edit"][..]),
        ("unstaged.txt", &["+worktree edit"][..]),
        ("both.txt", &["+staged edit", "+worktree edit"][..]),
        ("deleted.txt", &["-deleted base"][..]),
        ("renamed_to.txt", &["rename from renamed_from.txt"][..]),
        ("untracked.txt", &["+untracked body"][..]),
    ] {
        // `d` displays the diff in the DOCUMENT window, so focus has to
        // come back to the panel before the next gesture.
        refocus_panel(&mut s);
        seat_on(&mut s, needle);
        let diff = press_d_and_wait(&mut s, needle);
        for fragment in expect {
            assert!(
                diff.contains(fragment),
                "d on the {needle:?} row must render {fragment:?}; got:\n{diff}"
            );
        }
        assert!(
            !diff.contains("exited with code"),
            "…and not a failure body: {diff}"
        );
    }
}

/// Focus the `*git-status*` panel and seat the cursor on its first data
/// row, whatever `d` last displayed.
fn refocus_panel(s: &mut EditorState) {
    exec(
        s,
        "for _, id in ipairs(pmacs.buffer.list()) do\n\
           if pmacs.describe.buffer(id).name == '*git-status*' then\n\
             pmacs.window.display(id, { side = 'bottom', select = true })\n\
           end\n\
         end\n\
         pmacs.editor.move_to_line(1)",
    );
    assert_eq!(
        active_name(s),
        "*git-status*",
        "the panel must be focused again before the next gesture"
    );
}

/// The untracked diff renders **on exit 1**, not a failure row.
///
/// `git diff --no-index` implies `--exit-code`: it exits 1 when it
/// SUCCESSFULLY finds differences. Under a plain "non-zero is failure"
/// predicate every untracked diff — the case `--no-index` exists to
/// serve — would render a failure instead of the diff it just produced.
///
/// Measured rather than assumed: the same invocation run by hand
/// against a scratch repository exits 1 and prints the patch.
#[test]
fn g6_5_the_untracked_diff_renders_on_exit_one() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");
    seat_on(&mut s, "untracked.txt");
    let diff = press_d_and_wait(&mut s, "untracked.txt");

    // The invocation really was the `--no-index` one…
    let last: Vec<String> = eval(&s, "return pmacs.git._last_spawn.args");
    assert!(
        last.iter().any(|a| a == "--no-index") && last.iter().any(|a| a == "/dev/null"),
        "the untracked row must diff against /dev/null: {last:?}"
    );
    // …and the same argv really does exit 1 here, so this test is about
    // the predicate rather than about a git that happens to exit 0.
    let out = std::process::Command::new("git")
        .current_dir(&root)
        .args([
            "--no-optional-locks",
            "diff",
            "--no-index",
            "--",
            "/dev/null",
            "untracked.txt",
        ])
        .output()
        .expect("run the same invocation by hand");
    assert_eq!(
        out.status.code(),
        Some(1),
        "fixture premise: --no-index exits 1 when it finds differences"
    );

    assert!(
        diff.contains("+untracked body"),
        "the patch must be rendered: {diff}"
    );
    assert!(
        !diff.contains("exited with code 1"),
        "exit 1 must NOT be read as a failure here: {diff}"
    );
}

// ---------------------------------------------------------------------------
// §6 — the unborn repository (Q#G-7b)
// ---------------------------------------------------------------------------

/// An unborn repository, end to end, with an **`AM`** fixture — staged
/// then edited again, the shape a first commit actually has partway
/// through.
///
/// `git init`, stage, edit, open the panel, press `d`, and get **two
/// labelled patches** with the split-view header — not
/// `fatal: bad revision 'HEAD'`, and not a single `--cached` patch that
/// silently drops the worktree edit.
#[test]
fn g6_6_an_unborn_am_row_renders_two_labelled_patches() {
    let (_dir, root) = tempdir();
    unborn_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "am.txt");

    let text = panel_text(&s);
    assert!(
        text.contains("no commits yet"),
        "the header names the unborn state: {text}"
    );
    assert!(text.contains("AM  am.txt"), "the AM row is present: {text}");

    seat_on(&mut s, "am.txt");
    let diff = press_d_and_wait(&mut s, "am.txt");

    assert!(
        !diff.contains("bad revision"),
        "`git diff HEAD` must never be attempted here: {diff}"
    );
    assert!(
        diff.contains(
            "no commits yet --- split view: staged (index) above, unstaged (worktree) below"
        ),
        "the header must describe a SPLIT view, not a total against HEAD: {diff}"
    );
    assert!(
        diff.contains("=== staged (index) ===") && diff.contains("=== unstaged (worktree) ==="),
        "two labelled patches: {diff}"
    );
    assert!(
        diff.contains("+am base"),
        "the staged half carries the index patch: {diff}"
    );
    assert!(
        diff.contains("+worktree edit"),
        "the unstaged half carries the worktree delta — the half a \
         --cached-only answer silently drops: {diff}"
    );
}

/// `AD` rides the same fixture, since one repository can hold both, and
/// its second patch renders the deletion.
#[test]
fn g6_6b_an_unborn_ad_row_renders_the_deletion_in_its_second_patch() {
    let (_dir, root) = tempdir();
    unborn_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "am.txt");
    seat_on(&mut s, "ad.txt");
    let diff = press_d_and_wait(&mut s, "ad.txt");

    assert!(
        diff.contains("=== staged (index) ===") && diff.contains("=== unstaged (worktree) ==="),
        "two labelled patches for AD too: {diff}"
    );
    assert!(
        diff.contains("deleted file mode"),
        "the unstaged half renders the deletion: {diff}"
    );
}

/// A single-state unborn row takes ONE patch, and says which one.
///
/// Without this the split above could be an unconditional two-command
/// answer that happens to look right, and the `A.` row would carry a
/// header describing a view it is not.
#[test]
fn g6_6c_a_single_state_unborn_row_takes_one_patch_and_says_so() {
    let (_dir, root) = tempdir();
    unborn_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "am.txt");
    seat_on(&mut s, "r_new.txt");
    let diff = press_d_and_wait(&mut s, "r_new.txt");

    assert!(
        diff.contains("no commits yet --- staged (index) only"),
        "the header must name the single state it shows: {diff}"
    );
    assert!(
        !diff.contains("=== unstaged (worktree) ==="),
        "and there is no second patch to label: {diff}"
    );
}

/// A two-step plan runs **every** step against the repository the user
/// was looking at when they pressed `d`.
///
/// An unborn `AM` row is the only shape that makes this observable: it
/// produces a **two-step** plan (staged patch, then unstaged patch), and
/// the second step used to be spawned from the first one's completion
/// callback against whatever `state.root` held **by then**. So a
/// `git.status` for another repository, whose root lookup lands while
/// the first patch is still in flight, moved the plan's second step into
/// a different repository — carrying the first repository's path, which
/// git there resolves to nothing at all.
///
/// Driven through `_deliver_root` for the same reason `g6_21` is: no
/// arrangement of real subprocess timing can guarantee that the
/// interleaving happens, and a test that merely hoped for it would pass
/// on the broken code most of the time. Nothing is pumped between the
/// keypress and the reassignment, so step 1 is genuinely in flight.
///
/// The argv assertion is the load-bearing one. A test that checked only
/// the FIRST step — or only that a diff rendered — passes on the broken
/// code, since step 1 is spawned synchronously from the keypress and
/// step 2 against the wrong repository merely produces an empty patch.
#[test]
fn g6_22_a_two_step_plan_keeps_the_root_it_started_with() {
    let (_dir_a, root_a) = tempdir();
    unborn_repo(&root_a);
    // An unrelated repository, with no `am.txt` in it: a step that
    // escaped into B would find nothing and render "(no changes)".
    let (_dir_b, root_b) = tempdir();
    mixed_repo(&root_b);

    let mut s = editor();
    open_panel(&mut s, &root_a, "am.txt");
    seat_on(&mut s, "am.txt");

    let a = root_a.display().to_string();
    let b = root_b.display().to_string();

    // `d` spawns step 1 synchronously against A…
    press(&mut s, KeyCode::Char('d'));
    // …and now, before a single frame is pumped, a `git.status` for B
    // resolves its root and reassigns `state.root`.
    exec(
        &s,
        &format!(
            "pmacs.git._deliver_root(\n\
               {{ generation = pmacs.git._generation(), dir = {b:?} }},\n\
               {{ ok = true, code = 0, stdout = {b:?}, stderr = '' }})"
        ),
    );
    assert!(
        pump_until(&mut s, 15_000, |s| diff_text(s)
            .contains("=== unstaged (worktree) ===")),
        "the plan must run to completion; diff was:\n{}\nstatus: {:?}",
        diff_text(&s),
        status(&s)
    );

    let diffs: Vec<String> = eval(
        &s,
        "local out = {}\n\
         for _, args in ipairs(pmacs.git._spawn_log) do\n\
           for _, a in ipairs(args) do\n\
             if a == 'diff' then\n\
               out[#out + 1] = table.concat(args, ' ')\n\
               break\n\
             end\n\
           end\n\
         end\n\
         return out",
    );
    assert_eq!(
        diffs.len(),
        2,
        "premise: the AM row's plan really is two steps: {diffs:?}"
    );
    for argv in &diffs {
        assert!(
            argv.contains(&format!("-C {a}")),
            "every step of one plan runs against the captured root: {argv:?}"
        );
        assert!(
            !argv.contains(&b),
            "and none of them may follow `state.root` into another \
             repository: {argv:?}"
        );
    }

    // The user-visible half: the second patch is still A's worktree
    // delta, not the empty answer B would have given.
    let diff = diff_text(&s);
    assert!(
        diff.contains("+worktree edit"),
        "the unstaged half must still carry A's worktree delta: {diff}"
    );
}

/// Rename/copy under an unborn `HEAD` is **unreachable**.
///
/// The fixture `git mv`s a staged-but-uncommitted file and the parser
/// sees a `1 A.` record, never a `2`. Pinned so a future reader does not
/// "fix" the missing unborn rename policy by inventing one.
#[test]
fn g6_8_rename_under_an_unborn_head_is_unreachable() {
    let (_dir, root) = tempdir();
    unborn_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "am.txt");

    let text = panel_text(&s);
    assert!(
        text.contains("A.  r_new.txt"),
        "the `git mv` produces an ordinary ADD of the new path: {text}"
    );
    assert!(
        !text.contains("<-"),
        "and no rename row exists at all — with no HEAD there is nothing \
         to rename FROM: {text}"
    );
    // Asserted against the REAL status output too, so a display-only
    // change cannot make this pass.
    let raw = git(
        &root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
        ],
    );
    exec(&s, &format!("_G.RAW = {raw:?}"));
    let has_rename: bool = eval(
        &s,
        "for _, r in ipairs(pmacs.git.parse_status(_G.RAW).rows) do\n\
           if r.kind == 'rename' then return true end\n\
         end\n\
         return false",
    );
    assert!(
        !has_rename,
        "the real unborn status output contains no `2` record"
    );
}

// ---------------------------------------------------------------------------
// §6 — the `keys` lifecycle (Q#G-7)
// ---------------------------------------------------------------------------

/// Two successive refreshes on a live panel: `d` still works and no
/// `DuplicateBinding` surfaces.
///
/// This is the one that would have broken on **every** refresh.
/// `Keymap::bind` refuses duplicates and the completion model calls
/// `listview.open` again each time, so a naive `keys` implementation
/// errors on the second open.
#[test]
fn g6_9_two_refreshes_keep_d_working_and_raise_nothing() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    for round in 1..=2 {
        let before: i64 = eval(&s, "return pmacs.git._generation()");
        press(&mut s, KeyCode::Char('g'));
        assert!(
            pump_until(&mut s, 15_000, |s| !panel_text(s).contains("refreshing")),
            "refresh {round} must complete; panel:\n{}",
            panel_text(&s)
        );
        let after: i64 = eval(&s, "return pmacs.git._generation()");
        assert_eq!(after, before + 1, "refresh {round} really ran");
        assert!(
            !status(&s).contains("already bound"),
            "refresh {round} status: {:?}",
            status(&s)
        );
    }

    let errs = errors_text(&s);
    assert!(
        !errs.contains("already bound"),
        "no DuplicateBinding may reach the errors buffer:\n{errs}"
    );

    // And `d` is still a live binding, not merely un-erroring.
    seat_on(&mut s, "staged.txt");
    let diff = press_d_and_wait(&mut s, "staged.txt");
    assert!(
        diff.contains("+staged edit"),
        "d after two refreshes: {diff}"
    );
}

/// A `keys` table colliding with the fixed set is rejected **at install
/// time**, as is a prefix conflict — in either direction.
#[test]
fn g6_10_a_colliding_or_prefixing_keys_table_is_rejected() {
    let s = editor();

    let fixed: String = eval(
        &s,
        "local ok, e = pcall(pmacs.listview.open, {\n\
           name = '*keys-a*', rows = {}, keys = { g = 'git.status' } })\n\
         return tostring(e)",
    );
    assert!(
        fixed.contains("own key surface"),
        "rebinding `g` must be refused by name: {fixed}"
    );

    let extends: String = eval(
        &s,
        "local ok, e = pcall(pmacs.listview.open, {\n\
           name = '*keys-b*', rows = {}, keys = { ['g x'] = 'git.status' } })\n\
         return tostring(e)",
    );
    assert!(
        extends.contains("prefix"),
        "`g x` would turn the `g` leaf into a submap: {extends}"
    );

    let internal: String = eval(
        &s,
        "local ok, e = pcall(pmacs.listview.open, {\n\
           name = '*keys-c*', rows = {},\n\
           keys = { ['d'] = 'git.status', ['d x'] = 'git.diff-file' } })\n\
         return tostring(e)",
    );
    assert!(
        internal.contains("prefix"),
        "two `keys` entries may not prefix each other: {internal}"
    );

    // Nothing was created: a rejected table must leave no half-built
    // panel behind, which is why validation runs before `ensure_panel`.
    let names: Vec<String> = eval(
        &s,
        "local out = {}\n\
         for _, id in ipairs(pmacs.buffer.list()) do out[#out+1] = pmacs.describe.buffer(id).name end\n\
         return out",
    );
    for name in ["*keys-a*", "*keys-b*", "*keys-c*"] {
        assert!(
            !names.iter().any(|n| n == name),
            "{name} must not exist after a rejected open: {names:?}"
        );
    }
}

/// An ALIAS spelling of a fixed key is rejected too — and rejecting it
/// leaves **no orphan buffer**.
///
/// The raw-token preflight cannot see these: `parse_key_code`
/// (`src/key.rs`) uppercases and then folds `RET`/`RETURN`/`ENTER`,
/// `SPC`/`SPACE`, `TAB` onto the same `KeyCode`, so `keys = { RETURN =
/// … }` compares unequal to every fixed token and sails through, only
/// for `Keymap::bind` to refuse it later — *after* the buffer has been
/// created, made read-only, marked round-trip and given the fixed
/// keymap, and *before* the panel is registered. The buffer then
/// survives owned by nothing, and the next `open` for that name finds it
/// and silently disambiguates itself to `<2>`.
///
/// So the error message is the weaker half of this test. Asserting only
/// that would pass on the broken code, because the broken code does
/// raise — it just leaves wreckage behind. The buffer count and the
/// undisambiguated reopen are what actually bite.
#[test]
fn g6_10c_an_alias_spelling_is_rejected_and_leaves_no_orphan_buffer() {
    let s = editor();
    let before: i64 = eval(&s, "return #pmacs.buffer.list()");

    // Every alias the parser folds onto a key the panel already owns.
    for alias in ["RETURN", "ENTER", "ret", "enter", "SPACE", "space", "tab"] {
        let err: String = eval(
            &s,
            &format!(
                "local ok, e = pcall(pmacs.listview.open, {{\n\
                   name = '*alias*', rows = {{}}, keys = {{ [{alias:?}] = 'git.status' }} }})\n\
                 return tostring(e)"
            ),
        );
        assert!(
            err.contains("listview:"),
            "{alias:?} must be refused by the primitive, with its own \
             message rather than a bare keymap error: {err}"
        );
        let now: i64 = eval(&s, "return #pmacs.buffer.list()");
        assert_eq!(
            now, before,
            "refusing {alias:?} must leave no buffer behind (this is the \
             half that bites: the broken code raises too)"
        );
    }

    // …and the name is genuinely still free: a subsequent legitimate
    // open gets the plain name, not `*alias*<2>`.
    exec(
        &s,
        "pmacs.listview.open { name = '*alias*', rows = { { text = 'x', item = 1 } },\n\
                               keys = { d = 'git.diff-file' } }",
    );
    assert_eq!(
        active_name(&s),
        "*alias*",
        "a rejected `keys` table must not have consumed the panel's name"
    );
}

/// Reopening a live panel with a DIFFERENT `keys` table errors rather
/// than silently keeping the old binding.
///
/// Silently keeping it would hand the consumer a key that does
/// something other than what it just asked for — a dead or lying key,
/// which is the defect this primitive already condemns for `g`.
#[test]
fn g6_10b_reopening_with_different_keys_errors_instead_of_lying() {
    let s = editor();
    exec(
        &s,
        "pmacs.listview.open { name = '*keys-d*', rows = {}, keys = { d = 'git.status' } }",
    );
    let err: String = eval(
        &s,
        "local ok, e = pcall(pmacs.listview.open, {\n\
           name = '*keys-d*', rows = {}, keys = { d = 'git.diff-file' } })\n\
         return tostring(e)",
    );
    assert!(
        err.contains("already open with keys"),
        "divergence must be reported, not ignored: {err}"
    );
    // The same table reopens fine — otherwise the refresh path itself
    // would be broken and this rule would be unusable.
    exec(
        &s,
        "pmacs.listview.open { name = '*keys-d*', rows = {}, keys = { d = 'git.status' } }",
    );
}

/// No new interaction island (COHERENCE.md §6): `d` is bound
/// **buffer-locally** through listview's own binding path, so
/// `describe-key` reports the truth and the key is rebindable from
/// `init.lua` — the observable difference between the keymap idiom and
/// a hardcoded shadow.
#[test]
fn g6_20_d_is_an_ordinary_buffer_local_binding() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    let (command, scope): (String, String) = eval(
        &s,
        "local info = pmacs.describe.key('d')\n\
         return info.command, info.scope",
    );
    assert_eq!(command, "git.diff-file");
    assert!(
        scope.starts_with("buffer"),
        "`d` must live at BUFFER scope, not global: {scope}"
    );

    // Away from the panel it is not bound at all, which is what "no
    // island" means: nothing intercepts `d` anywhere else.
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*plain*'))",
    );
    let elsewhere: Option<String> = eval(
        &s,
        "local info = pmacs.describe.key('d')\n\
         return info and info.command or nil",
    );
    assert_ne!(
        elsewhere.as_deref(),
        Some("git.diff-file"),
        "`d` must not be bound outside the panel"
    );
}

// ---------------------------------------------------------------------------
// §6 — refresh semantics (Q#G-1)
// ---------------------------------------------------------------------------

/// `g` is **never a no-op**: it re-renders and marks that work started,
/// even mid-flight. A dead `g` is a defect this primitive already names.
#[test]
fn g6_16_g_marks_that_work_started_immediately() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");
    assert!(
        !panel_text(&s).contains("refreshing"),
        "precondition: settled"
    );

    // No pumping: the marker must be there the instant `g` returns.
    press(&mut s, KeyCode::Char('g'));
    assert!(
        panel_text(&s).contains("(refreshing...)"),
        "g must re-render with a marker before any output arrives:\n{}",
        panel_text(&s)
    );
    // And again mid-flight, rather than being swallowed as "already
    // running".
    press(&mut s, KeyCode::Char('g'));
    assert!(
        panel_text(&s).contains("(refreshing...)"),
        "a second g mid-flight still re-renders:\n{}",
        panel_text(&s)
    );

    assert!(
        pump_until(&mut s, 15_000, |s| !panel_text(s).contains("refreshing")),
        "both refreshes settle"
    );
}

/// Concurrent refresh **discards the stale generation** rather than
/// racing.
///
/// Driven by completing two refreshes out of order, which no
/// arrangement of real subprocess timing can guarantee — so the
/// completion handler is called directly, with a request carrying the
/// older generation. The positive control at the end is what makes the
/// discard attributable to the generation and not to the payload.
#[test]
fn g6_17_a_stale_completion_discards_its_rows() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    let row = format!("1 .M N... 100644 100644 100644 {H} {H} SENTINEL.txt");
    let payload = z_payload(&["# branch.oid deadbeef", "# branch.head main", &row]);

    // A completion from one generation ago.
    exec(
        &s,
        &format!(
            "local current = pmacs.git._generation()\n\
             pmacs.git._deliver_status(\n\
               {{ generation = current - 1 }},\n\
               {{ ok = true, code = 0, stdout = {payload}, stderr = '' }})"
        ),
    );
    assert!(
        !panel_text(&s).contains("SENTINEL.txt"),
        "a stale completion must not reach the panel:\n{}",
        panel_text(&s)
    );

    // The positive control: the same payload at the CURRENT generation
    // does land, so the discard above was about the generation.
    //
    // Asserted on a ROW, not on the panel as a whole. The panel text
    // includes the header, and a malformed payload can put arbitrary
    // text there through `# branch.head` — which is precisely how the
    // first draft of this test passed while parsing nothing.
    exec(
        &s,
        &format!(
            "pmacs.git._deliver_status(\n\
               {{ generation = pmacs.git._generation() }},\n\
               {{ ok = true, code = 0, stdout = {payload}, stderr = '' }})"
        ),
    );
    assert_eq!(
        row_line(&s, "SENTINEL.txt"),
        Some(1),
        "the current generation must land as a ROW:\n{}",
        panel_text(&s)
    );
}

/// Two `git.status` invocations against DIFFERENT repositories, where
/// the **first** invocation's root lookup completes **second**. The
/// second invocation must win.
///
/// This is the ordering `g6_17` cannot see. That test drives the STATUS
/// completions out of order, and the generation each of those carries
/// was already fixed; this one drives the **root lookups** out of order,
/// which is where the generation used to be minted. `git.status` started
/// an unversioned `rev-parse` and the generation was claimed later, from
/// the callback — so whichever `rev-parse` returned last claimed the
/// newest generation and replaced the newer request. The counter that is
/// supposed to make the newest INVOCATION win instead made the slowest
/// SUBPROCESS win.
///
/// Driven through `_deliver_root` for the same reason `g6_17` uses
/// `_deliver_status`: no arrangement of real subprocess timing can
/// guarantee that two `rev-parse` runs finish in a chosen order, and a
/// test that merely hoped for the bad order would pass on the broken
/// code roughly half the time.
///
/// The assertion is on the argv of the last spawn, because the contract
/// is precisely that the superseded lookup "must not proceed to spawn a
/// status" — and that is observable without pumping, so the real
/// `rev-parse` children still in flight cannot muddy it.
#[test]
fn g6_21_a_superseded_root_lookup_does_not_spawn_its_status() {
    let (_dir_a, root_a) = tempdir();
    mixed_repo(&root_a);
    let (_dir_b, root_b) = tempdir();
    mixed_repo(&root_b);

    let mut s = editor();
    // Invocation 1 (repo A), then invocation 2 (repo B). Neither is
    // pumped, so both root lookups are genuinely in flight and each has
    // reserved its generation at the command.
    for root in [&root_a, &root_b] {
        let root_str = root.display().to_string();
        let seed = root.join("staged.txt").display().to_string();
        exec(
            &s,
            &format!(
                "pmacs.project.set_search_boundary({root_str:?})\n\
                 pmacs.buffer.find_or_open({seed:?})"
            ),
        );
        exec(&s, "pmacs.git.status()");
    }

    let gen_b: i64 = eval(&s, "return pmacs.git._generation()");
    let gen_a = gen_b - 1;
    assert!(
        gen_a >= 1,
        "premise: each invocation reserved its own generation at the \
         command, so the two differ"
    );

    let a = root_a.display().to_string();
    let b = root_b.display().to_string();

    // The NEWER invocation's root lands first…
    exec(
        &s,
        &format!(
            "pmacs.git._deliver_root(\n\
               {{ generation = {gen_b}, dir = {b:?} }},\n\
               {{ ok = true, code = 0, stdout = {b:?}, stderr = '' }})"
        ),
    );
    let after_b: Vec<String> = eval(&s, "return pmacs.git._last_spawn.args");
    assert!(
        after_b.contains(&b),
        "premise: the newer invocation spawned its status against B: {after_b:?}"
    );
    let statuses_after_b: i64 = eval(
        &s,
        "local n = 0\n\
         for _, args in ipairs(pmacs.git._spawn_log) do\n\
           for _, a in ipairs(args) do if a == 'status' then n = n + 1 end end\n\
         end\n\
         return n",
    );

    // …and the OLDER invocation's root lands second, superseded.
    exec(
        &s,
        &format!(
            "pmacs.git._deliver_root(\n\
               {{ generation = {gen_a}, dir = {a:?} }},\n\
               {{ ok = true, code = 0, stdout = {a:?}, stderr = '' }})"
        ),
    );

    let after_a: Vec<String> = eval(&s, "return pmacs.git._last_spawn.args");
    assert!(
        !after_a.contains(&a),
        "the superseded root lookup must NOT spawn a status against A; \
         last argv was {after_a:?}"
    );
    assert_eq!(
        after_a, after_b,
        "…so the last spawn is still the newer invocation's"
    );
    let statuses_after_a: i64 = eval(
        &s,
        "local n = 0\n\
         for _, args in ipairs(pmacs.git._spawn_log) do\n\
           for _, a in ipairs(args) do if a == 'status' then n = n + 1 end end\n\
         end\n\
         return n",
    );
    assert_eq!(
        statuses_after_a, statuses_after_b,
        "and it spawned nothing at all — one status invocation, not two"
    );

    // The user-visible half: pumping settles on B's repository, whatever
    // order the two real `rev-parse` children happen to finish in.
    assert!(
        pump_until(&mut s, 15_000, |s| !panel_text(s).is_empty()
            && !panel_text(s).contains("refreshing")),
        "the winning invocation's panel must render; status was {:?}",
        status(&s)
    );
    let last: Vec<String> = eval(&s, "return pmacs.git._last_spawn.args");
    assert!(
        last.contains(&b) && !last.contains(&a),
        "the settled panel belongs to the second invocation: {last:?}"
    );
}

/// Two `d` requests in flight: the **newer** one wins, even when the
/// older one completes **last**.
///
/// `*git-diff*` is a singleton buffer, so a diff plan that finishes
/// after a newer one would otherwise overwrite it — the newest
/// invocation losing to the slowest subprocess, which is exactly the
/// defect the status channel was fixed for. The diff channel now
/// reserves its own ticket at the keypress and discards a superseded
/// completion **before any effect**.
///
/// Two halves, and both are needed:
///
/// * the **real** half presses `d` twice with nothing pumped between,
///   so two plans are genuinely in flight and each really did reserve
///   its own ticket at the command;
/// * the **driven** half then completes the OLDER request, after the
///   newer one has already rendered. That ordering is the whole
///   contract and no arrangement of real subprocess timing can produce
///   it on demand — both diffs take milliseconds, and the first one
///   spawned normally finishes first, which is the order that passes on
///   the broken code. Same reason `g6_17` and `g6_21` drive their own
///   completions.
///
/// The positive control at the end is what makes the discard
/// attributable to the ticket rather than to the payload.
#[test]
fn g6_23_a_superseded_diff_does_not_replace_the_newer_one() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    // Request A, then request B, with no frame pumped between them. `d`
    // renders into the DOCUMENT window only at completion, so the panel
    // is still focused for the second press.
    let status_gen_before: i64 = eval(&s, "return pmacs.git._generation()");
    seat_on(&mut s, "staged.txt");
    press(&mut s, KeyCode::Char('d'));
    let gen_a: i64 = eval(&s, "return pmacs.git._diff_generation()");
    seat_on(&mut s, "unstaged.txt");
    press(&mut s, KeyCode::Char('d'));
    let gen_b: i64 = eval(&s, "return pmacs.git._diff_generation()");
    assert_eq!(
        gen_b,
        gen_a + 1,
        "premise: each `d` reserves its own ticket at the keypress"
    );
    // …and the diff channel is its OWN: two `d` presses must not have
    // touched the status channel, which a single module-wide counter
    // would have done — making `d` cancel an in-flight `g`.
    let status_gen_after: i64 = eval(&s, "return pmacs.git._generation()");
    assert_eq!(
        status_gen_before, status_gen_after,
        "a diff must not consume the status channel's ticket"
    );

    assert!(
        pump_until(&mut s, 15_000, |s| diff_text(s).contains("+worktree edit")),
        "the newest request must render; diff was:\n{}\nstatus: {:?}",
        diff_text(&s),
        status(&s)
    );
    let settled = diff_text(&s);
    assert!(
        settled.contains("git diff --- unstaged.txt"),
        "…and it is the row `d` was last pressed on: {settled}"
    );

    // The older request finally answers. It must change nothing.
    let stale = format!(
        "pmacs.git._deliver_diff(\n\
           {{ generation = {gen_a}, row = {{ path = 'staged.txt' }},\n\
             plan = {{ header = 'against HEAD', steps = {{}} }},\n\
             root = '/', pieces = {{}}, index = 0 }},\n\
           {{}},\n\
           {{ ok = true, code = 0, stdout = 'STALE-DIFF-SENTINEL\\n', stderr = '' }})"
    );
    exec(&s, &stale);
    assert_eq!(
        diff_text(&s),
        settled,
        "a superseded diff must not replace the newer one"
    );

    // …and it must not reach the status band either, which is the half a
    // buffer-only check would miss.
    exec(&s, "pmacs.editor.set_status('')");
    exec(
        &s,
        &format!(
            "pmacs.git._deliver_diff(\n\
               {{ generation = {gen_a}, row = {{ path = 'staged.txt' }},\n\
                 plan = {{ header = 'against HEAD', steps = {{}} }},\n\
                 root = '/', pieces = {{}}, index = 0 }},\n\
               {{}},\n\
               {{ ok = true, code = 128, stdout = '',\n\
                 stderr = 'fatal: STALE-FAILURE' }})"
        ),
    );
    assert_eq!(
        status(&s),
        "",
        "a superseded FAILURE is as wrong as a superseded patch"
    );
    assert_eq!(
        diff_text(&s),
        settled,
        "and it wrote no failure body either"
    );

    // The positive control: the same delivery at the CURRENT ticket does
    // land, so the two discards above were about the ticket.
    exec(
        &s,
        &stale.replace(
            &format!("generation = {gen_a}"),
            &format!("generation = {gen_b}"),
        ),
    );
    let now = diff_text(&s);
    assert!(
        now.contains("STALE-DIFF-SENTINEL"),
        "the current ticket must be delivered: {now}"
    );
}

/// Selection is re-seated **by the completion handler**, across a
/// refresh that reorders rows.
///
/// `listview.open` resets collapse and always seats line 1; it does not
/// preserve selection. Only `listview.refresh` does, and that is the
/// synchronous path this model cannot use — so the re-seating is owned
/// here, and this test is what says so.
#[test]
fn g6_11_selection_follows_the_path_across_a_reordering_refresh() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    seat_on(&mut s, "unstaged.txt");
    let line_before: i64 = eval(&s, "return pmacs.editor.cursor_line()");

    // Dirty TWO tracked files that sort before it, so its line moves by
    // two. Both details are load-bearing, and both were found by biting
    // this test:
    //
    // * TRACKED, not a new untracked file — porcelain v2 emits every
    //   `1`/`2` record before any `?` record, so an untracked addition
    //   lands at the end and moves nothing at all;
    // * TWO, not one — dropping the handler's re-seating entirely
    //   leaves the cursor exactly one row lower than it was, which a
    //   one-row insertion happens to match.
    write(&root, "a1.txt", "a1 base\nedit\n");
    write(&root, "a2.txt", "a2 base\nedit\n");
    press(&mut s, KeyCode::Char('g'));
    assert!(
        pump_until(&mut s, 15_000, |s| !panel_text(s).contains("refreshing")),
        "refresh must settle"
    );

    let line_after: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    let row = panel_text(&s)
        .lines()
        .nth(usize::try_from(line_after).expect("line fits"))
        .unwrap_or_default()
        .to_string();
    assert!(
        row.contains("unstaged.txt"),
        "selection follows the PATH, not the line; landed on {row:?} in:\n{}",
        panel_text(&s)
    );
    assert_ne!(
        line_before, line_after,
        "fixture: the inserted row must have moved the selected one"
    );
}

/// …and falls back to line 1 **without complaint** when the selected
/// path drops out of status — the common case, not an error, since a
/// file that stopped being modified simply leaves the list.
#[test]
fn g6_11b_a_vanished_selection_seats_line_one_silently() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    seat_on(&mut s, "untracked.txt");
    std::fs::remove_file(root.join("untracked.txt")).expect("rm untracked.txt");
    exec(&s, "pmacs.editor.set_status('')");
    press(&mut s, KeyCode::Char('g'));
    assert!(
        pump_until(&mut s, 15_000, |s| {
            !panel_text(s).contains("refreshing") && !panel_text(s).contains("untracked.txt")
        }),
        "the refresh must settle without the vanished row:\n{}",
        panel_text(&s)
    );

    let line: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    assert_eq!(line, 1, "the cursor seats on the first data row");
    assert_eq!(
        status(&s),
        "",
        "and nothing is reported — this is the correct answer, not a failure"
    );
}

// ---------------------------------------------------------------------------
// §6 — the root rule (Q#G-2) and failure surfaces (§1.2)
// ---------------------------------------------------------------------------

/// The root rule is witnessed on a repository whose `ProjectKind` is
/// **not** `Git`.
///
/// `ProjectKind::Git` means a BARE repository — "no language marker
/// found inside" — and a language marker beside `.git` wins, so an
/// ordinary Rust project reports `kind = "rust"`. A lane gating on
/// `kind == "git"` would have been invisible on it, and on this
/// repository too.
#[test]
fn g6_14_the_root_rule_works_where_project_kind_is_not_git() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let root_str = root.display().to_string();

    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    let kind: String = eval(
        &s,
        &format!("return pmacs.project.detect({root_str:?}).kind"),
    );
    assert_eq!(
        kind, "rust",
        "premise: the fixture's ProjectKind is NOT `git`, because \
         Cargo.toml sits beside .git"
    );
    assert!(
        panel_text(&s).contains("staged.txt"),
        "and the panel resolved its root through git anyway:\n{}",
        panel_text(&s)
    );
    // The root really is the repository's, so a row's REPOSITORY-relative
    // path resolves to a real file. Porcelain paths are relative to the
    // repository root, not to the directory git ran in, so a root that
    // was merely "some ancestor" would open nothing.
    seat_on(&mut s, "unstaged.txt");
    press(&mut s, KeyCode::Enter);
    let visited = active_name(&s);
    assert_eq!(
        visited,
        root.join("unstaged.txt").display().to_string(),
        "RET opens the file the row names, resolved against the git root; \
         status was {:?}",
        status(&s)
    );
}

/// A repository rooted at a directory literally named `leaf` resolves
/// **whole**, and the status command really runs there.
///
/// End to end, not at the parser: the directory really is created, the
/// real `git` really resolves it, and the assertion is on the cwd of the
/// spawn the module actually made. `_last_spawn` is the status
/// invocation here, since `rev-parse` runs first and carries no cwd of
/// its own. A root that lost a byte would still SPAWN — with a `-C` and
/// a cwd naming a directory that does not exist — so the panel is
/// checked for a failure row as well as for real ones.
///
/// `open_panel` binds `pmacs.project.set_search_boundary` to the fixture
/// (R8's lesson), which matters most here: these leaf names are exactly
/// the shape that makes a detection walk out of the tempdir hard to
/// read when it goes wrong.
fn assert_root_resolves_whole(leaf: &str) {
    let (_dir, base) = tempdir();
    let root = base.join(leaf);
    std::fs::create_dir_all(&root).unwrap_or_else(|e| {
        panic!("a root named {leaf:?} must be creatable — every byte in it is a legal POSIX path byte: {e}")
    });
    mixed_repo(&root);

    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    let cwd: String = eval(&s, "return pmacs.git._last_spawn.cwd");
    assert_eq!(
        cwd,
        root.display().to_string(),
        "the resolved root must be the WHOLE path, every byte of {leaf:?} included"
    );

    let text = panel_text(&s);
    assert!(
        !text.contains("exited with code"),
        "…so the status command ran somewhere that exists: {text}"
    );
    assert!(
        text.contains("staged.txt"),
        "…and produced real rows: {text}"
    );

    // And the root is usable for the gestures built on it: a
    // repository-relative row path resolves against it to a real file.
    seat_on(&mut s, "unstaged.txt");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        root.join("unstaged.txt").display().to_string(),
        "RET resolves against the untruncated root; status was {:?}",
        status(&s)
    );
}

/// A repository root containing a **newline** resolves **whole**, and
/// the status command really runs there.
///
/// A newline is a legal byte in a POSIX path — the fixture builds one
/// and `git rev-parse --show-toplevel` prints it, terminator and all —
/// so parsing that output with a first-line match truncates
/// `/tmp/…/nl\nroot` to `/tmp/…/nl`, and every command this module runs
/// afterwards gets a `-C` and a cwd naming a directory that does not
/// exist. The right answer is to strip git's final terminator and
/// nothing else.
///
/// It rides beside the one-line-status rule rather than replacing it:
/// the helper this uses is deliberately **separate** from `first_line`,
/// whose other three callers all feed the single-line status band and
/// would be corrupted by a multi-line message.
#[test]
fn g6_14c_a_root_containing_a_newline_is_not_truncated() {
    assert_root_resolves_whole("nl\nroot");
}

/// A repository root ending in a **carriage return** resolves whole too
/// — the byte the newline fix's own strip still ate.
///
/// `\r` is as legal in a POSIX directory name as `\n` is, and it is the
/// byte that makes `\r?\n$` ambiguous: for a root named `trailing\r`,
/// git prints `…/trailing` `0d` `0a`, where the `0d` is the PATH and
/// only the `0a` is the terminator. A strip tolerant of an optional
/// preceding carriage return cannot tell those apart and takes both,
/// resolving the root as `…/trailing` — a directory that does not
/// exist. Only git's final `\n` may be removed.
///
/// The second case sends the two hazards in together, because a root may
/// hold both and neither fix may mask the other: an embedded newline
/// (which forbids a first-line read) ahead of a trailing carriage return
/// (which forbids an over-eager terminator strip).
#[test]
fn g6_14d_a_root_ending_in_a_carriage_return_is_not_truncated() {
    assert_root_resolves_whole("trailing\r");
    assert_root_resolves_whole("nl\nand-trailing\r");
}

/// A directory outside any repository reports it, rather than opening
/// an empty panel or saying nothing.
#[test]
fn g6_14b_a_directory_outside_a_repository_is_reported() {
    let (_dir, root) = tempdir();
    write(&root, "loose.txt", "not in a repo\n");
    let root_str = root.display().to_string();
    let seed = root.join("loose.txt").display().to_string();

    let mut s = editor();
    exec(
        &s,
        &format!(
            "pmacs.project.set_search_boundary({root_str:?})\n\
             pmacs.buffer.find_or_open({seed:?})"
        ),
    );
    exec(&s, "pmacs.git.status()");
    assert!(
        pump_until(&mut s, 15_000, |s| status(s)
            .contains("not inside a repository")),
        "the non-zero rev-parse exit IS the answer; status was {:?}",
        status(&s)
    );
    assert!(
        panel_text(&s).is_empty(),
        "and no panel is opened: {:?}",
        panel_text(&s)
    );
}

/// A missing `git` on `PATH` is witnessed, and surfaces **guidance**
/// rather than silence (§1.2's asymmetry; the same lesson #204 landed
/// for a missing language server).
///
/// Reached by pointing `pmacs.git._program` at a name that is not on
/// `PATH`, which produces exactly the ENOENT an absent git produces.
/// There is no other in-process route: Rust's `Command` resolves the
/// program against the **parent** process's `PATH`, so a child `env`
/// cannot hide git, and `std::env::set_var` is `unsafe` in edition 2024
/// — which this project forbids.
#[test]
fn g6_15_a_missing_git_binary_surfaces_guidance() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let root_str = root.display().to_string();
    let seed = root.join("staged.txt").display().to_string();

    let mut s = editor();
    exec(
        &s,
        &format!(
            "pmacs.project.set_search_boundary({root_str:?})\n\
             pmacs.buffer.find_or_open({seed:?})\n\
             pmacs.git._program = 'pmacs-no-such-git-binary'"
        ),
    );
    exec(&s, "pmacs.git.status()");
    // The spawn fails synchronously, so the guidance is already there;
    // pumping anyway proves nothing later overwrites it with silence.
    pump_for(&mut s, 200);

    let reported = status(&s);
    assert!(
        reported.contains("pmacs-no-such-git-binary") && reported.contains("PATH"),
        "the failure must name the program and point at PATH; got {reported:?}"
    );
    assert!(
        panel_text(&s).is_empty(),
        "and no panel is opened on a failed root resolution: {:?}",
        panel_text(&s)
    );
}

/// A failing `git status` renders a **row**, carrying the exit code and
/// the first line of stderr, plus a status message.
///
/// Driven by removing `.git` under a live panel and pressing `g`: the
/// panel already holds its root, so the refresh really does run and
/// really does fail.
#[test]
fn g6_18_a_failing_status_renders_a_row_with_code_and_stderr() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    std::fs::remove_dir_all(root.join(".git")).expect("rm -rf .git");
    press(&mut s, KeyCode::Char('g'));
    assert!(
        pump_until(&mut s, 15_000, |s| panel_text(s)
            .contains("exited with code")),
        "the failure must become a ROW, not a silence; panel:\n{}\nstatus: {:?}",
        panel_text(&s),
        status(&s)
    );

    let text = panel_text(&s);
    assert!(
        text.contains("exited with code 128"),
        "the row carries the exit code: {text}"
    );
    assert!(
        text.contains("not a git repository"),
        "…and the first stderr line: {text}"
    );
    assert!(
        status(&s).contains("git status:"),
        "…and a status message rides with it: {:?}",
        status(&s)
    );
}

// ---------------------------------------------------------------------------
// §6 — structural claims
// ---------------------------------------------------------------------------

/// The panel **is** a `listview` adopter, asserted structurally, so a
/// future re-implementation of list behaviour inside git code fails the
/// test rather than passing review.
///
/// Structural alone would not be enough — a comparison of two
/// authorities does not catch a misrouted consumer — so the behavioural
/// half rides with it: `n`, `p` and `q` are listview's own bindings and
/// are driven through `dispatch_key`.
#[test]
fn g6_19_the_panel_is_a_listview_adopter() {
    const GIT: &str = include_str!("../builtin/runtime/git.lua");

    assert!(
        GIT.contains("pmacs.listview.open"),
        "the panel must go through the primitive"
    );
    let creates: Vec<&str> = GIT
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .filter(|l| l.contains("pmacs.buffer.create"))
        .collect();
    assert_eq!(
        creates.len(),
        1,
        "the ONLY buffer this module creates itself is *git-diff*; the \
         status panel is listview's. Found: {creates:?}"
    );
    assert!(
        creates[0].contains("DIFF_BUFFER"),
        "and it is the diff buffer: {creates:?}"
    );
    let binds: Vec<&str> = GIT
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .filter(|l| l.contains("pmacs.keymap.bind"))
        .collect();
    assert!(
        binds.is_empty(),
        "`d` is bound through listview's `keys` table, never by this \
         module reaching for the keymap itself: {binds:?}"
    );
    // No second `rev-parse` for unborn detection: that fact comes from
    // the status output already being parsed. Checked on CODE lines
    // only — the module's own comment explains the rule and names the
    // process it forbids, and a substring sweep would trip on it.
    let verifies: Vec<&str> = GIT
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .filter(|l| l.contains("--verify"))
        .collect();
    assert!(
        verifies.is_empty(),
        "unborn detection must not reintroduce `rev-parse --verify HEAD`: {verifies:?}"
    );
    assert!(
        GIT.contains("(initial)"),
        "…it reads `# branch.oid (initial)` instead"
    );

    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let mut s = editor();
    open_panel(&mut s, &root, "staged.txt");

    let start: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    press(&mut s, KeyCode::Char('n'));
    let down: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    assert_eq!(down, start + 1, "`n` is listview's own motion");
    press(&mut s, KeyCode::Char('p'));
    let up: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    assert_eq!(up, start, "`p` too");
    press(&mut s, KeyCode::Char('q'));
    assert_ne!(
        active_name(&s),
        "*git-status*",
        "`q` leaves the panel, through listview's quit"
    );
}

/// `git.enabled` is a real config-registry setting (Q#G-4), and turning
/// it off means nothing is spawned — reported, not silently inert.
#[test]
fn g6_config_git_enabled_is_registry_defined_and_honoured() {
    let (_dir, root) = tempdir();
    mixed_repo(&root);
    let root_str = root.display().to_string();
    let seed = root.join("staged.txt").display().to_string();

    let mut s = editor();
    let (default, kind, described): (bool, String, bool) = eval(
        &s,
        "local d = pmacs.config.describe('git.enabled')\n\
         return pmacs.config.get('git.enabled'), d.type, #d.description > 0",
    );
    assert!(default, "default is true");
    assert_eq!(kind, "boolean");
    assert!(described, "the registry carries a real description");

    exec(
        &s,
        &format!(
            "pmacs.project.set_search_boundary({root_str:?})\n\
             pmacs.buffer.find_or_open({seed:?})\n\
             pmacs.config.set('git.enabled', false)\n\
             pmacs.git._spawn_log = {{}}"
        ),
    );
    exec(&s, "pmacs.git.status()");
    pump_for(&mut s, 300);
    assert!(
        status(&s).contains("git.enabled"),
        "the refusal names the setting; got {:?}",
        status(&s)
    );
    let spawned: i64 = eval(&s, "return #pmacs.git._spawn_log");
    assert_eq!(spawned, 0, "and nothing was spawned");
    assert!(panel_text(&s).is_empty(), "and no panel opened");
}

// Isolated bootstrap storage roots: an integration test is compiled
// without `cfg(test)`, so a raw `EditorState::new()` would read the
// developer's real `init.lua` and write into their real data root.
#[path = "common/iso.rs"]
mod iso;
