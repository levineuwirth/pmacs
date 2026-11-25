// tests/e10v_3_restart_reopens_acceptance.rs --- E10v.3 (#355): a
// restarted server is sent its documents.

//! When a server crashed, the manager restarted it under the same id and
//! dropped the dead generation's documents (`start_generation`), saying
//! the editor's reattach path would send them again. That path is
//! `attach_buffer`, which returns a live server's attachment as it is, so
//! nothing was sent: the new generation answered every request about the
//! buffers `file not found`, and `initialized`'s re-pull asked about
//! documents it had never been given (#355's trace on rust-analyzer).
//!
//! Now the client reopens every attached document into the restarted
//! generation, each with its buffer's current text, before any request
//! about a document reaches it. Behind E10v.2's bound, which stops a
//! server its document crashes three times in a row.

use std::path::PathBuf;
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

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_until(
    s: &mut EditorState,
    secs: u64,
    mut done: impl FnMut(&mut EditorState) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if done(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// `text` goes in at point as one key, `C-y`, the kill ring holding it.
fn yank(s: &mut EditorState, text: &str) {
    exec(s, &format!("pmacs.editor.clipboard_set({text:?})"));
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

fn uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
}

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

fn server_rows(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local out = {}
         for _, i in ipairs(pmacs.lsp.list()) do
           out[#out + 1] = tostring(i.id) .. ':' .. tostring(i.state and i.state.kind)
             .. ':' .. tostring(i.attempt)
         end
         return out",
    )
}

/// The messages each server process received, in order.
fn messages(sink: &std::path::Path) -> Vec<Value> {
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

/// Two buffers attached, the one in the background holding unsaved
/// text; the server crashes once, on a marker typed into the shown
/// buffer, and the generation restarted after it serves. That generation
/// receives exactly one `didOpen` for each buffer, the background one's
/// carrying its unsaved text, and both before any request about a
/// document: a hover is asked on every tick from the crash on, so a
/// request that could go out ahead of the documents would. At the base
/// it receives no `didOpen` at all.
#[test]
#[allow(clippy::too_many_lines)] // one user's path, from two buffers through a crash to the replacement's first request
fn e10v_3_a_restarted_server_is_sent_each_document_once_with_its_text_before_any_request() {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.rs");
    let b = dir.join("b.rs");
    std::fs::write(&a, "fn a() {}\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let sink = dir.join("messages.jsonl");
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(
        &s,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.project.set_search_boundary({dir:?})\n\
             pmacs.lsp.config.rust = {{ command = '{cmd}', env = {{ \
               PMACS_FAKE_LSP_MODE = 'crashonmarker', \
               PMACS_FAKE_LSP_ABORT_ONCE = '{once}', \
               PMACS_FAKE_LSP_MESSAGE_SINK = '{sink}' }} }}\n",
            dir = dir.display().to_string(),
            cmd = fake_lsp_path(),
            once = dir.join("crashed-once").display(),
            sink = sink.display(),
        ),
    );
    let mut s = s;
    let open = |s: &EditorState, p: &PathBuf| {
        exec(
            s,
            &format!("pmacs.buffer.find_or_open({:?})", p.display().to_string()),
        );
    };
    let initialized = |s: &EditorState| server_rows(s).iter().any(|r| r.contains(":initialized:"));
    open(&s, &a);
    open(&s, &b);
    assert!(
        pump_until(&mut s, 10, |s| initialized(s)),
        "premise: the first server initialized; {:?}",
        server_rows(&s)
    );
    // Unsaved text in b.rs, which then goes to the background.
    exec(&s, "pmacs.editor.goto_byte(0)");
    yank(&mut s, "// unsaved in b\n");
    open(&s, &a);
    let b_text = named_text(&s, "b.rs");
    assert!(
        b_text.starts_with("// unsaved in b\n"),
        "premise: {b_text:?}"
    );
    let first_pids = pids_in_order(&messages(&sink));
    assert_eq!(first_pids.len(), 1, "premise: one process so far");

    // The marker, typed into the shown buffer, crashes the first
    // generation once. From then on a hover is asked on every tick.
    exec(&s, "pmacs.editor.goto_byte(0)");
    yank(&mut s, "// PMACS-CRASH-NOW\n");
    exec(&s, "pmacs.lsp._attachment_for_command()");
    assert!(
        pump_until(&mut s, 10, |s| server_rows(s)
            .iter()
            .any(|r| r.contains(":crashed:"))),
        "premise: the marker crashed the first server; {:?}",
        server_rows(&s)
    );
    let served = pump_until(&mut s, 15, |s| {
        exec(s, "pmacs.command.invoke('lsp.hover')");
        exec(s, "pmacs.command.invoke('lsp.find-references')");
        let msgs = messages(&sink);
        let pids = pids_in_order(&msgs);
        pids.len() >= 2
            && msgs
                .iter()
                .any(|m| m["pid"].as_u64() == Some(pids[1]) && m["method"] == "textDocument/hover")
    });
    let msgs = messages(&sink);
    let pids = pids_in_order(&msgs);
    assert!(
        served,
        "premise: a second generation was asked a hover; processes {pids:?}; servers {:?}",
        server_rows(&s)
    );
    let second: Vec<&Value> = msgs
        .iter()
        .filter(|m| m["pid"].as_u64() == Some(pids[1]))
        .collect();
    let shown: Vec<String> = second
        .iter()
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
        .collect();
    eprintln!("E10V3 the restarted server received {shown:?}");
    let opens = |u: &str| -> Vec<(usize, &Value)> {
        second
            .iter()
            .enumerate()
            .filter(|(_, m)| m["method"] == "textDocument/didOpen" && m["uri"] == u)
            .map(|(i, m)| (i, *m))
            .collect()
    };
    let a_opens = opens(&uri(&a));
    let b_opens = opens(&uri(&b));
    assert!(
        a_opens.len() == 1 && b_opens.len() == 1,
        "the restarted server received exactly one didOpen for each of the two buffers; \
         it received {shown:?}"
    );
    assert_eq!(
        b_opens[0].1["text"].as_str(),
        Some(b_text.as_str()),
        "the background buffer's didOpen carries its unsaved text"
    );
    assert_eq!(
        a_opens[0].1["text"].as_str(),
        Some(named_text(&s, "a.rs").as_str()),
        "the shown buffer's didOpen carries its text"
    );
    let first_request = second
        .iter()
        .position(|m| !m["id"].is_null() && m["uri"].is_string())
        .expect("premise: a document request");
    assert!(
        a_opens[0].0 < first_request && b_opens[0].0 < first_request,
        "both documents before the first request about one; the second generation received \
         {shown:?}"
    );
    assert!(
        second
            .iter()
            .take(first_request)
            .all(|m| m["method"] != "textDocument/didChange"),
        "no change ahead of the documents it changes; {shown:?}"
    );
}
