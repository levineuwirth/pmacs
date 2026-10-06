// tests/e8_review2_real_server_probes.rs --- E8 review round 2, the
// popup against rust-analyzer.

//! E8 review round 2 --- the popup against rust-analyzer, through the
//! TUI's production path (keys through `EditorState::dispatch_key`, the
//! grid through `pmacs::editor::paint_frame`) and the semantic
//! producer's own frame (`SemanticRenderState::render_frame`, what a GPU
//! session is sent).
//!
//! Three questions fix round 1 left open:
//!
//! - the label offsets are read in the negotiated unit and then in
//!   UTF-16, the first reading under which every parameter is a whole
//!   run kept (`signature::coherent`). rust-analyzer negotiates UTF-8
//!   and counts UTF-16, so its offsets are read as bytes first; where a
//!   non-ASCII identifier's extra bytes shift every edge onto a
//!   punctuation or a word boundary, the byte reading is whole too, and
//!   it is the one kept. The rows below fail at `57a980e` for that
//!   reason, each mark computed from the predicate before it was run;
//! - `C-c H` opens the last hover popup's text where the caret's place
//!   has none (`44dfb24`): what the user is shown about where it came
//!   from, and whether an edit to the symbol it documents retires it;
//! - #317, rust-analyzer silent on CI's macOS runners: pmacs sends a
//!   `rootUri` with symlinks resolved (`pmacs.project.detect`
//!   canonicalizes) and a `didOpen` URI with them kept (a buffer's path
//!   is normalized lexically), and macOS's `$TMPDIR` is a symlink. The
//!   row opens a project through a symlinked directory on Linux.

#[path = "common/iso.rs"]
mod iso;
#[path = "support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lsp_popup::LspPopup;
use pmacs::protocol::FrontendId;
use pmacs::semantic_render::SemanticRenderState;
use pmacs_protocol::{ByteRange, InstanceMessage, PopupFrame, PopupKind, PopupPayload};

const ROWS: u32 = 40;
const COLS: u32 = 120;

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

/// As `tests/e8_review1_real_server_probes.rs`: rust-analyzer on PATH,
/// and Linux unless `PMACS_REQUIRE_LSP` is armed (#317).
fn rust_analyzer_provisioned() -> bool {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return false;
    }
    let armed = std::env::var_os("PMACS_REQUIRE_LSP").is_some_and(|v| !v.is_empty());
    if !cfg!(target_os = "linux") && !armed {
        eprintln!("rust-analyzer rows run on Linux, where CI provisions them (#317); skipping");
        return false;
    }
    true
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

fn typed(state: &mut EditorState, text: &str) {
    for ch in text.chars() {
        key(state, ch, KeyModifiers::NONE);
        tick(state);
    }
}

/// Write a cargo project holding `source` as `src/main.rs` under `root`.
fn write_project(root: &Path, source: &str) -> PathBuf {
    std::fs::create_dir_all(root.join("src")).expect("dirs");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"e8r2\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    )
    .expect("Cargo.toml");
    let main_rs = root.join("src/main.rs");
    std::fs::write(&main_rs, source).expect("source");
    main_rs
}

/// An editor visiting `path`, once its server has initialized.
fn visit(path: &Path) -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
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
    state
}

/// A cargo project holding `source`, opened by its own path.
fn open(tag: &str, source: &str) -> (EditorState, tempfile::TempDir) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("e8r2-{tag}-"))
        .tempdir()
        .expect("tempdir");
    let main_rs = write_project(dir.path(), source);
    (visit(&main_rs), dir)
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
/// `want` accepts, or `secs` pass; the last status on failure.
fn try_ask(
    state: &mut EditorState,
    ch: char,
    secs: u64,
    want: impl Fn(&LspPopup) -> bool,
) -> Result<LspPopup, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut last = None;
    while Instant::now() < deadline {
        chord(state, ch);
        let next = Instant::now() + Duration::from_secs(1);
        while Instant::now() < next {
            tick(state);
            if let Some(p) = popup(state) {
                if want(&p) {
                    return Ok(p);
                }
                last = Some(p);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Err(format!(
        "no accepted popup for C-c {ch} in {secs} s; last {:?}; status {:?}",
        last.map(|p| (p.kind, p.lines.first().cloned(), p.lines.len())),
        state.core.borrow().status
    ))
}

fn ask_until(
    state: &mut EditorState,
    ch: char,
    secs: u64,
    want: impl Fn(&LspPopup) -> bool,
) -> LspPopup {
    try_ask(state, ch, secs, want).unwrap_or_else(|e| panic!("{e}"))
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

/// The parameters of the active signature in the store, as
/// `(label, span)`: a span is the offset path, none the string path.
fn stored_parameters(state: &EditorState) -> Vec<(String, Option<(u32, u32)>)> {
    let store = state.lsp_manager.borrow().signature_store();
    let guard = store.lock().expect("signature store");
    let keys: Vec<_> = guard.keys().cloned().collect();
    let help = keys
        .iter()
        .find_map(|k| guard.get(k).filter(|h| h.active().is_some()))
        .expect("an answer in the store");
    help.active()
        .expect("an active signature")
        .parameters
        .iter()
        .map(|p| (p.label.clone(), p.span))
        .collect()
}

/// The characters the grid paints as the active parameter (bold and
/// underlined), in order, over the whole frame.
fn grid_marked(cells: &[Cell]) -> String {
    cells
        .iter()
        .filter_map(|cell| {
            let marked =
                cell.style.bold && cell.style.underline != pmacs::cell::UnderlineStyle::None;
            match (&cell.glyph, marked) {
                (Glyph::Char(ch), true) => Some(*ch),
                _ => None,
            }
        })
        .collect()
}

// ---- 1. Both readings whole ------------------------------------------------

/// Signatures whose parameters' UTF-16 offsets, read as bytes, still
/// land every edge on a whole run of the label (`signature::coherent`):
/// a short type right after a CJK name, and a generic list before the
/// `(`. Each was found by running the predicate over generated labels
/// before rust-analyzer was asked.
const BOTH_WHOLE: &str = "fn f(名前: u8, a: u8) -> u8 { 名前 + a }\n\
fn g(名前: u8, größe: &str, a: u8) -> usize { 名前 as usize + a as usize + größe.len() }\n\
fn 长度计算<'a, U>(a: i32, b: i32) -> i32 { a + b }\n\
fn main() {\n\
    let _ = f(1, 2);\n\
    let _ = f(3, 4);\n\
    let _ = g(1, \"x\", 3);\n\
    let _ = 长度计算::<u8>(1, 2);\n\
}\n";

/// FAILS at `57a980e`. rust-analyzer counts its label offsets in UTF-16
/// and negotiates UTF-8, so pmacs reads them as bytes first and keeps
/// that reading whenever every parameter is a whole run under it. Read
/// as bytes, `f(名前: u8, a: u8)`'s second parameter is `u8, a` (the
/// first parameter's type, the comma and the second's name) and its
/// first `名前`; `g`'s third is `&str,`, the second parameter's type and
/// the comma after it; and
/// `长度计算<'a, U>(a: i32, b: i32)`'s second is `a: i32`, the first
/// parameter exactly. Every one of those is a whole run, so the byte
/// reading is kept and the UTF-16 one, which is right, never tried.
/// The row checks the popup's mark, the frame a GPU session is sent,
/// and the grid's painted mark, and reports every call before failing.
#[test]
fn review2_rust_analyzer_marks_the_parameter_where_its_byte_reading_is_also_whole() {
    if !rust_analyzer_provisioned() {
        return;
    }
    let (mut state, _dir) = open("bothwhole", BOTH_WHOLE);
    let mut wrong = Vec::new();
    for (call, want) in [
        ("f(1, ", "a: u8"),
        ("f(3", "名前: u8"),
        ("g(1, \"x\", ", "a: u8"),
        ("长度计算::<u8>(1, ", "b: i32"),
    ] {
        goto(&mut state, after(BOTH_WHOLE, call));
        let p = ask_until(&mut state, 's', 120, |p| {
            p.kind == PopupKind::Signature && p.active_range.is_some()
        });
        let label = p.lines[0].clone();
        let r = p.active_range.expect("marked");
        let marked = label[r.start as usize..r.end as usize].to_owned();
        let frame = wire_frame(&state).expect("the producer sends the popup");
        let gpu = frame
            .active_range
            .map(|r| label[r.start as usize..r.end as usize].to_owned());
        let grid = grid_marked(&paint(&state));
        let stored = stored_parameters(&state);
        eprintln!(
            "e8r2: {call:?} label {label:?} marked {marked:?} (gpu {gpu:?}, grid {grid:?}); \
             stored {stored:?}"
        );
        if marked != want || gpu.as_deref() != Some(want) || grid != want {
            wrong.push(format!(
                "{call:?}: want {want:?} in {label:?}; the popup marks {marked:?}, a GPU \
                 session is sent {gpu:?}, the grid paints {grid:?}"
            ));
        }
        key(&mut state, 'g', KeyModifiers::CONTROL);
        tick(&mut state);
    }
    assert!(
        wrong.is_empty(),
        "the byte reading of UTF-16 offsets was kept: {wrong:#?}"
    );
}

// ---- 1b. Fix round 2: the marks the stronger predicate keeps --------------

/// Each `(call, parameter)` of `source`: the caret after `call`, `C-c s`
/// until a marked signature popup arrives, then the popup's mark, the
/// frame a GPU session is sent and the grid's painted mark compared with
/// `parameter`. The calls that disagree, each with all three.
fn wrong_marks(state: &mut EditorState, source: &str, calls: &[(&str, &str)]) -> Vec<String> {
    let mut wrong = Vec::new();
    for &(call, want) in calls {
        goto(state, after(source, call));
        let p = match try_ask(state, 's', 120, |p| {
            p.kind == PopupKind::Signature && p.active_range.is_some()
        }) {
            Ok(p) => p,
            Err(e) => {
                wrong.push(format!("{call:?}: {e}"));
                continue;
            }
        };
        let label = p.lines[0].clone();
        let r = p.active_range.expect("marked");
        let marked = label[r.start as usize..r.end as usize].to_owned();
        let gpu = wire_frame(state)
            .and_then(|f| f.active_range)
            .map(|r| label[r.start as usize..r.end as usize].to_owned());
        let grid = grid_marked(&paint(state));
        eprintln!("e8fr2: {call:?} label {label:?} marked {marked:?} (gpu {gpu:?}, grid {grid:?})");
        if marked != want || gpu.as_deref() != Some(want) || grid != want {
            wrong.push(format!(
                "{call:?}: want {want:?} in {label:?}; the popup marks {marked:?}, a GPU \
                 session is sent {gpu:?}, the grid paints {grid:?}"
            ));
        }
        key(state, 'g', KeyModifiers::CONTROL);
        tick(state);
    }
    wrong
}

/// E8 fix round 2, the balance rule on rust-analyzer. `move_to`'s one
/// parameter is a Japanese name with a tuple type; its UTF-16 offsets
/// (11..26) read as bytes are `現在地: (i32`, opened by the `(` and
/// closed by the tuple's own `,`. Only the bracket left open says it is
/// no parameter: without that rule both readings pass, they differ, and
/// nothing is marked.
#[test]
fn fix2_rust_analyzer_marks_a_parameter_whose_byte_reading_leaves_a_bracket_open() {
    if !rust_analyzer_provisioned() {
        return;
    }
    let source = "fn move_to(現在地: (i32, i32)) -> i32 { 現在地.0 }\n\
fn main() {\n\
    let _ = move_to((1, 2));\n\
}\n";
    let (mut state, _dir) = open("balance", source);
    let wrong = wrong_marks(
        &mut state,
        source,
        &[("let _ = move_to(", "現在地: (i32, i32)")],
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
}

fn clangd_provisioned() -> bool {
    if on_path("clangd") {
        return true;
    }
    support::skip_or_fail("clangd", "PMACS_REQUIRE_LSP");
    false
}

/// E8 fix round 2, a server that counts its label offsets in the bytes
/// it negotiated: clangd, on C++ with the same non-ASCII shapes as
/// review 2's rust-analyzer row. Its offsets read as bytes are each
/// parameter, delimited and balanced, and read as UTF-16 they are not
/// (`f`'s second is `a) ->`), so the byte reading is kept and the rule
/// that two differing readings mark nothing never fires. Passes at
/// `57a980e` too: it holds the stronger predicate to the server whose
/// reading it must not refuse.
#[test]
fn fix2_clangd_counts_label_offsets_in_bytes_and_each_parameter_is_marked() {
    if !clangd_provisioned() {
        return;
    }
    let source = "int f(int 名前, int a) { return 名前 + a; }\n\
int g(int 名前, const char *größe, int a) { return 名前 + a + größe[0]; }\n\
template <typename U> int 长度计算(int a, int b) { return a + b; }\n\
int main() {\n\
  int r = f(1, 2);\n\
  r += f(3, 4);\n\
  r += g(1, \"x\", 3);\n\
  r += 长度计算<char>(1, 2);\n\
  return r;\n\
}\n";
    let dir = tempfile::Builder::new()
        .prefix("e8fr2-clangd-")
        .tempdir()
        .expect("tempdir");
    std::fs::write(dir.path().join("compile_flags.txt"), "-std=c++20\n").expect("flags");
    let main_cpp = dir.path().join("main.cpp");
    std::fs::write(&main_cpp, source).expect("source");
    let mut state = visit(&main_cpp);
    let wrong = wrong_marks(
        &mut state,
        source,
        &[
            ("f(1, ", "int a"),
            ("f(3", "int 名前"),
            ("g(1, \"x\", ", "int a"),
            ("长度计算<char>(1, ", "int b"),
        ],
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
}

// ---- 2. The kept hover -----------------------------------------------------

const TWO_FUNCTIONS: &str = "/// Alpha adds one to its argument.\n\
fn alpha(x: u8) -> u8 {\n\
    x + 1\n\
}\n\
\n\
/// Beta doubles its argument.\n\
fn beta(y: u8) -> u8 {\n\
\n\
    y * 2\n\
}\n\
\n\
fn main() {\n\
    let _ = alpha(1) + beta(2);\n\
}\n";

/// `*lsp-help*`'s text, or `None` while another buffer is shown.
fn lsp_help(state: &EditorState) -> Option<String> {
    state
        .lua_host
        .lua()
        .load(
            "local b = pmacs.window.buffer()\n\
             if b:name() ~= '*lsp-help*' then return nil end\n\
             return b:slice(0, b:len())",
        )
        .eval()
        .expect("lua eval")
}

/// `C-c H` up to six times, ten seconds each, until `*lsp-help*` opens;
/// its text, or the statuses seen.
fn press_c_c_h(state: &mut EditorState) -> Result<String, Vec<String>> {
    let mut statuses: Vec<String> = Vec::new();
    for _ in 0..6 {
        key(state, 'c', KeyModifiers::CONTROL);
        key(state, 'H', KeyModifiers::SHIFT);
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            tick(state);
            if let Some(text) = lsp_help(state) {
                return Ok(text);
            }
            let status = state.core.borrow().status.clone();
            if status == "LSP: no hover info" {
                return Err(vec![status]);
            }
            if statuses.last() != Some(&status) {
                statuses.push(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    Err(statuses)
}

/// PASSES at `57a980e`: what the brief asked to be looked at, recorded.
/// The hover on `alpha`, the caret moved to the blank line inside
/// `beta`'s body (no hover there), `C-c H`: `*lsp-help*` opens alpha's
/// text under the header "hover documentation, the last popup's (none
/// at the caret)". The header says the text is not the caret's; it does
/// not say where it came from. On rust-analyzer the text names its
/// symbol itself (`fn alpha(x: u8) -> u8`), which a server's text need
/// not do (the fake's is `# pmacs-fake-lsp`).
#[test]
fn review2_c_c_h_in_another_function_opens_the_last_popups_text_under_its_header() {
    if !rust_analyzer_provisioned() {
        return;
    }
    let (mut state, _dir) = open("otherfn", TWO_FUNCTIONS);
    goto(&mut state, after(TWO_FUNCTIONS, "let _ = alph"));
    let p = ask_until(&mut state, 'h', 120, |p| {
        p.kind == PopupKind::Hover && p.lines.iter().any(|l| l.contains("Alpha adds one"))
    });
    eprintln!("e8r2: the hover on alpha: {:?}", p.lines);
    goto(&mut state, after(TWO_FUNCTIONS, "fn beta(y: u8) -> u8 {\n"));
    tick(&mut state);
    let _ = paint(&state);
    assert!(popup(&state).is_none(), "the motion closed the popup");
    let text = press_c_c_h(&mut state)
        .unwrap_or_else(|seen| panic!("C-c H opened nothing; statuses {seen:?}"));
    let header = text.lines().next().unwrap_or_default().to_owned();
    eprintln!(
        "e8r2: *lsp-help* header {header:?}; first lines {:?}",
        text.lines().take(8).collect::<Vec<_>>()
    );
    assert!(text.contains("Alpha adds one"), "alpha's text: {text:?}");
    assert!(
        header.contains("the last popup's (none at the caret)"),
        "the header says the text is not the caret's: {header:?}"
    );
}

/// FAILS at `57a980e`. The kept text outlives an edit to the very
/// documentation it holds. Hover on `alpha` (the popup shows "Alpha adds
/// one"), the doc comment edited to say "Alpha adds two" (the edit
/// closes the popup), the caret to the blank line in `beta`, `C-c H`:
/// `*lsp-help*` opens "Alpha adds one", a sentence the buffer no longer
/// holds, under the same header as any other kept text. The record of
/// E7i is a buffer that painted the last parse's colours over text that
/// had moved; this is the last answer shown over text that has changed.
/// The row accepts any of: the kept text dropped at the edit, or a
/// header saying the text predates an edit.
#[test]
fn review2_c_c_h_after_an_edit_does_not_open_documentation_the_buffer_no_longer_has() {
    if !rust_analyzer_provisioned() {
        return;
    }
    let (mut state, _dir) = open("edited", TWO_FUNCTIONS);
    goto(&mut state, after(TWO_FUNCTIONS, "let _ = alph"));
    ask_until(&mut state, 'h', 120, |p| {
        p.kind == PopupKind::Hover && p.lines.iter().any(|l| l.contains("Alpha adds one"))
    });
    // `one` -> `two` in the doc comment, through the keys.
    goto(&mut state, after(TWO_FUNCTIONS, "/// Alpha adds "));
    for _ in 0.."one".len() {
        key(&mut state, 'd', KeyModifiers::CONTROL);
        tick(&mut state);
    }
    typed(&mut state, "two");
    let now: String = eval(
        &state,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    assert!(now.contains("Alpha adds two") && !now.contains("Alpha adds one"));
    let _ = paint(&state);
    assert!(popup(&state).is_none(), "the edit closed the popup");
    goto(
        &mut state,
        now.find("fn beta(y: u8) -> u8 {\n").expect("beta") + 23,
    );
    tick(&mut state);
    match press_c_c_h(&mut state) {
        Err(seen) => eprintln!("e8r2: C-c H opened nothing after the edit: {seen:?}"),
        Ok(text) => {
            let header = text.lines().next().unwrap_or_default().to_owned();
            eprintln!(
                "e8r2: after the edit *lsp-help* header {header:?}; first lines {:?}",
                text.lines().take(8).collect::<Vec<_>>()
            );
            assert!(
                !text.contains("Alpha adds one") || header.contains("edit"),
                "C-c H opened documentation the buffer no longer holds (\"Alpha adds one\"; \
                 the buffer says \"Alpha adds two\") under {header:?}"
            );
        }
    }
}

/// E8 fix round 2 (review 2's Low 1), both halves through the keys. The
/// hover on `alpha` at line 13, the caret to the blank line in `beta`,
/// `C-c H`: the kept text opens under a header naming where it was asked
/// (`main.rs:13`), not only that it is not the caret's. Then, with
/// `*lsp-help*` quit, the doc comment edited from "one" to "two", the
/// caret back on that blank line, `C-c H` again: the kept text predates
/// the edit and is gone, so nothing opens and the status says there is
/// no hover, where at `57a980e` "Alpha adds one" came back.
#[test]
fn fix2_the_kept_hover_names_its_place_and_is_dropped_by_an_edit() {
    if !rust_analyzer_provisioned() {
        return;
    }
    let (mut state, _dir) = open("kept", TWO_FUNCTIONS);
    goto(&mut state, after(TWO_FUNCTIONS, "let _ = alph"));
    ask_until(&mut state, 'h', 120, |p| {
        p.kind == PopupKind::Hover && p.lines.iter().any(|l| l.contains("Alpha adds one"))
    });
    let blank_in_beta = after(TWO_FUNCTIONS, "fn beta(y: u8) -> u8 {\n");
    goto(&mut state, blank_in_beta);
    let _ = paint(&state);
    assert!(popup(&state).is_none(), "the motion closed the popup");
    let text = press_c_c_h(&mut state)
        .unwrap_or_else(|seen| panic!("before the edit C-c H opened nothing; statuses {seen:?}"));
    let header = text.lines().next().unwrap_or_default().to_owned();
    eprintln!("e8fr2: before the edit *lsp-help* header {header:?}");
    assert!(text.contains("Alpha adds one"), "the kept text: {text:?}");
    assert!(
        header.contains("the last popup's (none at the caret), from main.rs:13"),
        "the header names where the kept text was asked: {header:?}"
    );
    key(&mut state, 'q', KeyModifiers::NONE);
    tick(&mut state);
    assert!(lsp_help(&state).is_none(), "q quit *lsp-help*");
    // `one` -> `two` in the doc comment, through the keys.
    goto(&mut state, after(TWO_FUNCTIONS, "/// Alpha adds "));
    for _ in 0.."one".len() {
        key(&mut state, 'd', KeyModifiers::CONTROL);
        tick(&mut state);
    }
    typed(&mut state, "two");
    let now: String = eval(
        &state,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    assert!(now.contains("Alpha adds two") && !now.contains("Alpha adds one"));
    goto(&mut state, blank_in_beta);
    let _ = paint(&state);
    assert!(popup(&state).is_none(), "no popup is open");
    match press_c_c_h(&mut state) {
        Err(seen) => assert_eq!(
            seen.last().map(String::as_str),
            Some("LSP: no hover info"),
            "after the edit C-c H says there is no hover: {seen:?}"
        ),
        Ok(text) => panic!(
            "after the edit C-c H opened kept text the buffer no longer holds: {:?}",
            text.lines().take(6).collect::<Vec<_>>()
        ),
    }
}

// ---- 3. #317: a project reached through a symlink --------------------------

/// The server's `rootUri` as pmacs holds it, and the path of the buffer
/// whose URI the server is told about.
fn uris(state: &EditorState) -> String {
    eval(
        state,
        "local out = {} \
         for _, r in ipairs(pmacs.lsp.list()) do \
           for k, v in pairs(r) do \
             if type(v) == 'string' and (k:find('uri') or k:find('root')) then \
               out[#out + 1] = k .. '=' .. v \
             end \
           end \
         end \
         table.sort(out) \
         local b = pmacs.window.buffer() \
         out[#out + 1] = 'buffer=' .. tostring(b and b:path()) \
         return table.concat(out, ' ')",
    )
}

/// FAILS at `57a980e`, on Linux, and is #317's mechanism. A cargo
/// project reached through a symlinked directory: pmacs resolves the
/// symlink for the server's root (`pmacs.project.detect` canonicalizes)
/// and keeps it in the document's URI (`normalize_buffer_path` is
/// lexical), so rust-analyzer loads the crate at the real path and is
/// told about a file at the linked one, which is in no crate it has; it
/// answers `null` to every hover. That is CI's macOS failure exactly
/// ("LSP: no hover info" after negotiating UTF-8): `tempfile` puts the
/// fixtures under `$TMPDIR`, `/var/folders/…`, and `/var` is a symlink
/// to `/private/var`. The control is the same project opened by its
/// real path, which answers.
#[test]
fn review2_rust_analyzer_answers_a_file_opened_through_a_symlinked_directory() {
    if !rust_analyzer_provisioned() {
        return;
    }
    let source = "/// Alpha adds one to its argument.\n\
fn alpha(x: u8) -> u8 { x + 1 }\n\
fn main() { let _ = alpha(1); }\n";
    let dir = tempfile::Builder::new()
        .prefix("e8r2-symlink-")
        .tempdir()
        .expect("tempdir");
    let real = dir.path().join("real");
    let real_main = write_project(&real, source);
    // Control: by its own path.
    let mut state = visit(&real_main);
    goto(&mut state, after(source, "let _ = alph"));
    let control = try_ask(&mut state, 'h', 120, |p| p.kind == PopupKind::Hover);
    eprintln!(
        "e8r2: by the real path: {} -> {:?}",
        uris(&state),
        control.as_ref().map(|p| p.lines.len())
    );
    control.expect("control: rust-analyzer answers the project by its real path");
    drop(state);
    // The same project through a symlink to its directory.
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let mut state = visit(&link.join("src/main.rs"));
    goto(&mut state, after(source, "let _ = alph"));
    let linked = try_ask(&mut state, 'h', 60, |p| p.kind == PopupKind::Hover);
    let held = uris(&state);
    eprintln!("e8r2: through the link: {held} -> {linked:?}");
    linked.unwrap_or_else(|e| {
        panic!(
            "a project opened through a symlinked directory gets no hover: {e}; pmacs holds {held}"
        )
    });
}
