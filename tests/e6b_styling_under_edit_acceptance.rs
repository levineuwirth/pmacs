//! E6b --- styling under edit, through the real attach path.
//!
//! The unit rows in `src/semantic_tokens.rs`, `src/highlight.rs` and
//! `src/semantic_render.rs` seed the store and attach the recorder by
//! hand. These rows do neither: a real `EditorState` with the bundled
//! `lsp.lua`, a fake server in `semantichold` mode holding its answer,
//! a Rust file opened through `find_or_open`, and a character typed
//! through `dispatch_key`. What they read is what a grid user sees ---
//! the painted frame --- and what the store holds, between the
//! keystroke and the answer.
//!
//! The marker face is an RGB the bundled theme never emits, merged over
//! the server's `namespace` type, so a cell is the server's by its
//! foreground alone.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::cell::{CellGrid, CellSize, Color};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::window::{LINE_NUMBER_GUTTER_PAD, decimal_digits};

#[path = "common/iso.rs"]
mod iso;

/// The refinement's face; `pmacs.theme.merge` below sets it on the fake
/// server's one token type.
const MARK: Color = Color::Rgb(0x7b, 0x1f, 0xa2);

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

/// The text columns of row 0 painted in the marker foreground, as
/// `(start, end)` runs, through one real `paint_frame` at 4 x 40. Text
/// columns: the line-number gutter (one digit plus its padding for the
/// fixtures here, all under ten lines) is subtracted, so a run is the
/// byte range it paints on this ASCII text.
fn marked_runs(state: &EditorState) -> Vec<(u32, u32)> {
    let gutter = decimal_digits(1) + LINE_NUMBER_GUTTER_PAD;
    let (rows, cols) = (4u32, 40u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![pmacs::cell::Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(state, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    let mut out: Vec<(u32, u32)> = Vec::new();
    for col in gutter..cols {
        if cells[col as usize].style.fg != MARK {
            continue;
        }
        let text_col = col - gutter;
        match out.last_mut() {
            Some((_, end)) if *end == text_col => *end += 1,
            _ => out.push((text_col, text_col + 1)),
        }
    }
    out
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
    let sink = range_sink.map_or(String::new(), |p| {
        format!("PMACS_FAKE_RANGE_SINK = '{}',", p.display())
    });
    state
        .lua_host
        .lua()
        .load(format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
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

/// E6b.1 --- **the recorder attaches with the server, and a typed
/// character reaches the store's log with its position.** After the
/// attach pull, the buffer carries the edit recorder as a buffer view
/// (not a window overlay: `pmacs.lsp._track_edits`, called from
/// `attach_buffer`). Typing `x` at the top of `fn main() {}` shifts
/// the held tokens by one on the same tick, before any answer, and the
/// grid paints them where the words are now --- `fn` at [1,3), `main`
/// at [4,8) --- with nothing at the unshifted cells. When the held
/// answer lands, `xfn` is one word and the tokens are the server's
/// again.
#[test]
fn e6b_1_a_typed_character_shifts_the_held_tokens_before_the_server_answers() {
    let (mut state, _dir, uri) = attached("fn main() {}\n");

    let bid = state.core.borrow().active_window().buffer_id;
    {
        let registry = state.core.borrow().registry.clone();
        let reg = registry.borrow();
        let kinds = reg.get(bid).expect("a.rs buffer").view_kinds();
        assert!(
            kinds.contains(&"semantic-edit-recorder"),
            "attach_buffer attaches the edit recorder to the buffer; views: {kinds:?}"
        );
    }
    assert_eq!(
        positioned(&state, &uri),
        Some(vec![(0, 2), (3, 7)]),
        "the attach pull's tokens: fn, main"
    );
    assert!(!store_is_stale(&state, &uri), "fresh after the pull");
    assert_eq!(marked_runs(&state), vec![(0, 2), (3, 7)]);

    state
        .lua_host
        .lua()
        .load("pmacs.editor.goto_byte(0)")
        .exec()
        .expect("cursor to the top");
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    );

    // Same tick, no pump: the store is stale by the keystroke and
    // holds the tokens shifted, and the grid paints them there.
    assert!(
        store_is_stale(&state, &uri),
        "the keystroke leaves the store stale until the server answers"
    );
    assert_eq!(
        positioned(&state, &uri),
        Some(vec![(1, 3), (4, 8)]),
        "held tokens shifted by the typed byte, not dropped"
    );
    assert_eq!(
        marked_runs(&state),
        vec![(1, 3), (4, 8)],
        "the grid paints the shifted tokens: `xfn main`"
    );

    // The held answer lands: `xfn` is one word to the fake tokenizer.
    let deadline = Instant::now() + Duration::from_secs(10);
    while store_is_stale(&state, &uri) {
        assert!(
            Instant::now() < deadline,
            "the server's answer to the edit never landed"
        );
        tick(&mut state);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        positioned(&state, &uri),
        Some(vec![(0, 3), (4, 8)]),
        "the server's answer for the edited text"
    );
    assert_eq!(marked_runs(&state), vec![(0, 3), (4, 8)]);
}

/// E6b.3, the grid half --- **no painted frame between the keystroke
/// and the answer lacks the refinement outside the edited token.** The
/// same sequence as the GPU probe, read through `paint_frame` on every
/// tick from the keystroke until the store is fresh again: `main`'s
/// cells carry the marker in every frame, shifted by the byte typed in
/// front of it. Reverting E6b.2 (the grid site returning while the
/// store is stale) fails this on the first tick.
#[test]
fn e6b_3_the_grid_keeps_the_refinement_on_every_tick_until_the_answer_lands() {
    let (mut state, _dir, uri) = attached("fn main() {}\n");
    assert_eq!(marked_runs(&state), vec![(0, 2), (3, 7)]);

    state
        .lua_host
        .lua()
        .load("pmacs.editor.goto_byte(0)")
        .exec()
        .expect("cursor to the top");
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    );

    let mut frames = 0u32;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let runs = marked_runs(&state);
        frames += 1;
        assert!(
            runs.contains(&(4, 8)),
            "frame {frames} after the keystroke lost `main`'s refinement: {runs:?}"
        );
        if !store_is_stale(&state, &uri) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the server's answer to the edit never landed"
        );
        tick(&mut state);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        frames >= 2,
        "the answer must land after at least one stale frame was read (hold {HOLD_MS} ms); frames={frames}"
    );
    assert_eq!(
        marked_runs(&state),
        vec![(0, 3), (4, 8)],
        "settled: the server's tokens for `xfn main`"
    );
}

/// E6b.4 --- **the visible lines are asked for first, and their answer
/// aligns the store before the whole document's.** The fake server
/// answers `/range` at once and holds `/full` for 1.5 s. After the
/// attach pull has fully landed (the whole-document `resultId` is in
/// the store), typing `x` sends one `didChange`, one `/range` for the
/// lines on screen and one `/full`; the store is fresh again within
/// 800 ms of the keystroke, which only the range answer can do, and
/// the range the server was asked covers line 0 with the margin the
/// runtime adds. Without the range pull the store stays stale until
/// the held whole-document answer, past the bound.
#[test]
fn e6b_4_the_visible_range_is_pulled_first_and_lands_ahead_of_the_whole_document() {
    let sink_dir = tempfile::tempdir().expect("sink tempdir");
    let sink = sink_dir.path().join("ranges.jsonl");
    let (mut state, _dir, uri) = attached_with("fn main() {}\n", 1500, 0, Some(&sink));

    // The whole-document answer from the attach pull, so the server is
    // idle when the keystroke's requests reach it.
    let full_landed = "(function() \
       for _, r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind == 'initialized' then \
           local rid = pmacs.semantic_tokens.result_id(r.id, '"
        .to_owned()
        + &uri
        + "') \
           return rid ~= nil and rid:sub(1, 5) == 'hold-' and rid:sub(1, 11) ~= 'hold-range-' \
         end \
       end \
       return false \
     end)()";
    assert!(
        pump_lua_flag(&mut state, &full_landed, 10),
        "the attach pull's whole-document answer never landed"
    );
    let ranges_before = std::fs::read_to_string(&sink).unwrap_or_default();
    let asked_before = ranges_before.lines().count();
    assert!(
        asked_before >= 1,
        "the attach pull asked for the visible range"
    );

    state
        .lua_host
        .lua()
        .load("pmacs.editor.goto_byte(0)")
        .exec()
        .expect("cursor to the top");
    let typed_at = Instant::now();
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    assert!(store_is_stale(&state, &uri), "stale by the keystroke");
    let deadline = typed_at + Duration::from_millis(800);
    while store_is_stale(&state, &uri) {
        assert!(
            Instant::now() < deadline,
            "the store did not turn fresh within 800 ms of the keystroke; only the \
             range answer could, the whole-document one is held 1.5 s"
        );
        tick(&mut state);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        positioned(&state, &uri),
        Some(vec![(0, 3), (4, 8)]),
        "the range answer for `xfn main`"
    );
    let ranges_after = std::fs::read_to_string(&sink).expect("range sink");
    let asked_after: Vec<&str> = ranges_after.lines().collect();
    assert!(
        asked_after.len() > asked_before,
        "the keystroke's flush asked for a range: {ranges_after}"
    );
    let last: serde_json::Value =
        serde_json::from_str(asked_after.last().expect("a range")).expect("range json");
    let start_line = last["start"]["line"].as_u64().expect("start line");
    let end_line = last["end"]["line"].as_u64().expect("end line");
    assert!(
        start_line == 0 && end_line >= 1,
        "the range covers the one visible line: {last}"
    );
}
