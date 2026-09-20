// tests/e7c_indicator_acceptance.rs --- C7c fix round 3, the mode line
// while typing: the activity indicator shows slow work, at one width.

//! The owner's window run saw the mode line change for the duration of
//! any typing after a save and settle when the typing stopped. Fix
//! round 2 measured it frame by frame: the activity indicator's purpose
//! for an LSP request was `lsp <method> <uri>` --- ninety characters
//! with the document's URI --- and the semantic-token and inlay-hint
//! pulls a keystroke draws, each answered in tens of milliseconds, kept
//! it on the row from the first frame after the save until 2.5 s after
//! the last key, reflowing the right group on every frame. The owner's
//! ruling, witnessed here through the production dispatch against the
//! fake:
//!
//! * An LSP request reaches the indicator only after it has been in
//!   flight past `ui.activity-indicator-threshold-ms` (default 300), and
//!   appears as the method alone, never the URI, in a segment of one
//!   fixed width; a request answered under the threshold never appears.
//! * The busy suffix on `ready` (`ready·check`) takes a fixed slot, so
//!   the LSP segment is one width with the suffix and without it.
//! * The owner's sequence --- save, type for five seconds --- recorded
//!   on both frontends' composition (the grid row `paint_frame` paints,
//!   and the `StatuslineSegments` a semantic frontend composes its own
//!   from): no segment to the left or right of the indicator or the
//!   suffix moves, and the indicator never appears.
//! * A slow request --- a rename the fake holds for 1.5 s --- does
//!   appear, as its method, past the threshold, moving nothing beside
//!   it, and vanishes when answered.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::async_runtime::JobKind;
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};

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
    let dir = std::env::temp_dir().join(format!("pmacs-e7c-ind-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The right-side segments of the one window, as `(face, text)`, the
/// text untrimmed: the widths are the question here.
fn right_segments(s: &EditorState) -> Vec<(String, String)> {
    let outcome = evaluate_statusline(
        s.lua_host.lua(),
        &s.core,
        &s.statusline_registry,
        StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        },
    );
    let StatuslineEvaluationOutcome::Ready(windows) = outcome.outcome else {
        return Vec::new();
    };
    windows
        .into_iter()
        .flat_map(|w| w.right)
        .map(|seg| (seg.face, seg.text))
        .collect()
}

fn segment_with_face(segments: &[(String, String)], face: &str) -> Option<String> {
    segments
        .iter()
        .find(|(f, _)| f == face)
        .map(|(_, t)| t.clone())
}

fn cells(text: &str) -> usize {
    text.chars().count()
}

const GRID_COLS: u32 = 160;

/// The grid's mode line row, painted through `paint_frame` at 40 x 160,
/// read by position: the row above the status row.
fn grid_modeline(s: &EditorState) -> String {
    let (rows, cols) = (40u32, GRID_COLS);
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
                Glyph::Cluster(bytes) => std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|t| t.chars().next())
                    .unwrap_or(' '),
                Glyph::Continuation => ' ',
            })
            .collect()
    };
    let row = text_of(rows - 2);
    assert!(
        row.contains(":C"),
        "the row above the status row is the mode line: {row:?}"
    );
    row
}

/// Where the LSP segment and the protected cursor group start on the
/// grid row, in cells, and the cursor readout's own width.
struct GridColumns {
    lsp: usize,
    protected: usize,
    readout_width: usize,
}

fn grid_columns(row: &str) -> GridColumns {
    let chars: Vec<char> = row.chars().collect();
    let find = |needle: &str| -> Option<usize> {
        let n: Vec<char> = needle.chars().collect();
        (0..chars.len().saturating_sub(n.len())).find(|&i| chars[i..i + n.len()] == n[..])
    };
    let lsp = find("LSP:").expect("the LSP segment is on the row");
    let colon_c = find(":C").expect("the cursor readout is on the row");
    // Back from `:C` to the space before `L<line>`.
    let mut protected = colon_c;
    while protected > 0 && chars[protected - 1] != ' ' {
        protected -= 1;
    }
    let mut end = colon_c + 2;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    GridColumns {
        lsp,
        protected,
        readout_width: end - protected,
    }
}

/// What a semantic frontend composes its right group from: the last
/// `StatuslineSegments` it received (suppressed while unchanged, so the
/// last is carried).
#[derive(Default)]
struct WireRight {
    texts: Vec<String>,
}

impl WireRight {
    fn absorb(&mut self, frame: &[InstanceMessage]) {
        for m in frame {
            if let InstanceMessage::StatuslineSegments { right, .. } = m {
                self.texts = right.iter().map(|seg| seg.text.clone()).collect();
            }
        }
    }
}

struct Fixture {
    s: EditorState,
    render: SemanticRenderState,
    wire: WireRight,
}

/// An editor visiting `<dir>/a.rs` against the fake in `didsave` mode
/// with the rust config's retry and check sources, the handshake
/// answered, and a semantic frontend's render state over the top of
/// the file.
fn open_with_fake(tag: &str, env: &str) -> Fixture {
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
               env = {{ PMACS_FAKE_LSP_MODE = 'didsave'{env} }},
               save_retry = true,
               check_sources = {{ 'rustc' }},
             }}",
            fake_lsp_path(),
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
    let buffer_id = s.core.borrow().active_window().buffer_id;
    let mut render = SemanticRenderState::new(FrontendId::LOCAL);
    render.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 4096,
        },
        0,
    );
    let mut f = Fixture {
        s,
        render,
        wire: WireRight::default(),
    };
    settle(&mut f, 2);
    f
}

/// Tick until no LSP request is in flight and the label reads `ready`,
/// or `secs` pass.
fn settle(f: &mut Fixture, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        tick(&mut f.s);
        let frame = f.render.render_frame(&f.s);
        f.wire.absorb(&frame);
        let quiet: bool = eval(
            &f.s,
            "for _, job in ipairs(pmacs.workers.snapshot().active) do
               if job.kind == 'lsp_request' then return false end
             end
             return true",
        );
        let segs = right_segments(&f.s);
        let label = segment_with_face(&segs, "ui.modeline.lsp").unwrap_or_default();
        if quiet && label.trim_end() == "LSP:ready" {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One frame on both frontends' composition.
struct Frame {
    at_ms: u128,
    grid: String,
    segments: Vec<(String, String)>,
    wire_right: Vec<String>,
}

fn read_frame(f: &mut Fixture, t0: Instant) -> Frame {
    let frame = f.render.render_frame(&f.s);
    f.wire.absorb(&frame);
    Frame {
        at_ms: t0.elapsed().as_millis(),
        grid: grid_modeline(&f.s),
        segments: right_segments(&f.s),
        wire_right: f.wire.texts.clone(),
    }
}

fn activity_of(frame: &Frame) -> Option<String> {
    segment_with_face(&frame.segments, "ui.modeline.activity")
}

fn lsp_of(frame: &Frame) -> String {
    segment_with_face(&frame.segments, "ui.modeline.lsp").unwrap_or_default()
}

/// The knob, as `pmacs.config.describe` reports it: an integer, live,
/// 300 by default.
#[test]
fn e7c_fix_3_the_indicator_threshold_is_a_registered_knob_at_300_ms() {
    let s = EditorState::new_with_roots(&iso::roots());
    let (kind, default, mutability): (String, i64, String) = eval(
        &s,
        "local d = pmacs.config.describe('ui.activity-indicator-threshold-ms')
         return d.type, d.default, d.mutability",
    );
    assert_eq!(kind, "integer");
    assert_eq!(default, 300);
    assert_eq!(mutability, "live");
}

/// The indicator is one width for a short purpose and for a long one:
/// a short method is padded to the width, a method longer than the
/// room keeps its tail behind `…` --- the method's own name is its last
/// component --- and an LSP request is named by its method alone, the
/// URI staying in `*workers*`. Read at a zero threshold, since the
/// width and not the delay is the question.
#[test]
fn e7c_fix_3_the_indicator_is_one_width_for_a_short_and_a_long_purpose() {
    let mut s = EditorState::new_with_roots(&iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        "pmacs.config.set('ui.activity-indicator-threshold-ms', 0)",
    );
    let width: usize = eval(&s, "return pmacs._async._activity_width");
    let (short, _) = s.async_runtime.register_external_shown_as(
        JobKind::LspRequest,
        None,
        "lsp textDocument/rename file:///tmp/x.rs",
        "textDocument/rename",
    );
    let text = segment_with_face(&right_segments(&s), "ui.modeline.activity")
        .expect("one request in flight shows");
    assert_eq!(cells(&text), width, "{text:?}");
    assert_eq!(text.trim_end(), "⋯1 textDocument/rename");
    assert!(!text.contains("file://"), "never the URI: {text:?}");
    s.async_runtime.complete_external_cancelled(short);
    // A pending entry leaves `Running` inside the runtime's tick.
    s.tick_async();
    let (long, _) = s.async_runtime.register_external_shown_as(
        JobKind::LspRequest,
        None,
        "lsp textDocument/semanticTokens/full/delta file:///tmp/x.rs",
        "textDocument/semanticTokens/full/delta",
    );
    let text = segment_with_face(&right_segments(&s), "ui.modeline.activity")
        .expect("one request in flight shows");
    assert_eq!(cells(&text), width, "{text:?}");
    assert!(
        text.starts_with("⋯1 …") && text.ends_with("Tokens/full/delta"),
        "the tail is kept behind an ellipsis: {text:?}"
    );
    let purposes: Vec<String> = eval(
        &s,
        "local out = {}
         for _, job in ipairs(pmacs.workers.snapshot().active) do out[#out + 1] = job.purpose end
         return out",
    );
    assert!(
        purposes
            .iter()
            .any(|p| p == "lsp textDocument/semanticTokens/full/delta file:///tmp/x.rs"),
        "*workers* keeps the whole purpose: {purposes:?}"
    );
    s.async_runtime.complete_external_cancelled(long);
}

/// The owner's sequence: with a check's diagnostic on the file from a
/// save, a flycheck cycle open (the fake's own begins and ends in one
/// write, so one is held open through the progress echo and closed in
/// the middle of the typing, which is when the owner's check ended),
/// save and type for five seconds --- thirty-four keys, one every 150
/// ms --- every frame read on both frontends' composition. The
/// indicator never appears, though every keystroke's pulls went out and
/// were answered under the threshold; the LSP segment is one width with
/// the suffix and without it; on the grid the distance from the LSP
/// segment to the cursor group never changes and the LSP segment's
/// column moves only with the cursor readout's own width; on the wire
/// the right group is the same segments at the same widths on every
/// frame. Bitten by giving the threshold zero (the indicator appears),
/// by dropping the slot (the LSP segment changes width when the cycle
/// ends), or by the indicator's old priority together with a zero
/// threshold (the LSP column moves).
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one sequence, read frame by frame on two compositions"
)]
fn e7c_fix_3_typing_after_a_save_moves_nothing_on_the_mode_line() {
    let mut f = open_with_fake("typing", "");
    exec(
        &f.s,
        "_G.__e7c_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7c_saved = true end)
         pmacs.editor.goto_byte(0)",
    );
    for ch in "// CHECKME".chars() {
        press(&mut f.s, KeyCode::Char(ch));
    }
    if f.s.core.borrow().completion_popup.lock().unwrap().is_some() {
        press(&mut f.s, KeyCode::Esc);
    }
    press(&mut f.s, KeyCode::Enter);
    exec(&f.s, "pmacs.command.invoke('buffer.save')");
    assert!(pump_lua_flag(&mut f.s, "_G.__e7c_saved", 10), "saved");
    settle(&mut f, 3);
    let sources: Vec<String> = eval(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, d in ipairs(pmacs.diag.list(rec.uri)) do out[#out + 1] = tostring(d.source) end
         return out",
    );
    assert!(
        sources.contains(&"rustc".to_owned()),
        "the check's diagnostic landed: {sources:?}"
    );
    // The cycle held open: the suffix is on from here.
    exec(
        &f.s,
        "local rec = pmacs.lsp.active_attachment()
         pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
           { token = 'rust-analyzer/flycheck/held', value = { kind = 'begin', title = 'cargo check' } })",
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        tick(&mut f.s);
        if lsp_of(&read_frame(&mut f, deadline)).trim_end() == "LSP:ready·check" {
            break;
        }
        assert!(Instant::now() < deadline, "the held cycle's suffix arrives");
        std::thread::sleep(Duration::from_millis(5));
    }
    // The sequence: a comment opener so the save writes, the save, and
    // thirty-four keys at 150 ms, the held cycle ended at the
    // seventeenth.
    exec(&f.s, "pmacs.editor.goto_byte(0)\n_G.__e7c_saved = false");
    for ch in "//".chars() {
        press(&mut f.s, KeyCode::Char(ch));
    }
    let t0 = Instant::now();
    exec(&f.s, "pmacs.command.invoke('buffer.save')");
    let typing: Vec<char> = " e7c typing a comment above the warning, "
        .chars()
        .take(34)
        .collect();
    let mut frames: Vec<Frame> = vec![read_frame(&mut f, t0)];
    let mut typed = 0usize;
    let mut next_key_at = Duration::from_millis(50);
    let end_at = Duration::from_millis(50 + 150 * 34 + 1000);
    while t0.elapsed() < end_at {
        if typed < typing.len() && t0.elapsed() >= next_key_at {
            press(&mut f.s, KeyCode::Char(typing[typed]));
            typed += 1;
            next_key_at += Duration::from_millis(150);
            if typed == 17 {
                exec(
                    &f.s,
                    "local rec = pmacs.lsp.active_attachment()
                     pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
                       { token = 'rust-analyzer/flycheck/held', value = { kind = 'end' } })",
                );
            }
        }
        tick(&mut f.s);
        frames.push(read_frame(&mut f, t0));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        pump_lua_flag(&mut f.s, "_G.__e7c_saved", 1),
        "the save wrote"
    );
    assert_eq!(typed, 34, "every key went in");
    // Printed for the record: every distinct grid row and wire group.
    let mut last_grid = String::new();
    let mut last_wire = Vec::new();
    for fr in &frames {
        if fr.grid != last_grid {
            eprintln!("MODELINE {} G {:?}", fr.at_ms, fr.grid.trim_end());
            last_grid.clone_from(&fr.grid);
        }
        if fr.wire_right != last_wire {
            eprintln!("MODELINE {} W {:?}", fr.at_ms, fr.wire_right);
            last_wire.clone_from(&fr.wire_right);
        }
    }
    // Requests went out and were answered: the completed ring holds
    // LSP requests from the typing.
    let answered: usize = eval(
        &f.s,
        "local n = 0
         for _, job in ipairs(pmacs.workers.snapshot().completed) do
           if job.kind == 'lsp_request' then n = n + 1 end
         end
         return n",
    );
    assert!(answered > 0, "the typing's pulls were answered");
    // The indicator never appeared, on either composition.
    for fr in &frames {
        assert_eq!(
            activity_of(fr),
            None,
            "no request outlived the threshold, so no indicator at {} ms: {:?}",
            fr.at_ms,
            fr.segments
        );
        assert!(
            !fr.grid.contains('⋯') && !fr.wire_right.iter().any(|t| t.contains('⋯')),
            "no indicator on the grid or the wire at {} ms: {:?} {:?}",
            fr.at_ms,
            fr.grid.trim_end(),
            fr.wire_right
        );
    }
    // The LSP segment is one width throughout, with the suffix and
    // without, and both were seen.
    let widths: std::collections::BTreeSet<usize> =
        frames.iter().map(|fr| cells(&lsp_of(fr))).collect();
    assert_eq!(widths.len(), 1, "one width for the LSP segment: {widths:?}");
    assert!(
        frames
            .iter()
            .any(|fr| lsp_of(fr).trim_end() == "LSP:ready·check"),
        "the suffix was on the row while the cycle ran"
    );
    assert!(
        frames.iter().any(|fr| lsp_of(fr).trim_end() == "LSP:ready"),
        "and off it after the cycle ended"
    );
    // On the grid: the distance from the LSP segment to the cursor
    // group never changes, and the LSP column moves only when the
    // cursor readout's own width does.
    let first = grid_columns(&frames[0].grid);
    let mut readout_widths = std::collections::BTreeSet::new();
    for fr in &frames {
        let c = grid_columns(&fr.grid);
        assert_eq!(
            c.protected - c.lsp,
            first.protected - first.lsp,
            "the LSP segment and the cursor group keep their distance at {} ms: {:?}",
            fr.at_ms,
            fr.grid.trim_end()
        );
        assert_eq!(
            c.protected + c.readout_width,
            first.protected + first.readout_width,
            "the cursor readout ends where it began at {} ms: {:?}",
            fr.at_ms,
            fr.grid.trim_end()
        );
        readout_widths.insert(c.readout_width);
    }
    eprintln!(
        "MODELINE LSP column {} on the first frame, readout widths seen {readout_widths:?}",
        first.lsp
    );
    // On the wire: the same number of segments at the same widths on
    // every frame.
    let shapes: std::collections::BTreeSet<Vec<usize>> = frames
        .iter()
        .filter(|fr| !fr.wire_right.is_empty())
        .map(|fr| fr.wire_right.iter().map(|t| cells(t)).collect())
        .collect();
    assert_eq!(
        shapes.len(),
        1,
        "one shape for the wire's right group across the sequence: {shapes:?}"
    );
}

/// A request answered under the threshold never appears: a hover the
/// fake answers at once, the frames read for 600 ms, the request in
/// the completed ring.
#[test]
fn e7c_fix_3_a_request_answered_under_the_threshold_never_appears() {
    let mut f = open_with_fake("hover", "");
    let before: usize = eval(
        &f.s,
        "local n = 0
         for _, job in ipairs(pmacs.workers.snapshot().completed) do
           if job.purpose:find('textDocument/hover', 1, true) then n = n + 1 end
         end
         return n",
    );
    exec(&f.s, "pmacs.editor.goto_byte(3)");
    let t0 = Instant::now();
    exec(&f.s, "pmacs.command.invoke('lsp.hover')");
    let mut seen = Vec::new();
    while t0.elapsed() < Duration::from_millis(600) {
        tick(&mut f.s);
        let fr = read_frame(&mut f, t0);
        if let Some(a) = activity_of(&fr) {
            seen.push((fr.at_ms, a));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let after: usize = eval(
        &f.s,
        "local n = 0
         for _, job in ipairs(pmacs.workers.snapshot().completed) do
           if job.purpose:find('textDocument/hover', 1, true) then n = n + 1 end
         end
         return n",
    );
    assert!(after > before, "the hover went out and was answered");
    assert!(seen.is_empty(), "and never reached the indicator: {seen:?}");
}

/// A slow request appears, as its method, past the threshold, moving
/// nothing beside it, and vanishes when answered: `lsp.rename` through
/// the command and the minibuffer, the fake holding its answer 1.5 s.
/// Before the threshold no indicator; from about 300 ms `⋯1
/// textDocument/rename` at the fixed width, never the URI, with the
/// LSP segment and the cursor group at the columns they had before
/// (the indicator being leftmost of the right group); after the
/// answer, no indicator. Bitten by the old priority (the LSP column
/// moves while the indicator shows) or by the old purpose (the URI).
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one request read frame by frame from before it goes out to after its answer"
)]
fn e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered() {
    let mut f = open_with_fake("rename", ", PMACS_FAKE_LSP_RENAME_HOLD_MS = '1500'");
    exec(&f.s, "pmacs.editor.goto_byte(3)");
    settle(&mut f, 2);
    let t0 = Instant::now();
    let before = read_frame(&mut f, t0);
    let base = grid_columns(&before.grid);
    assert_eq!(activity_of(&before), None, "quiet before the request");
    // The command, then the new name into the minibuffer and RET: the
    // request goes out on the accept.
    exec(&f.s, "pmacs.command.invoke('lsp.rename')");
    for ch in "renamed".chars() {
        press(&mut f.s, KeyCode::Char(ch));
    }
    let t_request = Instant::now();
    press(&mut f.s, KeyCode::Enter);
    let mut frames: Vec<Frame> = Vec::new();
    while t_request.elapsed() < Duration::from_millis(2600) {
        tick(&mut f.s);
        frames.push(read_frame(&mut f, t_request));
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut last = None;
    for fr in &frames {
        let a = activity_of(fr);
        if a != last {
            eprintln!(
                "INDICATOR {} {:?} grid {:?}",
                fr.at_ms,
                a,
                fr.grid.trim_end()
            );
            last = a;
        }
    }
    let early: Vec<&Frame> = frames
        .iter()
        .filter(|fr| fr.at_ms < 250 && activity_of(fr).is_some())
        .collect();
    assert!(
        early.is_empty(),
        "nothing on the indicator under the threshold: {:?}",
        early
            .iter()
            .map(|fr| (fr.at_ms, activity_of(fr)))
            .collect::<Vec<_>>()
    );
    let shown: Vec<&Frame> = frames
        .iter()
        .filter(|fr| activity_of(fr).is_some())
        .collect();
    assert!(
        !shown.is_empty(),
        "the held rename reached the indicator past the threshold"
    );
    let first_shown = shown[0].at_ms;
    assert!(
        (300..1500).contains(&first_shown),
        "it appeared after the threshold and before the answer: {first_shown} ms"
    );
    let width: usize = eval(&f.s, "return pmacs._async._activity_width");
    for fr in &shown {
        let text = activity_of(fr).unwrap();
        assert!(
            text.starts_with("⋯1 textDocument/rename"),
            "the method alone at {} ms: {text:?}",
            fr.at_ms
        );
        assert!(!text.contains("file://"), "never the URI: {text:?}");
        assert_eq!(cells(&text), width, "at the fixed width: {text:?}");
        let c = grid_columns(&fr.grid);
        assert_eq!(
            (c.lsp, c.protected),
            (base.lsp, base.protected),
            "the LSP segment and the cursor group did not move while the indicator showed, at {} ms: {:?}",
            fr.at_ms,
            fr.grid.trim_end()
        );
    }
    // Both compositions carried it. A frame reads the wire, then the
    // grid, then the evaluator, a few milliseconds apart, so at the
    // threshold's edge the wire can trail the evaluator by one read
    // (CI's `--test-threads=1` leg read exactly that at 300 ms); the
    // claim is that each surface showed the method at the width, not
    // that they crossed the edge in the same read.
    let shown_text = activity_of(shown[0]).unwrap();
    assert!(
        shown
            .iter()
            .any(|fr| fr.grid.contains("⋯1 textDocument/rename")),
        "the grid row carried the indicator while it showed"
    );
    assert!(
        shown
            .iter()
            .any(|fr| fr.wire_right.iter().any(|t| t == &shown_text)),
        "the wire's right group carried the indicator at the width while it showed"
    );
    for fr in &frames {
        assert!(
            !fr.grid.contains("file://") && !fr.wire_right.iter().any(|t| t.contains("file://")),
            "never the URI on either composition at {} ms",
            fr.at_ms
        );
    }
    let last_shown = shown.last().unwrap().at_ms;
    let after: Vec<&Frame> = frames
        .iter()
        .filter(|fr| fr.at_ms > last_shown + 200)
        .collect();
    assert!(
        !after.is_empty() && after.iter().all(|fr| activity_of(fr).is_none()),
        "gone once answered (last shown at {last_shown} ms)"
    );
    let renamed: String = eval(&f.s, "return pmacs.window.buffer():slice(0, 12)");
    assert!(
        renamed.contains("renamed"),
        "the rename's edit was applied when the answer came: {renamed:?}"
    );
}
