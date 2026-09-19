// tests/e7c_positions_acceptance.rs --- E7c.3, diagnostics against
// the text they were computed for.

//! A check's diagnostics arrive seconds after a save, computed for the
//! saved text, while the user keeps typing; until E7c.3 they were
//! converted against the text the server held at the time and painted
//! where the error used to be. Now each diagnostic and inlay hint is
//! placed against the text it was computed for --- the file on disk
//! for a source in the server's `check_sources`, the text last sent for
//! the rest --- and carried through E6b.1's edit log to the current
//! text, as semantic tokens are.
//!
//! The fake in `didsave` mode publishes, on every save, one `rustc`
//! error per `CHECKME` in the saved text, and on every `didChange` one
//! `pmacs-fake-lsp` warning per `CHECKME` in the text it holds, both
//! in one notification, which is rust-analyzer's mix. The rows save,
//! type a line above the marker before the check's answer is drained,
//! and read where each diagnostic is:
//!
//! * both land on `CHECKME` on its new line, in the store's spans, the
//!   Lua surface's line and column, the grid's underline and the wire's
//!   decorations;
//! * after the typed line's own `didChange` is answered --- the check's
//!   set republished at its old positions beside a fresh native set ---
//!   the check's error is still on the marker (bitten by emptying
//!   `check_sources`: it then lands one line too high);
//! * an inlay hint answered for the text before a typed line sits at
//!   its byte after it;
//! * `M-g n` visits the marker where it is now.

use std::path::PathBuf;
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

/// Type `text` through the production dispatch. The completion popup
/// opens on a typed word and RET would accept its candidate, so it is
/// dismissed before each newline; the rows are about positions.
fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            if s.core.borrow().completion_popup.lock().unwrap().is_some() {
                press(s, KeyCode::Esc);
            }
            press(s, KeyCode::Enter);
        } else {
            press(s, KeyCode::Char(ch));
        }
    }
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
    let dir = std::env::temp_dir().join(format!("pmacs-e7c-pos-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const BODY: &str = "fn main() {\n    let x = CHECKME;\n}\n";

/// The fake in `didsave` mode on `<dir>/a.rs` holding `BODY`, the
/// handshake answered, with `check_sources` as given.
fn open_with_fake(tag: &str, check_sources: &str) -> (EditorState, PathBuf) {
    let dir = temp_dir(tag);
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = {:?},
               env = {{ PMACS_FAKE_LSP_MODE = 'didsave' }},
               check_sources = {check_sources},
             }}",
            fake_lsp_path()
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, BODY).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    (s, file)
}

/// The diagnostics for the active document, one line each:
/// `<source> <severity> <line>:<col>-<end_line>:<end_col>`, in the
/// order the store holds them.
fn positions(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, d in ipairs(pmacs.diag.list(rec.uri)) do
           out[#out + 1] = string.format('%s %s %d:%d-%d:%d', tostring(d.source), tostring(d.severity),
             d.start_line, d.start_col, d.end_line, d.end_col)
         end
         return out",
    )
}

/// The text each diagnostic's current position covers in the buffer.
fn covered(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local buf = pmacs.window.buffer()
         local text = buf:slice(0, buf:len())
         local lines = {}
         for l in (text .. '\\n'):gmatch('(.-)\\n') do lines[#lines + 1] = l end
         local out = {}
         for _, d in ipairs(pmacs.diag.list(rec.uri)) do
           local l = lines[d.start_line + 1] or ''
           out[#out + 1] = l:sub(d.start_col + 1, d.end_col)
         end
         return out",
    )
}

fn buffer_text(s: &EditorState) -> String {
    eval(
        s,
        "local buf = pmacs.window.buffer() return buf:slice(0, buf:len())",
    )
}

/// Tick until `pred` holds or `secs` pass.
fn wait(s: &mut EditorState, secs: u64, pred: impl Fn(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if pred(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Save, and before the fake's answer is drained type a comment line
/// above the marker; then drain. Returns the byte the marker is at.
fn save_then_type_above(s: &mut EditorState) -> usize {
    exec(
        s,
        "_G.__e7c_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7c_saved = true end)",
    );
    exec(s, "pmacs.command.invoke('buffer.save')");
    assert!(
        pump_lua_flag(s, "_G.__e7c_saved", 10),
        "saved; status {:?}",
        s.core.borrow().status
    );
    // No tick between here and the typing: the check's answer, sent
    // for the saved text, waits in the pipe while the buffer moves.
    exec(s, "pmacs.editor.goto_byte(0)");
    type_str(s, "// e7c typed\n");
    let text = buffer_text(s);
    assert!(
        text.starts_with("// e7c typed\nfn main()"),
        "the typed line is above the marker: {text:?}"
    );
    text.find("CHECKME").expect("the marker")
}

/// The witness the row names: save with an error, type above it before
/// the diagnostic lands, and the squiggle is on the error --- in the
/// store, on the Lua surface, and still after the typed line's own
/// `didChange` is answered with the check's set at its old positions.
#[test]
fn e7c_3_a_check_diagnostic_lands_on_the_error_after_typing_above_it() {
    let (mut s, _file) = open_with_fake("check", "{ 'rustc' }");
    // Positive control: the open's native warning is on the marker.
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    assert_eq!(positions(&s), vec!["pmacs-fake-lsp warning 1:12-1:19"]);
    assert_eq!(covered(&s), vec!["CHECKME"]);

    let marker = save_then_type_above(&mut s);
    // Drain: the check's answer (rustc) and the native set, both
    // published for the saved text, land after the typed line.
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 2),
        "the check's error arrived beside the warning: {:?}",
        positions(&s)
    );
    let after = positions(&s);
    eprintln!(
        "POS after the typed line: {after:?} covering {:?}",
        covered(&s)
    );
    assert_eq!(
        after,
        vec![
            "rustc error 2:12-2:19".to_owned(),
            "pmacs-fake-lsp warning 2:12-2:19".to_owned()
        ],
        "both on the marker's new line"
    );
    assert_eq!(covered(&s), vec!["CHECKME", "CHECKME"]);
    let spans: Vec<(u64, u64)> = {
        let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
        let store = s.lsp_manager.borrow().diag_store();
        let guard = store.lock().unwrap();
        guard
            .for_uri(&uri)
            .iter()
            .map(|d| d.span.unwrap())
            .collect()
    };
    assert_eq!(spans, vec![(marker as u64, marker as u64 + 7); 2]);

    // The typed line's didChange goes out on the quiet timer and the
    // fake answers it with a fresh native set for the new text beside
    // the check's set at its OLD positions (line 1). Wait for that
    // republish: the native warning's epoch moves twice.
    let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
    let epoch = |s: &EditorState| -> u64 {
        let store = s.lsp_manager.borrow().diag_store();
        let guard = store.lock().unwrap();
        guard.epoch_for(&uri)
    };
    let before = epoch(&s);
    assert!(
        wait(&mut s, 5, |s| epoch(s) > before),
        "the didChange was answered with a republish"
    );
    let republished = positions(&s);
    eprintln!(
        "POS after the republish: {republished:?} covering {:?}",
        covered(&s)
    );
    assert_eq!(
        republished,
        vec![
            "rustc error 2:12-2:19".to_owned(),
            "pmacs-fake-lsp warning 2:12-2:19".to_owned()
        ],
        "the check's set, published at line 1 for the saved text, is placed against that text and carried to line 2"
    );
    assert_eq!(covered(&s), vec!["CHECKME", "CHECKME"]);

    // And an edit after the answer landed moves what is held, before
    // any republish: the recorder carries the spans on every edit, not
    // only the absorb's replay. Read without a tick, so no didChange
    // has gone out and nothing new has arrived.
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "// more\n");
    let moved = positions(&s);
    eprintln!(
        "POS after a second typed line, before any republish: {moved:?} covering {:?}",
        covered(&s)
    );
    assert_eq!(
        moved,
        vec![
            "rustc error 3:12-3:19".to_owned(),
            "pmacs-fake-lsp warning 3:12-3:19".to_owned()
        ],
        "both carried across the edit as it was typed"
    );
    assert_eq!(covered(&s), vec!["CHECKME", "CHECKME"]);
}

/// Without `check_sources` the check's republished set is placed
/// against the text last sent --- the typed text --- and lands one
/// line too high: the control that shows what the anchor buys, and the
/// bite of the row above.
#[test]
fn e7c_3_without_check_sources_the_republished_check_error_lands_a_line_too_high() {
    let (mut s, _file) = open_with_fake("nosources", "{}");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    save_then_type_above(&mut s);
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 2),
        "the check's error arrived: {:?}",
        positions(&s)
    );
    let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
    let epoch = |s: &EditorState| -> u64 {
        let store = s.lsp_manager.borrow().diag_store();
        let guard = store.lock().unwrap();
        guard.epoch_for(&uri)
    };
    let before = epoch(&s);
    assert!(wait(&mut s, 5, |s| epoch(s) > before));
    let republished = positions(&s);
    eprintln!(
        "POS control after the republish: {republished:?} covering {:?}",
        covered(&s)
    );
    // Taken for the typed text, the check's line 1 is now `fn main()
    // {`, eleven bytes long, so its columns clamp to the line's end:
    // one line too high and on no text at all.
    assert!(
        republished.iter().any(|p| p.starts_with("rustc error 1:")),
        "the check's set is taken for the typed text and misses: {republished:?}"
    );
    assert!(
        republished.contains(&"pmacs-fake-lsp warning 2:12-2:19".to_owned()),
        "the native set, computed for the typed text, is right: {republished:?}"
    );
}

/// An inlay hint answered for the text before a typed line sits at its
/// byte after it: the fake answers a type hint at line 0 column 9 of
/// whatever it holds, the answer is drained after a line is typed
/// above, and the hint's anchor is the byte that position moved to.
#[test]
fn e7c_3_an_inlay_hint_answered_before_a_typed_line_sits_at_its_byte_after_it() {
    let (mut s, _file) = open_with_fake("inlay", "{ 'rustc' }");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
    let hint_at = |s: &EditorState| -> Option<(u32, u32, Option<u64>)> {
        let store = s.lsp_manager.borrow().inlay_hint_store();
        let guard = store.lock().unwrap();
        guard
            .for_uri(&uri)
            .and_then(|r| r.hints.first().map(|h| (h.line, h.col, h.at)))
    };
    // The open's pull answered for BODY: line 0 col 9 is byte 9.
    assert!(
        wait(&mut s, 5, |s| hint_at(s).is_some()),
        "the open's inlay hints arrived"
    );
    assert_eq!(hint_at(&s), Some((0, 9, Some(9))));
    // Ask again, and type a line above before the answer is drained.
    exec(
        &s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.request_inlay_hint(rec.server, rec.uri, 0, 0, 3, 0)",
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "// e7c typed\n");
    let shifted = "// e7c typed\n".len() as u64;
    // Drain, recording every state the store passes through: the
    // answer for the text before the typed line lands carried past
    // it (byte 9 + 13), and then the typed line's own `didChange` is
    // answered for the new text --- the fake still says line 0 column
    // 9, now inside the comment --- and that is where it sits. Both
    // may be drained within one tick, so the sequence is what is read.
    let mut seen: Vec<Option<(u32, u32, Option<u64>)>> = vec![hint_at(&s)];
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        tick(&mut s);
        let now = hint_at(&s);
        if seen.last() != Some(&now) {
            seen.push(now);
        }
        if seen.contains(&Some((0, 9, Some(9 + shifted)))) && now == Some((0, 9, Some(9))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    eprintln!("POS inlay states after the typed line: {seen:?}");
    assert!(
        seen.contains(&Some((0, 9, Some(9 + shifted)))),
        "the hint answered for the old text was carried past the typed line: {seen:?}"
    );
    assert_eq!(
        hint_at(&s),
        Some((0, 9, Some(9))),
        "the fresh answer for the typed text is placed as answered"
    );
}

/// `M-g n` after typing above the error visits the marker where it is
/// now, not where the check said it was.
#[test]
fn e7c_3_diag_next_visits_the_carried_position() {
    let (mut s, _file) = open_with_fake("nav", "{ 'rustc' }");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    let marker = save_then_type_above(&mut s);
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 2),
        "the check's error arrived: {:?}",
        positions(&s)
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    exec(&s, "pmacs.command.invoke('diag.next')");
    let at: u64 = eval(&s, "return pmacs.editor.cursor()");
    assert_eq!(at, marker as u64, "the cursor is on the marker");
    let status = s.core.borrow().status.clone();
    assert!(
        status.contains("e7c"),
        "the status names the diagnostic: {status:?}"
    );
}
