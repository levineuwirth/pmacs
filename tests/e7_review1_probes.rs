// tests/e7_review1_probes.rs --- C7 review round 1's behavioral probes
// against E7's rows: the states a user reaches that the handoff's
// witnesses did not choose.

//! Six probes, one per question the round asked:
//!
//! * E7.4's substrate: the token absorb's prune (`SemanticTokenStore::
//!   set`) drops log entries without raising `dropped_below`, so a
//!   consumer whose base is older than the pruned edits is misled
//!   rather than refused. The store-level row seeds that ordering ---
//!   a newer completion answer moving the floor, a token answer
//!   absorbed at that base, then the older popup's carry asked for
//!   --- and asserts the log never places a range it cannot carry.
//! * E7.4 on the GPU route: a completion accepted after a letter typed
//!   since the request, with the letters arriving as the GPU sends them
//!   (one optimistic op each) and the accept as the key it forwards,
//!   against a real daemon.
//! * E7.3's late answer at the default bound: a server answering at
//!   1200 ms against 1000 ms, the user editing after the save came
//!   back unformatted, the late answer touching nothing.
//! * E7.3's server dying mid-wait: the save proceeds unformatted, says
//!   so, and the editor does not hang or panic.
//! * E7.2: `p` from the last line of a diff lands on the last hunk.
//! * E7.1: after an empty RET and a wrong word, the question still
//!   works --- one `y` discards exactly once.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::{CellSize, FrontendId};
use pmacs::semantic_tokens::{
    DocumentEdit, SemanticToken, SemanticTokenKey, SemanticTokenStore, SemanticTokensResponse,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod common;

// ---------------------------------------------------------------------------
// Harness (the E7 suites' helpers, copied so the probes stand alone)
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

/// Read only by the Linux-gated killed-server probe; gated with it so
/// the macOS legs, which deny warnings, see no dead helper.
#[cfg(target_os = "linux")]
fn errors_text(s: &EditorState) -> String {
    s.lua_host.errors_buffer_text()
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7rev-{tag}-{}", std::process::id()));
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

/// The format-on-save suite's fixture: the fake's formatting reply
/// deletes columns 0--4 of line 0 and inserts `;` at line 3 column 7.
const BODY: &str = "    fn main() {\n    let a = 1;\n    let b = 2;\n    let c = a + b\n}\n";

/// An editor with isolated roots, the rust server pointed at the fake
/// with `extra_env` in its environment, visiting `dir/a.rs` holding
/// `BODY`, returning once the fake has initialized.
fn fake_editor(dir: &Path, extra_env: &[(&str, &str)]) -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut env = String::from("PMACS_FAKE_LSP_MODE = ''");
    for (k, v) in extra_env {
        env.push_str(", ");
        env.push_str(k);
        env.push_str(" = '");
        env.push_str(v);
        env.push('\'');
    }
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{ {env} }},
             }}"
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, BODY).unwrap();
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

fn disk(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("a.rs")).expect("read a.rs")
}

/// Run `buffer.save` and return how long it took.
fn save(s: &EditorState) -> Duration {
    let t0 = Instant::now();
    exec(s, "pmacs.command.invoke('buffer.save')");
    t0.elapsed()
}

fn modified(s: &EditorState) -> bool {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).modified",
    )
}

// ---------------------------------------------------------------------------
// E7.4's substrate: the prune is silent about what it drops
// ---------------------------------------------------------------------------

fn one_token(line: u32, start: u32, length: u32) -> SemanticTokensResponse {
    SemanticTokensResponse {
        tokens: vec![SemanticToken {
            line,
            start,
            length,
            token_type: 1,
            token_modifiers: 0,
        }],
        ..SemanticTokensResponse::default()
    }
}

/// A document `foo\n` takes three one-byte inserts at its start (edit
/// numbers 0, 1, 2). A completion was answered at edit number 1 and
/// its popup is still open; a newer completion answer at 3 has moved
/// the floor; a full token answer at 3 is absorbed and its prune
/// keeps the log from 3, dropping edits 1 and 2 and raising nothing.
/// The older popup is then accepted: its carry asks the log to place
/// byte 0 of the text at edit 1. The two inserts since have pushed
/// that position to 2, but the log has forgotten them, `dropped_below`
/// still says 0, and `translate_range_since` answers 0 --- misled, not
/// refused. The contract this pins: a range the log cannot carry is
/// `None` (nothing applied, said), never a wrong position. With the
/// prune raising `dropped_below` to what it kept from, it is.
#[test]
fn e7_review1_the_prune_misleads_an_older_completion_carry() {
    let uri = "file:///probe.rs";
    let key = SemanticTokenKey::new("1", uri);
    let mut store = SemanticTokenStore::new();
    store.open_log(uri);
    for _ in 0..3 {
        store.record_edit(
            uri,
            DocumentEdit {
                start: 0,
                old_end: 0,
                inserted_len: 1,
            },
            "x",
        );
    }
    // Positive control: with the log whole, the carry from edit 1 is
    // moved past the two inserts since.
    assert_eq!(
        store.translate_range_since(uri, 1, 0, 0),
        Some((2, 2)),
        "the whole log carries byte 0 at edit 1 to byte 2 now"
    );
    // The newer completion answer moves the floor; the token absorb at
    // the same base prunes the log below it.
    store.note_completion_base(uri, 3);
    assert!(store.set(
        key,
        one_token(0, 3, 3),
        "xxxfoo\n",
        pmacs::lsp::PositionEncoding::Utf8,
        3,
    ));
    // The older popup's carry: refused, or placed where the text has
    // it. Never byte 0.
    let placed = store.translate_range_since(uri, 1, 0, 0);
    assert!(
        placed.is_none() || placed == Some((2, 2)),
        "a carry the log no longer reaches was placed at {placed:?}: the prune dropped \
         edits 1 and 2 without raising dropped_below, so the accept would apply the \
         import two bytes early"
    );
}

// ---------------------------------------------------------------------------
// E7.3: the late answer at the default bound, then the user edits
// ---------------------------------------------------------------------------

/// The knob on, the bound left at its default. The server answers at
/// 1200 ms: the save gives up at 1000, writes the buffer as typed and
/// says so; the user then types; the answer lands; nothing but the
/// user's letter is in the buffer, the disk is as the save wrote it,
/// and a later save with time to spare formats what is there now.
#[test]
fn e7_review1_a_late_answer_past_the_default_bound_never_touches_an_edited_buffer() {
    let dir = temp_dir("late-default");
    let mut s = fake_editor(&dir, &[("PMACS_FAKE_LSP_FORMAT_HOLD_MS", "1200")]);
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    assert_eq!(
        eval::<i64>(
            &s,
            "return pmacs.config.get('lsp.format-on-save.timeout-ms')"
        ),
        1000,
        "positive control: the default bound"
    );
    let took = save(&s);
    assert!(
        took >= Duration::from_secs(1) && took < Duration::from_millis(1200),
        "the save waits the default bound and gives up before the answer: {took:?}"
    );
    assert_eq!(disk(&dir), BODY, "written as typed");
    assert!(
        status(&s).contains("unformatted: the language server did not answer within 1000 ms"),
        "status: {:?}",
        status(&s)
    );
    assert!(!modified(&s), "the save left the buffer clean");

    // The user types at the top of the file.
    exec(&s, "local b = pmacs.window.buffer(); b:insert(0, 'Z')");
    let typed = format!("Z{BODY}");
    assert_eq!(buffer_text(&s), typed);

    // The held answer arrives and is processed.
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        buffer_text(&s),
        typed,
        "the late answer touched nothing but the user's letter stands"
    );
    assert_eq!(disk(&dir), BODY, "the disk is what the save wrote");
    assert!(modified(&s), "the user's edit is still unsaved");
    assert!(
        !status(&s).contains("formatted ("),
        "no formatting was reported after the fact: {:?}",
        status(&s)
    );

    // With room, the next save formats the text as it stands now.
    exec(
        &s,
        "pmacs.config.set('lsp.format-on-save.timeout-ms', 3000)",
    );
    let took = save(&s);
    assert!(took < Duration::from_secs(3), "answered in time: {took:?}");
    assert!(
        status(&s).contains("formatted (2 edits)"),
        "status: {:?}",
        status(&s)
    );
    assert_ne!(disk(&dir), typed, "the second save wrote formatted text");
    assert!(!modified(&s));
}

// ---------------------------------------------------------------------------
// E7.3: the server dies while the save is waiting on it
// ---------------------------------------------------------------------------

/// The pid of the fake server this test spawned, found through
/// `/proc` by the tag in its environment (Linux; the sweep's leg).
#[cfg(target_os = "linux")]
fn fake_pid_by_tag(tag: &str) -> Option<u32> {
    let needle = format!("PMACS_E7_REVIEW1_TAG={tag}");
    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = entry.ok()?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(environ) = std::fs::read(entry.path().join("environ")) else {
            continue;
        };
        if environ.split(|b| *b == 0).any(|kv| kv == needle.as_bytes()) {
            return Some(pid);
        }
    }
    None
}

/// The server is holding its answer for five seconds; the bound is
/// three. A third of a second into the wait the server is killed. The
/// save must come back well before the bound, unformatted and said,
/// and the editor must still be usable afterwards.
#[cfg(target_os = "linux")]
#[test]
fn e7_review1_a_server_killed_during_the_wait_saves_unformatted_and_says_so() {
    let dir = temp_dir("killed");
    let tag = format!("{}", std::process::id());
    let mut s = fake_editor(
        &dir,
        &[
            ("PMACS_FAKE_LSP_FORMAT_HOLD_MS", "5000"),
            ("PMACS_E7_REVIEW1_TAG", &tag),
        ],
    );
    let pid = fake_pid_by_tag(&tag).expect("the fake server's pid");
    exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
    exec(
        &s,
        "pmacs.config.set('lsp.format-on-save.timeout-ms', 3000)",
    );
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let status = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status()
            .expect("kill");
        assert!(status.success(), "kill {pid}");
    });
    let took = save(&s);
    killer.join().expect("killer thread");
    assert!(
        took < Duration::from_millis(2500),
        "the wait ends when the server does, not at the bound: {took:?}"
    );
    assert_eq!(disk(&dir), BODY, "saved unformatted");
    assert!(
        status(&s).contains("unformatted"),
        "the user is told: {:?}",
        status(&s)
    );
    assert!(
        errors_text(&s).contains("format-on-save: unformatted"),
        "*errors* keeps the line: {:?}",
        errors_text(&s)
    );
    assert!(!modified(&s), "the save wrote the buffer");
    // The editor is still usable: ticks run, a later edit lands.
    for _ in 0..20 {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(5));
    }
    exec(&s, "local b = pmacs.window.buffer(); b:insert(0, 'Z')");
    assert_eq!(buffer_text(&s), format!("Z{BODY}"));
}

// ---------------------------------------------------------------------------
// E7.2 and E7.1: through the real panel against a real repository
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

fn write(root: &Path, rel: &str, body: &str) {
    std::fs::write(root.join(rel), body).unwrap_or_else(|e| panic!("write {rel}: {e}"));
}

fn numbered(n: usize) -> String {
    use std::fmt::Write as _;
    (1..=n).fold(String::new(), |mut out, i| {
        let _ = writeln!(out, "line {i}");
        out
    })
}

fn init_repo(root: &Path) {
    git(root, &["init", "-q", "-b", "main", "."]);
    git(root, &["config", "user.email", "gate@example.invalid"]);
    git(root, &["config", "user.name", "Gate"]);
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

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
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

fn pump_for(s: &mut EditorState, ms: u64) {
    let stop = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < stop {
        s.tick_processes();
        std::thread::sleep(Duration::from_millis(5));
    }
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

fn cursor_line(s: &EditorState) -> usize {
    let n: i64 = eval(s, "return pmacs.editor.cursor_line()");
    usize::try_from(n).expect("cursor line fits")
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

fn seat_on(s: &mut EditorState, needle: &str) {
    let target = panel_text(s)
        .lines()
        .enumerate()
        .find(|(i, line)| *i > 0 && line.contains(needle))
        .map_or_else(
            || panic!("no row matching {needle:?} in:\n{}", panel_text(s)),
            |(i, _)| i,
        );
    let current = cursor_line(s);
    assert!(current <= target, "fixture: walk down to {needle:?}");
    for _ in current..target {
        press(s, KeyCode::Char('n'));
    }
    assert_eq!(cursor_line(s), target);
}

fn hunk_lines(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("@@"))
        .map(|(i, _)| i)
        .collect()
}

/// `p` from the last line of the diff, below every hunk, lands on the
/// last hunk; `n` from there says there is no next.
#[test]
fn e7_review1_p_from_the_diff_s_last_line_lands_on_the_last_hunk() {
    let (_td, root) = tempdir();
    init_repo(&root);
    let mut body = numbered(60);
    body = body.replace("line 5\n", "line 5 edited\n");
    body = body.replace("line 55\n", "line 55 edited\n");
    write(&root, "long.txt", &body);
    let mut s = editor();
    open_panel(&mut s, &root, "Cargo.toml");
    seat_on(&mut s, "long.txt");
    let before = diff_text(&s);
    press(&mut s, KeyCode::Char('d'));
    assert!(
        pump_until(&mut s, 15_000, |s| {
            let now = diff_text(s);
            now != before && now.contains("long.txt")
        }),
        "d must render the diff; status: {:?}",
        status(&s)
    );
    let text = diff_text(&s);
    let hunks = hunk_lines(&text);
    assert_eq!(hunks.len(), 2, "positive control: two hunks in\n{text}");
    let last_line = text.lines().count() - 1;
    exec(&s, &format!("pmacs.editor.move_to_line({last_line})"));
    assert!(cursor_line(&s) > hunks[1], "seated below the last hunk");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(
        cursor_line(&s),
        hunks[1],
        "p from the tail lands on the last hunk"
    );
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(cursor_line(&s), hunks[1], "n at the last hunk stays");
    assert_eq!(status(&s), "diff: no next hunk");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(cursor_line(&s), hunks[0]);
}

fn xy(root: &Path, rel: &str) -> String {
    let out = git(root, &["status", "--porcelain=v2", "--", rel]);
    for line in out.lines() {
        let mut fields = line.splitn(2, ' ');
        let tag = fields.next().unwrap_or("");
        let rest = fields.next().unwrap_or("");
        if tag == "1" && rest.rsplit(' ').next() == Some(rel) {
            return rest[..2].to_owned();
        }
    }
    String::new()
}

/// After an empty RET and a wrong word have each re-asked, `y` still
/// discards --- exactly once: one restore child, the file back at HEAD.
#[test]
fn e7_review1_x_s_question_survives_two_refusals_and_then_discards_once() {
    let (_td, root) = tempdir();
    init_repo(&root);
    write(&root, "long.txt", "changed\n");
    let mut s = editor();
    open_panel(&mut s, &root, "Cargo.toml");
    seat_on(&mut s, "long.txt");
    assert_eq!(xy(&root, "long.txt"), ".M", "positive control");
    let ring_before: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");

    press(&mut s, KeyCode::Char('x'));
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 100);
    type_str(&mut s, "sure");
    press(&mut s, KeyCode::Enter);
    pump_for(&mut s, 100);
    assert_eq!(status(&s), "please answer yes or no");
    assert_eq!(
        std::fs::read_to_string(root.join("long.txt")).unwrap(),
        "changed\n"
    );

    let before = panel_text(&s);
    press(&mut s, KeyCode::Char('y'));
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 15_000, |s| {
            let now = panel_text(s);
            now != before && !now.contains("(refreshing...)")
        }),
        "the panel must refresh after the discard; status: {:?}",
        status(&s)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("long.txt")).unwrap(),
        numbered(60),
        "back at HEAD"
    );
    assert_eq!(xy(&root, "long.txt"), "", "no longer a status row");
    let ring_after: Vec<Vec<String>> = eval(&s, "return pmacs.git._spawn_log");
    let restores = ring_after
        .iter()
        .skip(ring_before.len())
        .filter(|args| args.iter().any(|a| a == "restore"))
        .count();
    assert_eq!(restores, 1, "exactly one restore child ran: {ring_after:?}");
}

// ---------------------------------------------------------------------------
// E7.4 on the GPU route
// ---------------------------------------------------------------------------

#[cfg(feature = "crdt")]
mod gpu_route {
    use super::common::daemon::{TestDaemon, attach_multi};
    use pmacs::crdt::CrdtState;
    use pmacs::protocol::{FrontendEvent, FrontendId, InstanceMessage, Key, KeyEvent, Modifiers};
    use pmacs::rope::CrdtOp as RopeCrdtOp;
    use pmacs::transport::{read_message, write_message};
    use std::time::{Duration, Instant};

    struct Replica {
        stream: std::os::unix::net::UnixStream,
        state: CrdtState,
        fid: FrontendId,
        buffer_id: pmacs::buffer::BufferId,
        popup_rows: usize,
        popup_anchor: Option<u64>,
        cursor: Option<u64>,
    }

    fn attach_replica(daemon: &TestDaemon) -> Replica {
        let (hello, mut stream) = attach_multi(daemon);
        let fid = hello.assigned_frontend_id;
        let (buffer_id, snap) = match read_message::<InstanceMessage>(&mut stream)
            .expect("read initial BufferSnapshot")
        {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => (buffer_id, crdt_snapshot),
            other => panic!("expected initial BufferSnapshot, got {other:?}"),
        };
        let state = CrdtState::new(fid.0).expect("CrdtState::new");
        state.import_snapshot(&snap).expect("import_snapshot");
        Replica {
            stream,
            state,
            fid,
            buffer_id,
            popup_rows: 0,
            popup_anchor: None,
            cursor: None,
        }
    }

    fn send_optimistic_insert(replica: &mut Replica, at: usize, text: &str) {
        let v = replica.state.version();
        replica.state.insert(at, text).expect("insert");
        let op_bytes = replica
            .state
            .export_updates_since(&v)
            .expect("export updates after local mutation");
        write_message(
            &mut replica.stream,
            &FrontendEvent::CrdtOp {
                frontend_id: replica.fid,
                buffer_id: replica.buffer_id,
                op: RopeCrdtOp {
                    peer_id: replica.fid.0,
                    bytes: op_bytes,
                },
            },
        )
        .expect("write CrdtOp");
    }

    fn send_key(replica: &mut Replica, key: Key) {
        write_message(
            &mut replica.stream,
            &FrontendEvent::Key(KeyEvent {
                frontend_id: replica.fid,
                key,
                mods: Modifiers::NONE,
                timestamp_ns: 0,
            }),
        )
        .expect("send Key");
    }

    /// Read daemon messages until `pred` holds, importing ops and
    /// noting the popup's state and the caret as they arrive.
    fn pump_until(
        replica: &mut Replica,
        timeout: Duration,
        what: &str,
        pred: impl Fn(&Replica) -> bool,
    ) {
        let deadline = Instant::now() + timeout;
        loop {
            if pred(replica) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "pump timeout waiting for {what}; text={:?} popup_rows={} anchor={:?} cursor={:?}",
                replica.state.materialize_string(),
                replica.popup_rows,
                replica.popup_anchor,
                replica.cursor
            );
            let remaining = deadline.saturating_duration_since(Instant::now());
            replica
                .stream
                .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
                .ok();
            match read_message::<InstanceMessage>(&mut replica.stream) {
                Ok(InstanceMessage::CrdtOp { buffer_id: b, op }) if b == replica.buffer_id => {
                    let _ = replica.state.import_updates(&op.bytes);
                }
                Ok(InstanceMessage::CursorByte {
                    buffer_id: b,
                    byte_pos,
                }) if b == replica.buffer_id => {
                    replica.cursor = Some(byte_pos);
                }
                Ok(InstanceMessage::CompletionPopup {
                    buffer_id: b,
                    anchor,
                    rows,
                    ..
                }) if b == replica.buffer_id => {
                    replica.popup_anchor = anchor;
                    replica.popup_rows = if anchor.is_some() { rows.len() } else { 0 };
                }
                Ok(_) | Err(_) => {}
            }
        }
    }

    /// The GPU's user types `printl` as six optimistic ops, the popup
    /// opens on the fake's answer (every item carrying an import at
    /// line 3), one more letter goes the same way with the popup still
    /// open, and RET is forwarded as the GPU forwards every command
    /// chord. What the replica then mirrors must be the accept's
    /// replace and the import on its own line, carried across the
    /// letter and the replace, with the daemon's caret at the end of
    /// the completed word.
    #[test]
    fn e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import() {
        let fixture_dir = tempfile::TempDir::new().expect("fixture tempdir");
        let fixture = fixture_dir.path().join("a.rs");
        let body = "fn main() {\n    \n}\n// tail\n";
        std::fs::write(&fixture, body).expect("write a.rs");
        let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
        let init_lua = format!(
            "pmacs.lsp.config = {{}}\npmacs.lsp.config.rust = {{\n  command = {fake:?},\n  env = {{ PMACS_FAKE_LSP_COMPLETION_EXTRA_EDIT = '3:0:// imported\\\\n' }},\n}}\npmacs.buffer.find_or_open({fixture:?})\nfor _, b in ipairs(pmacs.buffer.list()) do\n  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n"
        );
        let daemon = TestDaemon::spawn_with_env_and_init(
            &[
                ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
                ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
            ],
            &init_lua,
        );
        let mut source = attach_replica(&daemon);
        assert_eq!(
            source.state.materialize_string(),
            body,
            "positive control: the daemon holds the fixture"
        );

        let at = "fn main() {\n    ".len();
        for (i, ch) in "printl".chars().enumerate() {
            send_optimistic_insert(&mut source, at + i, &ch.to_string());
        }
        // Let the fake answer and the daemon publish; a replica session
        // may or may not be sent the popup message, so the wait is a
        // window and the popup's state is reported, not required.
        let settle = Instant::now() + Duration::from_millis(1500);
        pump_until(
            &mut source,
            Duration::from_secs(3),
            "the settle window",
            |_| Instant::now() >= settle,
        );
        let popup_seen = (source.popup_rows, source.popup_anchor);
        // One more letter with the popup open: the buffer has changed
        // since the answer the popup shows.
        send_optimistic_insert(&mut source, at + 6, "n");
        pump_until(
            &mut source,
            Duration::from_secs(5),
            "the letter mirrored",
            |r| r.state.materialize_string().contains("println"),
        );
        send_key(&mut source, Key::Enter);
        let expected = "fn main() {\n    println!\n}\n// imported\n// tail\n";
        let newline_instead = "fn main() {\n    println\n\n}\n// tail\n";
        pump_until(
            &mut source,
            Duration::from_secs(10),
            "the accept and its import",
            |r| {
                let t = r.state.materialize_string();
                t == expected || t == newline_instead
            },
        );
        assert_eq!(
            source.state.materialize_string(),
            expected,
            "RET on the GPU route after typing since the request; popup (rows, anchor) seen \
             by this replica before the letter: {popup_seen:?}"
        );
        pump_until(
            &mut source,
            Duration::from_secs(5),
            "the caret after the word",
            |r| r.cursor == Some("fn main() {\n    println!".len() as u64),
        );
    }
}
