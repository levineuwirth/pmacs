// tests/e10v_2_restart_bound_acceptance.rs --- E10v.2: the restart
// bound, before #355.

//! The owner's condition of 2026-10-10 on #355: whoever sends a restarted
//! server its documents first gives the client a restart bound, or a
//! server that crashes on a file (rust-analyzer on #299's) is restarted,
//! sent the file and crashes again, for ever. The manager restarted after
//! a fixed 500 ms and never stopped; and any LSP command run while the
//! server was down forgot it and spawned another under a new id, so even
//! a bound kept per id would have been started over by a keystroke.
//!
//! The bound is E7i's for parse workers: a back-off doubling from one
//! second, and after three crashes in a row no restart until the file is
//! opened again, said once. "In a row" is the owner's ruling of
//! 2026-11-23 (E10v review 1's Low 4, option (c)): a generation that
//! crashes within `CRASH_COUNT_WINDOW` (30 s) of its start counts toward
//! the three whatever it answered, a handshake, a result or an error; one
//! that lived longer starts the count, and the back-off, over. A held
//! server, backing off or stopped, is not replaced by a command.
//!
//! Each row counts generations by the fake's pid sink, one line per
//! process started.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;

mod common;
#[path = "support/mod.rs"]
mod support;

use common::iso;

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_until(s: &mut EditorState, secs: u64, mut done: impl FnMut(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if done(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settle(s: &mut EditorState, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        tick(s);
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct Fixture {
    dir: PathBuf,
    file: PathBuf,
    pids: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    Fixture {
        pids: dir.join("pids.txt"),
        dir,
        file,
    }
}

impl Fixture {
    fn editor(&self, mode: &str) -> EditorState {
        self.editor_env(mode, "")
    }

    /// The editor, its rust server the fake in `mode` with `extra` more
    /// environment entries (a Lua table body, each ending in a comma).
    fn editor_env(&self, mode: &str, extra: &str) -> EditorState {
        let s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(self.dir.clone()));
        exec(
            &s,
            &format!(
                "pmacs.lsp.config = {{}}\n\
                 pmacs.project.set_search_boundary({dir:?})\n\
                 pmacs.lsp.config.rust = {{ command = '{cmd}', env = {{ {extra} \
                   PMACS_FAKE_LSP_MODE = '{mode}', PMACS_FAKE_LSP_PID_SINK = '{pids}' }} }}\n",
                dir = self.dir.display().to_string(),
                cmd = fake_lsp_path(),
                pids = self.pids.display(),
            ),
        );
        self.open(&s);
        s
    }

    fn open(&self, s: &EditorState) {
        exec(
            s,
            &format!(
                "pmacs.buffer.find_or_open({:?})",
                self.file.display().to_string()
            ),
        );
    }

    /// Server processes started so far.
    fn generations(&self) -> usize {
        std::fs::read_to_string(&self.pids).map_or(0, |t| t.lines().count())
    }
}

/// Every server the manager holds: `id:kind:crashes:pending:stopped`.
fn servers(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local out = {}
         for _, i in ipairs(pmacs.lsp.list()) do
           out[#out + 1] = string.format('%s:%s:%s:%s:%s', tostring(i.id),
             tostring(i.state and i.state.kind), tostring(i.crashes),
             tostring(i.restart_pending), tostring(i.crash_stopped))
         end
         return out",
    )
}

fn ids(s: &EditorState) -> Vec<String> {
    servers(s)
        .iter()
        .map(|r| r.split(':').next().unwrap().to_owned())
        .collect()
}

fn stopped(s: &EditorState) -> bool {
    servers(s).iter().any(|r| r.ends_with(":true"))
}

fn errors(s: &EditorState) -> String {
    s.lua_host.errors_buffer_text()
}

const STOP_SENTENCE: &str =
    "not restarted until a file of its project is opened again, or M-x lsp.restart";

/// A server that crashes on every start, having answered `initialize`
/// and nothing else (the fake's `crash` mode), is started three times,
/// one second and then two apart, and not again: the third crash says so
/// once, and no fourth process starts in the four seconds a fourth
/// back-off would take and two more. The handshake each generation
/// answered leaves the count rising. At the base the server is started
/// again every 500 ms, for ever.
#[test]
fn e10v_2_a_server_crashing_on_every_start_stops_after_three_and_says_so_once() {
    let fx = fixture("stops");
    let mut s = fx.editor("crash");
    let t0 = Instant::now();
    let mut counts = Vec::new();
    let reached = pump_until(&mut s, 15, |s| {
        let row = servers(s);
        if counts.last() != Some(&row) {
            counts.push(row);
        }
        stopped(s)
    });
    let when = t0.elapsed();
    settle(&mut s, 6000);
    let generations = fx.generations();
    assert!(
        reached && generations == 3,
        "the server stops after its third crash in a row; stopped {reached} after {when:?}, \
         {generations} processes started by 6 s later; states {counts:?}"
    );
    // A handshake alone is not something between two crashes: the count
    // rose through 1 and 2 to 3 although each generation answered
    // `initialize`.
    let crashes: Vec<String> = counts
        .iter()
        .filter_map(|row| row.first())
        .map(|r| r.split(':').nth(2).unwrap().to_owned())
        .collect();
    for n in ["1", "2", "3"] {
        assert!(
            crashes.iter().any(|c| c == n),
            "the crash count passed through {n}; counts {crashes:?}"
        );
    }
    assert!(
        when >= Duration::from_millis(2900),
        "two back-offs, one second and two, before the third start: stopped after {when:?}"
    );
    let errors = errors(&s);
    assert_eq!(
        errors.matches(STOP_SENTENCE).count(),
        1,
        "the stop is said once: {errors:?}"
    );
    assert_eq!(
        errors.matches("LSP: default-rust crashed").count(),
        3,
        "each crash is reported: {errors:?}"
    );
}

/// No command starts the count over. During the back-off after the first
/// crash, `lsp.hover` finds no server to ask and starts none; after the
/// stop, `lsp.hover`, `lsp.find-references`, the attach `C-c` commands
/// resolve through, and a switch away and back start none either. The
/// manager keeps the one id, three processes are started in all, and the
/// command says why it asked nothing. At the base the first `lsp.hover`
/// forgets the crashed server and spawns another under a new id.
#[test]
fn e10v_2_a_command_during_the_back_off_or_after_the_stop_starts_no_server_under_a_new_id() {
    let fx = fixture("backdoor");
    let mut s = fx.editor("crash");
    assert!(
        pump_until(&mut s, 10, |_| fx.generations() >= 1),
        "premise: the first server started"
    );
    let first = ids(&s);
    assert_eq!(first.len(), 1, "premise: one server; {:?}", servers(&s));
    assert!(
        pump_until(&mut s, 10, |s| servers(s)
            .iter()
            .any(|r| r.contains(":crashed:1:true:"))),
        "premise: the first crash, its restart pending; {:?}",
        servers(&s)
    );
    exec(&s, "pmacs.command.invoke('lsp.hover')");
    settle(&mut s, 100);
    assert_eq!(
        ids(&s),
        first,
        "a command during the back-off started no server under a new id; servers {:?}; \
         processes {}",
        servers(&s),
        fx.generations()
    );
    let status = s.core.borrow().status.clone();
    assert!(
        status.contains("crashed; restarting"),
        "the command says the server is restarting: {status:?}"
    );
    assert!(
        pump_until(&mut s, 15, stopped),
        "premise: the stop; servers {:?}",
        servers(&s)
    );
    for command in ["lsp.hover", "lsp.find-references", "lsp.hover"] {
        exec(&s, &format!("pmacs.command.invoke({command:?})"));
        settle(&mut s, 100);
    }
    exec(&s, "pmacs.lsp._attach_buffer()");
    exec(
        &s,
        "for _, b in ipairs(pmacs.buffer.list()) do \
           if b:name() == '*scratch*' then pmacs.window.switch_buffer(b) end \
         end",
    );
    settle(&mut s, 50);
    fx.open(&s);
    settle(&mut s, 2500);
    assert_eq!(
        (ids(&s), fx.generations()),
        (first, 3),
        "after the stop no command, attach or switch started a server; servers {:?}",
        servers(&s)
    );
    exec(&s, "pmacs.command.invoke('lsp.hover')");
    let status = s.core.borrow().status.clone();
    assert!(
        status.contains(
            "stopped after 3 crashes in a row; open a file of its project, or M-x lsp.restart"
        ),
        "the command says why it asked nothing: {status:?}"
    );
}

/// A server that answers a real result and then crashes at once (the
/// fake's `crashafteranswer`: one request of the user's work answered
/// with its result, then an exit) stops after three, as one that answers
/// nothing does: each generation died within the window of its start, so
/// what it answered does not end the run (the owner's ruling (c)). The
/// rows of E10v.2 as first approved had it restarted for ever, at count
/// one, at one second; this row stated that rule until fix round 1.
#[test]
fn e10v_2_a_server_that_crashes_soon_after_serving_stops_after_three() {
    let fx = fixture("serves");
    let mut s = fx.editor("crashafteranswer");
    let mut counts = Vec::new();
    let mut asked = 0;
    let reached = pump_until(&mut s, 20, |s| {
        let row = servers(s);
        if counts.last() != Some(&row) {
            counts.push(row.clone());
        }
        // A request of the user's work once each generation is up, so
        // each one has something to answer before it crashes.
        if row.iter().any(|r| r.contains(":initialized:")) && asked < fx.generations() {
            asked = fx.generations();
            exec(s, "pmacs.command.invoke('lsp.hover')");
        }
        stopped(s)
    });
    settle(&mut s, 5000);
    assert!(
        reached && fx.generations() == 3,
        "a server that serves and crashes at once stops after three; stopped {reached}, \
         {} processes; states {counts:?}",
        fx.generations()
    );
    assert!(
        counts.iter().flatten().any(|r| r.contains(":initialized:")),
        "premise: a generation came up to answer; states {counts:?}"
    );
    assert_eq!(
        errors(&s).matches(STOP_SENTENCE).count(),
        1,
        "the stop is said once: {:?}",
        errors(&s)
    );
}

/// A generation that lived past the window starts the count over: the
/// fake's `crashlater` dies 0.3 s after its first two starts and 31 s
/// after its third. The first two crashes count one and two; the third,
/// past `CRASH_COUNT_WINDOW`, counts one again and is restarted after
/// the first back-off, where a count of three would have stopped it.
/// Each generation answers only `initialize`, so at the base of this
/// round (`8bbb338`, "nothing answered between") the third crash is the
/// third in a row and the server stops.
#[test]
fn e10v_2_a_crash_after_the_window_starts_the_count_over() {
    let fx = fixture("window");
    let mut s = fx.editor_env(
        "crashlater",
        "PMACS_FAKE_LSP_LIFETIMES_MS = '300,300,31000,300',",
    );
    let mut counts: Vec<Vec<String>> = Vec::new();
    let record = |counts: &mut Vec<Vec<String>>, s: &EditorState| {
        let row = servers(s);
        if counts.last() != Some(&row) {
            counts.push(row);
        }
    };
    assert!(
        pump_until(&mut s, 15, |s| {
            record(&mut counts, s);
            fx.generations() == 3
        }),
        "premise: a third generation started; states {counts:?}"
    );
    let third = Instant::now();
    assert!(
        pump_until(&mut s, 45, |s| {
            record(&mut counts, s);
            fx.generations() == 4 || stopped(s)
        }),
        "the third generation crashed and the bound decided; states {counts:?}"
    );
    let lived = third.elapsed();
    assert!(
        lived >= Duration::from_secs(30),
        "premise: the third generation outlived the window ({lived:?}); states {counts:?}"
    );
    let rows: Vec<&String> = counts.iter().flatten().collect();
    assert!(
        !stopped(&s) && fx.generations() == 4,
        "a crash after the window is restarted, not the third in a row; {} processes; \
         states {counts:?}",
        fx.generations()
    );
    assert!(
        rows.iter().any(|r| r.contains(":crashed:2:")),
        "premise: the two quick crashes counted two; states {counts:?}"
    );
    let after_long: Vec<&&String> = rows
        .iter()
        .skip_while(|r| !r.contains(":crashed:2:"))
        .filter(|r| r.contains(":crashed:"))
        .collect();
    assert!(
        after_long.iter().any(|r| r.contains(":crashed:1:true:")),
        "the long-lived generation's crash counts one, its restart armed; states {counts:?}"
    );
}

/// After the stop, the file opened again starts its server again: the
/// buffer killed and the file found again, a fourth process starts.
#[test]
fn e10v_2_reopening_the_file_after_the_stop_starts_the_server_again() {
    let fx = fixture("reopen");
    let mut s = fx.editor("crash");
    assert!(
        pump_until(&mut s, 15, stopped),
        "premise: the stop; servers {:?}",
        servers(&s)
    );
    assert_eq!(fx.generations(), 3, "premise");
    let before = ids(&s);
    exec(&s, "pmacs.buffer.kill(pmacs.window.buffer())");
    fx.open(&s);
    assert!(
        pump_until(&mut s, 10, |_| fx.generations() >= 4),
        "the reopened file started its server again; servers {:?}",
        servers(&s)
    );
    assert_ne!(ids(&s), before, "under a new id, its count new");
}
