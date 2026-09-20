// tests/e7c_didsave_acceptance.rs --- E7c.1, a save reaches the server.

//! `textDocument/didSave` after every save of an attached buffer,
//! through the production save command and the real drain, against
//! the fake in the three shapes a server can declare:
//!
//! * `save: { includeText: true }` --- the notification carries the
//!   saved text, and it is the text on disk, the edit typed just
//!   before the save included: the pending `didChange` is flushed
//!   first, so the server's document and the file agree.
//! * `save: true` --- the notification carries no text.
//! * no `save` (the fake's default, a bare sync kind) --- nothing is
//!   sent on save, as the spec has it.
//!
//! And the flycheck the fake runs on a save --- rust-analyzer's own
//! cycle on its `rust-analyzer/flycheck/<n>` token --- shows on the
//! modeline as `LSP:ready·check`, never as anything but `ready`, which
//! is E7b.1's row doing what it was built for. The real server's
//! version of this row is in `tests/e7b_review_wire_acceptance.rs`.
//!
//! The save retry (E7c.1's second half, its trigger ruled at fix round
//! 3): a save whose check has not begun goes again behind the next
//! `didChange` for its document, never on a clock; the flycheck
//! token's begin stands the watch down and no other token's does.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};
use serde_json::Value;

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
        std::thread::sleep(Duration::from_millis(5));
    }
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7c-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lsp_segment(s: &EditorState) -> Option<String> {
    let outcome = evaluate_statusline(
        s.lua_host.lua(),
        &s.core,
        &s.statusline_registry,
        StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        },
    );
    let StatuslineEvaluationOutcome::Ready(windows) = outcome.outcome else {
        return None;
    };
    windows
        .into_iter()
        .flat_map(|w| w.right)
        .find(|seg| seg.face == "ui.modeline.lsp")
        .map(|seg| seg.text)
}

/// The fake in `mode`, its didSave sink at `<dir>/saves.jsonl`, the
/// editor visiting `<dir>/a.rs` with the handshake answered.
struct Fixture {
    s: EditorState,
    file: PathBuf,
    sink: PathBuf,
}

fn open_with_fake(tag: &str, mode: &str) -> Fixture {
    open_with_fake_config(tag, mode, "", "")
}

/// `open_with_fake` with extra config fields (Lua source after the
/// command) and extra environment for the fake.
fn open_with_fake_config(tag: &str, mode: &str, fields: &str, env: &str) -> Fixture {
    let dir = temp_dir(tag);
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    let sink = dir.join("saves.jsonl");
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = {:?},
               env = {{ PMACS_FAKE_LSP_MODE = {mode:?}, PMACS_FAKE_LSP_SAVE_SINK = {:?}{env} }},
               {fields}
             }}",
            fake_lsp_path(),
            sink.display().to_string()
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    Fixture { s, file, sink }
}

/// The sink's complete lines: the fake appends one JSON line per
/// save, and a read can land while a line is half written, so only
/// lines the file has finished (ended by a newline) are parsed.
fn saves(sink: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(sink).unwrap_or_default();
    let complete = text.rsplit_once('\n').map_or("", |(done, _)| done);
    complete
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line per save"))
        .collect()
}

/// Type at the top of the file and save through the production
/// command; tick until `buffer.after-save` has fired.
fn type_and_save(f: &mut Fixture, typed: &str) {
    exec(
        &f.s,
        "_G.__e7c_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7c_saved = true end)
         pmacs.editor.goto_byte(0)",
    );
    for ch in typed.chars() {
        press(&mut f.s, KeyCode::Char(ch));
    }
    // The completion popup opens on the last word typed and RET would
    // accept its first candidate; the row is about the save, so the
    // popup is dismissed first.
    if f.s.core.borrow().completion_popup.lock().unwrap().is_some() {
        press(&mut f.s, KeyCode::Esc);
    }
    press(&mut f.s, KeyCode::Enter);
    exec(&f.s, "pmacs.command.invoke('buffer.save')");
    assert!(
        pump_lua_flag(&mut f.s, "_G.__e7c_saved", 10),
        "buffer.after-save fires; status {:?}",
        f.s.core.borrow().status
    );
}

/// Tick for `secs`, returning every distinct `LSP:` text seen and
/// whether `pred` held at any tick.
fn watch(
    s: &mut EditorState,
    secs: u64,
    pred: impl Fn(&EditorState) -> bool,
) -> (Vec<String>, bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut seen = Vec::new();
    let mut held = false;
    while Instant::now() < deadline {
        tick(s);
        let text = lsp_segment(s).unwrap_or_default();
        if seen.last() != Some(&text) {
            seen.push(text);
        }
        held |= pred(s);
        std::thread::sleep(Duration::from_millis(2));
    }
    (seen, held)
}

/// `save: { includeText: true }`: one `didSave` per save, carrying the
/// bytes on disk --- the typed line included, since the didChange was
/// flushed before it --- and the fake's flycheck cycle showing as a
/// suffix on `ready`. Bitten by removing the after-save hook in
/// `lsp.lua` (no line in the sink) or the flush before the send (the
/// text lags the file).
#[test]
fn e7c_1_a_save_sends_did_save_with_the_saved_text_when_asked() {
    let mut f = open_with_fake("text", "didsave");
    let negotiated: Option<bool> = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         return pmacs.lsp.save_negotiated(rec.server)",
    );
    assert_eq!(negotiated, Some(true), "the fake asked for the text");
    type_and_save(&mut f, "// saved by E7c");
    let (seen, _) = watch(&mut f.s, 1, |_| false);
    let on_disk = std::fs::read_to_string(&f.file).unwrap();
    assert_eq!(on_disk, "// saved by E7c\nfn main() {}\n");
    let lines = saves(&f.sink);
    assert_eq!(lines.len(), 1, "one didSave for one save: {lines:?}");
    assert!(
        lines[0]["uri"].as_str().unwrap().ends_with("/a.rs"),
        "{:?}",
        lines[0]
    );
    assert_eq!(
        lines[0]["text"].as_str(),
        Some(on_disk.as_str()),
        "the notification carries the bytes on disk"
    );
    assert!(
        !seen
            .iter()
            .any(|t| t.contains("idx") || t.contains("degraded")),
        "a flycheck never leaves ready: {seen:?}"
    );
    eprintln!("SAVE labels seen: {seen:?}");
}

/// `save: true`: the notification goes out without `text`.
#[test]
fn e7c_1_a_save_sends_did_save_without_text_when_not_asked() {
    let mut f = open_with_fake("notext", "didsavenotext");
    let negotiated: Option<bool> = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         return pmacs.lsp.save_negotiated(rec.server)",
    );
    assert_eq!(negotiated, Some(false), "save asked for, the text not");
    type_and_save(&mut f, "// no text");
    watch(&mut f.s, 1, |_| false);
    let lines = saves(&f.sink);
    assert_eq!(lines.len(), 1, "one didSave: {lines:?}");
    assert!(lines[0]["text"].is_null(), "no text: {:?}", lines[0]);
}

/// No `save` declared (a bare sync kind): a save sends nothing, and
/// the manager says so.
#[test]
fn e7c_1_a_server_that_declared_no_save_gets_no_did_save() {
    let mut f = open_with_fake("none", "");
    let negotiated: Option<bool> = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         return pmacs.lsp.save_negotiated(rec.server)",
    );
    assert_eq!(negotiated, None);
    type_and_save(&mut f, "// nothing");
    watch(&mut f.s, 1, |_| false);
    assert!(
        !f.sink.exists(),
        "no didSave reached the fake: {:?}",
        saves(&f.sink)
    );
    let sent: bool = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         return pmacs.lsp.did_save(rec.server, rec.uri, 'x')",
    );
    assert!(!sent, "did_save reports that nothing was sent");
}

/// The flycheck cycle the fake runs on the save: `LSP:ready·check`
/// while it is in flight and `LSP:ready` after, `idx` never. Read
/// through the segment with the fake holding the cycle open until the
/// test closes it by a second save, so the suffix is observable rather
/// than a race against the fake's own end.
#[test]
fn e7c_1_the_flycheck_a_save_starts_is_a_suffix_on_ready() {
    let mut f = open_with_fake("suffix", "didsave");
    // The fake's cycle begins and ends in the same write; hold it
    // open instead through the echo, then let a save's own cycle run
    // and end. The begin arrives through the drain like the save's.
    exec(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
           { token = 'rust-analyzer/flycheck/held', value = { kind = 'begin', title = 'cargo check' } })",
    );
    let (seen, held) = watch(&mut f.s, 1, |s| {
        lsp_segment(s).as_deref() == Some("LSP:ready·check")
    });
    assert!(
        held,
        "the suffix shows while a flycheck is in flight: {seen:?}"
    );
    type_and_save(&mut f, "// checked");
    let (seen2, _) = watch(&mut f.s, 1, |_| false);
    let busy: Option<String> = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         local st = pmacs.lsp.status_summary(rec.server)
         return st and st.busy or nil",
    );
    assert_eq!(
        busy.as_deref(),
        Some("cargo check"),
        "the held cycle is still the busy work after the save's own ended"
    );
    let all: Vec<&String> = seen.iter().chain(seen2.iter()).collect();
    assert!(
        all.iter().all(|t| t.starts_with("LSP:ready")),
        "the label never leaves ready: {all:?}"
    );
    exec(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
           { token = 'rust-analyzer/flycheck/held', value = { kind = 'end' } })",
    );
    let (_, back) = watch(&mut f.s, 1, |s| {
        lsp_segment(s).as_deref() == Some("LSP:ready")
    });
    assert!(back, "ready without a suffix once every cycle ended");
}

/// E7c.1's retry, on the trigger the owner ruled at fix round 3: a save
/// whose check never began is announced again behind the next
/// `didChange` for the document, and on nothing else. The fake drops
/// the first save --- records it, runs no cycle, publishes nothing, as
/// rust-analyzer does with a save whose check trigger a write cancelled
/// --- and answers the second with the cycle and the check's
/// diagnostics. For two seconds after the save nothing is typed and
/// nothing is resent (the 1.5 s timer this replaces would have sent
/// the save again at 1.5 s, #285's cost); then one keystroke, and the
/// save goes out right behind its `didChange` --- the fake had read
/// exactly one more change when the second save arrived --- carrying
/// the text saved, not the buffer's; the cycle's begin stands the watch
/// down, and a further keystroke sends nothing. Bitten by removing the
/// resend call from `flush_did_change`, or by keeping the timer.
#[test]
fn e7c_1_a_save_whose_check_never_began_is_sent_again_behind_the_next_change() {
    let mut f = open_with_fake_config(
        "retry",
        "didsave",
        "save_retry = true, check_sources = { 'rustc' },",
        ", PMACS_FAKE_LSP_DROP_SAVES = '1'",
    );
    let t0 = Instant::now();
    type_and_save(&mut f, "// CHECKME");
    let sink = f.sink.clone();
    let watches = |s: &EditorState| -> Vec<(String, u32)> {
        let t: std::collections::HashMap<String, u32> = eval(s, "return pmacs.lsp._save_watches()");
        t.into_iter().collect()
    };
    assert_eq!(
        watches(&f.s).len(),
        1,
        "the save armed a watch: {:?}",
        watches(&f.s)
    );
    // Two seconds with nothing typed: the dropped save produced no
    // cycle, and no clock sent it again.
    let (seen, _) = watch(&mut f.s, 2, |_| false);
    assert_eq!(
        saves(&f.sink).len(),
        1,
        "one didSave after two idle seconds: nothing resends on a clock"
    );
    assert!(seen.iter().all(|t| t == "LSP:ready"), "no cycle: {seen:?}");
    assert_eq!(watches(&f.s).len(), 1, "the watch is still armed");
    // One keystroke: its didChange goes out at the flush cadence, and
    // the save right behind it.
    press(&mut f.s, KeyCode::Char('x'));
    let (seen, _) = watch(&mut f.s, 4, |_| saves(&sink).len() >= 2);
    let lines = saves(&f.sink);
    eprintln!(
        "SAVE resend: {} didSaves {} ms after the save, labels {seen:?}, lines {lines:?}",
        lines.len(),
        t0.elapsed().as_millis()
    );
    assert_eq!(lines.len(), 2, "the save was sent again: {lines:?}");
    assert_eq!(lines[0]["dropped"], serde_json::json!(true));
    assert_eq!(lines[1]["dropped"], serde_json::json!(false));
    assert_eq!(
        lines[1]["text"], lines[0]["text"],
        "the resend carries the text saved, not the buffer's"
    );
    assert_eq!(
        lines[1]["changes_seen"].as_u64(),
        lines[0]["changes_seen"].as_u64().map(|n| n + 1),
        "the resent save came behind exactly one didChange: {lines:?}"
    );
    let (_, cycled) = watch(&mut f.s, 3, |s| {
        lsp_segment(s).as_deref() == Some("LSP:ready·check")
            || eval::<Vec<String>>(
                s,
                "local rec = pmacs.lsp.active_attachment()
                 local out = {}
                 for _, d in ipairs(pmacs.diag.list(rec.uri)) do out[#out + 1] = tostring(d.source) end
                 return out",
            )
            .contains(&"rustc".to_owned())
    });
    assert!(
        cycled,
        "the resent save's check ran and its diagnostic landed"
    );
    assert!(
        watch(&mut f.s, 2, |s| watches(s).is_empty()).1,
        "the watch stood down on the cycle's begin: {:?}",
        watches(&f.s)
    );
    if f.s.core.borrow().completion_popup.lock().unwrap().is_some() {
        press(&mut f.s, KeyCode::Esc);
    }
    press(&mut f.s, KeyCode::Char('y'));
    let (_, _) = watch(&mut f.s, 1, |_| false);
    assert_eq!(
        saves(&f.sink).len(),
        2,
        "a keystroke after the check began sends no third save"
    );
}

/// Without `save_retry` a dropped save stays dropped: one didSave and
/// no cycle, however long the wait and whatever is typed --- the
/// control for the row above.
#[test]
fn e7c_1_without_save_retry_a_dropped_save_is_not_sent_again() {
    let mut f = open_with_fake_config(
        "noretry",
        "didsave",
        "check_sources = { 'rustc' },",
        ", PMACS_FAKE_LSP_DROP_SAVES = '1'",
    );
    type_and_save(&mut f, "// CHECKME");
    let watches: std::collections::HashMap<String, u32> =
        eval(&f.s, "return pmacs.lsp._save_watches()");
    assert!(
        watches.is_empty(),
        "no watch without save_retry: {watches:?}"
    );
    let (seen, _) = watch(&mut f.s, 2, |_| false);
    press(&mut f.s, KeyCode::Char('x'));
    let (seen2, _) = watch(&mut f.s, 2, |_| false);
    assert_eq!(saves(&f.sink).len(), 1, "one didSave, never repeated");
    assert!(
        seen.iter().chain(seen2.iter()).all(|t| t == "LSP:ready"),
        "no cycle: {seen:?} {seen2:?}"
    );
}

/// #285's case, with the fake standing in for a server slow to begin
/// its check: the save answered, but the server's first frame 1.7 s
/// late. Nothing is typed, so nothing is resent --- one `didSave`, one
/// cycle when the server gets to it, the watch standing down on its
/// begin. The timer this row retires resent the save at 1.5 s here and
/// E7c.1's wire row counted two; bitten by restoring it.
#[test]
fn e7c_fix_3_a_save_nothing_is_typed_after_is_sent_once_however_late_the_check_begins() {
    let mut f = open_with_fake_config(
        "latecheck",
        "didsave",
        "save_retry = true, check_sources = { 'rustc' },",
        ", PMACS_FAKE_LSP_SAVE_HOLD_MS = '1700'",
    );
    let t0 = Instant::now();
    type_and_save(&mut f, "// CHECKME");
    let sink = f.sink.clone();
    let watches = |s: &EditorState| -> Vec<(String, u32)> {
        let t: std::collections::HashMap<String, u32> = eval(s, "return pmacs.lsp._save_watches()");
        t.into_iter().collect()
    };
    let (seen, checked) = watch(&mut f.s, 4, |s| {
        eval::<Vec<String>>(
            s,
            "local rec = pmacs.lsp.active_attachment()
             local out = {}
             for _, d in ipairs(pmacs.diag.list(rec.uri)) do out[#out + 1] = tostring(d.source) end
             return out",
        )
        .contains(&"rustc".to_owned())
    });
    eprintln!(
        "SAVE late check: {} didSaves {} ms after the save, labels {seen:?}",
        saves(&sink).len(),
        t0.elapsed().as_millis()
    );
    assert!(checked, "the late check ran and its diagnostic landed");
    assert_eq!(
        saves(&f.sink).len(),
        1,
        "one didSave for one save, the server's lateness notwithstanding: {:?}",
        saves(&f.sink)
    );
    assert!(
        seen.iter().all(|t| t.starts_with("LSP:ready")),
        "the label never left ready: {seen:?}"
    );
    assert!(
        watch(&mut f.s, 2, |s| watches(s).is_empty()).1,
        "the watch stood down on the late begin: {:?}",
        watches(&f.s)
    );
}

/// The watch stands down on the flycheck token's begin and on no
/// other: a `$/progress` begin on a workspace reload's token ---
/// `rustAnalyzer/Fetching`, which rust-analyzer sent on #285's runner
/// before the check --- leaves it armed, so the save still goes behind
/// the next `didChange`, and the check's own begin then stands it
/// down. Bitten by disarming on any begin.
#[test]
fn e7c_fix_3_a_begin_on_another_token_leaves_the_watch_armed() {
    let mut f = open_with_fake_config(
        "othertoken",
        "didsave",
        "save_retry = true, check_sources = { 'rustc' },",
        ", PMACS_FAKE_LSP_DROP_SAVES = '1'",
    );
    type_and_save(&mut f, "// CHECKME");
    let sink = f.sink.clone();
    let watches = |s: &EditorState| -> Vec<(String, u32)> {
        let t: std::collections::HashMap<String, u32> = eval(s, "return pmacs.lsp._save_watches()");
        t.into_iter().collect()
    };
    assert_eq!(watches(&f.s).len(), 1, "the save armed a watch");
    exec(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
           { token = 'rustAnalyzer/Fetching', value = { kind = 'begin', title = 'Fetching' } })",
    );
    let (seen, indexed) = watch(&mut f.s, 1, |s| {
        lsp_segment(s).as_deref() == Some("LSP:idx")
    });
    assert!(indexed, "the reload's begin reached the tracker: {seen:?}");
    assert_eq!(
        watches(&f.s).len(),
        1,
        "a begin on another token leaves the watch armed: {:?}",
        watches(&f.s)
    );
    exec(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
           { token = 'rustAnalyzer/Fetching', value = { kind = 'end' } })",
    );
    press(&mut f.s, KeyCode::Char('x'));
    let (_, resent) = watch(&mut f.s, 4, |_| saves(&sink).len() >= 2);
    assert!(
        resent,
        "the save went behind the keystroke's didChange: {:?}",
        saves(&f.sink)
    );
    assert!(
        watch(&mut f.s, 3, |s| watches(s).is_empty()).1,
        "the check's own begin stood the watch down: {:?}",
        watches(&f.s)
    );
    assert_eq!(saves(&f.sink).len(), 2, "and no third save");
}

/// E7c.4: a `window/showMessage` from the server reaches `*lsp*` as
/// the server's own words, and an error or warning the status line
/// too; until this phase it reached nothing, which is how
/// rust-analyzer's refusal of the shipped config (#281) stayed unseen.
#[test]
fn e7c_4_a_server_message_reaches_the_log_and_the_status_line() {
    let mut f = open_with_fake("showmessage", "didsave");
    exec(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/showMessage',
           { type = 2, message = 'invalid config value: /checkOnSave: invalid type: map, expected a boolean' })",
    );
    let (_, shown) = watch(&mut f.s, 3, |s| {
        s.core
            .borrow()
            .status
            .contains("says: invalid config value")
    });
    let status = f.s.core.borrow().status.clone();
    assert!(shown, "the warning is on the status line: {status:?}");
    assert!(status.starts_with("LSP: "), "{status:?}");
    let logged: Vec<String> = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
           if m.summary:find('server says', 1, true) then out[#out + 1] = m.channel .. ' ' .. m.summary end
         end
         return out",
    );
    assert_eq!(
        logged,
        vec![
            "warn server says: invalid config value: /checkOnSave: invalid type: map, expected a boolean"
        ]
    );
    // An informational message stays out of the status line.
    exec(
        &f.s,
        "pmacs.editor.set_status('untouched')
         local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/showMessage', { type = 3, message = 'loaded' })",
    );
    let (_, logged_info) = watch(&mut f.s, 3, |s| {
        eval::<bool>(
            s,
            "local rec = pmacs.lsp.active_attachment()
             for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
               if m.summary == 'server says: loaded' then return true end
             end
             return false",
        )
    });
    assert!(logged_info, "the info message is in *lsp*");
    assert_eq!(f.s.core.borrow().status, "untouched");
}
