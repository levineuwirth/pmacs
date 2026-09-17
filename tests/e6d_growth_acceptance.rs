//! E6d.5 --- what grows.
//!
//! The daemon grew by about 3 MB per keystroke on `src/editor.rs` with
//! rust-analyzer attached, to 9.5 GB in sixteen minutes of scripted
//! typing. The object was the async runtime's job table: the handler
//! for the server's `workspace/semanticTokens/refresh` answered with a
//! bare `pmacs.lsp.request_semantic_tokens`, a handle nobody awaited,
//! and the runtime keeps a settled job's result until someone takes
//! it --- so every refresh, and rust-analyzer asks for one after nearly
//! every edit, left a whole document's tokens (a JSON value of some 25
//! MB on a 14k-line file) in the table forever. The witness: against a
//! fake that asks for a refresh after every `didChange`, twenty
//! flushed keystrokes leave the job table empty once the answers land,
//! the server having asked twenty times and jobs having existed.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;

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

fn pump_until(s: &mut EditorState, flag: &str, secs: u64) -> bool {
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
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

/// Every refresh the server asks for is answered by a pull whose
/// result is taken: after twenty flushed keystrokes, each answered
/// with a `workspace/semanticTokens/refresh`, the runtime's job table
/// drains to nothing.
#[test]
fn a_semantic_tokens_refresh_per_edit_leaves_no_job_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a_path = dir.path().join("a.rs");
    std::fs::write(&a_path, b"fn a() {}\n\n").expect("write a");
    let a_disp = a_path.display().to_string();
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut s = EditorState::new_with_roots(&iso::roots());
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{ PMACS_FAKE_LSP_MODE = 'semantictokensrefresh' }},
             }}"
        ),
    );
    exec(&s, &format!("pmacs.buffer.find_or_open('{a_disp}')"));
    exec(&s, "pmacs.editor.goto_byte(10)");
    let tokens_held = format!(
        "(function() \
           local sid \
           for _,r in ipairs(pmacs.lsp.list()) do \
             if r.state and r.state.kind=='initialized' then sid=r.id end \
           end \
           if not sid then return false end \
           local t = pmacs.semantic_tokens.tokens(sid, 'file://{a_disp}') \
           return t ~= nil and #t > 0 \
         end)()"
    );
    assert!(
        pump_until(&mut s, &tokens_held, 10),
        "the attach-time refresh re-pulled tokens into the store"
    );
    // The refresh requests the server has made so far, from the
    // status layer's log of server-to-client requests.
    let refreshes = "(function() \
       local n = 0 \
       for _, r in ipairs(pmacs.lsp.list()) do \
         for _, m in ipairs(pmacs.lsp.recent_messages(r.id)) do \
           if m.summary == 'server→client request: workspace/semanticTokens/refresh' then n = n + 1 end \
         end \
       end \
       return n \
     end)()";
    let before: u64 = eval(&s, &format!("return {refreshes}"));

    // Twenty keystrokes, each flushed so the fake sees a didChange and
    // asks for a refresh, with the answers ticked in between; jobs
    // exist while the answers are in flight.
    let mut jobs_seen = 0u64;
    for i in 0..20u32 {
        press(
            &mut s,
            KeyCode::Char(char::from(b'a' + u8::try_from(i % 26).unwrap())),
        );
        exec(&s, "pmacs.lsp._flush_did_changes()");
        let until = Instant::now() + Duration::from_millis(20);
        while Instant::now() < until {
            tick(&mut s);
            jobs_seen = jobs_seen.max(eval(&s, "return pmacs._async._pending_len()"));
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    // The positive control: the server asked for twenty refreshes and
    // the client raised jobs answering them.
    let asked = format!("{refreshes} >= {}", before + 20);
    assert!(
        pump_until(&mut s, &asked, 10),
        "the fake asked for a refresh after every didChange; {} seen",
        eval::<u64>(&s, &format!("return {refreshes}"))
    );
    assert!(jobs_seen > 0, "the refreshes raised jobs");
    // The bound: once every answer has landed, no settled job is left
    // untaken. A bare request in the refresh handler leaves one per
    // refresh, twenty here, and gigabytes on a real file.
    assert!(
        pump_until(&mut s, "pmacs._async._pending_len() == 0", 10),
        "the async job table drains after the refreshes; {} left",
        eval::<u64>(&s, "return pmacs._async._pending_len()")
    );
}
