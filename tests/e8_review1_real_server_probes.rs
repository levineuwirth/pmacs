// tests/e8_review1_real_server_probes.rs --- E8 review round 1, the
// popup against a real language server.

//! E8 review round 1 --- the hover and signature popup against
//! rust-analyzer, through the TUI's production path (keys through
//! `EditorState::dispatch_key`, the grid through
//! `pmacs::editor::paint_frame`) and the semantic producer's own frame
//! (`SemanticRenderState::render_frame`, what a GPU session is sent).
//!
//! The handoff's witnesses chose `HashMap` (219 lines, under the
//! 256-line bound) and `Vec::with_capacity(` (one parameter, ASCII).
//! These rows choose what a user meets next: `use std::fmt;` (590 lines
//! of module documentation), a doc comment past every byte bound, and
//! signatures whose parameter labels repeat, beside lines that carry
//! multi-byte text before the call.
//!
//! rust-analyzer negotiates UTF-8 positions with pmacs (pmacs offers
//! `utf-8` first), and pmacs declares no `labelOffsetSupport`, so the
//! server sends parameter labels as strings: the popup finds the active
//! one by search (`lsp_popup::parameter_byte_range`), and E8.5's
//! encoding conversion is not reached. The rows that fail at `1ace8b3`
//! say so in their doc comment.

#[path = "common/iso.rs"]
mod iso;
#[path = "support/mod.rs"]
mod support;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lsp_popup::LspPopup;
use pmacs::protocol::FrontendId;
use pmacs::semantic_render::SemanticRenderState;
use pmacs_protocol::{
    ByteRange, InstanceMessage, MAX_POPUP_LINE_BYTES, MAX_POPUP_LINES, MAX_POPUP_TEXT_BYTES,
    PopupFrame, PopupKind, PopupPayload,
};

const ROWS: u32 = 40;
const COLS: u32 = 120;

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

fn exec(state: &EditorState, src: &str) {
    state
        .lua_host
        .lua()
        .load(src.to_string())
        .exec()
        .expect("lua exec");
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, src: &str) -> T {
    state
        .lua_host
        .lua()
        .load(src.to_string())
        .eval()
        .expect("lua eval")
}

fn tick(state: &mut EditorState) {
    state.tick_processes();
    state.tick_lsp();
    state.tick_async();
}

fn key(state: &mut EditorState, k: char, mods: KeyModifiers) {
    state.dispatch_key(FrontendId::LOCAL, KeyEvent::new(KeyCode::Char(k), mods));
}

fn chord(state: &mut EditorState, ch: char) {
    key(state, 'c', KeyModifiers::CONTROL);
    key(state, ch, KeyModifiers::NONE);
}

/// A cargo project holding `source` as `src/main.rs`, opened in an
/// editor whose default configuration attaches rust-analyzer, once the
/// server has initialized.
fn open(tag: &str, source: &str) -> (EditorState, tempfile::TempDir, PathBuf) {
    open_file(tag, "src/main.rs", source)
}

/// [`open`] for any file: `rel` under a fresh project (a cargo project
/// when it is Rust, a `pyproject.toml` root when it is Python).
fn open_file(tag: &str, rel: &str, source: &str) -> (EditorState, tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("e8r1-{tag}-"))
        .tempdir()
        .expect("tempdir");
    if std::path::Path::new(rel)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("rs"))
    {
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"e8r1\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        )
        .expect("Cargo.toml");
    } else {
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"e8r1\"\n",
        )
        .expect("pyproject.toml");
    }
    let main_rs = dir.path().join(rel);
    std::fs::create_dir_all(main_rs.parent().expect("parent")).expect("dirs");
    std::fs::write(&main_rs, source).expect("source");
    let state = EditorState::new_with_roots(&iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            main_rs.display().to_string()
        ),
    );
    let mut state = state;
    let initialized = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind=='initialized' then return true end \
       end \
       return false end)()";
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        tick(&mut state);
        if eval::<bool>(&state, &format!("return {initialized}")) {
            break;
        }
        assert!(Instant::now() < deadline, "rust-analyzer never initialized");
        std::thread::sleep(Duration::from_millis(20));
    }
    (state, dir, main_rs)
}

/// The caret at byte `at` of the buffer (no edit).
fn goto(state: &mut EditorState, at: usize) {
    exec(state, &format!("pmacs.editor.goto_byte({at})"));
    tick(state);
}

fn popup(state: &EditorState) -> Option<LspPopup> {
    state
        .core
        .borrow()
        .lsp_popup
        .lock()
        .expect("lsp popup")
        .clone()
}

/// Ask with `C-c ch` every second until the popup holds one that
/// `want` accepts; rust-analyzer answers nothing useful until it has
/// loaded the crate and the sysroot.
fn ask_until(
    state: &mut EditorState,
    ch: char,
    secs: u64,
    want: impl Fn(&LspPopup) -> bool,
) -> LspPopup {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut last = None;
    while Instant::now() < deadline {
        chord(state, ch);
        let next = Instant::now() + Duration::from_secs(1);
        while Instant::now() < next {
            tick(state);
            if let Some(p) = popup(state) {
                if want(&p) {
                    return p;
                }
                last = Some(p);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    panic!(
        "no accepted popup for C-c {ch} in {secs} s; last {:?}; status {:?}",
        last.map(|p| (p.kind, p.lines.first().cloned(), p.lines.len())),
        state.core.borrow().status
    );
}

fn paint(state: &EditorState) -> Vec<Cell> {
    let mut backing = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: COLS,
        size: CellSize::new(ROWS, COLS),
    };
    let _ = pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        CellSize::new(ROWS, COLS),
    );
    backing
}

fn row_text(cells: &[Cell], row: u32) -> String {
    (0..COLS)
        .map(|c| match &cells[(row * COLS + c) as usize].glyph {
            Glyph::Char(ch) => *ch,
            _ => ' ',
        })
        .collect()
}

fn find_row(cells: &[Cell], needle: &str) -> Option<u32> {
    (0..ROWS).find(|&r| row_text(cells, r).contains(needle))
}

/// The popup frame the semantic producer sends this editor's own
/// frontend, as a GPU session would receive it.
fn wire_frame(state: &EditorState) -> Option<PopupFrame> {
    let buffer_id = state.core.borrow().active_window().buffer_id;
    let mut sem = SemanticRenderState::new(FrontendId::LOCAL);
    sem.set_viewport(buffer_id, ByteRange { start: 0, end: 0 }, 0);
    sem.render_frame(state).into_iter().find_map(|m| match m {
        InstanceMessage::Popup(PopupPayload::Present(frame)) => Some(frame),
        _ => None,
    })
}

/// The 0-based byte at which `needle` ends in `source`.
fn after(source: &str, needle: &str) -> usize {
    source.find(needle).unwrap_or_else(|| panic!("{needle:?}")) + needle.len()
}

/// PASSES at `1ace8b3` and shows the bound is reached by an ordinary
/// line, not only by the fake's 300: rust-analyzer's hover on `fmt` in
/// `use std::fmt;` is the module's documentation, 590 doc lines in this
/// toolchain's `alloc/src/fmt.rs`, so the popup carries 256 lines and
/// counts the rest, the frame validates, and the grid's closing row is
/// what the user reads. The handoff's largest real hover was 219.
#[test]
fn review1_rust_analyzer_hover_on_use_std_fmt_passes_the_line_bound() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let source = "use std::fmt;\n\nfn main() {\n    let _ = fmt::Error;\n}\n";
    let (mut state, _dir, _file) = open("fmt", source);
    goto(&mut state, after(source, "use std::fm"));
    let p = ask_until(&mut state, 'h', 120, |p| {
        p.kind == PopupKind::Hover && p.lines.len() > 50
    });
    assert_eq!(p.lines.len(), MAX_POPUP_LINES, "cut at the line bound");
    assert!(
        p.omitted_lines > 100,
        "most of the module's documentation is counted, not shipped: {}",
        p.omitted_lines
    );
    let frame = wire_frame(&state).expect("the semantic producer sends it");
    assert_eq!(frame.validate(), Ok(()));
    assert_eq!(frame.lines.len(), MAX_POPUP_LINES);
    assert_eq!(frame.omitted_lines, p.omitted_lines);
    let cells = paint(&state);
    let footer = find_row(&cells, "more lines").expect("the grid's closing row");
    let text = row_text(&cells, footer);
    assert!(
        text.contains("C-c H opens *lsp-help*"),
        "the closing row names the persistent form: {text:?}"
    );
    eprintln!(
        "e8r1: std::fmt hover: {} lines shipped, {} omitted; grid footer {:?}",
        p.lines.len(),
        p.omitted_lines,
        text.trim()
    );
}

/// PASSES at `1ace8b3`: a doc comment past every bound at once, served
/// by rust-analyzer. One line of 5,000 bytes (past
/// `MAX_POPUP_LINE_BYTES`) and a hundred of 400 (past
/// `MAX_POPUP_TEXT_BYTES` before `MAX_POPUP_LINES`): the long line is
/// cut at a character and ends in `…`, the text stops at the byte bound
/// with the rest counted, and the frame the producer sends validates ---
/// so the user sees a popup and a closing row, not nothing.
#[test]
fn review1_rust_analyzer_hover_past_the_line_and_text_byte_bounds() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let mut doc = String::new();
    doc.push_str("/// ");
    for i in 0..1000 {
        let _ = write!(doc, "w{:03} ", i % 1000);
    }
    doc.push('\n');
    for i in 0..100 {
        doc.push_str("/// ");
        doc.push_str(&format!("line {i:03} ").repeat(40));
        doc.push('\n');
    }
    let source = format!("{doc}pub fn documented() {{}}\n\nfn main() {{\n    documented();\n}}\n");
    let long = doc.lines().next().unwrap().len();
    assert!(long > MAX_POPUP_LINE_BYTES, "the fixture's line: {long}");
    assert!(
        doc.len() > MAX_POPUP_TEXT_BYTES,
        "the fixture's text: {}",
        doc.len()
    );
    let (mut state, _dir, _file) = open("bytes", &source);
    goto(&mut state, after(&source, "    documen"));
    let p = ask_until(&mut state, 'h', 120, |p| {
        p.kind == PopupKind::Hover && p.lines.iter().any(|l| l.starts_with("w000 w001"))
    });
    let total: usize = p.lines.iter().map(String::len).sum();
    assert!(total <= MAX_POPUP_TEXT_BYTES, "text {total}");
    assert!(p.lines.iter().all(|l| l.len() <= MAX_POPUP_LINE_BYTES));
    assert!(
        p.lines.iter().any(|l| l.ends_with('…') && l.len() > 4000),
        "the 5,000-byte line is cut and says so"
    );
    assert!(
        p.lines.len() < MAX_POPUP_LINES,
        "the byte bound binds first"
    );
    assert!(p.omitted_lines > 0, "the rest is counted");
    let frame = wire_frame(&state).expect("the producer sends a frame, not Absent");
    assert_eq!(frame.validate(), Ok(()));
    // `goto_byte` moved the caret without a key, so the window has not
    // followed it; `C-l` recenters on it, as a user's view would be.
    key(&mut state, 'l', KeyModifiers::CONTROL);
    tick(&mut state);
    let cells = paint(&state);
    let shown: Vec<String> = (0..ROWS)
        .map(|r| row_text(&cells, r).trim_end().to_owned())
        .collect();
    assert!(
        find_row(&cells, "C-c H opens *lsp-help*").is_some(),
        "the grid closes with what it left out: {shown:#?}"
    );
    eprintln!(
        "e8r1: crafted hover: {} lines, {total} bytes shipped, {} omitted",
        p.lines.len(),
        p.omitted_lines
    );
}

/// The source the signature rows open: three calls whose parameter
/// labels repeat or contain one another, and three beside multi-byte
/// text --- an emoji in a string literal before the call, a CJK comment
/// before it, a non-ASCII function and parameter name.
const SIGNATURES: &str = "struct Pair(i32, i32);\n\
fn clamp_len(max_len: usize, len: usize) -> usize { max_len.min(len) }\n\
fn pick(first: u8, second: u8) -> u8 { first + second }\n\
fn größe(höhe: u8, b: u8) -> u8 { höhe + b }\n\
fn main() {\n\
    let add = |a: i32, b: i32| a + b;\n\
    let _ = add(1, 2);\n\
    let _ = Pair(1, 2);\n\
    let _ = clamp_len(1, 2);\n\
    let _ = (\"😀😀\", pick(3, 4));\n\
    /* 中文注释 */ let _ = pick(5, 6);\n\
    let _ = größe(7, 8);\n\
}\n";

/// Open the signature for the call ending `call` (the caret placed
/// after its first argument's `, `) and return the label and the
/// marked span, the label's second parameter's start, and the raw
/// marked text.
fn second_parameter(state: &mut EditorState, call: &str) -> (String, usize, usize, usize) {
    second_parameter_in(state, SIGNATURES, call)
}

/// [`second_parameter`] in `source`.
fn second_parameter_in(
    state: &mut EditorState,
    source: &str,
    call: &str,
) -> (String, usize, usize, usize) {
    goto(state, after(source, call));
    let p = ask_until(state, 's', 120, |p| {
        p.kind == PopupKind::Signature && p.active_range.is_some()
    });
    let label = p.lines[0].clone();
    let r = p.active_range.expect("marked");
    let open = label.find('(').expect("an open paren") + 1;
    let second = open + label[open..].find(", ").expect("two parameters") + 2;
    // Close it so the next ask starts from nothing.
    key(state, 'g', KeyModifiers::CONTROL);
    tick(state);
    (label, r.start as usize, r.end as usize, second)
}

/// FAILS at `1ace8b3`. With the caret on a call's second argument
/// rust-analyzer says the active parameter is the second, and sends its
/// label as a string; the popup finds that string from the label's
/// first `(`, so a second parameter whose label repeats the first's
/// (`i32` in a closure's `impl Fn(i32, i32)` or a tuple struct's
/// `Pair(i32, i32)`) or is contained in it (`len: usize` in
/// `max_len: usize`) is marked on the first. The user is told the
/// wrong argument is the one being typed.
#[test]
fn review1_rust_analyzer_marks_the_second_of_two_alike_parameters() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut state, _dir, _file) = open("alike", SIGNATURES);
    let mut wrong = Vec::new();
    for call in ["add(1, ", "Pair(1, ", "clamp_len(1, "] {
        let (label, start, end, second) = second_parameter(&mut state, call);
        eprintln!(
            "e8r1: {call:?} label {label:?} marked {:?} at {start}, second parameter at {second}",
            &label[start..end]
        );
        if start != second {
            wrong.push(format!(
                "{call:?}: marked {:?} at byte {start} of {label:?}, the second parameter starts at {second}",
                &label[start..end]
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the wrong parameter is marked: {wrong:#?}"
    );
}

/// PASSES at `1ace8b3`: the round's brief's hypothesis, tested. Multi-
/// byte text before the call on its line --- an emoji in a string, a
/// CJK comment --- and a non-ASCII function and first parameter do not
/// move the mark: positions are UTF-8 with rust-analyzer and the label
/// is searched as bytes, so it lands on the second parameter's bytes.
#[test]
fn review1_rust_analyzer_marks_the_parameter_after_multi_byte_text() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let (mut state, _dir, _file) = open("multibyte", SIGNATURES);
    let encoding: String = eval(
        &state,
        "local caps = pmacs.lsp.capabilities(pmacs.lsp.list()[1].id) \
         return tostring(caps and caps.positionEncoding)",
    );
    eprintln!("e8r1: rust-analyzer's positionEncoding {encoding:?}");
    for (call, want) in [
        ("pick(3, ", "second: u8"),
        ("pick(5, ", "second: u8"),
        ("größe(7, ", "b: u8"),
    ] {
        let (label, start, end, second) = second_parameter(&mut state, call);
        eprintln!(
            "e8r1: {call:?} label {label:?} marked {:?}",
            &label[start..end]
        );
        assert_eq!(&label[start..end], want, "{call:?} in {label:?}");
        assert_eq!(
            start, second,
            "{call:?}: the span is the second parameter's"
        );
    }
}

/// PASSES at `1ace8b3`: the same question against a server that speaks
/// UTF-16 only, basedpyright (pmacs's default Python server), where a
/// byte offset and a UTF-16 unit differ on every non-ASCII character
/// before the caret. Two emoji (four bytes, two units each) and a CJK
/// string before the call, and a non-ASCII function and parameter: the
/// request position is converted at the transport and the label is
/// searched as bytes, so the mark lands on the second parameter. Named
/// for basedpyright, so the gate's `--skip basedpyright` skips it.
#[test]
fn review1_basedpyright_marks_the_parameter_after_multi_byte_text() {
    if !on_path("basedpyright-langserver") {
        support::skip_or_fail("basedpyright-langserver", "PMACS_REQUIRE_PYRIGHT");
        return;
    }
    let source = "def größe(höhe, b):\n    return höhe + b\n\n\
def pick(first, second):\n    return first + second\n\n\
x = (\"😀😀\", pick(3, 4))\n\
w = (\"中文注释\", pick(5, 6))\n\
y = größe(7, 8)\n";
    let (mut state, _dir, _file) = open_file("pyright", "main.py", source);
    let encoding: String = eval(
        &state,
        "local caps = pmacs.lsp.capabilities(pmacs.lsp.list()[1].id) \
         return tostring(caps and caps.positionEncoding)",
    );
    eprintln!("e8r1: basedpyright's positionEncoding {encoding:?}");
    for (call, want) in [
        ("pick(3, ", "second"),
        ("pick(5, ", "second"),
        ("größe(7, ", "b"),
    ] {
        let (label, start, end, second) = second_parameter_in(&mut state, source, call);
        eprintln!(
            "e8r1: {call:?} label {label:?} marked {:?}",
            &label[start..end]
        );
        assert!(
            label[start..end].starts_with(want),
            "{call:?} in {label:?}: marked {:?}",
            &label[start..end]
        );
        assert_eq!(
            start, second,
            "{call:?}: the span is the second parameter's"
        );
    }
}
