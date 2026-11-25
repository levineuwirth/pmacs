// tests/e10v_4b_restart_command_acceptance.rs --- E10v.4b (#480): a
// command that restarts a server, ruled by the owner on E10v.4's table.

//! A buffer whose server holds the right text and publishes nothing (HLS
//! 2.4.0.0 after a burst typed during its setup) came back, in E10v.4's
//! sixty runs on the real server, only when the server's process was
//! killed: 21 of 21, against 0 of 4 for a save and 0 of 11 for closing
//! the file and opening it again. pmacs had no command that restarts a
//! server. `M-x lsp.restart` restarts the active buffer's server under
//! the same id, its crash count new, and the new generation is sent the
//! documents (E10v.3).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use serde_json::Value;

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

fn press(s: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
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

/// `M-x lsp.restart RET`, as a user types it.
fn restart_by_keys(s: &mut EditorState) {
    press(s, KeyCode::Char('x'), KeyModifiers::ALT);
    for ch in "lsp.restart".chars() {
        press(s, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    press(s, KeyCode::Enter, KeyModifiers::NONE);
}

fn messages(sink: &Path) -> Vec<Value> {
    std::fs::read_to_string(sink)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn pids(msgs: &[Value]) -> Vec<u64> {
    let mut out = Vec::new();
    for m in msgs {
        let p = m["pid"].as_u64().unwrap();
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

struct Fixture {
    dir: PathBuf,
    a: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v4b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    Fixture { dir, a }
}

impl Fixture {
    fn editor(&self, env: &str) -> EditorState {
        let s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(self.dir.clone()));
        exec(
            &s,
            &format!(
                "pmacs.lsp.config = {{}}\n\
                 pmacs.project.set_search_boundary({dir:?})\n\
                 pmacs.lsp.config.rust = {{ command = '{cmd}', env = {{ {env} }} }}\n\
                 pmacs.buffer.find_or_open({a:?})",
                dir = self.dir.display().to_string(),
                cmd = fake_lsp_path(),
                a = self.a.display().to_string(),
            ),
        );
        s
    }
}

/// The server stops answering (the fake's `stopanswering`, on a command
/// typed into the buffer): it is alive, holds the text, and answers
/// nothing. `M-x lsp.restart`: the old process is gone, a new one under
/// the same id is sent the buffer's current text in one `didOpen`, and a
/// hover asked after it is answered. At the base there is no such
/// command and nothing restarts.
#[test]
fn e10v_4b_lsp_restart_brings_back_a_server_that_stopped_answering() {
    let fx = fixture("silent");
    let sink = fx.dir.join("messages.jsonl");
    let stopped = fx.dir.join("stopped");
    let mut s = fx.editor(&format!(
        "PMACS_FAKE_LSP_MODE = 'stopanswering', PMACS_FAKE_LSP_STOP_SINK = '{}', \
         PMACS_FAKE_LSP_ABORT_ONCE = '{}', PMACS_FAKE_LSP_MESSAGE_SINK = '{}'",
        stopped.display(),
        fx.dir.join("once").display(),
        sink.display()
    ));
    assert!(
        pump_until(&mut s, 10, |s| eval::<bool>(
            s,
            "local r = pmacs.lsp.active_attachment() return r ~= nil and pmacs.lsp.status(r.server).kind == 'initialized'"
        )),
        "premise: the server initialized"
    );
    exec(
        &s,
        "pmacs.editor.clipboard_set('// PMACS-STOP-ANSWERING\\n')",
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    press(&mut s, KeyCode::Char('y'), KeyModifiers::CONTROL);
    exec(&s, "pmacs.lsp._attachment_for_command()");
    assert!(
        pump_until(&mut s, 10, |_| stopped.exists()),
        "premise: the server has stopped answering"
    );
    let first = pids(&messages(&sink));
    assert_eq!(first.len(), 1, "premise: one process");
    let ids_before: String = eval(&s, "return tostring(pmacs.lsp.active_attachment().server)");
    restart_by_keys(&mut s);
    let came_back = pump_until(&mut s, 10, |s| {
        exec(s, "pmacs.command.invoke('lsp.hover')");
        let msgs = messages(&sink);
        let p = pids(&msgs);
        p.len() >= 2
            && msgs
                .iter()
                .any(|m| m["pid"].as_u64() == Some(p[1]) && m["method"] == "textDocument/hover")
    });
    let msgs = messages(&sink);
    let p = pids(&msgs);
    assert!(
        came_back,
        "M-x lsp.restart started a new server that was asked a hover; processes {p:?}; status {:?}",
        status(&s)
    );
    let second: Vec<&Value> = msgs
        .iter()
        .filter(|m| m["pid"].as_u64() == Some(p[1]))
        .collect();
    let opens: Vec<&&Value> = second
        .iter()
        .filter(|m| m["method"] == "textDocument/didOpen")
        .collect();
    let text: mlua::String = eval(
        &s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    assert_eq!(opens.len(), 1, "one didOpen for the buffer");
    assert_eq!(
        opens[0]["text"].as_str(),
        Some(String::from_utf8_lossy(&text.as_bytes()).as_ref()),
        "carrying its current text"
    );
    assert_eq!(
        eval::<String>(&s, "return tostring(pmacs.lsp.active_attachment().server)"),
        ids_before,
        "under the same id"
    );
    assert!(
        pump_until(&mut s, 5, |_| !Path::new(&format!("/proc/{}", first[0]))
            .exists()),
        "the old process is gone"
    );
}

/// A server the restart bound stopped (three crashes in a row) is
/// started again by `M-x lsp.restart`, its count new.
#[test]
fn e10v_4b_lsp_restart_starts_a_server_the_bound_stopped() {
    let fx = fixture("bound");
    let pids_file = fx.dir.join("pids.txt");
    let mut s = fx.editor(&format!(
        "PMACS_FAKE_LSP_MODE = 'crash', PMACS_FAKE_LSP_PID_SINK = '{}'",
        pids_file.display()
    ));
    let generations = || std::fs::read_to_string(&pids_file).map_or(0, |t| t.lines().count());
    assert!(
        pump_until(&mut s, 15, |s| eval::<bool>(
            s,
            "for _, r in ipairs(pmacs.lsp.list()) do if r.crash_stopped then return true end end return false"
        )),
        "premise: the bound stopped the server"
    );
    assert_eq!(generations(), 3, "premise");
    restart_by_keys(&mut s);
    assert!(
        pump_until(&mut s, 5, |_| generations() >= 4),
        "M-x lsp.restart started it again; status {:?}",
        status(&s)
    );
    assert!(
        pump_until(&mut s, 5, |s| eval::<bool>(
            s,
            "for _, r in ipairs(pmacs.lsp.list()) do if r.crashes == 1 and r.restart_pending then return true end end return false"
        )),
        "its count started over: one crash, a restart armed"
    );
}
