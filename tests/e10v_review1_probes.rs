// tests/e10v_review1_probes.rs --- E10v review 1's behavioral probes,
// committed by fix round 1 as witnesses and guards (resume §6.3).
//! E10v review 1's probes (`~/apocrypha/Passus/pmacs E10v review 1
//! (2026-11-23).md`), each under the row it reads. A witness names the
//! finding it reproduces and fails at the reviewed head `8bbb338` by its
//! own assertion; a guard names the state the review found holding.
//! The review ran some rows on a stand-in server or with the head's fake
//! at the base; here every row drives `pmacs_fake_lsp`, with the modes
//! and plans fix round 1 gave it (`docs/fake-lsp-modes.md`).

use std::path::{Path, PathBuf};
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

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e10vr1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An editor whose rust server is `command` with `env` (Lua table body).
fn editor_with(dir: &Path, command: &str, env: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(
        &s,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.project.set_search_boundary({d:?})\n\
             pmacs.lsp.config.rust = {{ command = '{command}', env = {{ {env} }} }}\n",
            d = dir.display().to_string(),
        ),
    );
    s
}

fn open(s: &EditorState, path: &Path) {
    exec(
        s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
}

fn lines(path: &Path) -> usize {
    std::fs::read_to_string(path).map_or(0, |t| t.lines().count())
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

// ---------------------------------------------------------------------------
// E10v.2: what counts toward the bound
// ---------------------------------------------------------------------------

/// Medium 1, a witness. #355's condition is that a server cannot be
/// restarted for ever. A server that answers the first request the
/// editor sends after `didOpen` with an error, and then exits, every
/// generation (the fake's `crashafteranswer` with
/// `PMACS_FAKE_LSP_ERROR_CODE`; the review's stand-in did the same), was
/// restarted at one second for ever at `8bbb338`, because an error
/// counted as served and ended the run. Under the owner's ruling (c) a
/// generation that dies within the window of its start counts whatever
/// it answered, so the bound stops it after three. No key is pressed
/// after the file is opened. Both an internal error and the spec's retry
/// code (`ContentModified`) are read.
#[test]
fn r1_p1_a_server_that_answers_only_errors_and_exits_is_not_restarted_for_ever() {
    let mut failed = Vec::new();
    for code in ["-32603", "-32801"] {
        let d = dir(&format!("errcrash{code}"));
        let pids = d.join("pids.txt");
        let file = d.join("a.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut s = editor_with(
            &d,
            &fake_lsp_path(),
            &format!(
                "PMACS_FAKE_LSP_MODE = 'crashafteranswer', PMACS_FAKE_LSP_ERROR_CODE = '{code}', \
                 PMACS_FAKE_LSP_PID_SINK = '{}'",
                pids.display()
            ),
        );
        open(&s, &file);
        let t0 = Instant::now();
        let mut rows = Vec::new();
        let stopped = pump_until(&mut s, 12, |s| {
            let row = servers(s);
            if rows.last() != Some(&row) {
                rows.push(row.clone());
            }
            row.iter().any(|r| r.ends_with(":true"))
        });
        let elapsed = t0.elapsed();
        let generations = lines(&pids);
        let crash_lines = s
            .lua_host
            .errors_buffer_text()
            .lines()
            .filter(|l| l.contains("crashed"))
            .count();
        eprintln!(
            "R1P1 code {code}: {generations} processes in {elapsed:?}, stopped {stopped}; \
             {crash_lines} crash lines in *errors*; status {:?}; states {rows:?}",
            status(&s)
        );
        assert!(
            generations >= 1 && crash_lines >= 1,
            "premise: the server started and crashed; code {code}, {generations} processes, \
             states {rows:?}"
        );
        if !(stopped && generations <= 3) {
            failed.push(format!(
                "error code {code}: {generations} processes in {elapsed:?}, stopped {stopped}, \
                 {crash_lines} crash lines in *errors*"
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "a server that answered nothing but errors is stopped by the bound after three \
         crashes; {failed:?}"
    );
}
