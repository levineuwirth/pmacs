//! E6d.2 --- `ContentModified` is not degradation.
//!
//! A hundred keystrokes into a buffer attached to the fake server in
//! `contentmodified` mode, where every request the typing raises (the
//! semantic-token and inlay-hint pulls behind each coalesced
//! `didChange`) is answered with the spec's retry code, leave the
//! server's status ready: never `degraded`, no last error. The same
//! hundred against the `error` mode, whose answers are real errors,
//! degrade it, so the exclusion is exactly the retry codes and not
//! error responses as a class.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
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

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e6d-status-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An editor visiting `dir/a.rs`, the rust server the fake in `mode`,
/// returned once the fake has initialized.
fn editor_with_fake(dir: &Path, mode: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{ PMACS_FAKE_LSP_MODE = '{mode}' }},
             }}"
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    exec(&s, "pmacs.editor.goto_byte(12)");
    let mut s = s;
    let initialized = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind=='initialized' then return true end \
       end \
       return false \
     end)()";
    assert!(pump_lua_flag(&mut s, initialized, 10), "fake server init");
    s
}

/// The modeline's label for the attached server and whether the
/// status layer holds a last error.
fn label_and_error(s: &EditorState) -> (String, bool) {
    eval(
        s,
        "local rec = pmacs.lsp.active_attachment() \
         return pmacs.lsp.modeline_label(rec.server), pmacs.lsp.last_error(rec.server) ~= nil",
    )
}

/// Type a hundred characters, one per tick with the coalescer flushed
/// every few, ticking the server's answers in between, and return
/// every distinct modeline label seen and whether a last error was
/// ever held. Every flush raises a semantic-token pull the fake
/// answers at once, so the answers land while typing continues.
fn type_a_hundred(s: &mut EditorState) -> (Vec<String>, bool) {
    let mut labels: Vec<String> = Vec::new();
    let mut errored = false;
    for i in 0..100u32 {
        press(
            s,
            KeyCode::Char(char::from(b'a' + u8::try_from(i % 26).unwrap())),
        );
        if i % 3 == 2 {
            exec(s, "pmacs.lsp._flush_did_changes()");
        }
        let until = Instant::now() + Duration::from_millis(15);
        while Instant::now() < until {
            tick(s);
            let (label, err) = label_and_error(s);
            if !labels.contains(&label) {
                labels.push(label);
            }
            errored |= err;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    // Let the last answers land.
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        tick(s);
        let (label, err) = label_and_error(s);
        if !labels.contains(&label) {
            labels.push(label);
        }
        errored |= err;
        std::thread::sleep(Duration::from_millis(2));
    }
    (labels, errored)
}

#[test]
fn a_hundred_keystrokes_answered_content_modified_leave_the_status_healthy() {
    let dir = temp_dir("cm");
    let mut s = editor_with_fake(&dir, "contentmodified");
    let (labels, errored) = type_a_hundred(&mut s);
    assert!(
        !labels.iter().any(|l| l == "degraded"),
        "the modeline never read degraded; labels seen: {labels:?}"
    );
    assert!(!errored, "no last error was held");
    assert_eq!(
        label_and_error(&s).0,
        "ready",
        "and the server is ready when the typing stops"
    );
    // The positive control: the typing raised requests and the fake
    // answered every one with ContentModified, which the status layer
    // counts. A run that raised no request would pass the assertions
    // above vacuously.
    let responses: u64 = eval(
        &s,
        "return pmacs.lsp.status_summary(pmacs.lsp.active_attachment().server).retry_responses",
    );
    assert!(
        responses >= 30,
        "the typing raised requests the fake answered ContentModified: {responses}"
    );
}

#[test]
fn a_hundred_keystrokes_answered_with_a_real_error_still_degrade_it() {
    let dir = temp_dir("err");
    let mut s = editor_with_fake(&dir, "error");
    let (labels, errored) = type_a_hundred(&mut s);
    assert!(
        labels.iter().any(|l| l == "degraded"),
        "a real error response degrades the status; labels seen: {labels:?}"
    );
    assert!(errored, "and the last error is held");
}
