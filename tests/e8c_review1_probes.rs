// tests/e8c_review1_probes.rs --- E8c review 1's behavioral probes.

//! The states a user reaches that E8c's witnesses did not choose, left
//! for the fix round to commit as witnesses or to retire.
//!
//! E8c changed what counts as unsaved work: a buffer the editor writes
//! for itself reads as unmodified after its writes. The risk is the
//! other direction, a user's own text cleared from the question. These
//! rows put text a user typed beside each writer E8c touched, and ask
//! quit about it:
//!
//! - `*scratch*`, the buffer every session starts in, while each of
//!   E8c's writers runs elsewhere;
//! - `*help*`, which a user can type into and the editor redraws whole;
//! - a REPL, whose typed input lives beside its process's output and
//!   which the owner has ruled keeps asking.
//!
//! And two about E8c's second fix, a reaped process whose child holds
//! its output. The join is now bounded: one row asks whether the user is
//! told when the bound is reached, and a PTY row whether the bound is
//! one for the teardown or one per such server.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/mod.rs"]
mod common;

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

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
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

fn prompt(s: &EditorState) -> String {
    eval::<Option<String>>(s, "return pmacs.minibuffer.prompt()").unwrap_or_default()
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

/// The text of the buffer named `name`, or empty when there is none.
fn named_text(s: &EditorState, name: &str) -> String {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do
               if pmacs.describe.buffer(id).name == {name:?} then
                 return id:slice(0, id:len())
               end
             end
             return ''"
        ),
    )
}

/// Whether the buffer named `name` reads as modified.
fn named_modified(s: &EditorState, name: &str) -> bool {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do
               local d = pmacs.describe.buffer(id)
               if d.name == {name:?} then return d.modified end
             end
             return false"
        ),
    )
}

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn pump_until(s: &mut EditorState, timeout: Duration, pred: impl Fn(&EditorState) -> bool) -> bool {
    let stop = Instant::now() + timeout;
    while !pred(s) {
        if Instant::now() >= stop {
            return false;
        }
        s.tick_processes();
        s.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// `C-x C-c` must ask, and name `buffer`.
fn require_quit_asks_naming(s: &mut EditorState, buffer: &str) {
    ctrl(s, 'x');
    ctrl(s, 'c');
    assert!(
        !s.core.borrow().quit,
        "text the user typed into {buffer} is unsaved; quit must ask"
    );
    assert!(
        prompt(s).contains(buffer),
        "and the question names {buffer}; it asks {:?}",
        prompt(s)
    );
}

/// **`*scratch*` through every writer E8c touched.** The user types into
/// `*scratch*`; then `C-x C-b`, `*help*`, a dired listing, a reported
/// error, a compile run and a project search each write their own
/// buffers and mark them clean. None of that may clear `*scratch*`:
/// quit asks, naming it, and its text is intact.
#[test]
fn r1_scratch_text_outlives_every_generated_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "needle\n").unwrap();
    let mut s = editor();
    assert_eq!(
        active_name(&s),
        "*scratch*",
        "precondition: the session starts in *scratch*"
    );
    type_str(&mut s, "my unsaved thought");
    assert!(
        named_modified(&s, "*scratch*"),
        "precondition: typing modifies *scratch*"
    );

    let root = dir.path().display().to_string();
    // C-x C-b: `set_generated_contents`.
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'b');
    assert_eq!(
        active_name(&s),
        "*buffer-list*",
        "precondition: C-x C-b lists"
    );
    // *help*: the Lua mutators, then `buf:mark_clean()`.
    alt(&mut s, 'x');
    type_str(&mut s, "help.list-commands");
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), "*help*", "precondition: help shows");
    // A reported error: `append_to_errors_buffer`.
    exec(
        &s,
        "pmacs.error('LSP: default-rust crashed: signal SIGABRT', 'r1')",
    );
    assert!(
        named_text(&s, "*errors*").contains("crashed: signal SIGABRT"),
        "precondition: the error is in *errors*"
    );
    // A compile run and a project search, each to its end.
    exec(
        &s,
        &format!("pmacs.compile.run('echo r1-output', {{ cwd = {root:?}, display = 'current' }})"),
    );
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| named_text(
            s,
            "*compilation*"
        )
        .contains("[compile ")),
        "precondition: the compile run reaches its exit marker"
    );
    exec(
        &s,
        &format!("pmacs.project.search('needle', {{ root = {root:?} }})"),
    );
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| named_text(
            s,
            "*search-results*"
        )
        .contains("notes.txt")),
        "precondition: the search lists its match"
    );
    // A dired listing, read off the main thread.
    exec(
        &s,
        &format!("pmacs.async(function() pmacs.dired.open({root:?}) end)"),
    );
    pump_until(&mut s, Duration::from_secs(20), |s| {
        active_name(s).starts_with("*dired:")
    });
    assert!(
        active_name(&s).starts_with("*dired:"),
        "precondition: dired lists; active is {:?}",
        active_name(&s)
    );

    assert!(
        named_text(&s, "*scratch*").contains("my unsaved thought"),
        "*scratch* keeps the user's text"
    );
    assert!(
        named_modified(&s, "*scratch*"),
        "no editor write may clear the user's *scratch*"
    );
    require_quit_asks_naming(&mut s, "*scratch*");
}

/// **`*help*`, typed into.** The one generated buffer the user can type
/// into keeps the text as unsaved work until the editor next draws it:
/// quit asks, naming `*help*`. The next help command then replaces the
/// text and asks nothing, as it did before E8c, and quit no longer asks.
#[test]
fn r1_text_typed_into_help_holds_quit_until_help_is_redrawn() {
    let mut s = editor();
    alt(&mut s, 'x');
    type_str(&mut s, "help.list-commands");
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), "*help*", "precondition: help shows");
    assert!(
        !named_modified(&s, "*help*"),
        "precondition: E8c's *help* reads clean"
    );
    type_str(&mut s, "NOTE");
    assert!(
        named_text(&s, "*help*").contains("NOTE"),
        "precondition: the text lands"
    );
    assert!(
        named_modified(&s, "*help*"),
        "typing into *help* modifies it"
    );
    require_quit_asks_naming(&mut s, "*help*");
    press(&mut s, KeyCode::Char('n'));

    alt(&mut s, 'x');
    type_str(&mut s, "help.list-commands");
    press(&mut s, KeyCode::Enter);
    assert!(
        prompt(&s).is_empty(),
        "the redraw asks nothing; it asks {:?}",
        prompt(&s)
    );
    assert!(
        !named_text(&s, "*help*").contains("NOTE"),
        "the redraw replaced the user's text"
    );
    assert!(!named_modified(&s, "*help*"), "and left *help* clean");
}

/// **A REPL keeps asking** (the owner's ruling). Its typed input lives
/// beside its process's output, and E8c's writers do not reach it: the
/// output alone leaves it modified, and so does input not yet sent.
#[test]
fn r1_a_repl_still_asks_at_quit() {
    let mut s = editor();
    exec(
        &s,
        "_G.r1_repl = pmacs.repl.spawn { argv = { '/bin/sh', '-c', 'echo r1-banner; exec cat' }, \
         name = '*r1-repl*' }",
    );
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| named_text(
            s,
            "*r1-repl*"
        )
        .contains("r1-banner")),
        "precondition: the REPL's output lands; it holds {:?}",
        named_text(&s, "*r1-repl*")
    );
    assert!(
        named_modified(&s, "*r1-repl*"),
        "the REPL's output alone reads as unsaved work"
    );
    require_quit_asks_naming(&mut s, "*r1-repl*");
    press(&mut s, KeyCode::Char('n'));

    assert_eq!(
        active_name(&s),
        "*r1-repl*",
        "precondition: the REPL is shown"
    );
    assert!(
        prompt(&s).is_empty(),
        "precondition: `n` closed the question; it asks {:?}",
        prompt(&s)
    );
    // Input not yet sent, through the edit path the REPL's intercept
    // guards, as the REPL suites type it (`m6_4_repl_acceptance`).
    exec(
        &s,
        "_G.r1_repl:buffer_id():insert(_G.r1_repl:prompt_end(), 'unsent input')",
    );
    assert!(
        named_text(&s, "*r1-repl*").contains("unsent input"),
        "precondition: the input lands in the REPL buffer; it holds {:?}",
        named_text(&s, "*r1-repl*")
    );
    require_quit_asks_naming(&mut s, "*r1-repl*");
}

/// A process whose own child keeps its stdout and stderr, the shape of
/// E8c's heir: the shell records the `sleep`'s pid and exits at once.
/// The heir lives 20 s, not for good, so that without E8c's bound the
/// reaping tick returns when it ends and the row fails on its stall
/// instead of holding the test thread (the tick is not interruptible).
fn spawn_process_with_an_heir(s: &EditorState, pid_file: &Path) {
    exec(
        s,
        &format!(
            "_G.r1_proc = pmacs.process.spawn {{ label = 'r1-heir-holder', purpose = 'review probe', \
             command = '/bin/sh', args = {{ '-c', 'sleep 20 & echo $! > {pid}; exit 0' }} }}",
            pid = pid_file.display()
        ),
    );
}

/// Ends the heir however the row ends.
struct Heir(std::path::PathBuf);

impl Drop for Heir {
    fn drop(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.0)
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

/// **The bound reached, and what the user is told.** A process exits
/// while its child holds its output. The tick that reaps it waits out
/// the final drain, then `READER_JOIN_BOUND`, on the main thread, and
/// leaves the reader blocked on the child's pipe. The stall is bounded
/// now (it was not before E8c), so the first assertion holds. The rest
/// ask whether anything says so: the editor froze for about four
/// seconds and left a thread and two descriptors behind for as long as
/// the child lives. The fix round says so where a failure is said, the
/// status line and `*errors*` with the mode line's mark, naming the
/// process and what was left.
#[test]
fn r1_a_reader_left_behind_is_told() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("heir.pid");
    let _heir = Heir(pid_file.clone());
    let mut s = editor();
    spawn_process_with_an_heir(&s, &pid_file);

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut longest = Duration::ZERO;
    loop {
        let started = Instant::now();
        s.tick_processes();
        longest = longest.max(started.elapsed());
        let kind: String = eval(
            &s,
            "return (pmacs.process.status(_G.r1_proc) or {}).kind or 'gone'",
        );
        if kind == "terminated" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "precondition: the process ends; it is {kind:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        std::fs::read_to_string(&pid_file).is_ok(),
        "precondition: the heir was started"
    );
    assert!(
        longest >= Duration::from_secs(3) && longest < Duration::from_secs(10),
        "the reaping tick waits out the drain and the join bound, about four \
         seconds; it took {longest:?}"
    );
    let errors = named_text(&s, "*errors*");
    assert!(
        errors.contains("r1-heir-holder"),
        "the editor stalled {longest:?} and left a reader behind; nothing says so. \
         *errors* holds {errors:?}"
    );
    assert!(
        errors.contains(
            "[process] r1-heir-holder exited, but another process still holds its output"
        ) && errors.contains("reader thread"),
        "*errors* says what happened and what was left; it holds {errors:?}"
    );
    let status = s.core.borrow().status.clone();
    assert!(
        status.starts_with("process: r1-heir-holder exited"),
        "and so does the status line; it reads {status:?}"
    );
    let unread: usize = eval(&s, "return pmacs.error_log.unread()");
    assert_eq!(unread, 1, "the mode line's mark counts it");
}

// ---------------------------------------------------------------------------
// The shipped binary in a PTY: two servers, each with an heir
// ---------------------------------------------------------------------------

/// The fake as a server under a shell that ignores `SIGHUP` and starts an
/// heir, a `sleep` holding the server's stdout and stderr, its pid first
/// in `pids` (E8c's `fake_with_heir`, for any language).
fn fake_with_heir(dir: &Path) -> String {
    let pids = dir.join("pids").display().to_string();
    format!(
        "{{ command = '/bin/sh', \
           args = {{ '-c', 'trap \"\" HUP; sleep 600 & echo $! >> \"{pids}\"; exec \"$0\"', {fake:?} }}, \
           env = {{ PMACS_FAKE_LSP_MODE = '', PMACS_FAKE_LSP_PID_SINK = {pids:?} }} }}",
        fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp"),
    )
}

/// Kills every pid the fakes recorded, however the row ends.
struct Pids(std::path::PathBuf);

impl Drop for Pids {
    fn drop(&mut self) {
        for line in std::fs::read_to_string(&self.0).unwrap_or_default().lines() {
            if let Ok(pid) = line.trim().parse::<i32>() {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }
    }
}

/// **The bound is per server.** E8c's suite says the editor's teardown
/// is "bounded at about four seconds", and with one heir it is: the
/// reaping tick waits the final drain, 2 s, then `READER_JOIN_BOUND`,
/// 2 s. With two servers each holding an heir, the same tick reaps them
/// one after the other, so quit takes about eight, with nothing on the
/// screen after the editor leaves it. The row holds the suite's own
/// figure, about four seconds and a margin, against two servers.
#[test]
fn r1_two_servers_with_heirs_quit_within_one_bound() {
    let dir = tempfile::tempdir().unwrap();
    let _pids = Pids(dir.path().join("pids"));
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    let c_file = dir.path().join("b.c");
    std::fs::write(&c_file, "int main(void) { return 0; }\n").unwrap();
    let config = dir.path().join("config").join("pmacs");
    std::fs::create_dir_all(&config).unwrap();
    let status = dir.path().join("status");
    let server = fake_with_heir(dir.path());
    std::fs::write(
        config.join("init.lua"),
        format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.lsp.config.rust = {server}\n\
             pmacs.lsp.config.c = {server}\n\
             pmacs.buffer.find_or_open({c:?})\n\
             pmacs.hook.add('process.after-tick', function()\n\
               local s = {{}}\n\
               for _, info in ipairs(pmacs.lsp.list()) do\n\
                 s[#s + 1] = info.state and info.state.kind or '?'\n\
               end\n\
               local f = io.open({part:?}, 'w')\n\
               f:write(table.concat(s, ','))\n\
               f:close()\n\
               os.rename({part:?}, {status:?})\n\
             end)\n",
            c = c_file.display().to_string(),
            part = format!("{}.part", status.display()),
            status = status.display().to_string(),
        ),
    )
    .unwrap();
    let file = dir.path().join("a.rs").display().to_string();
    let config_root = dir.path().join("config");
    let mut pty =
        common::pty::spawn_pmacs_in_pty(&[&file], &[("XDG_CONFIG_HOME", &config_root)], 40, 120);

    let deadline = Instant::now() + Duration::from_mins(1);
    while std::fs::read_to_string(&status).unwrap_or_default() != "initialized,initialized" {
        assert!(
            Instant::now() < deadline,
            "precondition: two servers initialized; status {:?}",
            std::fs::read_to_string(&status).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let pids = std::fs::read_to_string(dir.path().join("pids")).unwrap_or_default();
    assert_eq!(
        pids.lines().count(),
        4,
        "precondition: two heirs and two servers recorded; pids {pids:?}"
    );

    pty.write_input(b"\x18\x03").expect("type C-x C-c");
    let started = Instant::now();
    let status = pty
        .wait_for_exit(Duration::from_secs(30))
        .expect("the editor exits within 30 s");
    let took = started.elapsed();
    assert!(status.success(), "quit is a clean exit, got {status:?}");
    assert!(
        took < Duration::from_secs(6),
        "two servers with heirs: quit took {took:?}, where E8c's suite states the \
         teardown is bounded at about four seconds"
    );
}
