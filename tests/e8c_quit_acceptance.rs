// tests/e8c_quit_acceptance.rs --- E8c, quit must exit (#299).

//! `C-x C-c` against a language server that crashed, one that stopped
//! answering, and one that answers; and against the editor's own
//! buffers.
//!
//! **What #299 was, measured.** The TUI opened a Rust file nested
//! 20,000 deep and rust-analyzer aborted on it (`SIGABRT`, half a
//! second after the `didOpen`). The client reported the crash to
//! `*errors*` and restarted the server. The report was an ordinary edit,
//! so `*errors*` was modified, and `editor.quit` asked "Modified buffers
//! exist (*errors*); quit anyway? (y or n)". Nothing answered it, so the
//! editor never exited. No `shutdown` was sent at all: quit does not
//! wait on a server. The same question held quit after `C-x C-b`,
//! `*help*`, a dired listing, a compile run and a project search: every
//! buffer the editor writes for itself was "modified".
//!
//! **The controlled server.** `pmacs_fake_lsp` in `abortonopen` aborts
//! on the first `didOpen` it is sent, as rust-analyzer did, and serves
//! normally once restarted; `stopanswering` is an initialized server that
//! stops reading and writing on a command in the document. That one also
//! ignores `SIGTERM`, through the shell `trap` it runs under, so its end
//! needs the supervisor's `SIGKILL`. Every fake ignores `SIGHUP`, so
//! what ends it is the editor and not the PTY's hang-up
//! ([`fake_server`]).
//!
//! **The bound sits outside the body.** The PTY rows run the shipped
//! binary as a child process and poll its exit from the test's thread
//! with `try_wait`, so a quit that never returns cannot hold up the
//! deadline that judges it.
//!
//! The in-process rows dispatch the keys a user presses and read
//! `EditorState`'s own quit flag; `C-x C-c` invoked by name would pass
//! with the binding gone.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/mod.rs"]
mod common;

use common::pty::{PmacsPty, spawn_pmacs_in_pty};

/// The editor's own teardown is bounded at about four seconds: two of
/// `SIGTERM` grace, then `SIGKILL` and two of reaping
/// (`ProcessSupervisor::shutdown`). The rest is a slow runner's margin;
/// #299 was a quit that never ended.
const QUIT_BOUND: Duration = Duration::from_secs(15);

/// From spawn to the state each PTY row quits from.
const READY_BOUND: Duration = Duration::from_mins(1);

/// How long an orphan may take to be reaped once its parent is gone.
const REAP_BOUND: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The shipped binary in a PTY
// ---------------------------------------------------------------------------

/// One PTY row's world: a `.rs` file, an `init.lua` pointing rust at the
/// fake, and the files the fake and the hook write.
struct World {
    dir: tempfile::TempDir,
}

impl World {
    /// `server` gives the Lua table literal of `pmacs.lsp.config.rust`
    /// for the world's directory.
    fn new(text: &str, server: impl FnOnce(&Path) -> String) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = server(dir.path());
        std::fs::write(dir.path().join("a.rs"), text).expect("write a.rs");
        let config = dir.path().join("config").join("pmacs");
        std::fs::create_dir_all(&config).expect("config dir");
        let status = dir.path().join("status");
        let init = format!(
            r#"
pmacs.lsp.config = {{}}
pmacs.lsp.config.rust = {server}
-- Each frame, what quit depends on, written when it changes: the text
-- of `*errors*`, the question the minibuffer asks, the servers' states.
local last
pmacs.hook.add("process.after-tick", function()
  local errors = ""
  for _, id in ipairs(pmacs.buffer.list()) do
    local d = pmacs.describe.buffer(id)
    if d and d.name == "*errors*" then
      errors = id:slice(0, id:len()):gsub("\n", " | ")
    end
  end
  local prompt = pmacs.minibuffer.is_active() and (pmacs.minibuffer.prompt() or "") or ""
  local servers = {{}}
  for _, info in ipairs(pmacs.lsp.list()) do
    servers[#servers + 1] = info.state and info.state.kind or "?"
  end
  local text = "errors=" .. errors .. "\nprompt=" .. prompt
    .. "\nservers=" .. table.concat(servers, ",") .. "\n"
  if text ~= last then
    last = text
    local f = io.open({part:?}, "w")
    f:write(text)
    f:close()
    os.rename({part:?}, {status:?})
  end
end)
"#,
            part = format!("{}.part", status.display()),
            status = status.display().to_string(),
        );
        std::fs::write(config.join("init.lua"), init).expect("write init.lua");
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn spawn(&self) -> PmacsPty {
        let file = self.path("a.rs").display().to_string();
        let config = self.path("config");
        spawn_pmacs_in_pty(&[&file], &[("XDG_CONFIG_HOME", &config)], 40, 120)
    }

    /// The hook's last record, or empty before its first frame.
    fn status(&self) -> String {
        std::fs::read_to_string(self.path("status")).unwrap_or_default()
    }

    fn field(&self, name: &str) -> String {
        let prefix = format!("{name}=");
        self.status()
            .lines()
            .find_map(|line| line.strip_prefix(&prefix).map(str::to_owned))
            .unwrap_or_default()
    }

    /// Every process id the fake's generations recorded.
    fn server_pids(&self) -> Vec<u32> {
        std::fs::read_to_string(self.path("pids"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }

    /// Poll `ready` until it holds; panic naming `what` and the hook's
    /// record past [`READY_BOUND`].
    fn wait(&self, pty: &mut PmacsPty, what: &str, ready: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + READY_BOUND;
        while !ready(self) {
            assert!(
                pty.wait_for_exit(Duration::ZERO).is_none(),
                "the editor exited while waiting for {what}; last record:\n{}",
                self.status()
            );
            assert!(
                Instant::now() < deadline,
                "precondition: {what} within {READY_BOUND:?}; last record:\n{}",
                self.status()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A row that fails leaves no server behind. Every fake ignores
/// `SIGHUP` and the stopped one parks for good, so one the editor did
/// not end would otherwise outlive the run. Runs after the PTY's own
/// drop, which kills the editor.
impl Drop for World {
    fn drop(&mut self) {
        for pid in self.server_pids() {
            if pid_alive(pid)
                && let Ok(raw) = i32::try_from(pid)
            {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(raw),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }
    }
}

/// The fake as rust's server in `mode`, its generations recorded in
/// `dir`, exec'd by a shell after `prelude`, which ignores signals for
/// it (`trap ''` survives `exec`) and may start a child.
///
/// `SIGHUP` is always ignored. Here the editor leads the PTY's session,
/// so its exit hangs up every process in its group; started from a
/// shell it leads no session and nothing is hung up. Left default, the
/// kernel would end a server the editor left behind and no row could
/// see the leak (a supervisor with its `SIGKILL` removed passed the
/// stopped-server row that way).
fn fake_server(dir: &Path, mode: &str, prelude: &str) -> String {
    format!(
        "{{ command = '/bin/sh', \
           args = {{ '-c', '{prelude}; exec \"$0\"', {fake:?} }}, \
           env = {{ PMACS_FAKE_LSP_MODE = {mode:?}, \
                    PMACS_FAKE_LSP_PID_SINK = {pids:?}, \
                    PMACS_FAKE_LSP_ABORT_ONCE = {once:?}, \
                    PMACS_FAKE_LSP_STOP_SINK = {stop:?} }} }}",
        fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp"),
        pids = dir.join("pids").display().to_string(),
        once = dir.join("aborted").display().to_string(),
        stop = dir.join("stopped").display().to_string(),
    )
}

/// The fake, ended by `SIGTERM` as a server is by default.
fn fake(dir: &Path, mode: &str) -> String {
    fake_server(dir, mode, "trap \"\" HUP")
}

/// The fake deaf to `SIGTERM` as well, so only `SIGKILL` ends it.
fn fake_deaf_to_term(dir: &Path, mode: &str) -> String {
    fake_server(dir, mode, "trap \"\" HUP TERM")
}

/// The fake with an heir: before the shell becomes the server it starts
/// a `sleep` that inherits the server's stdout and stderr and records
/// its pid first in the pid sink. A shim that leaves its real work to a
/// process the editor did not start has this shape.
fn fake_with_heir(dir: &Path) -> String {
    let pids = dir.join("pids").display().to_string();
    fake_server(
        dir,
        "",
        &format!("trap \"\" HUP; sleep 600 & echo $! >> \"{pids}\""),
    )
}

fn pid_alive(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(raw), None).is_ok()
}

/// Every live process descended from `root`, by `ps`, which Linux and
/// macOS both have.
fn descendants(root: u32) -> Vec<u32> {
    let out = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid="])
        .output()
        .expect("ps");
    let rows: Vec<(u32, u32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();
    let mut found = vec![root];
    let mut i = 0;
    while i < found.len() {
        for &(pid, ppid) in &rows {
            if ppid == found[i] && !found.contains(&pid) {
                found.push(pid);
            }
        }
        i += 1;
    }
    found.remove(0);
    found.into_iter().filter(|&pid| pid_alive(pid)).collect()
}

/// Press `C-x C-c` in the PTY and require the editor to exit within
/// [`QUIT_BOUND`], leaving none of `children` behind.
fn quit_and_require_exit(world: &World, pty: &mut PmacsPty, children: &[u32]) {
    assert!(
        !children.is_empty(),
        "precondition: the editor has children to end"
    );
    pty.write_input(b"\x18\x03").expect("type C-x C-c");
    let started = Instant::now();
    let Some(status) = pty.wait_for_exit(QUIT_BOUND) else {
        panic!(
            "C-x C-c did not end the editor within {QUIT_BOUND:?}; the minibuffer asks {:?}, \
             *errors* holds {:?}, servers {:?}",
            world.field("prompt"),
            world.field("errors"),
            world.field("servers"),
        );
    };
    assert!(
        status.success(),
        "quit is a clean exit, got {status:?} after {:?}",
        started.elapsed()
    );
    let deadline = Instant::now() + REAP_BOUND;
    loop {
        let left: Vec<u32> = children.iter().copied().filter(|&p| pid_alive(p)).collect();
        if left.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the editor exited and left {left:?} running (of {children:?}): {}",
            left.iter()
                .map(|p| format!("{p}: {:?}", describe_pid(*p)))
                .collect::<Vec<_>>()
                .join("; ")
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn describe_pid(pid: u32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "pid=,ppid=,stat=,command=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// **#299, with the server it needed.** A server that aborts on the
/// file, restarted and idle, and its crash in `*errors*`: `C-x C-c` ends
/// the editor, and nothing it started outlives it.
///
/// Fails before the fix for the stated reason: the record shows the
/// minibuffer asking "Modified buffers exist (*errors*); quit anyway?".
#[test]
fn e8c_quit_exits_after_a_server_crash_was_reported() {
    let world = World::new("fn main() {}\n", |dir| fake(dir, "abortonopen"));
    let mut pty = world.spawn();
    let pid = pty.process_id().expect("the editor's pid");

    world.wait(
        &mut pty,
        "the crash reported and the restarted server ready",
        |w| {
            w.field("errors").contains("crashed: signal SIGABRT")
                && w.field("servers") == "initialized"
        },
    );
    assert!(
        world.path("aborted").exists() && world.server_pids().len() == 2,
        "precondition: one generation aborted and a second started; pids {:?}",
        world.server_pids()
    );

    let children = descendants(pid);
    quit_and_require_exit(&world, &mut pty, &children);
}

/// **The reviewer's model.** An initialized server that has stopped
/// answering, with requests in flight and `SIGTERM` ignored: `C-x C-c`
/// ends the editor within the bound and the server with it. Quit sends
/// the server nothing and waits on no answer; what ends it is the
/// supervisor's `SIGKILL` after its grace.
#[test]
fn e8c_quit_exits_when_an_initialized_server_stops_answering() {
    let world = World::new("fn main() {}\n// PMACS-STOP-ANSWERING\n", |dir| {
        fake_deaf_to_term(dir, "stopanswering")
    });
    let mut pty = world.spawn();
    let pid = pty.process_id().expect("the editor's pid");

    world.wait(&mut pty, "the server initialized, then stopped", |w| {
        w.path("stopped").exists() && w.field("servers") == "initialized"
    });
    let servers = world.server_pids();
    assert!(
        servers.len() == 1 && pid_alive(servers[0]),
        "precondition: one server, alive and silent; pids {servers:?}"
    );

    let children = descendants(pid);
    assert!(
        children.contains(&servers[0]),
        "precondition: the server is the editor's child; children {children:?}, server {servers:?}"
    );
    quit_and_require_exit(&world, &mut pty, &children);
}

/// **Normal shutdown keeps working.** A server that answers: `C-x C-c`
/// ends the editor and the server.
#[test]
fn e8c_quit_against_an_answering_server_exits_and_ends_it() {
    let world = World::new("fn main() {}\n", |dir| fake(dir, ""));
    let mut pty = world.spawn();
    let pid = pty.process_id().expect("the editor's pid");

    world.wait(&mut pty, "the server ready", |w| {
        w.field("servers") == "initialized"
    });
    assert!(
        world.field("errors").is_empty(),
        "precondition: nothing reported; *errors* holds {:?}",
        world.field("errors")
    );
    let children = descendants(pid);
    let servers = world.server_pids();
    assert!(
        servers.len() == 1 && children.contains(&servers[0]),
        "precondition: one server, the editor's child; children {children:?}, server {servers:?}"
    );
    quit_and_require_exit(&world, &mut pty, &children);
}

/// **A server whose child holds its output.** The server ends at quit's
/// `SIGTERM`; the child it started keeps the write end of its stdout
/// and stderr. `C-x C-c` ends the editor within the bound anyway, and
/// the server with it. Before E8c the editor never exited: dropping the
/// dead server's handles joined a reader blocked on a pipe the child
/// held, from inside the supervisor's shutdown tick. The child itself
/// outlives the editor, a descendant it did not start, which is #356's
/// question; the world's drop ends it.
#[test]
fn e8c_quit_exits_when_a_server_s_child_holds_its_output() {
    let world = World::new("fn main() {}\n", fake_with_heir);
    let mut pty = world.spawn();
    let pid = pty.process_id().expect("the editor's pid");

    world.wait(&mut pty, "the server ready", |w| {
        w.field("servers") == "initialized"
    });
    let pids = world.server_pids();
    let [heir, server] = pids[..] else {
        panic!("precondition: the heir's pid, then the server's; got {pids:?}");
    };
    assert!(
        pid_alive(heir) && pid_alive(server),
        "precondition: the server and its heir are running; pids {pids:?}"
    );
    let children: Vec<u32> = descendants(pid)
        .into_iter()
        .filter(|&p| p != heir)
        .collect();
    assert!(
        children.contains(&server),
        "precondition: the server is the editor's child; children {children:?}, server {server}"
    );
    quit_and_require_exit(&world, &mut pty, &children);
    assert!(
        pid_alive(heir),
        "the heir still held the pipe when the editor exited, so the exit did not wait on it"
    );
}

// ---------------------------------------------------------------------------
// In-process: the editor's own buffers
// ---------------------------------------------------------------------------

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

/// Length of the buffer named `name`, or `None` when there is none.
fn named_len(s: &EditorState, name: &str) -> Option<i64> {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do
               local d = pmacs.describe.buffer(id)
               if d.name == {name:?} then return d.length end
             end
             return nil"
        ),
    )
}

/// An editor with isolated roots, no developer `init.lua` and no
/// language servers.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

/// Drive frames until `pred` holds, or `false` past `timeout`.
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

/// A visited `.txt` file in a fresh directory, so nothing the user
/// wrote is at stake and no server starts.
fn visiting_a_clean_file(dir: &Path) -> EditorState {
    let s = editor();
    let path = dir.join("notes.txt");
    std::fs::write(&path, "needle in the notes\n").unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
    s
}

/// `C-x C-c`, and the editor quits without asking anything.
fn require_quit_without_a_question(s: &mut EditorState, buffer: &str) {
    assert!(
        named_len(s, buffer).is_some_and(|len| len > 0),
        "precondition: {buffer} exists and holds the editor's text"
    );
    ctrl(s, 'x');
    ctrl(s, 'c');
    assert!(
        s.core.borrow().quit,
        "C-x C-c must quit with nothing the user wrote unsaved; the minibuffer asks {:?}",
        prompt(s)
    );
}

/// `C-x C-b`'s `*buffer-list*`, written through `set_generated_contents`.
#[test]
fn e8c_quit_is_not_held_by_the_buffer_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'b');
    assert_eq!(
        active_name(&s),
        "*buffer-list*",
        "precondition: C-x C-b shows the list"
    );
    require_quit_without_a_question(&mut s, "*buffer-list*");
}

/// `*help*`, written by `show_help_text` with the Lua mutators.
#[test]
fn e8c_quit_is_not_held_by_help() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    alt(&mut s, 'x');
    type_str(&mut s, "help.list-commands");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        "*help*",
        "precondition: the command shows *help*"
    );
    require_quit_without_a_question(&mut s, "*help*");
}

/// A dired listing, `C-x d RET`.
#[test]
fn e8c_quit_is_not_held_by_a_dired_listing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('d'));
    press(&mut s, KeyCode::Enter);
    // The listing is read off the main thread (`open_async`).
    pump_until(&mut s, Duration::from_secs(20), |s| {
        active_name(s).starts_with("*dired:")
    });
    let name = active_name(&s);
    assert!(
        name.starts_with("*dired:"),
        "precondition: C-x d RET lists; active is {name:?}"
    );
    require_quit_without_a_question(&mut s, &name);
}

/// A reported error, the route #299's crash took into `*errors*`.
#[test]
fn e8c_quit_is_not_held_by_a_reported_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    exec(
        &s,
        "pmacs.error('LSP: default-rust crashed: signal SIGABRT', 'e8c')",
    );
    require_quit_without_a_question(&mut s, "*errors*");
}

/// A compile run's `*compilation*`, written by `compile.lua`.
#[test]
fn e8c_quit_is_not_held_by_compile_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    exec(
        &s,
        &format!(
            "pmacs.compile.run('echo e8c-output', {{ cwd = {:?}, display = 'current' }})",
            dir.path().display().to_string()
        ),
    );
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| {
            eval::<String>(
                s,
                "for _, id in ipairs(pmacs.buffer.list()) do
                   if pmacs.describe.buffer(id).name == '*compilation*' then
                     return id:slice(0, id:len())
                   end
                 end
                 return ''",
            )
            .contains("[compile ")
        }),
        "precondition: the run reaches its exit marker"
    );
    require_quit_without_a_question(&mut s, "*compilation*");
}

/// A project search's `*search-results*`, written by its producer.
#[test]
fn e8c_quit_is_not_held_by_search_results() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    exec(
        &s,
        &format!(
            "pmacs.project.search('needle', {{ root = {:?} }})",
            dir.path().display().to_string()
        ),
    );
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| {
            eval::<String>(
                s,
                "for _, id in ipairs(pmacs.buffer.list()) do
                   if pmacs.describe.buffer(id).name == '*search-results*' then
                     return id:slice(0, id:len())
                   end
                 end
                 return ''",
            )
            .contains("notes.txt")
        }),
        "precondition: the search lists its match"
    );
    require_quit_without_a_question(&mut s, "*search-results*");
}

/// **The other side.** Text the user typed into `*errors*` is theirs:
/// an error appended after it keeps the buffer modified, and quit asks,
/// naming it. An append that cleared the buffer outright would lose it.
#[test]
fn e8c_text_typed_into_errors_still_holds_quit() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    exec(&s, "pmacs.error('first', 'e8c')");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "*errors*");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        "*errors*",
        "precondition: C-x b reaches *errors*"
    );
    press(&mut s, KeyCode::Char('X'));
    assert!(
        eval::<bool>(
            &s,
            "return pmacs.describe.buffer(pmacs.window.buffer()).modified"
        ),
        "precondition: the user's keystroke modifies *errors*"
    );
    exec(&s, "pmacs.error('second', 'e8c')");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'c');
    assert!(
        !s.core.borrow().quit,
        "the user's text in *errors* is unsaved work; quit must ask"
    );
    assert!(
        prompt(&s).contains("*errors*"),
        "and the question names it; it asks {:?}",
        prompt(&s)
    );
}

/// `q` in `*help*` (`buffer.kill-this`) kills it without asking, by the
/// same fact: the editor's text is no one's unsaved change.
#[test]
fn e8c_killing_help_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = visiting_a_clean_file(dir.path());
    alt(&mut s, 'x');
    type_str(&mut s, "help.list-commands");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        "*help*",
        "precondition: the command shows *help*"
    );
    press(&mut s, KeyCode::Char('q'));
    assert!(
        named_len(&s, "*help*").is_none(),
        "q kills *help* without a question; the minibuffer asks {:?}",
        prompt(&s)
    );
}
