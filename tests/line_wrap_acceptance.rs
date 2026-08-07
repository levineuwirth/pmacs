//! Line-wrap acceptance (long lines, `docs/long-lines-framing.md`).
//!
//! `ui.line-wrap` is **buffer-local** (Q#LL2), and buffer-local is the
//! whole point rather than a nicety: prose wants wrapping and a log file
//! usually does not, in the same session.
//!
//! Which makes the toggle command's failure mode specific and invisible
//! in any single-buffer test. `pmacs.config.get(name)` reads the global
//! chain and `pmacs.config.set(name, ...)` writes the global layer, so a
//! toggle built from that pair reports and flips the **global** value.
//! In a buffer pinned to `truncate` it would leave that buffer exactly
//! as it was while silently changing every buffer that had not been
//! pinned — the opposite of what the user asked for, in both directions
//! at once.
//!
//! So these tests use a second buffer, and a buffer pinned against
//! the global layer, to make both halves of that failure visible.

use std::path::Path;

use pmacs::bootstrap::BootstrapRoots;
use pmacs::editor::EditorState;

fn session(name: &str) -> EditorState {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("line-wrap")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let roots = BootstrapRoots::isolated_under(&base);
    for (_, dir) in roots.child_env() {
        std::fs::create_dir_all(&dir).expect("create controlled root");
    }
    let state = EditorState::new_with_roots(&roots);
    state.install_state_dirs();
    state
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

/// The resolved mode for the buffer currently in the window.
fn mode_here(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.config.get('ui.line-wrap', pmacs.window.buffer())",
    )
}

/// The global layer's value, which is *not* what a buffer necessarily
/// resolves to.
fn mode_global(s: &EditorState) -> String {
    eval(s, "return pmacs.config.get('ui.line-wrap')")
}

/// Put `text` in the window's current buffer by typing it, which is the
/// only path Lua exposes and also the one a user takes.
fn fill_active(s: &EditorState, text: &str) {
    for ch in text.chars() {
        exec(
            s,
            &format!("pmacs.editor.insert_char_over_region({})", ch as u32),
        );
    }
}

/// Render one frame and return the text of the first `rows` grid rows,
/// reconstructed from the emitted `CellDelta` spans.
///
/// Two reasons for going through `RenderState` and the wire rather than
/// calling `TextView::render` with a hand-built viewport. The defect
/// this guards against lived in the *driver*, between the resolved mode
/// and the viewport it built — a test that constructed its own viewport
/// would have passed against it. And the spans are what the TUI
/// actually consumes, so this asserts on the bytes that reach a screen.
fn render_rows(s: &EditorState, rows: u32, cols: u32) -> Vec<String> {
    use std::collections::HashMap;

    let size = pmacs::cell::CellSize::new(rows, cols);
    let mut rs = pmacs::instance_render::RenderState::new(size);
    let msgs = rs.render_frame(s, pmacs::protocol::FrontendId::LOCAL, &HashMap::new(), &[]);

    let mut grid = vec![vec![' '; cols as usize]; rows as usize];
    for msg in &msgs {
        if let pmacs_protocol::InstanceMessage::CellDelta { spans, .. } = msg {
            for span in spans {
                for (i, cell) in span.cells.iter().enumerate() {
                    let r = span.start.row as usize;
                    let c = span.start.col as usize + i;
                    if r < rows as usize
                        && c < cols as usize
                        && let pmacs::cell::Glyph::Char(ch) = cell.glyph
                    {
                        grid[r][c] = ch;
                    }
                }
            }
        }
    }
    grid.into_iter().map(|r| r.into_iter().collect()).collect()
}

#[test]
fn the_default_is_wrap() {
    let s = session("default");
    assert_eq!(mode_global(&s), "wrap");
    assert_eq!(
        mode_here(&s),
        "wrap",
        "with no buffer-local override, a buffer resolves to the global default"
    );
}

#[test]
fn only_wrap_and_truncate_are_accepted() {
    let s = session("enum");
    let err: bool = eval(
        &s,
        "local ok = pcall(pmacs.config.set, 'ui.line-wrap', 'sideways'); return not ok",
    );
    assert!(
        err,
        "a closed choice set makes an unknown mode impossible rather than handled"
    );
}

/// The finding this suite exists for: the toggle must move the buffer
/// it is invoked in, and **only** that buffer.
///
/// There is no Lua buffer-switch, so "the other buffer" is a second
/// buffer that exists but is not shown — which is the case that matters
/// anyway: a global write reaches every buffer without an override of
/// its own, shown or not.
#[test]
fn the_toggle_moves_this_buffer_and_leaves_the_other_alone() {
    let s = session("two_buffers");
    exec(&s, "OTHER = pmacs.buffer.create('other')");
    assert_eq!(
        eval::<String>(&s, "return pmacs.config.get('ui.line-wrap', OTHER)"),
        "wrap",
        "precondition: the second buffer starts at the default"
    );

    exec(&s, "pmacs.command.invoke('ui.toggle-line-wrap')");
    assert_eq!(mode_here(&s), "truncate", "the invoking buffer moved");

    assert_eq!(
        eval::<String>(&s, "return pmacs.config.get('ui.line-wrap', OTHER)"),
        "wrap",
        "toggling in one buffer must not change another"
    );
    assert_eq!(
        mode_global(&s),
        "wrap",
        "a buffer-local toggle must not write the global layer — doing so \
         would change every buffer with no override of its own"
    );
}

/// A buffer pinned to `truncate` must toggle back to `wrap`, reading its
/// own value rather than the global one.
///
/// This is the half a global-read toggle gets wrong even if its write
/// were harmless: it would see `wrap` globally, decide the next mode is
/// `truncate`, and leave the pinned buffer exactly as it was.
#[test]
fn a_pinned_buffer_toggles_from_its_own_value() {
    let s = session("pinned");
    exec(
        &s,
        "pmacs.config.set_local(pmacs.window.buffer(), 'ui.line-wrap', 'truncate')",
    );
    assert_eq!(mode_here(&s), "truncate");
    assert_eq!(mode_global(&s), "wrap", "precondition: the layers differ");

    exec(&s, "pmacs.command.invoke('ui.toggle-line-wrap')");
    assert_eq!(
        mode_here(&s),
        "wrap",
        "the toggle read this buffer's value, not the global one"
    );
}

/// The default must reach the **rendered cells**, not merely the
/// resolved value.
///
/// This is the gap review found: the frame resolved `ui.line-wrap`,
/// recorded it on the window, and fed it to the coordinate mapping and
/// the scroll indicator — while the viewport handed the renderer a
/// hard-coded `Truncate`. Every "is the mode right?" assertion passed
/// and the text was still clipped. So this test reads the grid.
#[test]
fn the_default_actually_wraps_the_painted_text() {
    let s = session("rendered_default");
    // Wider than the viewport below, and distinctive.
    fill_active(&s, "ABCDEFGHIJKLMNOP");
    let rows = render_rows(&s, 4, 4);
    assert_eq!(rows[0], "ABCD");
    assert_eq!(
        rows[1], "EFGH",
        "the default is `wrap`, so the remainder continues on the next row \
         — a hard-coded Truncate in the viewport leaves this blank"
    );
}

/// And `truncate` still clips, so the witness above is discriminating
/// rather than merely true.
#[test]
fn truncate_clips_the_painted_text() {
    let s = session("rendered_truncate");
    fill_active(&s, "ABCDEFGHIJKLMNOP");
    exec(
        &s,
        "pmacs.config.set_local(pmacs.window.buffer(), 'ui.line-wrap', 'truncate')",
    );
    let rows = render_rows(&s, 4, 4);
    assert_eq!(rows[0], "ABCD");
    assert_eq!(rows[1], "    ", "truncate keeps one row per source line");
}
