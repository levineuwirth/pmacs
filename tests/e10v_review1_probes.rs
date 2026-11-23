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

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use serde_json::{Value, json};

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

fn settle(s: &mut EditorState, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        tick(s);
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

fn initialized(s: &EditorState) -> bool {
    eval(
        s,
        "for _, r in ipairs(pmacs.lsp.list()) do \
           if r.state and r.state.kind == 'initialized' then return true end \
         end return false",
    )
}

/// The messages each server process received, in order (the fake's
/// `PMACS_FAKE_LSP_MESSAGE_SINK`).
fn messages(sink: &Path) -> Vec<Value> {
    std::fs::read_to_string(sink)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn pids_in_order(msgs: &[Value]) -> Vec<u64> {
    let mut pids = Vec::new();
    for m in msgs {
        let pid = m["pid"].as_u64().unwrap();
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
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

/// Low 1, a witness. The stop sentence said the server is "not
/// restarted until its file is opened again", but any file of the same
/// language under the same root renews the bound: here `b.rs`, never
/// attached to the stopped server, starts it again under a new id. Fix
/// round 1 keeps the renewal (every door is the user's act, and none is
/// a loop) and makes the sentence say it: a file of its project, or
/// `M-x lsp.restart`. Fails at `8bbb338` on the sentence.
#[test]
fn r1_p2_after_the_stop_the_sentence_says_what_starts_the_server_again() {
    let d = dir("renew");
    let a = d.join("a.rs");
    let b = d.join("b.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let pids = d.join("pids.txt");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!(
            "PMACS_FAKE_LSP_MODE = 'crash', PMACS_FAKE_LSP_PID_SINK = '{}'",
            pids.display()
        ),
    );
    open(&s, &a);
    assert!(
        pump_until(&mut s, 15, |s| servers(s)
            .iter()
            .any(|r| r.ends_with(":true"))),
        "premise: the bound stopped the server; {:?}",
        servers(&s)
    );
    settle(&mut s, 500);
    assert_eq!(lines(&pids), 3, "premise: stopped after three");
    let said = s.lua_host.errors_buffer_text();
    open(&s, &b);
    settle(&mut s, 1500);
    eprintln!(
        "R1P2 after opening b.rs: {} processes; servers {:?}; said {said:?}",
        lines(&pids),
        servers(&s)
    );
    assert!(
        said.contains(
            "not restarted until a file of its project is opened again, or M-x lsp.restart"
        ),
        "the stop sentence names what starts the server again; *errors* {said:?}"
    );
    assert!(
        lines(&pids) > 3,
        "premise of the sentence: another file of the project starts the server again; \
         servers {:?}",
        servers(&s)
    );
}

/// Low 1's other door, a guard: after the stop, `M-x revert-buffer` on
/// the stopped server's unmodified file does not renew the bound (the
/// handoff's "a revert renews" was wrong at `8bbb338`, and is now): no
/// process starts and the server stays stopped.
#[test]
fn r1_p15_after_the_stop_a_revert_does_not_start_the_server_again() {
    let d = dir("revert");
    let a = d.join("a.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    let pids = d.join("pids.txt");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!(
            "PMACS_FAKE_LSP_MODE = 'crash', PMACS_FAKE_LSP_PID_SINK = '{}'",
            pids.display()
        ),
    );
    open(&s, &a);
    assert!(
        pump_until(&mut s, 15, |s| servers(s)
            .iter()
            .any(|r| r.ends_with(":true"))),
        "premise: the bound stopped the server; {:?}",
        servers(&s)
    );
    settle(&mut s, 500);
    assert_eq!(lines(&pids), 3, "premise: three processes");
    exec(&s, "pmacs.command.invoke('revert-buffer')");
    let said = status(&s);
    settle(&mut s, 8000);
    eprintln!(
        "R1P15 after revert-buffer: {} processes; status at the revert {said:?}; servers {:?}",
        lines(&pids),
        servers(&s)
    );
    assert!(
        said.starts_with("reverted"),
        "premise: the revert ran; status {said:?}"
    );
    assert!(
        lines(&pids) == 3 && servers(&s).iter().any(|r| r.ends_with(":true")),
        "a revert starts no process and the server stays stopped; {} processes; servers {:?}",
        lines(&pids),
        servers(&s)
    );
}

// ---------------------------------------------------------------------------
// E10v.1: a late publish from before a reopen
// ---------------------------------------------------------------------------

const RECORDER: &str = r"
e10v_publishes = {}
pmacs.lsp.on_notification('textDocument/publishDiagnostics', function(_sid, p)
  local v = p.version
  local label = 'v' .. (type(v) == 'number' and tostring(math.floor(v)) or '-')
    .. ':' .. tostring(#(p.diagnostics or {}))
  e10v_publishes[#e10v_publishes + 1] = { uri = p.uri, label = label }
end)
";

fn stored(s: &EditorState) -> Vec<String> {
    let rows: Vec<mlua::Table> = eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         if not rec then return {} end
         return pmacs.diag.list(rec.uri)",
    );
    rows.into_iter()
        .map(|d| {
            let m: String = d.get("message").unwrap();
            m.lines().next().unwrap_or("").trim().to_owned()
        })
        .collect()
}

fn received(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, p in ipairs(e10v_publishes) do
           if rec and p.uri == rec.uri then out[#out + 1] = p.label end
         end
         return out",
    )
}

fn diag(line: u32, message: &str) -> Value {
    json!({
        "range": { "start": { "line": line, "character": 0 },
                   "end": { "line": line, "character": 3 } },
        "severity": 1, "source": "e10v", "message": message
    })
}

/// Medium 2, a witness. A publish the server computed for the document
/// before the buffer was killed (version 24), arriving after the reopen's
/// `didOpen`, held the reopened document's diagnostics at `8bbb338`: the
/// highest version was reset at `didOpen` and the reopened document was
/// numbered from 1, so the old 24 stood and the reopened document's own
/// publishes, at 1 and then 2, were dropped. Under the owner's ruling the
/// reopened document is numbered past 24, so its publishes stand. The
/// fake publishes the reopened document's at the version it was sent
/// (`"received"`), as a server does; the review's stand-in plan wrote 1
/// and 2, the numbers the reviewed head sent.
#[test]
fn r1_p3_a_late_publish_from_before_a_reopen_does_not_hold_the_reopened_document() {
    let d = dir("reopen");
    let file = d.join("a.rs");
    std::fs::write(&file, "let first = 1;\nlet second = 2;\n").unwrap();
    let plan = d.join("plan.json");
    std::fs::write(
        &plan,
        json!([
            { "method": "textDocument/didOpen",
              "publish": [{ "version": 24, "diagnostics": [diag(1, "r1: the old document's error")] }] },
            { "method": "textDocument/didOpen",
              "publish": [
                { "version": 24, "diagnostics": [diag(1, "r1: the old document's error")] },
                { "version": "received", "diagnostics": [diag(0, "r1: the reopened document's error")] }
              ] },
            { "method": "textDocument/didChange",
              "publish": [{ "version": "received", "diagnostics": [diag(0, "r1: after a key")] }] },
        ])
        .to_string(),
    )
    .unwrap();
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!("PMACS_FAKE_LSP_PUBLISH_PLAN = '{}'", plan.display()),
    );
    exec(&s, RECORDER);
    open(&s, &file);
    assert!(
        pump_until(&mut s, 10, |s| received(s).len() == 1),
        "premise: the first publish; {:?}",
        received(&s)
    );
    exec(&s, "pmacs.buffer.kill(pmacs.window.buffer())");
    open(&s, &file);
    assert!(
        pump_until(&mut s, 10, |s| received(s).len() == 3),
        "premise: the late publish and the reopened document's arrived; {:?}",
        received(&s)
    );
    settle(&mut s, 300);
    let after_reopen = stored(&s);
    // One key: a didChange, and the server's publish for it.
    exec(&s, "pmacs.editor.goto_byte(0)");
    press(&mut s, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(
        pump_until(&mut s, 10, |s| received(s).len() == 4),
        "premise: the publish after the key arrived; {:?}",
        received(&s)
    );
    settle(&mut s, 300);
    let after_key = stored(&s);
    eprintln!(
        "R1P3 received {:?}; after the reopen {after_reopen:?}; after a key {after_key:?}",
        received(&s)
    );
    assert_eq!(
        after_reopen,
        ["r1: the reopened document's error"],
        "the reopened document's own publish stands over one computed before the reopen; \
         received {:?}",
        received(&s)
    );
    assert_eq!(
        after_key,
        ["r1: after a key"],
        "the publish for the key after the reopen stands; received {:?}",
        received(&s)
    );
    assert_eq!(
        received(&s),
        ["v24:1", "v24:1", "v25:1", "v26:1"],
        "the reopened document is numbered past version 24"
    );
}

/// The versions sent to a server's processes, in order, `method@version`
/// for each `didOpen` and `didChange` of `uri`, with the process's index.
fn sent_versions(sink: &Path, uri_tail: &str) -> Vec<(usize, String, i64)> {
    let msgs = messages(sink);
    let pids = pids_in_order(&msgs);
    msgs.iter()
        .filter(|m| {
            m["uri"].as_str().is_some_and(|u| u.ends_with(uri_tail)) && m["version"].is_i64()
        })
        .map(|m| {
            (
                pids.iter()
                    .position(|p| Some(*p) == m["pid"].as_u64())
                    .unwrap(),
                m["method"].as_str().unwrap_or("").to_owned(),
                m["version"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// Medium 2 through `lsp.restart`: the replacement is sent the document
/// past every version the first process was sent, so no version repeats
/// for the URI on the server. At `8bbb338` the reopen sent version 1.
#[test]
fn r1_p3_lsp_restart_numbers_the_reopened_document_past_what_was_sent() {
    let d = dir("restartver");
    let a = d.join("a.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    let sink = d.join("messages.jsonl");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!("PMACS_FAKE_LSP_MESSAGE_SINK = '{}'", sink.display()),
    );
    open(&s, &a);
    assert!(pump_until(&mut s, 10, initialized), "premise: initialized");
    exec(&s, "pmacs.editor.goto_byte(0)");
    for c in ['x', 'y', 'z'] {
        press(&mut s, KeyCode::Char(c), KeyModifiers::NONE);
        settle(&mut s, 150);
    }
    assert!(
        pump_until(&mut s, 5, |_| sent_versions(&sink, "/a.rs")
            .iter()
            .any(|(_, m, _)| m == "textDocument/didChange")),
        "premise: the keys reached the server; sent {:?}",
        sent_versions(&sink, "/a.rs")
    );
    let before = sent_versions(&sink, "/a.rs");
    let highest = before.iter().map(|(_, _, v)| *v).max().unwrap();
    exec(&s, "pmacs.command.invoke('lsp.restart')");
    assert!(
        pump_until(&mut s, 10, |_| sent_versions(&sink, "/a.rs")
            .iter()
            .any(|(p, m, _)| *p == 1 && m == "textDocument/didOpen")),
        "premise: the replacement was sent the document; sent {:?}",
        sent_versions(&sink, "/a.rs")
    );
    let reopened = sent_versions(&sink, "/a.rs")
        .into_iter()
        .find(|(p, m, _)| *p == 1 && m == "textDocument/didOpen")
        .unwrap()
        .2;
    assert!(
        reopened > highest,
        "the replacement's didOpen is numbered past version {highest}, the highest the \
         first process was sent; it carried {reopened}; sent {:?}",
        sent_versions(&sink, "/a.rs")
    );
}

/// Medium 2 through E10v.3's re-sent documents: after a crash, the
/// replacement's `didOpen` is numbered past the change that carried the
/// crash's marker. At `8bbb338` it carried version 1.
#[test]
fn r1_p3_a_restarted_generation_is_sent_its_document_past_what_was_sent() {
    let d = dir("crashver");
    let a = d.join("a.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    let sink = d.join("messages.jsonl");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!(
            "PMACS_FAKE_LSP_MODE = 'crashonmarker', PMACS_FAKE_LSP_ABORT_ONCE = '{}', \
             PMACS_FAKE_LSP_MESSAGE_SINK = '{}'",
            d.join("once").display(),
            sink.display()
        ),
    );
    open(&s, &a);
    assert!(pump_until(&mut s, 10, initialized), "premise: initialized");
    exec(&s, "pmacs.editor.goto_byte(0)");
    press(&mut s, KeyCode::Char('q'), KeyModifiers::NONE);
    settle(&mut s, 500);
    let highest = sent_versions(&sink, "/a.rs")
        .iter()
        .map(|(_, _, v)| *v)
        .max()
        .unwrap();
    // The marker's didChange is not recorded (the fake exits on it), so
    // the version it carried is at least one past the highest recorded.
    exec(&s, "pmacs.editor.clipboard_set('// PMACS-CRASH-NOW\\n')");
    press(&mut s, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert!(
        pump_until(&mut s, 15, |_| sent_versions(&sink, "/a.rs")
            .iter()
            .any(|(p, m, _)| *p == 1 && m == "textDocument/didOpen")),
        "premise: the replacement was sent the document; sent {:?}",
        sent_versions(&sink, "/a.rs")
    );
    let reopened = sent_versions(&sink, "/a.rs")
        .into_iter()
        .find(|(p, m, _)| *p == 1 && m == "textDocument/didOpen")
        .unwrap()
        .2;
    assert!(
        reopened > highest + 1,
        "the replacement's didOpen is numbered past the marker's change (past {}); it \
         carried {reopened}; sent {:?}",
        highest + 1,
        sent_versions(&sink, "/a.rs")
    );
}

// ---------------------------------------------------------------------------
// E10v.4b: lsp.restart at its edges
// ---------------------------------------------------------------------------

fn named_text(s: &EditorState, name: &str) -> String {
    let b: mlua::String = eval(
        s,
        &format!(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == {name:?} then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#
        ),
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn shown(msgs: &[&Value]) -> Vec<String> {
    msgs.iter()
        .map(|m| {
            format!(
                "{}{}",
                m["method"].as_str().unwrap_or(""),
                m["uri"]
                    .as_str()
                    .map(|u| format!(" {}", u.rsplit('/').next().unwrap()))
                    .unwrap_or_default()
            )
        })
        .collect()
}

fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// Low 2, a witness. `M-x lsp.restart` while a hover is in flight (the
/// fake answers no document request): the restart cancels the hover, and
/// at `8bbb338` that cancellation wrote "server unavailable (request
/// cancelled)" over the restart's word, where it stayed after the
/// replacement was up. The user asked for the restart; the echo line
/// ends on what the restart said.
#[test]
fn r1_p7_lsp_restart_with_a_request_in_flight_ends_on_the_restarts_word() {
    let d = dir("inflight");
    let a = d.join("a.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    let pids = d.join("pids.txt");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!(
            "PMACS_FAKE_LSP_MODE = 'silent', PMACS_FAKE_LSP_PID_SINK = '{}'",
            pids.display()
        ),
    );
    open(&s, &a);
    assert!(pump_until(&mut s, 10, initialized), "premise: initialized");
    exec(&s, "pmacs.editor.goto_byte(3)");
    exec(&s, "pmacs.command.invoke('lsp.hover')");
    settle(&mut s, 300);
    let before = status(&s);
    exec(&s, "pmacs.command.invoke('lsp.restart')");
    let said = status(&s);
    let mut statuses = vec![said.clone()];
    let replaced = pump_until(&mut s, 10, |s| {
        let st = status(s);
        if statuses.last() != Some(&st) {
            statuses.push(st);
        }
        lines(&pids) == 2 && initialized(s)
    });
    settle(&mut s, 1500);
    statuses.push(status(&s));
    eprintln!(
        "R1P7 before {before:?}; statuses {statuses:?}; servers {:?}",
        servers(&s)
    );
    assert!(
        replaced && said.starts_with("LSP: restarting"),
        "premise: the server replaced and the command's word said; statuses {statuses:?}"
    );
    assert_eq!(
        status(&s),
        said,
        "the echo line ends on the restart's word; statuses {statuses:?}"
    );
}

/// A guard: `M-x lsp.restart` with no server for the buffer, in
/// `*scratch*` and in a text file, says so.
#[test]
fn r1_p8_lsp_restart_with_no_server_says_so() {
    let d = dir("noserver");
    let notes = d.join("notes.txt");
    std::fs::write(&notes, "plain\n").unwrap();
    let s = editor_with(&d, &fake_lsp_path(), "");
    exec(&s, "pmacs.command.invoke('lsp.restart')");
    let scratch = status(&s);
    open(&s, &notes);
    exec(&s, "pmacs.command.invoke('lsp.restart')");
    let text = status(&s);
    eprintln!("R1P8 scratch {scratch:?}; notes.txt {text:?}");
    assert_eq!(
        (scratch.as_str(), text.as_str()),
        (
            "LSP: no server for active buffer",
            "LSP: no server for active buffer"
        )
    );
}

/// A guard: `M-x lsp.restart` from the window of the second of two
/// buffers on one server, the first holding unsaved text: the
/// replacement receives each buffer once, with its current text.
#[test]
fn r1_p9_lsp_restart_from_the_second_buffer_reopens_both() {
    let d = dir("twobufs");
    let a = d.join("a.rs");
    let b = d.join("b.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let sink = d.join("messages.jsonl");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!("PMACS_FAKE_LSP_MESSAGE_SINK = '{}'", sink.display()),
    );
    open(&s, &a);
    assert!(pump_until(&mut s, 10, initialized), "premise: initialized");
    exec(&s, "pmacs.editor.goto_byte(0)");
    exec(&s, "pmacs.editor.clipboard_set('// unsaved in a\\n')");
    press(&mut s, KeyCode::Char('y'), KeyModifiers::CONTROL);
    open(&s, &b);
    settle(&mut s, 500);
    let a_text = named_text(&s, "a.rs");
    exec(&s, "pmacs.command.invoke('lsp.restart')");
    let said = status(&s);
    let replaced = pump_until(&mut s, 10, |_| pids_in_order(&messages(&sink)).len() >= 2);
    settle(&mut s, 1000);
    let msgs = messages(&sink);
    let pids = pids_in_order(&msgs);
    assert!(
        replaced,
        "premise: a second process; {pids:?}; status {said:?}"
    );
    let second: Vec<&Value> = msgs
        .iter()
        .filter(|m| m["pid"].as_u64() == Some(pids[1]))
        .collect();
    let opens = |u: &str| -> Vec<&Value> {
        second
            .iter()
            .copied()
            .filter(|m| m["method"] == "textDocument/didOpen" && m["uri"] == u)
            .collect()
    };
    eprintln!(
        "R1P9 status {said:?}; the replacement received {:?}",
        shown(&second)
    );
    let (ao, bo) = (opens(&uri(&a)), opens(&uri(&b)));
    assert!(
        ao.len() == 1 && bo.len() == 1,
        "one didOpen for each; {:?}",
        shown(&second)
    );
    assert!(
        a_text.starts_with("// unsaved in a"),
        "premise: a.rs holds unsaved text; {a_text:?}"
    );
    assert_eq!(ao[0]["text"].as_str(), Some(a_text.as_str()));
}

// ---------------------------------------------------------------------------
// E10v.3: the order against a request from a coroutine
// ---------------------------------------------------------------------------

/// Low 3, E10v.3's ordering witness. A coroutine asks a hover on every
/// tick; coroutines resume in `tick_async` ahead of the LSP event drain
/// (the wrapper of `pmacs._async.tick` in `builtin/runtime/lsp.lua` runs
/// the prior tick, then the drain), which is where the `restarting` and
/// `initialized` events are read. The reopen is sent on `restarting`,
/// while the new process is starting, so the manager holds it and sends
/// it right after `initialized`, before it will send any request: the
/// replacement receives the document before the coroutine's hover. With
/// the reopen moved into the `initialized` handler (the handoff's own
/// plant, which its suite did not bite) the hover reaches it first.
#[test]
fn r1_p6_a_restarted_server_gets_its_document_before_a_coroutines_request() {
    let d = dir("order");
    let a = d.join("a.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    let sink = d.join("messages.jsonl");
    let mut s = editor_with(
        &d,
        &fake_lsp_path(),
        &format!(
            "PMACS_FAKE_LSP_MODE = 'crashonmarker', PMACS_FAKE_LSP_ABORT_ONCE = '{}', \
             PMACS_FAKE_LSP_MESSAGE_SINK = '{}'",
            d.join("once").display(),
            sink.display()
        ),
    );
    open(&s, &a);
    assert!(pump_until(&mut s, 10, initialized), "premise: initialized");
    exec(
        &s,
        "r1_asked = 0
         pmacs.async(function()
           while true do
             local rec = pmacs.lsp.active_attachment()
             if rec then
               local ok = pcall(pmacs.lsp.request_hover, rec.server, rec.uri, 0, 3)
               if ok then r1_asked = r1_asked + 1 end
             end
             pmacs.async.yield_to_next_tick()
           end
         end)",
    );
    settle(&mut s, 200);
    exec(&s, "pmacs.editor.goto_byte(0)");
    exec(&s, "pmacs.editor.clipboard_set('// PMACS-CRASH-NOW\\n')");
    press(&mut s, KeyCode::Char('y'), KeyModifiers::CONTROL);
    let served = pump_until(&mut s, 15, |_| {
        let msgs = messages(&sink);
        let pids = pids_in_order(&msgs);
        pids.len() >= 2
            && msgs
                .iter()
                .any(|m| m["pid"].as_u64() == Some(pids[1]) && m["method"] == "textDocument/hover")
    });
    let msgs = messages(&sink);
    let pids = pids_in_order(&msgs);
    let asked: i64 = eval(&s, "return r1_asked");
    assert!(
        served,
        "premise: the coroutine asked the replacement a hover; processes {pids:?}; asked \
         {asked}; servers {:?}",
        servers(&s)
    );
    let second: Vec<&Value> = msgs
        .iter()
        .filter(|m| m["pid"].as_u64() == Some(pids[1]))
        .collect();
    let seq = shown(&second);
    eprintln!("R1P6 the replacement received {seq:?}; the coroutine asked {asked}");
    let open_at = second
        .iter()
        .position(|m| m["method"] == "textDocument/didOpen")
        .unwrap_or(usize::MAX);
    let first_request = second
        .iter()
        .position(|m| !m["id"].is_null() && m["uri"].is_string())
        .expect("premise: a document request");
    assert!(
        open_at < first_request,
        "the document before the first request about it; the replacement received {seq:?}"
    );
}
