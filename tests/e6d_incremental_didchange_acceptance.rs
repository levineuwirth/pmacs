//! E6d.1 --- incremental `didChange`.
//!
//! The coalescer ships the edits since the last sync as ranged
//! `contentChanges` to a server that negotiated
//! `TextDocumentSyncKind.Incremental`, and the whole document to one
//! that did not. The witness is the server's own copy of the document:
//! one edit sequence, driven through dispatched keys and Lua buffer
//! writes, against the fake server in its full-sync default, in
//! `incremental` (ranges read in UTF-16 units) and in `incremental8`
//! (UTF-8), each recording the document it holds after every
//! notification through `PMACS_FAKE_LSP_CHANGE_SINK`; the three
//! recordings must be identical to each other and to the buffer at
//! every flush, and the incremental ones must have arrived as ranges.
//! Then the same against rust-analyzer: a type error typed into a
//! crate is diagnosed at the bytes it was typed at, which a range the
//! server applied to the wrong text could not produce.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;
#[path = "support/mod.rs"]
mod support;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
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

/// An editor with isolated roots whose state dir is `dir`, the rust
/// server pointed at the fake in `mode` with the change sink at
/// `sink`, visiting `dir/a.rs` holding `body`, cursor at byte 0,
/// returning once the fake has initialized.
fn fake_editor(dir: &Path, mode: &str, sink: &Path, body: &str) -> EditorState {
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

/// Flush the coalescer and pump until the fake has written at least
/// one more didChange to the sink (a completion request raised by the
/// typing flushes on its own, so a step may have sent more than one),
/// returning the last one --- what the server holds after the step ---
/// and how many didChanges the sink holds now.
fn flush_and_wait(
    s: &mut EditorState,
    sink: &Path,
    seen: usize,
    what: &str,
) -> ((String, bool), usize) {
    exec(s, "pmacs.lsp._flush_did_changes()");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tick(s);
        let changes = did_changes(sink);
        if changes.len() > seen {
            let last = changes.last().cloned().expect("at least one");
            return (last, changes.len());
        }
        assert!(
            Instant::now() < deadline,
            "no didChange reached the fake server after {what}; buffer: {:?}",
            buffer_text(s)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The edit sequence every mode takes: dispatched keys (self-inserts,
/// Enter, Backspace, an auto-paired opener) and Lua writes (an insert
/// of non-ASCII text, a delete across a line break, a replace, edits at
/// both ends of the document), some coalesced into one flush. Returns
/// the server's document after every flush, with its ranged flag.
fn drive(s: &mut EditorState, sink: &Path) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    let mut step = |s: &mut EditorState, what: &str| {
        let (got, total) = flush_and_wait(s, sink, seen, what);
        seen = total;
        assert_eq!(
            got.0,
            buffer_text(s),
            "after {what} the server holds what the buffer holds"
        );
        out.push(got);
    };
    // Two self-inserts at the top, coalesced.
    type_str(s, "ab");
    step(s, "typing ab");
    // A newline, then two Backspaces that cross it back. Escape first:
    // typing raised the completion popup, and with it open Enter
    // accepts a candidate instead of inserting a newline.
    press(s, KeyCode::Esc);
    press(s, KeyCode::Enter);
    step(s, "Enter");
    press(s, KeyCode::Backspace);
    press(s, KeyCode::Backspace);
    step(s, "two Backspaces");
    // Non-ASCII in the middle of a line that already holds some, so a
    // column in the wrong units lands somewhere else.
    // A Lua write fires no `buffer.after-edit` of its own (the hook is
    // the command's), so the write is followed by the hook a command
    // would have run, as the runtime's own scripted writers do.
    exec(
        s,
        "local b = pmacs.window.buffer(); local t = b:slice(0, b:len()) \
         b:insert(t:find('aïve', 1, true) - 1, 'é🦀x'); pmacs.hook.run('buffer.after-edit')",
    );
    step(s, "inserting é🦀x");
    // A delete spanning the first line break.
    exec(
        s,
        "local b = pmacs.window.buffer(); local t = b:slice(0, b:len()) \
         b:delete(t:find('(', 1, true) - 1, t:find('let', 1, true) - 1) \
         pmacs.hook.run('buffer.after-edit')",
    );
    step(s, "deleting across a line break");
    // A replace of a multi-byte run by a shorter ASCII one.
    exec(
        s,
        "local b = pmacs.window.buffer(); local t = b:slice(0, b:len()) \
         b:replace(t:find('é🦀', 1, true) - 1, t:find('ve\"', 1, true) - 1, 'q') \
         pmacs.hook.run('buffer.after-edit')",
    );
    step(s, "replacing");
    // An auto-paired opener: the keystroke and its closer are two
    // edits in one command, one flush.
    exec(
        s,
        "local b = pmacs.window.buffer(); pmacs.editor.goto_byte(b:len())",
    );
    type_str(s, "(");
    press(s, KeyCode::Esc);
    step(s, "an auto-paired (");
    // Both ends of the document.
    exec(
        s,
        "local b = pmacs.window.buffer(); b:insert(b:len(), 'tail\\n'); b:delete(0, 1); \
         pmacs.hook.run('buffer.after-edit')",
    );
    step(s, "the document's ends");
    // A burst: three inserts, a delete and an insert in one flush.
    exec(s, "pmacs.editor.goto_byte(5)");
    type_str(s, "xyz");
    press(s, KeyCode::Backspace);
    type_str(s, "w");
    press(s, KeyCode::Esc);
    step(s, "a burst of five edits");
    out
}

const BODY: &str = "fn main() {\n    let s = \"naïve\";\n    let n = 1;\n}\n";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e6d-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Under full sync, under incremental sync in UTF-16 and under
/// incremental sync in UTF-8, the server holds the same document after
/// every flush of the same edit sequence, and it is the buffer's; the
/// incremental modes received ranges and the full mode did not.
#[test]
fn the_same_edits_leave_every_server_holding_the_buffer_under_both_sync_kinds() {
    let mut recordings: Vec<(String, Vec<(String, bool)>)> = Vec::new();
    for mode in ["", "incremental", "incremental8"] {
        let tag = if mode.is_empty() { "full" } else { mode };
        let dir = temp_dir(tag);
        let sink = dir.join("changes.jsonl");
        let mut s = fake_editor(&dir, mode, &sink, BODY);
        let got = drive(&mut s, &sink);
        assert_eq!(got.len(), 9, "{tag}: nine steps");
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
    for mark in ["()", "nqve", "tail\n", "xyw"] {
        assert!(
            end.contains(mark),
            "every step's edit is in the final text ({mark:?} in {end:?})"
        );
    }
}

/// rust-analyzer's own reading of the document after the ranged
/// change: the token it reports at the typed bytes is a three-unit
/// `string` where a one-unit `number` stood, which a range applied to
/// the wrong text could not produce. The server negotiates
/// incremental sync, so every keystroke reached it as a range.
///
/// Its diagnostics were the oracle asked for and cannot be: probed by
/// hand (2026-09-16, rust-analyzer 2026-09-06), this build publishes
/// native diagnostics once at `didOpen` and again only on `didSave`,
/// where it re-reads the disk; a `didChange` alone, ranged or whole,
/// republishes nothing in thirty seconds, so a test on them would
/// witness the server's cadence and not the sync. The semantic tokens
/// answer a request against the document the changes produced.
#[test]
fn rust_analyzer_reads_the_typed_bytes_where_they_were_typed() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let dir = temp_dir("ra");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"e6d_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let main_rs = dir.join("src").join("main.rs");
    // A non-ASCII comment above the edited line: a UTF-16 column
    // slip on the line before would not show, one on the edited line
    // would, and the edited line's own prefix is ASCII either way.
    let body = "// naïve\nfn main() {\n    let total: i32 = 1;\n    let _ = total;\n}\n";
    std::fs::write(&main_rs, body).unwrap();
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            main_rs.display().to_string()
        ),
    );
    let mut s = s;
    let initialized = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind=='initialized' then return true end \
       end \
       return false \
     end)()";
    assert!(pump_lua_flag(&mut s, initialized, 60), "rust-analyzer init");
    let incremental: bool = eval(
        &s,
        "local caps = pmacs.lsp.capabilities(pmacs.lsp.list()[1].id) \
         local sync = caps and caps.textDocumentSync \
         return type(sync) == 'table' and sync.change == 2",
    );
    assert!(incremental, "rust-analyzer negotiates incremental sync");
    let line = 2u64;
    let col = u64::try_from("    let total: i32 = ".len()).unwrap();
    // The token the server reports at (line, col) of its document, as
    // `(length, type name)`, once the store holds a token there.
    let token_at = format!(
        "(function() \
           local rec = pmacs.lsp.active_attachment() \
           local legend = pmacs.lsp.capabilities(rec.server).semanticTokensProvider.legend.tokenTypes \
           for _, t in ipairs(pmacs.semantic_tokens.tokens(rec.server, rec.uri)) do \
             if t.line == {line} and t.start == {col} then \
               return t.length .. ':' .. tostring(legend[t.token_type + 1]) \
             end \
           end \
           return nil \
         end)()"
    );
    // The attach-time pull: a one-unit number stands at the bytes.
    assert!(
        pump_lua_flag(&mut s, &format!("{token_at} == '1:number'"), 60),
        "rust-analyzer's tokens for the opened file; at the bytes: {:?}",
        eval::<Option<String>>(&s, &format!("return {token_at}"))
    );
    // `let total: i32 = 1;` --- put the caret after the `1`, remove it,
    // and type `"s"`.
    let one = body.find("= 1;").expect("the initializer") + 2;
    exec(&s, &format!("pmacs.editor.goto_byte({})", one + 1));
    press(&mut s, KeyCode::Backspace);
    type_str(&mut s, "\"s\"");
    press(&mut s, KeyCode::Esc);
    let expected = "// naïve\nfn main() {\n    let total: i32 = \"s\";\n    let _ = total;\n}\n";
    assert_eq!(buffer_text(&s), expected, "the buffer after typing");
    // The flush ships the edits as ranges and pulls the tokens again;
    // the server's answer is against the document those ranges made.
    exec(&s, "pmacs.lsp._flush_did_changes()");
    assert!(
        pump_lua_flag(&mut s, &format!("{token_at} == '3:string'"), 60),
        "rust-analyzer reads a three-byte string at the typed bytes; at the bytes: {:?}",
        eval::<Option<String>>(&s, &format!("return {token_at}"))
    );
}
