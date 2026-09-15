//! C6b closure: actual edit broadcasts and request text, through the runtime.
//! No store or recorder is seeded by the fixture.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;

/// How long the fake server holds every semantic-token answer. Long
/// enough that a keystroke is observed against a stale store on the
/// tick it lands, short enough that the answer arrives inside the
/// deadline below.
const HOLD_MS: u64 = 700;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

/// One tick of the editor's own frame order.
fn tick(state: &mut EditorState) {
    state.tick_processes();
    state.tick_lsp();
    state.tick_async();
}

/// Tick until the Lua expression `flag` is true, or the deadline lapses.
fn pump_lua_flag(state: &mut EditorState, flag: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(state);
        let done: bool = state
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
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The store's positioned tokens for `uri`, as byte ranges, or `None`
/// when it would paint nothing.
fn positioned(state: &EditorState, uri: &str) -> Option<Vec<(u64, u64)>> {
    let store = state.lsp_manager.borrow().semantic_token_store();
    let guard = store.lock().expect("store");
    guard
        .positioned_tokens(uri)
        .map(|(_, t)| t.iter().map(|t| (t.start, t.end)).collect())
}

fn store_is_stale(state: &EditorState, uri: &str) -> bool {
    let store = state.lsp_manager.borrow().semantic_token_store();
    let guard = store.lock().expect("store");
    guard.is_stale(uri)
}

/// A state with the fake server on Rust, the marker face, and `a.rs`
/// open with its tokens pulled. Returns the state and the file URI.
fn attached(text: &str) -> (EditorState, tempfile::TempDir, String) {
    attached_with(text, HOLD_MS, HOLD_MS, None)
}

/// [`attached`] with the fake server's two holds chosen, and the file
/// its `/range` requests are appended to.
fn attached_with(
    text: &str,
    full_hold_ms: u64,
    range_hold_ms: u64,
    range_sink: Option<&std::path::Path>,
) -> (EditorState, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let a_path = dir.path().join("a.rs");
    std::fs::write(&a_path, text).expect("write a.rs");
    let a_disp = a_path.display().to_string();

    let mut state = EditorState::new_with_roots(&iso::roots());
    let fake = fake_lsp_path();
    let proxy = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/probes/e6b_lsp_proxy.py");
    let wire_sink = dir.path().join("wire.jsonl").display().to_string();
    let sink = range_sink.map_or(String::new(), |p| {
        format!("PMACS_FAKE_RANGE_SINK = '{}',", p.display())
    });
    state
        .lua_host
        .lua()
        .load(format!(
            "pmacs.lsp.config.rust = {{
               command = 'python3',
               args = {{ '{proxy}', '{wire_sink}', '{fake}' }},
               env = {{
                 PMACS_FAKE_LSP_MODE = 'semantichold',
                 PMACS_FAKE_LSP_SEMANTIC_HOLD_MS = '{full_hold_ms}',
                 PMACS_FAKE_LSP_SEMANTIC_RANGE_HOLD_MS = '{range_hold_ms}',
                 {sink}
               }},
             }}
             pmacs.theme.merge {{ namespace = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}"
        ))
        .exec()
        .expect("configure the fake server and the marker face");
    state
        .lua_host
        .lua()
        .load(format!("pmacs.buffer.find_or_open('{a_disp}')"))
        .exec()
        .expect("open a.rs");
    let uri = format!("file://{a_disp}");
    let has_tokens = format!(
        "(function() \
           local sid \
           for _,r in ipairs(pmacs.lsp.list()) do \
             if r.state and r.state.kind=='initialized' then sid=r.id end \
           end \
           if not sid then return false end \
           local t = pmacs.semantic_tokens.tokens(sid, '{uri}') \
           return t ~= nil and #t > 0 \
         end)()"
    );
    assert!(
        pump_lua_flag(&mut state, &has_tokens, 10),
        "the attach pull never filled the token store"
    );
    (state, dir, uri)
}

fn lua(state: &EditorState, code: &str) {
    state.lua_host.lua().load(code).exec().expect("Lua action");
}

fn key(state: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    state.dispatch_key(FrontendId::LOCAL, KeyEvent::new(code, mods));
}

fn wire(dir: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("wire.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered edit sequence followed across its actual server request and delayed response"
)]
fn c6b_recorder_covers_typing_delete_selection_paste_undo_and_request_anchor() {
    use pmacs::semantic_tokens::DocumentEdit;
    let original = "fn main() { alpha.beta; }\n";
    let (mut state, dir, uri) = attached(original);
    let store = state.lsp_manager.borrow().semantic_token_store();
    let bid = state.core.borrow().active_window().buffer_id;
    #[cfg(feature = "crdt")]
    assert!(
        !state
            .core
            .borrow()
            .registry
            .borrow()
            .get(bid)
            .unwrap()
            .is_crdt_backed()
    );
    let _ = bid;
    lua(&state, "pmacs.editor.goto_byte(0)");
    key(&mut state, KeyCode::Char('é'), KeyModifiers::NONE);
    key(&mut state, KeyCode::Backspace, KeyModifiers::NONE);
    lua(
        &state,
        "pmacs.editor.goto_byte(14); pmacs.command.invoke('region.set-mark'); pmacs.editor.goto_byte(19)",
    );
    // This is the document arm of the unified inbound paste handler.
    state.core.borrow_mut().paste_inbound(b"XY").unwrap();
    key(&mut state, KeyCode::Char('/'), KeyModifiers::CONTROL);
    let expected = vec![
        DocumentEdit {
            start: 0,
            old_end: 0,
            inserted_len: 2,
        },
        DocumentEdit {
            start: 0,
            old_end: 2,
            inserted_len: 0,
        },
        DocumentEdit {
            start: 14,
            old_end: 19,
            inserted_len: 2,
        },
        DocumentEdit {
            start: 14,
            old_end: 16,
            inserted_len: 5,
        },
    ];
    {
        let guard = store.lock().unwrap();
        assert_eq!(
            guard.pending_edits(&uri),
            expected,
            "exact byte deltas, once per edit"
        );
        assert_eq!(guard.next_seq(&uri), 4);
        assert_eq!(guard.synced_seq(&uri), 0, "server has only didOpen text");
    }
    lua(&state, "pmacs.lsp._flush_did_changes()");
    assert_eq!(store.lock().unwrap().synced_seq(&uri), 4);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tick(&mut state);
        let messages = wire(dir.path());
        let change = messages
            .iter()
            .rposition(|r| r["message"]["method"] == "textDocument/didChange");
        if let Some(i) = change {
            assert_eq!(
                messages[i]["message"]["params"]["contentChanges"][0]["text"],
                original
            );
            if messages[i + 1..].iter().any(|r| {
                r["message"]["method"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("textDocument/semanticTokens/"))
            }) {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no semantic request after didChange: {messages:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // A scripted edit bypasses the after-edit hook. The reply already
    // in flight must resolve against the sent text, then shift two bytes.
    lua(&state, "pmacs.window.buffer():insert(0, 'é')");
    assert_eq!(store.lock().unwrap().next_seq(&uri), 5);
    assert_eq!(store.lock().unwrap().synced_seq(&uri), 4);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tick(&mut state);
        if store.lock().unwrap().pending_edits(&uri).len() == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the anchored full response never pruned the acknowledged edits"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        positioned(&state, &uri),
        Some(vec![(2, 4), (5, 9), (14, 19), (20, 24)])
    );
    assert!(
        store_is_stale(&state, &uri),
        "the server never received the scripted edit"
    );
    assert_eq!(store.lock().unwrap().synced_seq(&uri), 4);
    let messages = wire(dir.path());
    let changes: Vec<_> = messages
        .iter()
        .filter(|r| r["message"]["method"] == "textDocument/didChange")
        .collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0]["message"]["params"]["contentChanges"][0]["text"],
        original
    );
}

#[cfg(feature = "crdt")]
#[test]
fn c6b_recorder_sees_a_gpu_peer_op_in_bytes() {
    use pmacs::crdt::CrdtState;
    use pmacs::semantic_tokens::DocumentEdit;
    let (state, _dir, uri) = attached("fn main() {}\n");
    let registry = state.core.borrow().registry.clone();
    let bid = state.core.borrow().active_window().buffer_id;
    let peer = CrdtState::new(707).expect("GPU peer");
    {
        let mut reg = registry.borrow_mut();
        let buf = reg.get_mut(bid).unwrap();
        buf.upgrade_to_crdt(1).unwrap();
        peer.import_snapshot(&buf.crdt_state().unwrap().export_snapshot().unwrap())
            .unwrap();
    }
    let version = peer.version();
    peer.insert(0, "é").unwrap();
    let op = peer.export_updates_since(&version).unwrap();
    registry
        .borrow_mut()
        .get_mut(bid)
        .unwrap()
        .apply_remote_crdt_op(&op)
        .unwrap();
    let store = state.lsp_manager.borrow().semantic_token_store();
    let guard = store.lock().unwrap();
    assert_eq!(guard.next_seq(&uri), 1);
    assert_eq!(guard.synced_seq(&uri), 0);
    assert_eq!(
        guard.pending_edits(&uri),
        vec![DocumentEdit {
            start: 0,
            old_end: 0,
            inserted_len: 2
        }]
    );
    assert_eq!(
        guard
            .positioned_tokens(&uri)
            .unwrap()
            .1
            .iter()
            .map(|t| (t.start, t.end))
            .collect::<Vec<_>>(),
        vec![(2, 4), (5, 9)]
    );
}

#[cfg(feature = "crdt")]
#[test]
fn c6b_gpu_edit_shapes_correct_when_the_held_answer_lands() {
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .unwrap();
    if !binary.join("pmacs-gpu").exists() {
        assert!(
            std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
            "GPU binary required"
        );
        eprintln!("skipping C6b edit shapes: build pmacs-gpu first");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/probes/e6b_closure.py"
        ))
        .arg(binary)
        .arg(dir.path())
        .arg("cases")
        .output()
        .unwrap();
    if output.status.code() == Some(3) && std::env::var_os("PMACS_REQUIRE_GPU").is_none() {
        eprintln!("skipping C6b edit shapes: no GPU adapter");
        return;
    }
    assert!(
        output.status.success(),
        "edit probe failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
