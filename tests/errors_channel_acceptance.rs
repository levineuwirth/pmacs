//! E5.1 --- `pmacs.error` exists, and every writer lands in one place.
//!
//! Before this phase `pmacs.error` was referenced by eighteen guarded
//! sites in `builtin/runtime` and defined nowhere in production, so each
//! guard was a report that never fired; and of the seven Rust-side
//! writers into `*errors*`, only `LuaHost::eval` and `run_hook` pushed
//! the record the status line reads, so a hook, package-load,
//! config-listener or statusline-provider failure left no status-line
//! trace and no unread mark. Now `crate::lua::report_error` is the one
//! writer behind all of them and `pmacs.error` is its Lua name.
//!
//! What is pinned here: the channel's four surfaces (the buffer, the
//! log, the status line while unread, the mode-line mark), the fact
//! that showing `*errors*` reads it, five of the seven Rust writers by
//! provocation, and the runtime sites that need no other suite's
//! fixture --- the five in `async.lua`, the three hook sites in
//! `syntax.lua`, and `fs.lua`'s watch callback. The sites that need a
//! fake server, a state directory or a PTY are witnessed in the suites
//! that already own those fixtures: `injection_acceptance` (the
//! injection cap), `lsp_multi_root_acceptance` (a raising root
//! resolver), `lsp_spawn_guidance_acceptance` (a server that does not
//! start), `m4_acceptance` (a file-watch scan failure),
//! `lsp_dispatch_seams_acceptance` (a raising notification
//! subscriber), `editops_acceptance` (trim-on-save),
//! `autosave_acceptance` (a failing sweep), `m9_5_acceptance` (an MCP
//! notification handler), and `lean4_server_acceptance` (the lean
//! fallback report).
//!
//! Bite: neuter `report_error`'s push onto the log and the status-line
//! and mode-line rows fail; neuter the buffer append and every
//! `*errors*` assertion fails; put any guard back and its row fails.

#![allow(clippy::cast_possible_truncation)]

use std::collections::HashMap;

use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;
#[path = "common/ready.rs"]
mod ready;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn editor() -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state
}

fn errors_text(state: &EditorState) -> String {
    state.lua_host.errors_buffer_text()
}

fn lua_str(s: &str) -> String {
    format!("{s:?}")
}

/// Paint one grid frame and return its cells.
fn paint(state: &EditorState, rows: u32, cols: u32) -> Vec<Cell> {
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    let _ = pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &HashMap::new(),
        &mut grid,
        CellSize::new(rows, cols),
    );
    cells
}

fn row_text(cells: &[Cell], cols: u32, row: u32) -> String {
    (0..cols)
        .map(|col| match cells[(row * cols + col) as usize].glyph {
            Glyph::Char(c) => c,
            _ => ' ',
        })
        .collect()
}

/// Tick the editor until `*errors*` carries `needle`, reporting the
/// buffer's text when it does not.
fn wait_errors_contain(state: &mut EditorState, needle: &str) {
    ready::tick_until(
        state,
        &format!("*errors* containing {needle:?}"),
        ready::DEADLINE,
        |s| {
            let text = errors_text(s);
            if text.contains(needle) {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(text)
            }
        },
    );
}

// ---------------------------------------------------------------------------
// The channel's surfaces
// ---------------------------------------------------------------------------

#[test]
fn pmacs_error_reaches_the_buffer_the_log_and_the_unread_count() {
    let state = editor();
    assert_eq!(state.lua_host.unread_errors(), 0);
    assert!(state.lua_host.errors().is_empty());
    exec(&state, "pmacs.error('the sky is falling')");
    let text = errors_text(&state);
    assert!(
        text.contains("] the sky is falling\n"),
        "the message lands in *errors* as one line: {text:?}"
    );
    assert!(
        text.starts_with('['),
        "the line is labeled with the reporting chunk: {text:?}"
    );
    let log = state.lua_host.errors();
    assert_eq!(log.len(), 1, "one record on the log the status line reads");
    assert_eq!(log[0].message, "the sky is falling");
    assert!(
        log[0].source.as_deref().is_some_and(|s| !s.is_empty()),
        "the record carries the label as its source: {:?}",
        log[0].source
    );
    assert_eq!(state.lua_host.unread_errors(), 1);
    let listed: i64 = eval(&state, "return #pmacs.error_log.list()");
    assert_eq!(listed, 1, "pmacs.error_log.list() sees the same log");
    let unread: i64 = eval(&state, "return pmacs.error_log.unread()");
    assert_eq!(unread, 1);
    // An explicit label replaces the chunk location.
    exec(&state, "pmacs.error('with a label', 'mypkg')");
    assert!(
        errors_text(&state).contains("[mypkg] with a label\n"),
        "{:?}",
        errors_text(&state)
    );
}

#[test]
fn an_unread_error_shows_on_the_status_line_and_the_mode_line_until_shown() {
    let state = editor();
    let quiet = paint(&state, 24, 80);
    assert!(
        !row_text(&quiet, 80, 22).contains('!'),
        "no mark before any error: {:?}",
        row_text(&quiet, 80, 22)
    );
    assert_eq!(row_text(&quiet, 80, 23).trim(), "");

    exec(&state, "pmacs.error('first'); pmacs.error('second')");
    let painted = paint(&state, 24, 80);
    let mode_line = row_text(&painted, 80, 22);
    let status = row_text(&painted, 80, 23);
    assert!(
        mode_line.contains("!2"),
        "the mode line carries the unread count: {mode_line:?}"
    );
    assert!(
        status.contains("lua: second"),
        "the status line shows the last error while it is unread: {status:?}"
    );
    // A keystroke clears `core.status`; the trace persists through the
    // fallback because the errors are still unread.
    state.core.borrow_mut().status.clear();
    let again = paint(&state, 24, 80);
    assert!(row_text(&again, 80, 23).contains("lua: second"));

    // Showing *errors* reads it: the mark and the trace go on the frame
    // the buffer appears in, and the records stay.
    exec(
        &state,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
          if pmacs.describe.buffer(id).name == "*errors*" then
            pmacs.window.switch_buffer(id)
          end
        end
        "#,
    );
    let shown = paint(&state, 24, 80);
    assert_eq!(state.lua_host.unread_errors(), 0, "shown means read");
    assert!(
        !row_text(&shown, 80, 22).contains("!2"),
        "the mark clears: {:?}",
        row_text(&shown, 80, 22)
    );
    assert_eq!(
        row_text(&shown, 80, 23).trim(),
        "",
        "the status-line trace clears: {:?}",
        row_text(&shown, 80, 23)
    );
    assert_eq!(state.lua_host.errors().len(), 2, "the records stay");
    let text = errors_text(&state);
    assert!(text.contains("first") && text.contains("second"));
    // A third error after reading is unread again.
    exec(&state, "pmacs.error('third')");
    assert_eq!(state.lua_host.unread_errors(), 1);
}

#[test]
fn pmacs_error_log_mark_read_clears_the_count_without_showing_the_buffer() {
    let state = editor();
    exec(&state, "pmacs.error('x')");
    assert_eq!(state.lua_host.unread_errors(), 1);
    exec(&state, "pmacs.error_log.mark_read()");
    assert_eq!(state.lua_host.unread_errors(), 0);
    let painted = paint(&state, 24, 80);
    assert!(!row_text(&painted, 80, 22).contains('!'));
    assert_eq!(row_text(&painted, 80, 23).trim(), "");
}

// ---------------------------------------------------------------------------
// The Rust-side writers, each provoked, each on the log (not only the buffer)
// ---------------------------------------------------------------------------

#[test]
fn a_chunk_error_is_one_record_on_the_log_and_unread() {
    let mut state = editor();
    let before = state.lua_host.errors().len();
    let _ = state
        .lua_host
        .eval(Some("bad_chunk"), "error('chunk boom')");
    let log = state.lua_host.errors();
    assert_eq!(log.len(), before + 1);
    assert!(log.last().unwrap().message.contains("chunk boom"));
    assert_eq!(log.last().unwrap().source.as_deref(), Some("bad_chunk"));
    assert!(errors_text(&state).contains("[bad_chunk] "));
    assert_eq!(state.lua_host.unread_errors(), 1);
}

#[test]
fn a_hook_callback_failure_counts_unread_and_reaches_the_status_line() {
    let state = editor();
    exec(
        &state,
        r#"
        pmacs.hook.define { name = "e5.test-hook", description = "a hook for the witness" }
        pmacs.hook.add("e5.test-hook", function() error("hook boom") end)
        pmacs.hook.run("e5.test-hook")
        "#,
    );
    let text = errors_text(&state);
    assert!(
        text.contains("[hook:e5.test-hook] callback at ") && text.contains("hook boom"),
        "the hook writer's line: {text:?}"
    );
    // Before E5.1 this writer reached the buffer alone: no record, no
    // status-line trace, no unread mark.
    assert_eq!(state.lua_host.unread_errors(), 1);
    assert!(
        state
            .lua_host
            .last_error()
            .unwrap()
            .message
            .contains("hook boom")
    );
    // Wide, because the status row is cut at the terminal's width and
    // the hook writer's line names the callback's source before the
    // cause.
    let painted = paint(&state, 24, 200);
    let status = row_text(&painted, 200, 23);
    assert!(
        status.starts_with("lua: callback at ") && status.contains("hook boom"),
        "the hook failure is on the status line: {status:?}"
    );
    assert!(row_text(&painted, 200, 22).contains("!1"));
}

#[test]
fn a_config_listener_failure_is_on_the_log() {
    let state = editor();
    exec(
        &state,
        r#"
        pmacs.config.define {
          name = "e5.witness", type = "boolean", default = false,
          description = "a setting for the witness",
        }
        pmacs.config.on_change("e5.witness", function() error("listener boom") end)
        pmacs.config.set("e5.witness", true)
        "#,
    );
    let text = errors_text(&state);
    assert!(
        text.contains("[config] on_change listener at ") && text.contains("listener boom"),
        "{text:?}"
    );
    assert_eq!(state.lua_host.unread_errors(), 1);
    assert!(
        state
            .lua_host
            .last_error()
            .unwrap()
            .message
            .contains("listener boom")
    );
}

#[test]
fn a_statusline_provider_failure_is_on_the_log() {
    let state = editor();
    exec(
        &state,
        r#"
        pmacs.statusline.register {
          name = "e5-raising", side = "left", priority = 1,
          fn = function() error("provider boom") end,
        }
        "#,
    );
    let _ = paint(&state, 24, 80);
    let text = errors_text(&state);
    assert!(
        text.contains("[statusline:e5-raising] provider registered at ")
            && text.contains("provider boom"),
        "{text:?}"
    );
    assert!(state.lua_host.unread_errors() >= 1);
    assert!(
        state
            .lua_host
            .errors()
            .iter()
            .any(|r| r.message.contains("provider boom")),
        "the record is on the log the status line reads"
    );
}

#[test]
fn a_buffer_on_removed_failure_is_on_the_log() {
    let state = editor();
    exec(
        &state,
        r#"
        local b = pmacs.buffer.create("doomed")
        pmacs.buffer.on_removed(b, function() error("removed boom") end)
        pmacs.buffer.kill(b)
        "#,
    );
    let text = errors_text(&state);
    assert!(
        text.contains("[buffer.on_removed] callback at ") && text.contains("removed boom"),
        "{text:?}"
    );
    assert_eq!(state.lua_host.unread_errors(), 1);
}

#[test]
fn the_buffer_side_append_error_record_writer_is_on_the_log() {
    let state = editor();
    exec(
        &state,
        "pmacs.buffer._append_error_record('resource', 'plan refused')",
    );
    assert!(errors_text(&state).contains("[resource] plan refused\n"));
    assert_eq!(state.lua_host.unread_errors(), 1);
    assert_eq!(
        state.lua_host.last_error().unwrap().source.as_deref(),
        Some("resource")
    );
}

// ---------------------------------------------------------------------------
// The runtime sites that needed the channel: async.lua
// ---------------------------------------------------------------------------

#[test]
fn async_site_a_coroutine_that_raises_is_reported() {
    let state = editor();
    exec(
        &state,
        "pmacs.async(function() error('coroutine kaboom') end)",
    );
    let text = errors_text(&state);
    assert!(
        text.contains("pmacs.async: coroutine raised:") && text.contains("coroutine kaboom"),
        "{text:?}"
    );
}

#[test]
fn async_site_a_non_handle_yield_is_reported() {
    let state = editor();
    exec(
        &state,
        "pmacs.async(function() coroutine.yield('not a handle') end)",
    );
    let text = errors_text(&state);
    assert!(
        text.contains("coroutine yielded a non-Handle value (string)"),
        "{text:?}"
    );
}

#[test]
fn async_site_a_failing_on_complete_callback_is_reported() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.compute_sum(5):on_complete(function() error('complete boom') end)",
    );
    wait_errors_contain(&mut state, "on_complete callback failed:");
    assert!(errors_text(&state).contains("complete boom"));
}

#[test]
fn async_site_a_failing_on_batch_callback_is_reported() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.emit_n(8, { max_batch = 4 }):on_batch(function() error('batch boom') end)",
    );
    wait_errors_contain(&mut state, "on_batch callback failed:");
    assert!(errors_text(&state).contains("batch boom"));
}

#[test]
fn async_site_a_failing_on_close_callback_is_reported() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.emit_n(3, { max_batch = 8 }):on_close(function() error('close boom') end)",
    );
    wait_errors_contain(&mut state, "on_close callback failed:");
    assert!(errors_text(&state).contains("close boom"));
}

// ---------------------------------------------------------------------------
// syntax.lua's three hook sites, and fs.lua's watch callback
// ---------------------------------------------------------------------------

fn rust_file(name: &str) -> std::path::PathBuf {
    let dir = iso::roots()
        .state_root()
        .expect("isolated state root")
        .join("e5-errors-channel");
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join(name);
    std::fs::write(&path, "fn main() {}\n").expect("write fixture");
    path
}

#[test]
fn syntax_sites_after_load_after_switch_and_after_edit_failures_are_reported() {
    let state = editor();
    // after-load, real: attaching succeeds and records the language,
    // which after-edit's site needs below.
    let first = rust_file("one.rs");
    exec(
        &state,
        &format!(
            "pmacs.buffer.find_or_open({})",
            lua_str(first.to_str().unwrap())
        ),
    );
    assert!(
        !errors_text(&state).contains("syntax."),
        "a clean attach reports nothing: {:?}",
        errors_text(&state)
    );

    // after-edit: the reparse's first probe of the buffer's parse view
    // raises (`reparse_active_buffer_after_edit` asks `_has_view` before
    // anything else).
    exec(
        &state,
        r#"
        _G.__has_view = pmacs.parse._has_view
        pmacs.parse._has_view = function() error("view boom") end
        pmacs.hook.run("buffer.after-edit")
        pmacs.parse._has_view = _G.__has_view
        "#,
    );
    let text = errors_text(&state);
    assert!(
        text.contains("syntax.after-edit:") && text.contains("view boom"),
        "{text:?}"
    );

    // after-switch and after-load: the parse dispatch raises
    // (`attach_for_active_buffer` dispatches once the language resolves,
    // on every attach and not only the first).
    exec(
        &state,
        r#"
        _G.__dispatch = pmacs.parse._dispatch
        pmacs.parse._dispatch = function() error("attach boom") end
        pmacs.hook.run("buffer.after-switch")
        "#,
    );
    let text = errors_text(&state);
    assert!(
        text.contains("syntax.after-switch:") && text.contains("attach boom"),
        "{text:?}"
    );
    let second = rust_file("two.rs");
    exec(
        &state,
        &format!(
            "pmacs.buffer.find_or_open({})",
            lua_str(second.to_str().unwrap())
        ),
    );
    exec(&state, "pmacs.parse._dispatch = _G.__dispatch");
    let text = errors_text(&state);
    assert!(
        text.contains("syntax.after-load:") && text.contains("attach boom"),
        "{text:?}"
    );
    assert!(state.lua_host.unread_errors() >= 3);
}

#[test]
fn fs_site_a_raising_watch_callback_is_reported() {
    let mut state = editor();
    let dir = iso::roots()
        .state_root()
        .expect("isolated state root")
        .join("e5-errors-channel-watch");
    std::fs::create_dir_all(&dir).expect("watch dir");
    let path = dir.join("watched.txt");
    std::fs::write(&path, "v").expect("seed");
    exec(
        &state,
        &format!(
            r#"pmacs.fs.watch({}, function() error("watch boom") end, {{ interval_ms = 5 }})"#,
            lua_str(path.to_str().unwrap())
        ),
    );
    // The watcher's baseline snapshot lands asynchronously; rewrite
    // with distinct-length content on every probe so whichever
    // snapshot it captured, a later write differs from it (the
    // `m8_1_acceptance` shape).
    let mut n = 1usize;
    ready::tick_until(
        &mut state,
        "the watch callback's failure in *errors*",
        ready::DEADLINE,
        |s| {
            n += 1;
            std::fs::write(&path, "v".repeat(n)).expect("rewrite");
            let text = errors_text(s);
            if text.contains("pmacs.fs.watch callback failed:") && text.contains("watch boom") {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(text)
            }
        },
    );
}

#[test]
fn review_pmacs_error_emits_the_message_on_the_semantic_status_line() {
    use pmacs::protocol::{ByteRange, InstanceMessage};
    use pmacs::semantic_render::SemanticRenderState;
    let state = EditorState::new_with_roots(&iso::roots());
    state
        .lua_host
        .lua()
        .load("pmacs.lsp.config = {}")
        .exec()
        .unwrap();
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    let bid = state.core.borrow().active_buffer_id();
    let mut render = SemanticRenderState::for_peer(FrontendId::LOCAL, 25);
    render.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);
    let _ = render.render_frame(&state);
    state
        .lua_host
        .lua()
        .load("pmacs.error('review background failure')")
        .exec()
        .unwrap();
    assert_eq!(state.lua_host.unread_errors(), 1);
    assert!(
        state
            .lua_host
            .errors_buffer_text()
            .contains("review background failure")
    );
    let frame = render.render_frame(&state);
    eprintln!("FRAME: {frame:?}");
    assert!(frame.iter().any(|m| matches!(m, InstanceMessage::StatusFacts{message:Some(s),..} if s.contains("review background failure"))), "GPU must receive the same error text as the TUI: {frame:?}");
}

fn review3_paint(state: &EditorState, rows: u32) {
    use pmacs::cell::{CellGrid, CellSize};
    let mut cells = vec![Cell::default(); usize::try_from(rows * 100).expect("small grid")];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: 100,
        size: CellSize::new(rows, 100),
    };
    let _ = pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        CellSize::new(rows, 100),
    );
}

fn review3_hidden_errors_panel() -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    exec(
        &state,
        r"
        pmacs.error('seed read by the visible panel')
        for _, b in ipairs(pmacs.buffer.list()) do
            if b:name() == '*errors*' then
                pmacs.window.display(b, { side = 'bottom', height = 8, select = false })
            end
        end
    ",
    );
    review3_paint(&state, 40);
    assert_eq!(state.lua_host.unread_errors(), 0);
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(4, 100));
    assert!(state.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    exec(&state, "pmacs.error('new error nobody has seen')");
    assert_eq!(state.lua_host.unread_errors(), 1);
    state
}

#[test]
fn review3_a_hidden_errors_panel_does_not_read_errors_on_the_grid() {
    let state = review3_hidden_errors_panel();
    review3_paint(&state, 4);
    assert_eq!(
        state.lua_host.unread_errors(),
        1,
        "a hidden panel cannot acknowledge a new error"
    );
}

#[test]
fn review3_a_hidden_errors_panel_does_not_read_errors_on_the_wire() {
    let state = review3_hidden_errors_panel();
    let bid = state.core.borrow().active_buffer_id();
    let mut renderer = pmacs::semantic_render::SemanticRenderState::for_peer(FrontendId::LOCAL, 25);
    renderer.set_viewport(bid, pmacs::protocol::ByteRange { start: 0, end: 64 }, 0);
    let frame = renderer.render_frame(&state);
    assert_eq!(
        state.lua_host.unread_errors(),
        1,
        "a hidden panel cannot acknowledge a new error; frame={frame:?}"
    );
}

#[test]
fn review3_errors_shown_only_on_another_frontend_stay_unread_here() {
    use pmacs::protocol::FrontendId as Fid;
    use pmacs::text_view::TextView;
    use pmacs::window::{FrontendView, Layout, Window, WindowId};
    let other = Fid(2);
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    exec(&state, "pmacs.error('seen elsewhere, not here')");
    let errors_id = state
        .lua_host
        .errors_buffer_id()
        .expect("errors buffer exists");
    // An unrelated frontend whose own window presents the errors
    // buffer: its frames may acknowledge, but LOCAL's must not.
    {
        let mut core = state.core.borrow_mut();
        let text_view = {
            let reg = core.registry.borrow();
            TextView::new(reg.get(errors_id).expect("errors buffer"))
        };
        let id = WindowId::next();
        core.windows
            .insert(id, Window::new(id, errors_id, text_view));
        core.register_frontend_view(
            other,
            FrontendView {
                layout: Layout::single(id),
                active: id,
                fold_projection: true,
                panel_capable: false,
                frame_geometry: None,
                panel_hidden: false,
                document_viewport: None,
            },
        );
    }
    review3_paint(&state, 40);
    assert_eq!(
        state.lua_host.unread_errors(),
        1,
        "a window outside this frontend's layout is not presentation here"
    );
    // And the other frontend's own frame does acknowledge.
    {
        use pmacs::cell::{CellGrid, CellSize};
        let mut cells = vec![Cell::default(); 40 * 100];
        let mut grid = CellGrid {
            cells: &mut cells,
            stride: 100,
            size: CellSize::new(40, 100),
        };
        let _ = pmacs::editor::paint_frame(
            &state,
            other,
            &std::collections::HashMap::new(),
            &mut grid,
            CellSize::new(40, 100),
        );
    }
    assert_eq!(state.lua_host.unread_errors(), 0);
}

fn error_panel_becomes_visible(semantic: bool) {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    exec(
        &state,
        r"
        pmacs.error('first visible error')
        for _, b in ipairs(pmacs.buffer.list()) do
            if b:name() == '*errors*' then
                pmacs.window.display(b, { side = 'bottom', height = 8, select = false })
            end
        end
    ",
    );
    let mut renderer = pmacs::semantic_render::SemanticRenderState::for_peer(FrontendId::LOCAL, 25);
    renderer.set_viewport(
        state.core.borrow().active_buffer_id(),
        pmacs::protocol::ByteRange { start: 0, end: 64 },
        0,
    );
    let mut paint = |rows| {
        if semantic {
            let _ = renderer.render_frame(&state);
        } else {
            review3_paint(&state, rows);
        }
    };
    paint(40);
    assert_eq!(
        state.lua_host.unread_errors(),
        0,
        "initial visible panel reads the seed"
    );
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(4, 100));
    assert!(state.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    exec(&state, "pmacs.error('arrived while hidden')");
    paint(4);
    assert_eq!(
        state.lua_host.unread_errors(),
        1,
        "the hidden frame keeps the new error unread"
    );
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    assert!(!state.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    paint(40);
    assert_eq!(
        state.lua_host.unread_errors(),
        0,
        "presenting the panel again reads the pending error"
    );
    assert_eq!(state.lua_host.unread_error_status_message(), None);
    assert!(
        state
            .lua_host
            .errors_buffer_text()
            .contains("arrived while hidden")
    );
}

#[test]
fn review4_unhiding_the_errors_panel_reads_pending_errors_on_grid() {
    error_panel_becomes_visible(false);
}

#[test]
fn review4_unhiding_the_errors_panel_reads_pending_errors_on_wire() {
    error_panel_becomes_visible(true);
}

fn review4_errors_in_other_document_split() -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    let scratch = state.core.borrow().active_buffer_id();
    exec(
        &state,
        r"
        REVIEW_DOC = pmacs.window.buffer()
        pmacs.error('seed')
        for _, b in ipairs(pmacs.buffer.list()) do
            if b:name() == '*errors*' then
                pmacs.window.switch_buffer(b)
                break
            end
        end
        pmacs.window.split_vertical()
        pmacs.window.switch_buffer(REVIEW_DOC)
        pmacs.error_log.mark_read()
    ",
    );
    let errors = state
        .lua_host
        .errors_buffer_id()
        .expect("seed created errors buffer");
    {
        let core = state.core.borrow();
        assert_eq!(core.active_buffer_id(), scratch);
        let ids = core
            .views
            .get(&FrontendId::LOCAL)
            .expect("local view")
            .layout
            .iter_ids();
        assert_eq!(ids.len(), 2);
        assert!(
            ids.iter().any(|id| core
                .windows
                .get(id)
                .is_some_and(|w| w.buffer_id == errors && !w.is_side())),
            "the other split holds errors as an ordinary document"
        );
    }
    state
}

#[test]
fn review4_grid_reads_errors_in_an_inactive_document_split() {
    let state = review4_errors_in_other_document_split();
    exec(&state, "pmacs.error('visible in the other grid split')");
    let cells = paint(&state, 40, 100);
    assert_eq!(state.lua_host.unread_errors(), 0);
    // Read across rows: under word wrap (D35) the entry breaks at a
    // space, here between `visible` and `in`, where character wrap put
    // the whole phrase on one row by the chance of the trace's length.
    // The other split is blank beside those rows, so the grid's text
    // with its runs of blanks collapsed is the entry's.
    let text = (0..40)
        .map(|row| row_text(&cells, 100, row))
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        text.contains("visible in the other grid split"),
        "the entry is on the grid: {text:?}"
    );
}

#[test]
fn review4_wire_does_not_read_errors_in_an_unprojected_document_split() {
    use pmacs::protocol::{ByteRange, InstanceMessage};
    use pmacs::semantic_render::SemanticRenderState;
    let state = review4_errors_in_other_document_split();
    let scratch = state.core.borrow().active_buffer_id();
    let errors = state.lua_host.errors_buffer_id().expect("errors buffer");
    let mut renderer = SemanticRenderState::for_peer(FrontendId::LOCAL, 25);
    renderer.set_viewport(scratch, ByteRange { start: 0, end: 64 }, 0);
    let initial = renderer.render_frame(&state);
    assert!(
        initial.iter().any(
            |m| matches!(m, InstanceMessage::StatusFacts { buffer_id, .. } if *buffer_id == scratch)
        ),
        "the semantic document projection is the scratch buffer"
    );
    exec(
        &state,
        "pmacs.error('new error in a document split the GPU does not project')",
    );
    assert_eq!(state.lua_host.unread_errors(), 1);
    let frame = renderer.render_frame(&state);
    assert_eq!(
        state.lua_host.unread_errors(),
        1,
        "layout membership cannot acknowledge a document the semantic frontend is not showing: {frame:?}"
    );
    assert!(frame.iter().any(|m| matches!(m, InstanceMessage::StatusFacts { message: Some(message), .. } if message.contains("new error in a document split"))));

    // Bringing the errors document into the projection acknowledges it.
    exec(&state, "pmacs.window.focus_next()");
    assert_eq!(state.core.borrow().active_buffer_id(), errors);
    renderer.set_viewport(errors, ByteRange { start: 0, end: 256 }, 0);
    let frame = renderer.render_frame(&state);
    assert_eq!(state.lua_host.unread_errors(), 0);
    assert!(frame.iter().any(|m| matches!(m, InstanceMessage::StatusFacts { buffer_id, message: None, .. } if *buffer_id == errors)));
}
