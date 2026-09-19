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
//! * `M-g n` visits the marker where it is now;
//! * (E7c fix 2) deleting the whole line the marker is on drops both
//!   diagnostics before any save or publish --- from the store, the
//!   grid's underlines and mode line, the wire's decorations and
//!   `StatusFacts` --- and the republish the deletion's own `didChange`
//!   draws, the check's set at its old line, lands nothing; deleting
//!   half the marker instead keeps both on what remains.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph, UnderlineStyle};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::{ByteRange, DecorationKind, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;

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
    // The typed line's last `didChange` is answered with the check's
    // set at its old line beside a fresh native set. Taken for the
    // typed text, the check's line 1 is now `fn main() {`, eleven
    // bytes long, so its columns clamp to the line's end: one line too
    // high and on no text at all. A flush that leaves mid-word (the
    // completion driver flushes as it asks) republishes once more on
    // the way, with a base the carry still reaches, so the miss is
    // waited for rather than read off the first republish (CI's
    // no-crdt leg read that intermediate one at `fcd1248`).
    assert!(
        wait(&mut s, 5, |s| positions(s)
            .iter()
            .any(|p| p.starts_with("rustc error 1:"))),
        "the check's set is taken for the typed text and misses: {:?}",
        positions(&s)
    );
    let republished = positions(&s);
    eprintln!(
        "POS control after the republish: {republished:?} covering {:?}",
        covered(&s)
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

// ---------------------------------------------------------------------------
// E7c fix 2: a diagnostic about deleted text is dropped
// ---------------------------------------------------------------------------

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

/// The grid frontend's frame at 8 x 120: the text under every
/// underlined cell of the window, in grid order, and the mode line's
/// text (the row carrying the LSP segment), which carries the
/// `E:`/`W:` count.
fn grid(s: &EditorState) -> (String, String) {
    let (rows, cols) = (8u32, 120u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    let text_of = |row: u32| -> String {
        (0..cols)
            .map(|col| match &cells[(row * cols + col) as usize].glyph {
                Glyph::Char(c) => *c,
                _ => ' ',
            })
            .collect()
    };
    let all: Vec<String> = (0..rows).map(text_of).collect();
    let modeline = all
        .iter()
        .find(|t| t.contains("LSP:"))
        .unwrap_or_else(|| panic!("a mode line with the LSP segment: {all:?}"))
        .clone();
    let underlined: String = (0..rows * cols)
        .filter(|i| cells[*i as usize].style.underline != UnderlineStyle::None)
        .map(|i| match &cells[i as usize].glyph {
            Glyph::Char(c) => *c,
            _ => ' ',
        })
        .collect();
    (underlined, modeline)
}

/// A semantic frontend's frame: the text under every diagnostic
/// decoration, and the `(errors, warnings)` its `StatusFacts` carries
/// (the message is suppressed while unchanged, so `last` is carried
/// forward and updated).
fn wire(s: &EditorState, r: &mut SemanticRenderState, last: &mut (u32, u32)) -> Vec<String> {
    let text = buffer_text(s);
    let mut out = Vec::new();
    for m in r.render_frame(s) {
        match m {
            InstanceMessage::Decorations { segments, .. } => {
                for d in segments.iter().flat_map(|seg| seg.decorations.iter()) {
                    if matches!(
                        d.kind,
                        DecorationKind::DiagnosticError | DecorationKind::DiagnosticWarning
                    ) {
                        let (lo, hi) = (d.range.start as usize, d.range.end as usize);
                        out.push(format!(
                            "{:?} {:?}",
                            d.kind,
                            &text[lo.min(text.len())..hi.min(text.len())]
                        ));
                    }
                }
            }
            InstanceMessage::StatusFacts {
                diag_errors,
                diag_warnings,
                ..
            } => *last = (diag_errors, diag_warnings),
            _ => {}
        }
    }
    out
}

/// Save with the marker on line 1 so both diagnostics --- the check's
/// error for the saved text and the native warning --- are on it, and
/// the semantic frontend attached, showing them.
fn both_on_the_marker(tag: &str) -> (EditorState, SemanticRenderState, (u32, u32)) {
    let (mut s, _file) = open_with_fake(tag, "{ 'rustc' }");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    exec(
        &s,
        "_G.__e7c_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7c_saved = true end)
         pmacs.command.invoke('buffer.save')",
    );
    assert!(pump_lua_flag(&mut s, "_G.__e7c_saved", 10), "saved");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 2),
        "the check's error arrived beside the warning: {:?}",
        positions(&s)
    );
    assert_eq!(covered(&s), vec!["CHECKME", "CHECKME"]);
    // Every tick's arrivals drained, so what follows is the deletion's
    // doing and nothing in flight.
    for _ in 0..20 {
        tick(&mut s);
    }
    assert_eq!(positions(&s).len(), 2, "settled: {:?}", positions(&s));
    let buffer_id = s.core.borrow().active_window().buffer_id;
    let mut r = SemanticRenderState::new(FrontendId::LOCAL);
    r.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    let mut facts = (u32::MAX, u32::MAX);
    let decorated = wire(&s, &mut r, &mut facts);
    assert_eq!(
        decorated,
        vec![
            "DiagnosticError \"CHECKME\"".to_owned(),
            "DiagnosticWarning \"CHECKME\"".to_owned()
        ],
        "both on the wire"
    );
    assert_eq!(facts, (1, 1), "E:1 W:1 on the wire");
    let (underlined, modeline) = grid(&s);
    assert_eq!(underlined, "CHECKME", "underlined on the grid");
    assert!(
        modeline.contains("E:1 W:1"),
        "the grid's mode line: {modeline:?}"
    );
    (s, r, facts)
}

/// The witness the owner's run asked for: with the check's warning on
/// its line, delete the whole line, and the squiggle and the count are
/// gone before any save --- on the grid and on the wire --- and stay
/// gone through the republish the deletion draws, the check's set at
/// its old line carried across the deletion. Bitten by restoring the
/// zero-width carry in `DiagnosticStore::translate_edit` and the
/// snapping carry at the absorb: the first leaves a mark at the
/// deletion point until the republish, the second brings it back.
#[test]
fn e7c_fix_2_deleting_the_line_drops_its_diagnostics_before_any_save() {
    let (mut s, mut r, mut facts) = both_on_the_marker("dropline");
    let marker = buffer_text(&s).find("CHECKME").expect("the marker");
    // The whole of `    let x = CHECKME;\n`: to its start, `C-k` the
    // text and `C-k` the newline, through the production dispatch.
    let line_start = buffer_text(&s)[..marker].rfind('\n').expect("line 0") + 1;
    exec(&s, &format!("pmacs.editor.goto_byte({line_start})"));
    ctrl(&mut s, 'k');
    ctrl(&mut s, 'k');
    assert_eq!(buffer_text(&s), "fn main() {\n}\n", "the line is gone");
    // Before any tick: nothing in flight has landed, no didChange has
    // gone out, no save.
    assert_eq!(
        positions(&s),
        Vec::<String>::new(),
        "dropped from the store as the edit was recorded"
    );
    let decorated = wire(&s, &mut r, &mut facts);
    assert_eq!(decorated, Vec::<String>::new(), "nothing on the wire");
    assert_eq!(facts, (0, 0), "the wire's count at zero");
    let (underlined, modeline) = grid(&s);
    assert_eq!(underlined, "", "nothing underlined on the grid");
    assert!(
        !modeline.contains("E:") && !modeline.contains("W:"),
        "the grid's mode line shows no count: {modeline:?}"
    );
    // The deletion's didChange goes out on the quiet timer and the
    // fake republishes: the native set for the new text (no marker,
    // so none) beside the check's set at its old line, computed for
    // the saved text and carried across the deletion --- which drops
    // it again at the absorb. Wait for that republish and read.
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
    for _ in 0..20 {
        tick(&mut s);
    }
    assert_eq!(
        positions(&s),
        Vec::<String>::new(),
        "the check's set, republished for the saved text, lands nothing on text that is gone"
    );
    let decorated = wire(&s, &mut r, &mut facts);
    assert_eq!(decorated, Vec::<String>::new());
    assert_eq!(facts, (0, 0));
    let (underlined, _) = grid(&s);
    assert_eq!(underlined, "");
}

/// The control: delete half the marker instead, and both diagnostics
/// stay on what remains --- `KME` --- on the grid and on the wire, before
/// the republish; after it the check's set, carried across the cut, is
/// still on `KME`, and the native set, computed for the cut text, has
/// no marker to report.
#[test]
fn e7c_fix_2_deleting_half_the_word_keeps_its_diagnostics_on_what_remains() {
    let (mut s, mut r, mut facts) = both_on_the_marker("cutword");
    let marker = buffer_text(&s).find("CHECKME").expect("the marker");
    exec(&s, &format!("pmacs.editor.goto_byte({marker})"));
    for _ in 0..4 {
        ctrl(&mut s, 'd');
    }
    assert_eq!(
        buffer_text(&s),
        "fn main() {\n    let x = KME;\n}\n",
        "half the marker is gone"
    );
    assert_eq!(covered(&s), vec!["KME", "KME"], "both kept on what remains");
    let decorated = wire(&s, &mut r, &mut facts);
    assert_eq!(
        decorated,
        vec![
            "DiagnosticError \"KME\"".to_owned(),
            "DiagnosticWarning \"KME\"".to_owned()
        ]
    );
    assert_eq!(facts, (1, 1), "still counted on the wire");
    let (underlined, modeline) = grid(&s);
    assert_eq!(underlined, "KME", "the underline on what remains");
    assert!(modeline.contains("E:1 W:1"), "{modeline:?}");
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
    for _ in 0..20 {
        tick(&mut s);
    }
    assert_eq!(
        positions(&s),
        vec!["rustc error 1:12-1:15".to_owned()],
        "the check's set carried across the cut; the native set has no marker in the cut text"
    );
    assert_eq!(covered(&s), vec!["KME"]);
    let decorated = wire(&s, &mut r, &mut facts);
    assert_eq!(decorated, vec!["DiagnosticError \"KME\"".to_owned()]);
    assert_eq!(facts, (1, 0));
}

/// `*diagnostics*` follows the drop without a publish: open on the
/// two rows, then the line deleted from the document, and the panel
/// reads `This buffer (0):` before any tick. The store's epoch moved
/// with the drop and the after-edit hook re-rendered on it. Bitten by
/// removing that hook from `lsp.lua`: the panel then still lists both
/// until the republish.
#[test]
fn e7c_fix_2_the_panel_follows_the_drop_before_any_publish() {
    let (mut s, _r, _facts) = both_on_the_marker("panel");
    exec(&s, "pmacs.command.invoke('lsp.diagnostics')");
    let panel = |s: &EditorState| -> String {
        eval(
            s,
            "for _, b in ipairs(pmacs.buffer.list()) do
               if b:name() == '*diagnostics*' then return b:slice(0, b:len()) end
             end
             return '<no panel>'",
        )
    };
    let opened = panel(&s);
    assert!(
        opened.contains("This buffer (2):"),
        "the panel lists both: {opened:?}"
    );
    // Back to the document (the panel took the focus), and the line
    // deleted as in the row above.
    let file: String = eval(
        &s,
        "return pmacs.lsp.path_for_uri(pmacs.lsp.active_attachment() and pmacs.lsp.active_attachment().uri or '') or ''",
    );
    let file = if file.is_empty() {
        let names: Vec<String> = eval(
            &s,
            "local out = {} for _, b in ipairs(pmacs.buffer.list()) do local p = b:path() if p then out[#out+1] = p end end return out",
        );
        names
            .into_iter()
            .find(|p| p.ends_with("a.rs"))
            .expect("the document")
    } else {
        file
    };
    exec(
        &s,
        &format!("pmacs.window.display_file({file:?}, {{ select = true }})"),
    );
    assert!(
        buffer_text(&s).contains("CHECKME"),
        "the document is active again"
    );
    let marker = buffer_text(&s).find("CHECKME").expect("the marker");
    let line_start = buffer_text(&s)[..marker].rfind('\n').expect("line 0") + 1;
    exec(&s, &format!("pmacs.editor.goto_byte({line_start})"));
    ctrl(&mut s, 'k');
    ctrl(&mut s, 'k');
    assert_eq!(buffer_text(&s), "fn main() {\n}\n", "the line is gone");
    let after = panel(&s);
    assert!(
        after.contains("This buffer (0):") && !after.contains("CHECKME"),
        "the panel followed the drop before any tick: {after:?}"
    );
}

/// An inlay hint answered for a line that was deleted before the
/// answer was drained is dropped at the absorb, as a held one is by
/// the recorder. The fake answers two hints for whatever it holds: a
/// type hint at line 0 column 9 and a parameter hint at line 1 column
/// 4. The open's answer is held at bytes 9 and 16; the first line is
/// deleted through the production dispatch and the held type hint goes
/// at once while the parameter hint moves to byte 4; the answer in
/// flight for the old text --- sent before the deletion, answered in
/// order on the one pipe before the deletion's own `didChange` is ---
/// lands the same way, its type hint nowhere, where the snapping carry
/// put it at byte 0; the fresh answer for the new text is placed as
/// answered. Bitten by snapping at the absorb: `at: Some(0)` then
/// shows until the fresh answer lands.
#[test]
fn e7c_fix_2_an_inlay_hint_answered_for_a_deleted_line_is_dropped() {
    let (mut s, _file) = open_with_fake("inlaydrop", "{ 'rustc' }");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    let uri: String = eval(&s, "return pmacs.lsp.active_attachment().uri");
    let hints = |s: &EditorState| -> Vec<(u32, u32, Option<u64>)> {
        let store = s.lsp_manager.borrow().inlay_hint_store();
        let guard = store.lock().unwrap();
        guard
            .for_uri(&uri)
            .map(|r| r.hints.iter().map(|h| (h.line, h.col, h.at)).collect())
            .unwrap_or_default()
    };
    assert!(
        wait(&mut s, 5, |s| !hints(s).is_empty()),
        "the open's inlay hints arrived"
    );
    assert_eq!(hints(&s), vec![(0, 9, Some(9)), (1, 4, Some(16))]);
    // Ask again, and delete the line the type hint is on before the
    // answer is drained: `fn main() {\n`, bytes 0..12, with the hint
    // at 9 strictly inside and the parameter hint's 16 past it.
    exec(
        &s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.request_inlay_hint(rec.server, rec.uri, 0, 0, 3, 0)",
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    ctrl(&mut s, 'k');
    ctrl(&mut s, 'k');
    assert!(
        buffer_text(&s).starts_with("    let x = CHECKME;\n"),
        "the first line is gone: {:?}",
        buffer_text(&s)
    );
    assert_eq!(
        hints(&s),
        vec![(1, 4, Some(4))],
        "the held type hint went with its line; the parameter hint moved up"
    );
    let mut seen: Vec<Vec<(u32, u32, Option<u64>)>> = vec![hints(&s)];
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        tick(&mut s);
        let now = hints(&s);
        if seen.last() != Some(&now) {
            seen.push(now.clone());
        }
        if now.first() == Some(&(0, 9, Some(9))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    eprintln!("POS inlay states after the deleted line: {seen:?}");
    assert_eq!(
        hints(&s).first(),
        Some(&(0, 9, Some(9))),
        "the fresh answer for the new text is placed as answered: {seen:?}"
    );
    assert!(
        !seen.iter().flatten().any(|&(_, _, at)| at == Some(0)),
        "the answer for the deleted line never showed at the deletion point: {seen:?}"
    );
}
