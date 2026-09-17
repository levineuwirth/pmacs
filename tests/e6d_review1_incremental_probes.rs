// tests/e6d_review1_incremental_probes.rs --- E6d review 1: the ranged
// didChange in the states the phase's witness left out.

//! Behavioral probes against E6d.1 through the same seam as
//! `e6d_incremental_didchange_acceptance`: dispatched keys and Lua
//! writes against the fake server in its full-sync default and its two
//! incremental modes, the fake recording the document it holds after
//! every notification, the three recordings compared to each other and
//! to the buffer.
//!
//! The states: an undo, which is the arbiter's compensation arriving
//! as an edit the recorder must log like any other; a multi-line
//! insert with an edit after it in the same flush, so the second
//! range's line is counted on the mirror the first one moved; a burst
//! on a line already holding non-ASCII text, where the second range's
//! UTF-16 column crosses the first edit on that line; a replacement
//! whose text carries a newline; and a keystroke typed before the
//! server has initialized, which must go whole and deferred, followed
//! by one typed after, which must go as a range.
//!
//! Helpers are the acceptance suite's, copied so the probe stands
//! alone.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

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

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

/// `C-/`, bound to `buffer.undo`.
fn undo(s: &mut EditorState) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('/'), KeyModifiers::CONTROL),
    );
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// An editor with isolated roots whose state dir is `dir`, the rust
/// server pointed at the fake in `mode` with the change sink at
/// `sink`, visiting `dir/a.rs` holding `body`, cursor at byte 0. Not
/// pumped: the caller decides whether to wait for the fake to
/// initialize.
fn fake_editor_unpumped(dir: &Path, mode: &str, sink: &Path, body: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = fake_lsp_path();
    let sink_disp = sink.display().to_string();
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{
                 PMACS_FAKE_LSP_MODE = '{mode}',
                 PMACS_FAKE_LSP_CHANGE_SINK = '{sink_disp}',
               }},
             }}"
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, body).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

fn fake_editor(dir: &Path, mode: &str, sink: &Path, body: &str) -> EditorState {
    let mut s = fake_editor_unpumped(dir, mode, sink, body);
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    s
}

/// Every didChange line of the sink, in arrival order, as `(text the
/// server holds afterward, whether the notification carried ranges)`.
fn did_changes(sink: &Path) -> Vec<(String, bool)> {
    let Ok(raw) = std::fs::read_to_string(sink) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("method").and_then(|m| m.as_str()) == Some("textDocument/didChange"))
        .filter_map(|v| {
            let text = v.get("text")?.as_str()?.to_owned();
            let ranged = v.get("ranged").and_then(serde_json::Value::as_bool)?;
            Some((text, ranged))
        })
        .collect()
}

/// Flush the coalescer and pump until the fake's latest didChange in
/// the sink is a new one that leaves it holding the buffer's text,
/// returning that one and how many didChanges the sink holds now. A
/// server left holding something else is the failure, reported with
/// what it holds.
fn flush_and_wait(
    s: &mut EditorState,
    sink: &Path,
    seen: usize,
    what: &str,
) -> ((String, bool), usize) {
    exec(s, "pmacs.lsp._flush_did_changes()");
    let expected = buffer_text(s);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tick(s);
        let changes = did_changes(sink);
        if changes.len() > seen {
            let last = changes.last().cloned().expect("at least one");
            if last.0 == expected {
                return (last, changes.len());
            }
        }
        assert!(
            Instant::now() < deadline,
            "after {what} the server holds {:?} where the buffer holds {expected:?} ({} didChanges, {seen} before the step)",
            changes.last().map_or("", |(t, _)| t.as_str()),
            changes.len()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

const BODY: &str = "fn main() {\n    let s = \"naïve\";\n    let n = 1;\n}\n";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e6d-r1-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs a Lua chunk over the visited buffer as `b` with its text as
/// `t`, then the after-edit hook a command would have run, as the
/// runtime's own scripted writers do.
fn lua_write(s: &EditorState, body: &str) {
    exec(
        s,
        &format!(
            "local b = pmacs.window.buffer(); local t = b:slice(0, b:len()) \
             {body} \
             pmacs.hook.run('buffer.after-edit')"
        ),
    );
}

/// The edit sequence every mode takes. Returns the server's document
/// after every flush, with its ranged flag.
fn drive(s: &mut EditorState, sink: &Path) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    let mut step = |s: &mut EditorState, what: &str| {
        let (got, total) = flush_and_wait(s, sink, seen, what);
        seen = total;
        out.push(got);
    };
    // Typed text, flushed, then undone: the undo is the arbiter's
    // compensation, an edit the recorder logs like any other.
    type_str(s, "ab");
    press(s, KeyCode::Esc);
    step(s, "typing ab");
    undo(s);
    step(s, "undoing ab");
    assert_eq!(buffer_text(s), BODY, "the undo restored the body");
    // A multi-line insert at the top and, in the same flush, a
    // keystroke at the end of the document: the second range's line
    // is counted on a mirror the first insert moved by two lines.
    lua_write(s, "b:insert(0, 'one\\ntwo\\n')");
    exec(
        s,
        "local b = pmacs.window.buffer(); pmacs.editor.goto_byte(b:len())",
    );
    type_str(s, "z");
    press(s, KeyCode::Esc);
    step(s, "a multi-line insert and a keystroke after it");
    // A burst on the line that already holds `ï`: `é` before it, then
    // `x` after the `e` of `naïve`, both in one flush, so the second
    // range's UTF-16 column is counted across the first edit on the
    // same line.
    lua_write(
        s,
        "b:insert(t:find('ïve', 1, true) - 1, 'é'); \
         t = b:slice(0, b:len()); \
         b:insert(t:find('ve\"', 1, true) + 1, 'x');",
    );
    step(s, "two inserts on the non-ASCII line");
    // A replacement whose text carries a newline.
    lua_write(
        s,
        "local a = t:find('let n', 1, true) - 1; b:replace(a, a + 5, 'let\\nn')",
    );
    step(s, "a replacement carrying a newline");
    // A delete that spans the two lines the replacement made, and a
    // keystroke inside the same flush.
    lua_write(
        s,
        "local a = t:find('let\\nn', 1, true) - 1; b:delete(a + 3, a + 4)",
    );
    type_str(s, "q");
    press(s, KeyCode::Esc);
    step(s, "a delete across the new line break and a keystroke");
    // Undo the keystroke: the compensation lands under the arbiter's
    // rules and is logged as an edit.
    undo(s);
    step(s, "undoing q");
    out
}

/// Under full sync and under both incremental encodings the server
/// holds the same document after every flush of the same sequence,
/// and it is the buffer's; the incremental modes received ranges.
#[test]
fn undo_multiline_and_non_ascii_bursts_leave_every_server_holding_the_buffer() {
    let mut recordings: Vec<(String, Vec<(String, bool)>)> = Vec::new();
    for mode in ["", "incremental", "incremental8"] {
        let tag = if mode.is_empty() { "full" } else { mode };
        let dir = temp_dir(tag);
        let sink = dir.join("changes.jsonl");
        let mut s = fake_editor(&dir, mode, &sink, BODY);
        let got = drive(&mut s, &sink);
        assert_eq!(got.len(), 7, "{tag}: seven steps");
        let ranged = !mode.is_empty();
        for (i, (_, was_ranged)) in did_changes(&sink).iter().enumerate() {
            assert_eq!(
                *was_ranged,
                ranged,
                "{tag}: didChange {i} {} ranges",
                if ranged { "carries" } else { "carries no" }
            );
        }
        recordings.push((tag.to_owned(), got));
    }
    let (full_tag, full) = &recordings[0];
    for (tag, got) in &recordings[1..] {
        let texts: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
        let full_texts: Vec<&str> = full.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            texts, full_texts,
            "{tag}: the server's document after every flush equals {full_tag}'s"
        );
    }
    let end = full.last().map(|(t, _)| t.as_str()).unwrap_or_default();
    assert!(
        end.starts_with("one\ntwo\nfn main()"),
        "the multi-line insert stands at the top: {end:?}"
    );
    assert!(
        end.contains("naéïvex\"") && end.contains("letn = 1") && end.ends_with("}\nz"),
        "every step's edit is in the final text: {end:?}"
    );
}

/// A keystroke typed before the fake has initialized goes as the whole
/// document, deferred until the handshake completes; the next one,
/// typed after, goes as a range; the server holds the buffer after
/// each.
#[test]
fn a_keystroke_before_initialization_goes_whole_and_the_next_as_a_range() {
    let dir = temp_dir("early");
    let sink = dir.join("changes.jsonl");
    let mut s = fake_editor_unpumped(&dir, "incremental", &sink, BODY);
    // The positive control: nothing has been ticked, so the initialize
    // response cannot have been read.
    let initialized: bool = eval(&s, &format!("return {INITIALIZED}"));
    assert!(!initialized, "the fake must still be initializing here");
    type_str(&mut s, "q");
    press(&mut s, KeyCode::Esc);
    exec(&s, "pmacs.lsp._flush_did_changes()");
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    let ((text, ranged), seen) = flush_and_wait(&mut s, &sink, 0, "typing q before init");
    assert_eq!(text, buffer_text(&s));
    assert!(
        !ranged,
        "a didChange issued before the capabilities were known ships the whole document"
    );
    type_str(&mut s, "r");
    press(&mut s, KeyCode::Esc);
    let ((text, ranged), _) = flush_and_wait(&mut s, &sink, seen, "typing r after init");
    assert_eq!(text, buffer_text(&s));
    assert!(
        ranged,
        "the keystroke after initialization ships as a range"
    );
    assert!(text.starts_with("qrfn main()"), "{text:?}");
}
