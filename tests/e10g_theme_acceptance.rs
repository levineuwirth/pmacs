//! E10g: theme machinery.
//!
//! Named themes in the runtime (E10g.1), the buffer's own face and every
//! painted surface on a face (E10g.2), the `ui.theme` setting and the
//! user's layer (E10g.3), the picker (E10g.4), color depth (E10g.5) and
//! the painted chrome's legibility at 16 colors (E10g.6). Each row is
//! named for the task it witnesses.

use std::collections::HashMap;

use pmacs::cell::{Cell, CellSize, Color, Style};
use pmacs::color_depth::{ColorDepth, Component, authored_index, nearest_16};
use pmacs::editor::EditorState;
use pmacs::highlight::Theme;
use pmacs::instance_render::RenderState;
use pmacs::protocol::{FrontendId, InstanceMessage};

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

mod common;
use common::iso;

fn fresh() -> EditorState {
    EditorState::new_with_roots(&iso::roots())
}

/// The whole screen of one frame painted at `depth`, as a row-major
/// grid: the first frame diffs against a blank grid, so a cell no span
/// names is a default cell.
fn frame_at(s: &EditorState, size: CellSize, depth: ColorDepth) -> Vec<Vec<Cell>> {
    frame_at_with(s, size, depth, &HashMap::new())
}

/// [`frame_at`] with the terminal panes' snapshots a frontend paints.
fn frame_at_with(
    s: &EditorState,
    size: CellSize,
    depth: ColorDepth,
    terminals: &HashMap<pmacs::window::WindowId, pmacs::terminal::session::TerminalSnapshot>,
) -> Vec<Vec<Cell>> {
    let mut r = RenderState::new(size);
    r.set_color_depth(depth);
    let mut grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
    for message in r.render_frame(s, FrontendId::LOCAL, terminals, &[]) {
        if let InstanceMessage::CellDelta { spans, .. } = message {
            for span in spans {
                for (i, cell) in span.cells.into_iter().enumerate() {
                    grid[span.start.row as usize][span.start.col as usize + i] = cell;
                }
            }
        }
    }
    grid
}

/// The column where `text` starts on `row`, read from the glyphs.
fn find_text(grid: &[Vec<Cell>], row: usize, text: &str) -> usize {
    let glyphs: Vec<char> = grid[row]
        .iter()
        .map(|c| match c.glyph {
            pmacs::cell::Glyph::Char(ch) => ch,
            _ => '\0',
        })
        .collect();
    let want: Vec<char> = text.chars().collect();
    glyphs
        .windows(want.len())
        .position(|w| w == want.as_slice())
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {}", String::from_iter(&glyphs)))
}

fn colors_of(style: &Style) -> [Color; 3] {
    [style.fg, style.bg, style.underline_color]
}

// ---------------------------------------------------------------------------
// E10g.1 — named themes
// ---------------------------------------------------------------------------

/// `default-dark`, defined by the runtime file, equals
/// `Theme::default_dark()` face by face: every name either side has,
/// the same style on both. Loading it on a fresh editor gives the same
/// map, so today's look is the named theme's.
#[test]
fn e10g1_default_dark_equals_the_rust_constructor_face_by_face() {
    let s = fresh();
    let constructor = Theme::default_dark();
    let named = s
        .syntax_registry
        .themes()
        .get("default-dark")
        .cloned()
        .expect("the runtime defines default-dark");
    let defined: HashMap<String, Style> = named
        .entries
        .iter()
        .map(|e| (e.name.clone(), e.style))
        .collect();
    let mut names: Vec<&String> = constructor
        .by_capture
        .keys()
        .chain(defined.keys())
        .collect();
    names.sort_unstable();
    names.dedup();
    assert!(names.len() > 30, "the walk covers the whole table");
    for name in &names {
        assert_eq!(
            defined.get(*name),
            constructor.by_capture.get(*name),
            "face {name:?}: the named theme and the constructor differ"
        );
    }
    assert!(
        named.entries.iter().all(|e| e.fallbacks.is_empty()),
        "every default-dark color is an index below 16, its own fallback"
    );

    exec(&s, r#"pmacs.theme.load("default-dark")"#);
    let theme = s.syntax_registry.theme();
    let th = theme.lock().unwrap();
    assert_eq!(
        th.by_capture, constructor.by_capture,
        "a load is today's map"
    );
    assert_eq!(th.default_style, constructor.default_style);
    drop(th);
    assert_eq!(
        eval::<Option<String>>(&s, "return pmacs.theme.loaded()").as_deref(),
        Some("default-dark")
    );
}

/// The faces of `theme` that lack a 256- or 16-color fallback, as
/// `(face, component, depth)`: authored, or an index already in range.
fn missing_fallbacks(s: &EditorState, theme: &str) -> Vec<String> {
    let named = s
        .syntax_registry
        .themes()
        .get(theme)
        .cloned()
        .unwrap_or_else(|| panic!("{theme} is defined"));
    let mut missing = Vec::new();
    for entry in &named.entries {
        for component in Component::ALL {
            let main = component.of(&entry.style);
            if main == Color::Default {
                continue;
            }
            for depth in [ColorDepth::TwoFiftySix, ColorDepth::Sixteen] {
                if authored_index(main, &entry.fallbacks, component, depth).is_none() {
                    missing.push(format!(
                        "{theme}: {} {} at {depth:?}",
                        entry.name,
                        component.name()
                    ));
                }
            }
        }
    }
    missing
}

/// Every shipped theme, the names a fresh editor defines before any
/// user config, has both fallbacks for every color of every face. The
/// planted theme is the control: an RGB face without a `fallback` is
/// reported at both depths, so the walk is not vacuous over a theme
/// whose colors are all indices.
#[test]
fn e10g1_every_shipped_theme_finds_both_fallbacks_for_every_face() {
    let s = fresh();
    let shipped: Vec<String> = eval(&s, "return pmacs.theme.names()");
    assert!(
        shipped.iter().any(|n| n == "default-dark"),
        "shipped: {shipped:?}"
    );
    for theme in &shipped {
        let missing = missing_fallbacks(&s, theme);
        assert!(missing.is_empty(), "missing fallbacks: {missing:#?}");
    }

    exec(
        &s,
        r#"pmacs.theme.define("planted", { faces = {
              ["ui.modeline"] = { fg = {250, 250, 250}, bg = 24,
                                  fallback = { [256] = { fg = 255 } } },
           } })"#,
    );
    assert_eq!(
        missing_fallbacks(&s, "planted"),
        vec![
            "planted: ui.modeline fg at Sixteen",
            "planted: ui.modeline bg at Sixteen",
        ],
        "an RGB fg authored only at 256, and an index past 15, each fail at 16"
    );
}

/// A theme a user defines with no fallbacks loads, and a frame painted
/// at 16 colors carries only indices below 16: each RGB its nearest of
/// the sixteen. The control is the same frame at truecolor, carrying the
/// RGB the theme wrote.
#[test]
fn e10g1_a_user_theme_without_fallbacks_loads_and_renders_at_16_colors() {
    let s = fresh();
    exec(
        &s,
        r#"pmacs.theme.define("mine", { faces = {
              ["ui.modeline"] = { fg = {250, 250, 250}, bg = {0, 0, 200} },
              ["ui.statusline"] = { fg = {200, 60, 60} },
              keyword = { fg = {10, 200, 120}, bold = true },
           } })
           pmacs.theme.load("mine")"#,
    );
    let names: Vec<String> = eval(&s, "return pmacs.theme.names()");
    assert!(names.iter().any(|n| n == "mine"), "{names:?}");

    let size = CellSize::new(10, 40);
    let modeline_row = 8;
    let at16 = frame_at(&s, size, ColorDepth::Sixteen);
    for (row, cells) in at16.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            for color in colors_of(&cell.style) {
                assert!(
                    matches!(color, Color::Default | Color::Indexed(0..=15)),
                    "({row},{col}) carries {color:?} at 16 colors"
                );
            }
        }
    }
    let cell = &at16[modeline_row][0];
    assert_eq!(cell.style.fg, Color::Indexed(nearest_16((250, 250, 250))));
    assert_eq!(cell.style.bg, Color::Indexed(nearest_16((0, 0, 200))));
    assert_eq!(cell.style.bg, Color::Indexed(4), "blue, by the palette");

    let truecolor = frame_at(&s, size, ColorDepth::TrueColor);
    assert_eq!(
        truecolor[modeline_row][0].style.fg,
        Color::Rgb(250, 250, 250)
    );
    assert_eq!(truecolor[modeline_row][0].style.bg, Color::Rgb(0, 0, 200));
}

/// `load` refuses a name no theme has, and leaves the theme alone.
#[test]
fn e10g1_loading_an_undefined_name_is_refused_and_changes_nothing() {
    let s = fresh();
    let before = s.syntax_registry.theme().lock().unwrap().by_capture.clone();
    let err: String = eval(
        &s,
        r#"local ok, err = pcall(pmacs.theme.load, "no-such-theme")
           assert(not ok)
           return tostring(err)"#,
    );
    assert!(err.contains("no theme named \"no-such-theme\""), "{err}");
    assert_eq!(s.syntax_registry.theme().lock().unwrap().by_capture, before);
}

// ---------------------------------------------------------------------------
// E10g.2 — the buffer and every painted surface (the grid frontend)
// ---------------------------------------------------------------------------

const FACE_FG: Color = Color::Rgb(200, 200, 190);
const FACE_BG: Color = Color::Rgb(18, 18, 18);

fn style(fg: Color, bg: Color) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

/// The buffer face paints every cell the frame would have left at the
/// terminal's default: the text, past the end of a line, below the last
/// line, and the reverse-video bands (reversed from the face's colors).
/// Unset, those cells stay at the default, today's frame.
#[test]
fn e10g2_the_grid_paints_the_buffer_face_in_every_cell() {
    let s = fresh();
    exec(&s, r#"pmacs.window.buffer():insert(0, "hello")"#);
    let size = CellSize::new(10, 40);

    let unset = frame_at(&s, size, ColorDepth::TrueColor);
    let h = find_text(&unset, 0, "hello");
    assert_eq!(unset[0][h].style.fg, Color::Default, "unset: today's text");
    assert_eq!(
        unset[0][30].style.bg,
        Color::Default,
        "unset: past the line"
    );
    assert_eq!(
        unset[5][h].style.bg,
        Color::Default,
        "unset: below the last"
    );

    exec(
        &s,
        r#"pmacs.theme.merge { ["ui.default"] = { fg = {200, 200, 190}, bg = {18, 18, 18} } }"#,
    );
    let set = frame_at(&s, size, ColorDepth::TrueColor);
    assert_eq!(set[0][h].style, style(FACE_FG, FACE_BG), "the text");
    assert_eq!(set[0][30].style, style(FACE_FG, FACE_BG), "past the line");
    assert_eq!(set[5][h].style, style(FACE_FG, FACE_BG), "below the last");
    assert_eq!(set[0][0].style.bg, FACE_BG, "the gutter's background");
    let modeline = &set[8][0].style;
    assert!(modeline.reverse, "the modeline keeps its reverse video");
    assert_eq!((modeline.fg, modeline.bg), (FACE_FG, FACE_BG));
    for (row, cells) in set.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            assert!(
                cell.style.fg != Color::Default && cell.style.bg != Color::Default,
                "({row},{col}) left at the terminal default: {:?}",
                cell.style
            );
        }
    }
}

/// The buffer face is looked up by its own name: a theme that colors
/// every chrome surface through the bare `ui` catch-all does not paint
/// the text area, which is not chrome.
#[test]
fn e10g2_the_bare_ui_catch_all_does_not_reach_the_buffer() {
    let s = fresh();
    exec(
        &s,
        "pmacs.theme.merge { ui = { fg = {250, 0, 0}, bg = {0, 0, 90} } }",
    );
    let frame = frame_at(&s, CellSize::new(10, 40), ColorDepth::TrueColor);
    assert_eq!(
        frame[5][0].style.bg,
        Color::Default,
        "the buffer is not chrome"
    );
    assert_eq!(
        frame[8][0].style.bg,
        Color::Rgb(0, 0, 90),
        "the control: the catch-all does reach the modeline"
    );
}

/// The context menu's rows, selected row and separator take `ui.menu`,
/// `ui.menu.selected` and `ui.menu.separator`; unset, today's constants.
#[test]
fn e10g2_the_grid_menu_takes_the_menu_faces() {
    use pmacs::menu::MenuRow;
    let s = fresh();
    s.core.borrow_mut().menu_open(
        vec![
            MenuRow::Item {
                label: "Copy".into(),
                command: "editor.copy".into(),
            },
            MenuRow::Separator,
            MenuRow::Item {
                label: "Paste".into(),
                command: "editor.paste".into(),
            },
        ],
        (1, 2),
    );
    let size = CellSize::new(10, 40);
    let unset = frame_at(&s, size, ColorDepth::TrueColor);
    let idx = Color::Indexed;
    assert_eq!(
        unset[1][3].style,
        style(idx(231), idx(24)),
        "the active row"
    );
    assert_eq!(unset[2][3].glyph, pmacs::cell::Glyph::Char('─'));
    assert_eq!(unset[2][3].style, style(idx(252), idx(236)), "the rule");
    assert_eq!(unset[3][3].style, style(idx(252), idx(236)), "a row");

    exec(
        &s,
        r#"pmacs.theme.merge {
             ["ui.menu"] = { fg = {1, 2, 3}, bg = {4, 5, 6} },
             ["ui.menu.selected"] = { fg = {7, 8, 9}, bg = {10, 11, 12} },
             ["ui.menu.separator"] = { fg = {13, 14, 15} },
           }"#,
    );
    let set = frame_at(&s, size, ColorDepth::TrueColor);
    let rgb = Color::Rgb;
    assert_eq!(set[1][3].style, style(rgb(7, 8, 9), rgb(10, 11, 12)));
    assert_eq!(set[2][3].style, style(rgb(13, 14, 15), rgb(4, 5, 6)));
    assert_eq!(set[3][3].style, style(rgb(1, 2, 3), rgb(4, 5, 6)));
}

/// The minibuffer's candidate band and the completion popup paint their
/// rows, selected row and kind column in `ui.menu` and
/// `ui.menu.selected`; unset, the dim fill, reverse video and the dim
/// kind glyph they always had.
#[test]
fn e10g2_the_band_and_the_completion_popup_take_the_menu_faces() {
    use pmacs::completion::{
        CompletionEditsCarry, CompletionItemKind, CompletionPopupState, PopupCandidate,
    };
    let s = fresh();
    exec(&s, r#"pmacs.window.buffer():insert(0, "hello")"#);
    let candidate = |label: &str| PopupCandidate {
        carry: CompletionEditsCarry::default(),
        label: label.to_owned(),
        kind: CompletionItemKind::Text,
        detail: None,
        insert_text: label.to_owned(),
    };
    let popup = {
        let core = s.core.borrow();
        CompletionPopupState::new(
            core.active_buffer_id(),
            0,
            String::new(),
            vec![candidate("alpha"), candidate("beta")],
            2,
        )
        .expect("a non-empty popup")
    };
    s.core.borrow_mut().completion_popup_open(popup);
    exec(
        &s,
        r#"pmacs.minibuffer.read {
             prompt = "Pick: ",
             source = function() return { "one", "two" } end,
             on_accept = function() end,
           }"#,
    );
    let size = CellSize::new(12, 40);
    let idx = Color::Indexed;
    let reverse = Style {
        reverse: true,
        ..Style::default()
    };
    // The popup sits under row 0 after the gutter, the band over the
    // prompt row (11) from column 0; a label starts two columns after
    // its row's kind glyph.
    let unset = frame_at(&s, size, ColorDepth::TrueColor);
    let rows = [
        (1, find_text(&unset, 2, "beta"), "popup"),
        (9, find_text(&unset, 10, "two"), "band"),
    ];
    for (row, label, what) in rows {
        assert_eq!(unset[row][label].style, reverse, "{what}: the selected row");
        let (next, kind) = (&unset[row + 1][label], &unset[row + 1][label - 2]);
        assert_eq!(next.style, style(idx(252), idx(236)), "{what}: a row");
        assert_eq!(kind.style, style(idx(8), idx(236)), "{what}: kind");
    }

    exec(
        &s,
        r#"pmacs.theme.merge {
             ["ui.menu"] = { fg = {1, 2, 3}, bg = {4, 5, 6} },
             ["ui.menu.selected"] = { fg = {7, 8, 9}, bg = {10, 11, 12} },
           }"#,
    );
    let set = frame_at(&s, size, ColorDepth::TrueColor);
    let rgb = Color::Rgb;
    let (row_style, selected) = (
        style(rgb(1, 2, 3), rgb(4, 5, 6)),
        style(rgb(7, 8, 9), rgb(10, 11, 12)),
    );
    for (row, label, what) in rows {
        assert_eq!(set[row][label].style, selected, "{what}: the selected row");
        assert_eq!(set[row + 1][label].style, row_style, "{what}: a row");
        assert_eq!(set[row + 1][label - 2].style, row_style, "{what}: kind");
    }
}

// ---------------------------------------------------------------------------
// E10g.3 — the setting and the user's layer
// ---------------------------------------------------------------------------

/// Per-test roots: their own config dir (a real `init.lua`) and state
/// dir (the kept choice), since both are the subject.
fn roots_for(name: &str) -> pmacs::bootstrap::BootstrapRoots {
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("e10g")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let roots = pmacs::bootstrap::BootstrapRoots::isolated_under(&base);
    for (_, dir) in roots.child_env() {
        std::fs::create_dir_all(&dir).expect("create a controlled root");
    }
    for dir in [roots.state_dir(), roots.config_dir()] {
        std::fs::create_dir_all(dir.expect("isolated roots name it")).expect("create");
    }
    roots
}

/// Two themes an `init.lua` defines, told apart by the buffer face.
const TWO_THEMES: &str = r#"
pmacs.theme.define("plain-a", { faces = { ["ui.default"] = { bg = {1, 1, 1} } } })
pmacs.theme.define("plain-b", { faces = { ["ui.default"] = { bg = {2, 2, 2} } } })
"#;

/// Start an editor the way a launch does: `init` as its `init.lua`,
/// `kept` as the choice in `pmacs.state`, then the state installed,
/// which is when the theme is resolved.
fn start(roots: &pmacs::bootstrap::BootstrapRoots, init: &str, kept: Option<&str>) -> EditorState {
    let config = roots.config_dir().expect("config dir");
    std::fs::write(config.join("init.lua"), init).expect("write init.lua");
    let state = roots.state_dir().expect("state dir").join("theme");
    match kept {
        Some(name) => std::fs::write(&state, format!("{name}\n")).expect("plant"),
        None => {
            let _ = std::fs::remove_file(&state);
        }
    }
    let s = EditorState::new_with_roots(roots);
    s.install_state_dirs();
    s
}

fn loaded(s: &EditorState) -> Option<String> {
    eval(s, "return pmacs.theme.loaded()")
}

fn buffer_bg(s: &EditorState) -> Option<Color> {
    let theme = s.syntax_registry.theme();
    let th = theme.lock().unwrap();
    th.buffer_face().map(|f| f.bg)
}

/// The ruled precedence, each order on its own launch: an explicit
/// `init.lua` setting beats a kept choice, which beats the default.
#[test]
fn e10g3_init_lua_beats_the_kept_choice_which_beats_the_default() {
    let roots = roots_for("precedence");
    let set_b = format!("{TWO_THEMES}\npmacs.config.set(\"ui.theme\", \"plain-b\")\n");

    let s = start(&roots, TWO_THEMES, None);
    assert_eq!(loaded(&s).as_deref(), Some("default-dark"), "the default");
    assert_eq!(buffer_bg(&s), None);
    drop(s);

    let s = start(&roots, TWO_THEMES, Some("plain-a"));
    assert_eq!(loaded(&s).as_deref(), Some("plain-a"), "kept beats default");
    assert_eq!(buffer_bg(&s), Some(Color::Rgb(1, 1, 1)));
    let setting: String = eval(&s, "return pmacs.config.get('ui.theme')");
    assert_eq!(setting, "plain-a", "the setting says what is showing");
    drop(s);

    let s = start(&roots, &set_b, Some("plain-a"));
    assert_eq!(
        loaded(&s).as_deref(),
        Some("plain-b"),
        "init.lua beats kept"
    );
    assert_eq!(buffer_bg(&s), Some(Color::Rgb(2, 2, 2)));
    drop(s);

    let s = start(&roots, &set_b, None);
    assert_eq!(
        loaded(&s).as_deref(),
        Some("plain-b"),
        "init.lua beats default"
    );
}

/// `init.lua`'s own `merge` is the user layer: kept over a restored
/// choice and over a later change of the setting, while a `set` made
/// after a load still replaces the whole map, as documented, and is
/// itself kept over the next load.
#[test]
fn e10g3_init_lua_s_own_theme_changes_are_kept_over_any_theme_loaded() {
    let roots = roots_for("user-layer");
    let init = format!("{TWO_THEMES}\npmacs.theme.merge {{ keyword = {{ fg = {{9, 9, 9}} }} }}\n");
    let s = start(&roots, &init, Some("plain-a"));
    let keyword = |s: &EditorState| -> Option<Style> {
        let theme = s.syntax_registry.theme();
        let th = theme.lock().unwrap();
        th.by_capture.get("keyword").copied()
    };
    assert_eq!(loaded(&s).as_deref(), Some("plain-a"));
    assert_eq!(buffer_bg(&s), Some(Color::Rgb(1, 1, 1)), "the kept theme");
    assert_eq!(
        keyword(&s).map(|k| k.fg),
        Some(Color::Rgb(9, 9, 9)),
        "and the merge"
    );

    exec(&s, r#"pmacs.config.set("ui.theme", "plain-b")"#);
    assert_eq!(buffer_bg(&s), Some(Color::Rgb(2, 2, 2)), "the new theme");
    assert_eq!(
        keyword(&s).map(|k| k.fg),
        Some(Color::Rgb(9, 9, 9)),
        "the merge kept"
    );

    exec(&s, "pmacs.theme.set { keyword = { fg = {1, 1, 1} } }");
    let map = |s: &EditorState| s.syntax_registry.theme().lock().unwrap().by_capture.clone();
    assert_eq!(
        map(&s).len(),
        1,
        "set replaces the whole map: {:?}",
        map(&s)
    );
    exec(&s, r#"pmacs.theme.load("plain-a")"#);
    assert_eq!(map(&s).len(), 1, "and is kept over the next load");
    assert_eq!(keyword(&s).map(|k| k.fg), Some(Color::Rgb(1, 1, 1)));
}

/// A kept choice naming a theme that is no longer defined falls back to
/// the default, and says so.
#[test]
fn e10g3_a_kept_theme_that_no_longer_exists_falls_back_with_a_message() {
    let roots = roots_for("gone");
    let s = start(&roots, TWO_THEMES, Some("gone"));
    assert_eq!(loaded(&s).as_deref(), Some("default-dark"));
    let status = s.core.borrow().status.clone();
    assert!(
        status.contains("the kept theme \"gone\" is not defined; using default-dark"),
        "{status:?}"
    );
}

/// `ui.theme`'s choices are the defined names: a name is refused until a
/// theme is defined under it, and accepted, and loaded, after.
#[test]
fn e10g3_defining_a_theme_makes_it_a_choice_of_the_setting() {
    let s = fresh();
    let refused: String = eval(
        &s,
        r#"local ok, err = pcall(pmacs.config.set, "ui.theme", "later")
           assert(not ok)
           return tostring(err)"#,
    );
    assert!(refused.contains("later"), "{refused}");
    exec(
        &s,
        r#"pmacs.theme.define("later", { faces = { ["ui.default"] = { bg = {3, 3, 3} } } })
           pmacs.config.set("ui.theme", "later")"#,
    );
    assert_eq!(loaded(&s).as_deref(), Some("later"));
    assert_eq!(buffer_bg(&s), Some(Color::Rgb(3, 3, 3)));
}

// ---------------------------------------------------------------------------
// E10g.4 — the picker
// ---------------------------------------------------------------------------

fn press(
    s: &mut EditorState,
    code: crossterm::event::KeyCode,
    mods: crossterm::event::KeyModifiers,
) {
    s.dispatch_key(
        FrontendId::LOCAL,
        crossterm::event::KeyEvent {
            code,
            modifiers: mods,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        },
    );
}

fn down(s: &mut EditorState) {
    use crossterm::event::{KeyCode, KeyModifiers};
    press(s, KeyCode::Down, KeyModifiers::NONE);
}

fn ret(s: &mut EditorState) {
    use crossterm::event::{KeyCode, KeyModifiers};
    press(s, KeyCode::Enter, KeyModifiers::NONE);
}

/// `M-x theme-select RET`, by keys, and the picker is open on the theme
/// in force, which opening it did not change.
fn open_picker(s: &mut EditorState) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let in_force = loaded(s);
    let faces = theme_state(s);
    press(s, KeyCode::Char('x'), KeyModifiers::ALT);
    for ch in "theme-select".chars() {
        press(s, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    ret(s);
    let core = s.core.borrow();
    let session = core
        .minibuffer
        .session
        .as_ref()
        .expect("the picker is open");
    assert_eq!(session.prompt, "Theme: ");
    assert_eq!(
        core.minibuffer.selected_candidate(),
        in_force,
        "it opens on the theme in force"
    );
    drop(core);
    assert_eq!(loaded(s), in_force, "opening it loads nothing");
    assert_eq!(theme_state(s), faces, "opening it changes no face");
}

/// The theme's whole state as a load writes it: every entry, its
/// fallbacks and the default style.
fn theme_state(s: &EditorState) -> (Vec<(String, Style)>, String, Style) {
    let theme = s.syntax_registry.theme();
    let th = theme.lock().unwrap();
    let mut entries: Vec<(String, Style)> =
        th.by_capture.iter().map(|(k, v)| (k.clone(), *v)).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut fallbacks: Vec<String> = th.fallbacks.iter().map(|f| format!("{f:?}")).collect();
    fallbacks.sort();
    (entries, fallbacks.join(";"), th.default_style)
}

/// Relaunch over the same roots: `init.lua` and the state as they are.
fn relaunch(roots: &pmacs::bootstrap::BootstrapRoots) -> EditorState {
    let s = EditorState::new_with_roots(roots);
    s.install_state_dirs();
    s
}

/// A move loads the selected theme at once: the frame's buffer
/// background is the selected theme's before any RET.
#[test]
fn e10g4_a_move_previews_the_theme_before_ret() {
    let roots = roots_for("preview");
    let mut s = start(&roots, TWO_THEMES, None);
    let size = CellSize::new(10, 40);
    open_picker(&mut s);
    assert_eq!(
        frame_at(&s, size, ColorDepth::TrueColor)[5][5].style.bg,
        Color::Default
    );
    down(&mut s);
    assert!(s.core.borrow().minibuffer.is_active(), "no RET yet");
    let frame = frame_at(&s, size, ColorDepth::TrueColor);
    assert_eq!(
        frame[5][5].style.bg,
        Color::Rgb(1, 1, 1),
        "plain-a, previewed"
    );
    down(&mut s);
    let frame = frame_at(&s, size, ColorDepth::TrueColor);
    assert_eq!(
        frame[5][5].style.bg,
        Color::Rgb(2, 2, 2),
        "plain-b, previewed"
    );
    assert_eq!(loaded(&s).as_deref(), Some("plain-b"));
}

/// `C-g` after three moves restores the theme in force when the picker
/// opened, every face byte-identically, the user layer included.
#[test]
fn e10g4_c_g_after_three_moves_restores_every_face_byte_identically() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let roots = roots_for("cancel");
    let init = format!(
        "{TWO_THEMES}
pmacs.theme.define(\"plain-c\", {{ faces = {{ [\"ui.default\"] = {{ bg = {{3, 3, 3}},
  fallback = {{ [256] = {{ bg = 16 }}, [16] = {{ bg = 0 }} }} }} }} }})
pmacs.theme.merge {{ keyword = {{ fg = {{9, 9, 9}} }} }}
"
    );
    let mut s = start(&roots, &init, Some("plain-a"));
    let before = theme_state(&s);
    assert_eq!(loaded(&s).as_deref(), Some("plain-a"));
    open_picker(&mut s);
    for _ in 0..3 {
        down(&mut s);
    }
    assert_eq!(loaded(&s).as_deref(), Some("plain-c"), "three moves away");
    assert_ne!(theme_state(&s), before);
    press(&mut s, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert!(!s.core.borrow().minibuffer.is_active());
    assert_eq!(loaded(&s).as_deref(), Some("plain-a"));
    assert_eq!(theme_state(&s), before, "every face as it was");
}

/// `RET` keeps the selected theme and writes it to `pmacs.state`, so a
/// relaunch comes back to it; an `init.lua` that sets `ui.theme` is not
/// overridden by that kept choice. The user layer outlasts the RET.
#[test]
fn e10g4_ret_persists_across_a_restart_but_not_over_init_lua() {
    let roots = roots_for("persist");
    let init = format!("{TWO_THEMES}\npmacs.theme.merge {{ keyword = {{ fg = {{9, 9, 9}} }} }}\n");
    let mut s = start(&roots, &init, None);
    open_picker(&mut s);
    down(&mut s);
    ret(&mut s);
    assert_eq!(loaded(&s).as_deref(), Some("plain-a"));
    let keyword = |s: &EditorState| {
        let theme = s.syntax_registry.theme();
        let th = theme.lock().unwrap();
        th.by_capture.get("keyword").map(|k| k.fg)
    };
    assert_eq!(
        keyword(&s),
        Some(Color::Rgb(9, 9, 9)),
        "the merge, kept over RET"
    );
    let kept = roots.state_dir().unwrap().join("theme");
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "plain-a\n");
    drop(s);

    let s = relaunch(&roots);
    assert_eq!(
        loaded(&s).as_deref(),
        Some("plain-a"),
        "RET survives a restart"
    );
    assert_eq!(keyword(&s), Some(Color::Rgb(9, 9, 9)));
    drop(s);

    let with_setting = format!("{init}\npmacs.config.set(\"ui.theme\", \"plain-b\")\n");
    std::fs::write(roots.config_dir().unwrap().join("init.lua"), with_setting).unwrap();
    let s = relaunch(&roots);
    assert_eq!(
        loaded(&s).as_deref(),
        Some("plain-b"),
        "init.lua's ui.theme over the kept plain-a"
    );
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "plain-a\n");
}

/// The preview hook fires on a move within one prompt and never at a
/// prompt's opening: a key that accepts one prompt and opens another
/// (`M-x theme-select RET`'s shape) has moved nothing in the new one.
#[test]
fn e10g4_on_select_fires_on_a_move_and_not_when_a_prompt_opens() {
    let mut s = fresh();
    exec(
        &s,
        r#"seen = {}
           pmacs.minibuffer.read {
             prompt = "First: ",
             source = function() return { "go" } end,
             on_accept = function()
               pmacs.minibuffer.read {
                 prompt = "Second: ",
                 source = function() return { "x", "y" } end,
                 on_select = function(c) seen[#seen + 1] = c end,
                 on_accept = function() end,
               }
             end,
           }"#,
    );
    ret(&mut s);
    assert_eq!(
        s.core
            .borrow()
            .minibuffer
            .session
            .as_ref()
            .map(|m| m.prompt.clone()),
        Some("Second: ".to_owned())
    );
    let seen = |s: &EditorState| -> Vec<String> { eval(s, "return seen") };
    assert!(seen(&s).is_empty(), "opening called it: {:?}", seen(&s));
    down(&mut s);
    assert_eq!(seen(&s), vec!["y"], "one move, one call");
}

// ---------------------------------------------------------------------------
// E10g.5 — color depth
// ---------------------------------------------------------------------------

/// The colors of `theme`'s faces that map to two different authored
/// indices at one depth, as `(color, depth)`.
fn fallback_conflicts(s: &EditorState, theme: &str) -> Vec<String> {
    let named = s.syntax_registry.themes().get(theme).cloned().unwrap();
    let mut seen: HashMap<(String, String), (u8, String)> = HashMap::new();
    let mut conflicts = Vec::new();
    for entry in &named.entries {
        for component in Component::ALL {
            let main = component.of(&entry.style);
            for depth in [ColorDepth::TwoFiftySix, ColorDepth::Sixteen] {
                let Some(index) = authored_index(main, &entry.fallbacks, component, depth) else {
                    continue;
                };
                let key = (format!("{main:?}"), format!("{depth:?}"));
                let at = format!("{} {}", entry.name, component.name());
                match seen.get(&key) {
                    Some((other, by)) if *other != index => {
                        conflicts.push(format!("{key:?}: {by} -> {other}, {at} -> {index}"));
                    }
                    Some(_) => {}
                    None => {
                        seen.insert(key, (index, at));
                    }
                }
            }
        }
    }
    conflicts
}

/// No shipped theme gives two faces the same color with different
/// fallbacks at one depth, so an attached terminal's map from a color
/// back to its face's fallback is well defined. The planted theme is
/// the control.
#[test]
fn e10g5_no_shipped_theme_gives_one_color_two_fallbacks() {
    let s = fresh();
    let shipped: Vec<String> = eval(&s, "return pmacs.theme.names()");
    for theme in &shipped {
        let conflicts = fallback_conflicts(&s, theme);
        assert!(conflicts.is_empty(), "{conflicts:#?}");
    }
    exec(
        &s,
        r#"pmacs.theme.define("planted", { faces = {
              ["ui.modeline"] = { bg = {9, 9, 9}, fallback = { [256] = { bg = 233 }, [16] = { bg = 0 } } },
              ["ui.menu"] = { bg = {9, 9, 9}, fallback = { [256] = { bg = 234 }, [16] = { bg = 0 } } },
           } })"#,
    );
    assert_eq!(fallback_conflicts(&s, "planted").len(), 1);
}

/// Attach a grid frontend declaring `true_color` to `daemon` and return
/// its first full frame as a grid.
fn attach_grid(
    daemon: &common::daemon::TestDaemon,
    size: CellSize,
    true_color: bool,
) -> (std::os::unix::net::UnixStream, Vec<Vec<Cell>>) {
    use pmacs_protocol::transport::{read_message, write_message};
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("read timeout");
    let _hello: pmacs_protocol::message::Hello = read_message(&mut stream).expect("Hello");
    let request = pmacs_protocol::message::AttachRequest {
        protocol_version: pmacs_protocol::message::PROTOCOL_VERSION,
        frontend_capabilities: pmacs_protocol::message::FrontendCapabilities {
            true_color,
            ..common::daemon::build_default_caps()
        },
        initial_size: size,
    };
    write_message(&mut stream, &request).expect("AttachRequest");
    let mut grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
    loop {
        let message: InstanceMessage = read_message(&mut stream).expect("a frame");
        if let InstanceMessage::CellDelta {
            spans,
            full_grid: true,
        } = message
        {
            for span in spans {
                for (i, cell) in span.cells.into_iter().enumerate() {
                    grid[span.start.row as usize][span.start.col as usize + i] = cell;
                }
            }
            return (stream, grid);
        }
    }
}

fn frame_colors(grid: &[Vec<Cell>]) -> Vec<Color> {
    grid.iter()
        .flatten()
        .flat_map(|c| colors_of(&c.style))
        .collect()
}

const DEEP: &str = r#"
pmacs.theme.define("deep", { faces = {
  ["ui.modeline"] = { fg = {250, 250, 250}, bg = {0, 0, 200},
    fallback = { [256] = { fg = 231, bg = 19 }, [16] = { fg = 15, bg = 4 } } },
  ["ui.statusline"] = { fg = {200, 60, 60} },
} })
pmacs.config.set("ui.theme", "deep")
"#;

/// A daemon paints an attachment whose `true_color` is false at 256:
/// a theme face's color as its authored 256 fallback, a color with none
/// authored as the nearest of the 256; a truecolor attachment to the
/// same configuration gets the RGB. `ui.color-depth` at 16 makes a truecolor attachment
/// 256 too: the wire carries no finer depth.
#[test]
fn e10g5_the_daemon_paints_an_attachment_below_truecolor_at_256() {
    let size = CellSize::new(10, 40);
    let daemon = common::daemon::TestDaemon::spawn_with_config(DEEP);
    let (_low_stream, low) = attach_grid(&daemon, size, false);
    let colors = frame_colors(&low);
    assert!(colors.contains(&Color::Indexed(231)) && colors.contains(&Color::Indexed(19)));
    let nearest = Color::Indexed(pmacs::color_depth::nearest_256((200, 60, 60)));
    assert!(
        colors.contains(&nearest),
        "the unauthored face: {nearest:?}"
    );
    assert!(
        !colors.iter().any(|c| matches!(c, Color::Rgb(..))),
        "no RGB reaches it"
    );
    drop(daemon);
    // A legacy grid session is the daemon's only one, so the control
    // attaches to a daemon of its own.
    let daemon = common::daemon::TestDaemon::spawn_with_config(DEEP);
    let (_high_stream, high) = attach_grid(&daemon, size, true);
    let colors = frame_colors(&high);
    assert!(colors.contains(&Color::Rgb(0, 0, 200)), "the control: RGB");
    drop(daemon);

    let capped = format!("{DEEP}\npmacs.config.set(\"ui.color-depth\", \"16\")\n");
    let daemon = common::daemon::TestDaemon::spawn_with_config(&capped);
    let (_stream, frame) = attach_grid(&daemon, size, true);
    let colors = frame_colors(&frame);
    assert!(colors.contains(&Color::Indexed(19)), "16 asked, 256 sent");
    assert!(
        !colors.contains(&Color::Indexed(4)),
        "never the authored 16"
    );
}

/// The in-process terminal's depth follows `ui.color-depth` live,
/// capped by what was detected.
#[test]
fn e10g5_the_in_process_depth_follows_the_setting() {
    let s = fresh();
    assert_eq!(
        s.local_color_depth(ColorDepth::TrueColor),
        ColorDepth::TrueColor
    );
    assert_eq!(
        s.local_color_depth(ColorDepth::Sixteen),
        ColorDepth::Sixteen
    );
    exec(&s, r#"pmacs.config.set("ui.color-depth", "256")"#);
    assert_eq!(
        s.local_color_depth(ColorDepth::TrueColor),
        ColorDepth::TwoFiftySix
    );
    assert_eq!(
        s.local_color_depth(ColorDepth::Sixteen),
        ColorDepth::Sixteen
    );
}

// ---------------------------------------------------------------------------
// E10g.6 — legible as painted
// ---------------------------------------------------------------------------

/// The two ways a terminal gets 16 colors (E10g.6): in process, each
/// theme color its authored 16; attached, the daemon's authored 256 and
/// then the client's fixed table.
#[derive(Clone, Copy, Debug)]
enum Path16 {
    InProcess,
    Attached,
}

fn frame_16(s: &EditorState, size: CellSize, path: Path16) -> Vec<Vec<Cell>> {
    frame_16_with(s, size, path, &HashMap::new())
}

fn frame_16_with(
    s: &EditorState,
    size: CellSize,
    path: Path16,
    terminals: &HashMap<pmacs::window::WindowId, pmacs::terminal::session::TerminalSnapshot>,
) -> Vec<Vec<Cell>> {
    match path {
        Path16::InProcess => frame_at_with(s, size, ColorDepth::Sixteen, terminals),
        Path16::Attached => {
            let mut grid = frame_at_with(s, size, ColorDepth::TwoFiftySix, terminals);
            for cell in grid.iter_mut().flatten() {
                let at16 = |c| pmacs::color_depth::reduce(c, ColorDepth::Sixteen);
                cell.style.fg = at16(cell.style.fg);
                cell.style.bg = at16(cell.style.bg);
            }
            grid
        }
    }
}

/// The first `(row, col)` where `text` is drawn.
fn find_anywhere(grid: &[Vec<Cell>], text: &str) -> (usize, usize) {
    for (row, cells) in grid.iter().enumerate() {
        let glyphs: String = cells
            .iter()
            .map(|c| match c.glyph {
                pmacs::cell::Glyph::Char(ch) => ch,
                _ => '\0',
            })
            .collect();
        if let Some(byte) = glyphs.find(text) {
            return (row, glyphs[..byte].chars().count());
        }
    }
    panic!("{text:?} is not drawn");
}

/// Every chrome surface of the grid frontend as painted at 16 colors on
/// `path`, and the buffer surface: the modeline, the status line, the
/// minibuffer's prompt row, its candidate band (a row and the selected
/// one), the context menu (a row, the selected one, the rule), the
/// completion popup (a row, the selected one) and the hover popup, then
/// every other surface painted from a `ui.*` face ([`more_chrome_at_16`]).
/// A surface is sampled where its text is drawn.
fn chrome_at_16(s: &mut EditorState, path: Path16) -> (Style, Vec<(String, Style)>) {
    use pmacs::completion::{
        CompletionEditsCarry, CompletionItemKind, CompletionPopupState, PopupCandidate,
    };
    let size = CellSize::new(16, 60);
    let mut surfaces = Vec::new();
    exec(s, r#"pmacs.window.buffer():insert(0, "hello")"#);

    let plain = frame_16(s, size, path);
    let buffer = plain[8][30].style;
    surfaces.push(("modeline".to_owned(), plain[14][1].style));
    surfaces.push(("status line".to_owned(), plain[15][1].style));

    let target = s.core.borrow().lsp_popup_target().expect("a popup target");
    let hover = pmacs::hover::Hover {
        contents: "Hover words".into(),
        range: None,
    };
    let _ = s
        .core
        .borrow_mut()
        .lsp_popup_open_hover(target, Some(&hover));
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "Hover words");
    surfaces.push(("hover popup".to_owned(), frame[row][col].style));
    s.core.borrow_mut().lsp_popup_close();

    s.core.borrow_mut().menu_open(
        vec![
            pmacs::menu::MenuRow::Item {
                label: "Alpha".into(),
                command: "editor.copy".into(),
            },
            pmacs::menu::MenuRow::Separator,
            pmacs::menu::MenuRow::Item {
                label: "Beta".into(),
                command: "editor.paste".into(),
            },
        ],
        (2, 20),
    );
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "Alpha");
    surfaces.push(("menu, selected row".to_owned(), frame[row][col].style));
    surfaces.push(("menu, rule".to_owned(), frame[row + 1][col].style));
    surfaces.push(("menu, row".to_owned(), frame[row + 2][col].style));
    s.core.borrow_mut().menu_close();

    let candidate = |label: &str| PopupCandidate {
        carry: CompletionEditsCarry::default(),
        label: label.to_owned(),
        kind: CompletionItemKind::Text,
        detail: None,
        insert_text: label.to_owned(),
    };
    let popup = {
        let core = s.core.borrow();
        CompletionPopupState::new(
            core.active_buffer_id(),
            0,
            String::new(),
            vec![candidate("gamma"), candidate("delta")],
            2,
        )
        .expect("a popup")
    };
    s.core.borrow_mut().completion_popup_open(popup);
    exec(
        s,
        r#"pmacs.minibuffer.read {
             prompt = "Pick: ",
             source = function() return { "one", "two" } end,
             on_accept = function() end,
           }"#,
    );
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "gamma");
    surfaces.push((
        "completion popup, selected row".to_owned(),
        frame[row][col].style,
    ));
    surfaces.push((
        "completion popup, row".to_owned(),
        frame[row + 1][col].style,
    ));
    let (row, col) = find_anywhere(&frame, "one");
    surfaces.push((
        "candidate band, selected row".to_owned(),
        frame[row][col].style,
    ));
    surfaces.push(("candidate band, row".to_owned(), frame[row + 1][col].style));
    let (row, col) = find_anywhere(&frame, "Pick: ");
    surfaces.push(("minibuffer".to_owned(), frame[row][col].style));
    exec(s, "pmacs.minibuffer.cancel()");
    s.core.borrow_mut().completion_popup_close();
    surfaces.extend(more_chrome_at_16(s, path));
    (buffer, surfaces)
}

/// The grid's other surfaces painted from a `ui.*` face (E10g.6 as
/// amended on 2026-10-29, review 1's Medium 1): the line-number gutter
/// (`ui.gutter`, on by default) on a numbered row and below the last
/// line; the minibuffer's inline candidate (`ui.minibuffer.candidate`);
/// the band's and the completion popup's kind column (`ui.menu`); the
/// hover popup's closing row (`ui.popup.footer`) and the signature
/// popup's active parameter (`ui.popup.active-parameter`); the search
/// prompt (`ui.minibuffer`) and its two washes (`ui.search.match`,
/// `ui.search.match.active`); the selection over the buffer's text and
/// over a terminal pane's (`ui.selection`); the divider above a lower
/// window (`ui.divider`); a mode-line segment's face (`ui.modeline.*`);
/// and a diagnostic's sign in the gutter (`ui.diag.*`), with the
/// column-0 mark that replaces it when line numbers are off. Each is
/// sampled where its glyphs are drawn, a wash over the buffer's plain
/// text. A diagnostic's underline is not a surface here: its face colors
/// the underline alone, which neither clause reads, over the cell the
/// text left.
fn more_chrome_at_16(s: &mut EditorState, path: Path16) -> Vec<(String, Style)> {
    let mut surfaces = gutter_and_band_chrome(s, path);
    surfaces.extend(lsp_popup_chrome(s, path));
    surfaces.extend(wash_chrome(s, path));
    surfaces.extend(modeline_row_chrome(s, path));
    surfaces.extend(diagnostic_chrome(s, path));
    surfaces
}

/// [`more_chrome_at_16`]: the gutter, the inline candidate and the kind
/// columns.
fn gutter_and_band_chrome(s: &mut EditorState, path: Path16) -> Vec<(String, Style)> {
    let size = CellSize::new(16, 60);
    let mut surfaces = Vec::new();
    let plain = frame_16(s, size, path);
    let (row, text_col) = find_anywhere(&plain, "hello");
    assert_eq!(row, 0, "the text is on the first row");
    let number_col = plain[0]
        .iter()
        .position(|c| c.glyph == pmacs::cell::Glyph::Char('1'))
        .expect("line 1's number is drawn");
    assert!(number_col < text_col, "the number sits in the gutter");
    surfaces.push(("gutter, a number".to_owned(), plain[0][number_col].style));
    surfaces.push(("gutter, past the last line".to_owned(), plain[8][0].style));

    exec(
        s,
        r#"pmacs.minibuffer.read {
             prompt = "Pick: ",
             source = function() return { "one", "two" } end,
             on_accept = function() end,
           }"#,
    );
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "[one]");
    surfaces.push((
        "minibuffer, its inline candidate".to_owned(),
        frame[row][col].style,
    ));
    let (row, col) = find_anywhere(&frame, "two");
    surfaces.push((
        "candidate band, kind column".to_owned(),
        frame[row][col - 2].style,
    ));
    exec(s, "pmacs.minibuffer.cancel()");

    let candidate = |label: &str| pmacs::completion::PopupCandidate {
        carry: pmacs::completion::CompletionEditsCarry::default(),
        label: label.to_owned(),
        kind: pmacs::completion::CompletionItemKind::Text,
        detail: None,
        insert_text: label.to_owned(),
    };
    let popup = {
        let core = s.core.borrow();
        pmacs::completion::CompletionPopupState::new(
            core.active_buffer_id(),
            0,
            String::new(),
            vec![candidate("gamma"), candidate("delta")],
            2,
        )
        .expect("a popup")
    };
    s.core.borrow_mut().completion_popup_open(popup);
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "delta");
    surfaces.push((
        "completion popup, kind column".to_owned(),
        frame[row][col - 2].style,
    ));
    s.core.borrow_mut().completion_popup_close();
    surfaces
}

/// [`more_chrome_at_16`]: the hover popup's footer and the signature
/// popup's active parameter.
fn lsp_popup_chrome(s: &mut EditorState, path: Path16) -> Vec<(String, Style)> {
    let size = CellSize::new(16, 60);
    let mut surfaces = Vec::new();
    let target = s.core.borrow().lsp_popup_target().expect("a popup target");
    let long = (1..=40)
        .map(|i| format!("Hover line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let hover = pmacs::hover::Hover {
        contents: long,
        range: None,
    };
    let _ = s
        .core
        .borrow_mut()
        .lsp_popup_open_hover(target, Some(&hover));
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "more line");
    surfaces.push(("hover popup, its footer".to_owned(), frame[row][col].style));
    s.core.borrow_mut().lsp_popup_close();

    let target = s.core.borrow().lsp_popup_target().expect("a popup target");
    let help = pmacs::signature::SignatureHelp {
        signatures: vec![pmacs::signature::Signature {
            label: "echo(name, count)".into(),
            documentation: None,
            parameters: vec![
                pmacs::signature::SignatureParameter {
                    label: "name".into(),
                    span: Some((5, 9)),
                    documentation: None,
                },
                pmacs::signature::SignatureParameter {
                    label: "count".into(),
                    span: Some((11, 16)),
                    documentation: None,
                },
            ],
            active_parameter: Some(0),
        }],
        active_signature: 0,
        active_parameter: Some(0),
    };
    let _ = s
        .core
        .borrow_mut()
        .lsp_popup_open_signature(target, Some(&help), false);
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "echo(name");
    surfaces.push((
        "signature popup, its active parameter".to_owned(),
        frame[row][col + 5].style,
    ));
    s.core.borrow_mut().lsp_popup_close();
    surfaces
}

/// [`more_chrome_at_16`]: the search prompt and its washes, and the
/// selection over the buffer and over a terminal pane.
fn wash_chrome(s: &mut EditorState, path: Path16) -> Vec<(String, Style)> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let size = CellSize::new(16, 60);
    let mut surfaces = Vec::new();
    let (row, text_col) = find_anywhere(&frame_16(s, size, path), "hello");
    assert_eq!(row, 0, "the text is on the first row");
    exec(s, r#"pmacs.window.buffer():insert(5, " hello")"#);
    press(s, KeyCode::Char('s'), KeyModifiers::CONTROL);
    for ch in "hello".chars() {
        press(s, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "I-search");
    surfaces.push(("search prompt".to_owned(), frame[row][col].style));
    let active = frame[0][text_col].style;
    let lazy = frame[0][text_col + 6].style;
    assert_ne!(active, lazy, "one match is active, the other lazy");
    surfaces.push(("search, a match".to_owned(), lazy));
    surfaces.push(("search, the active match".to_owned(), active));
    press(s, KeyCode::Char('g'), KeyModifiers::CONTROL);

    press(s, KeyCode::Char('a'), KeyModifiers::CONTROL);
    press(s, KeyCode::Char(' '), KeyModifiers::CONTROL);
    press(s, KeyCode::Char('e'), KeyModifiers::CONTROL);
    let frame = frame_16(s, size, path);
    surfaces.push(("selection".to_owned(), frame[0][text_col].style));
    press(s, KeyCode::Char('g'), KeyModifiers::CONTROL);

    let window = s.core.borrow().active_window_id();
    let buffer_id = s.core.borrow().active_buffer_id();
    let term = pmacs::terminal::session::TerminalSnapshot {
        buffer_id,
        size: CellSize::new(4, 20),
        cells: "term"
            .chars()
            .chain(std::iter::repeat(' '))
            .take(80)
            .map(|ch| Cell {
                glyph: pmacs::cell::Glyph::Char(ch),
                ..Cell::default()
            })
            .collect(),
        cursor: None,
        title: None,
        screen_generation: 1,
        selection: vec![pmacs_protocol::terminal::TerminalSelectionSpan {
            row: 0,
            start_col: 0,
            end_col: 4,
        }],
        scroll_offset: 0,
        at_bottom: true,
        pid: 1,
        process: pmacs_protocol::terminal::TerminalProcessState::Running,
    };
    let frame = frame_16_with(s, size, path, &HashMap::from([(window, term)]));
    let (row, col) = find_anywhere(&frame, "term");
    surfaces.push((
        "a terminal pane's selection".to_owned(),
        frame[row][col].style,
    ));
    surfaces
}

/// [`more_chrome_at_16`]: a segment's face and the divider, on the mode
/// line's row.
fn modeline_row_chrome(s: &mut EditorState, path: Path16) -> Vec<(String, Style)> {
    let size = CellSize::new(16, 60);
    let mut surfaces = Vec::new();
    exec(
        s,
        r#"pmacs.statusline.register {
             name = "e10g6-segment",
             side = "left",
             face = "ui.modeline.lsp",
             fn = function() return "SEGMENT" end,
           }"#,
    );
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "SEGMENT");
    surfaces.push((
        "modeline, a segment's face".to_owned(),
        frame[row][col].style,
    ));

    exec(s, "pmacs.window.split_horizontal()");
    let frame = frame_16(s, size, path);
    let (row, col) = find_anywhere(&frame, "⇕");
    surfaces.push(("divider".to_owned(), frame[row][col].style));
    exec(s, "pmacs.window.close_others()");
    surfaces
}

/// [`more_chrome_at_16`]: a diagnostic's sign in the gutter, and its mark
/// where line numbers are off.
fn diagnostic_chrome(s: &mut EditorState, path: Path16) -> Vec<(String, Style)> {
    use pmacs::diag::{Diagnostic, DiagnosticSeverity};
    let size = CellSize::new(16, 60);
    let mut surfaces = Vec::new();
    exec(s, r#"pmacs.window.buffer():insert(0, "w\ni\nh\n")"#);
    let uri: String = eval(
        s,
        r#"local uri = "file:///e10g6/marks"
           assert(pmacs.diag._attach_view(pmacs.window.buffer(), uri))
           return uri"#,
    );
    {
        let core = s.core.borrow();
        let registry = core.registry.clone();
        let mut reg = registry.borrow_mut();
        let buf = reg.get_mut(core.active_buffer_id()).unwrap();
        buf.set_file_path(Some(std::path::PathBuf::from("/e10g6/marks")));
    }
    let severities = [
        ("error", DiagnosticSeverity::Error),
        ("warning", DiagnosticSeverity::Warning),
        ("information", DiagnosticSeverity::Information),
        ("hint", DiagnosticSeverity::Hint),
    ];
    let diagnostics = severities
        .iter()
        .enumerate()
        .map(|(line, &(_, severity))| Diagnostic {
            start_line: line as u32,
            start_col: 0,
            end_line: line as u32,
            end_col: 1,
            severity,
            message: "boom".into(),
            source: None,
            code: None,
            span: None,
        })
        .collect();
    let store = s.lsp_manager.borrow().diag_store();
    store.lock().unwrap().set(uri, diagnostics);
    let frame = frame_16(s, size, path);
    for (line, (name, _)) in severities.iter().enumerate() {
        surfaces.push((
            format!("gutter, a diagnostic's sign ({name})"),
            frame[line][0].style,
        ));
    }
    exec(s, "pmacs.window.set_line_numbers('off')");
    let frame = frame_16(s, size, path);
    for (line, (name, _)) in severities.iter().enumerate() {
        surfaces.push((
            format!("a diagnostic's mark without a gutter ({name})"),
            frame[line][0].style,
        ));
    }
    surfaces
}

/// The loaded theme's chrome held to E10g.6 as ruled: the first clause
/// only where the theme sets `ui.default`.
fn legibility(s: &mut EditorState, path: Path16) -> Vec<String> {
    let buffer_face_set = s
        .syntax_registry
        .theme()
        .lock()
        .unwrap()
        .buffer_face()
        .is_some();
    let (buffer, surfaces) = chrome_at_16(s, path);
    pmacs::color_depth::legibility_violations(&buffer, &surfaces, buffer_face_set)
}

fn violations(theme: &str, path: Path16) -> Vec<String> {
    let mut s = fresh();
    exec(&s, LEGIBILITY_THEMES);
    exec(&s, &format!("pmacs.theme.load({theme:?})"));
    legibility(&mut s, path)
}

/// #428: what a theme without a buffer face paints that the second
/// clause cannot judge, a constant index against one of the terminal's
/// own defaults (the owner's rulings of 2026-10-29): the gutter, a
/// diagnostic's sign in it, and a diagnostic's mark where line numbers
/// are off. `default-dark` paints every one of them, on both paths; they
/// are pinned here by name, as `2a553f0` pinned the first clause's.
const ISSUE_428: &[&str] = &[
    "gutter, a number: its foreground does not differ from its background (Index(8) on DefaultBg)",
    "gutter, past the last line: its foreground does not differ from its background (Index(8) on DefaultBg)",
    "gutter, a diagnostic's sign (error): its foreground does not differ from its background (Index(1) on DefaultBg)",
    "gutter, a diagnostic's sign (warning): its foreground does not differ from its background (Index(3) on DefaultBg)",
    "gutter, a diagnostic's sign (information): its foreground does not differ from its background (Index(6) on DefaultBg)",
    "gutter, a diagnostic's sign (hint): its foreground does not differ from its background (Index(8) on DefaultBg)",
    "a diagnostic's mark without a gutter (error): its foreground does not differ from its background (DefaultFg on Index(1))",
    "a diagnostic's mark without a gutter (warning): its foreground does not differ from its background (DefaultFg on Index(3))",
    "a diagnostic's mark without a gutter (information): its foreground does not differ from its background (DefaultFg on Index(6))",
    "a diagnostic's mark without a gutter (hint): its foreground does not differ from its background (DefaultFg on Index(8))",
];

/// `found` without #428's pinned lines, after asserting it holds every
/// one of them: a theme with no buffer face paints them all.
fn beyond_428(found: Vec<String>, what: &str) -> Vec<String> {
    for pinned in ISSUE_428 {
        assert!(
            found.iter().any(|v| v == pinned),
            "{what}: #428's {pinned:?} is not reported: {found:#?}"
        );
    }
    found
        .into_iter()
        .filter(|v| !ISSUE_428.contains(&v.as_str()))
        .collect()
}

/// Planted themes, one failing each clause, and one that passes. Every
/// color is an index below 16, its own fallback on both paths.
const LEGIBILITY_THEMES: &str = r#"
local chrome = {
  ["ui.default"] = { fg = 7, bg = 0 },
  ["ui.modeline"] = { fg = 0, bg = 6 },
  ["ui.statusline"] = { fg = 6 },
  ["ui.minibuffer"] = { fg = 6 },
  ["ui.menu"] = { fg = 15, bg = 4 },
  ["ui.menu.selected"] = { fg = 0, bg = 3 },
  ["ui.menu.separator"] = { fg = 7 },
  ["ui.popup"] = { fg = 15, bg = 4 },
}
local function with(overrides)
  local t = {}
  for k, v in pairs(chrome) do t[k] = v end
  for k, v in pairs(overrides) do t[k] = v end
  return { faces = t }
end
pmacs.theme.define("legible", with({}))
pmacs.theme.define("planted-same-as-buffer", with({ ["ui.modeline"] = { fg = 7, bg = 0 } }))
-- The buffer's own pair, reversed: legible only as painted.
pmacs.theme.define("reversed", with({ ["ui.modeline"] = { fg = 7, bg = 0, reverse = true } }))
pmacs.theme.define("planted-fg-on-itself", with({ ["ui.modeline"] = { fg = 4, bg = 4 } }))
-- A buffer face with no background, so a surface's Default stays the
-- terminal's own as painted (with one, the grid fills it in).
pmacs.theme.define("planted-black-on-default", with({
  ["ui.default"] = { fg = 7 },
  ["ui.modeline"] = { fg = 0 },
}))
-- No buffer face: the menus and popups concrete, the status line and the
-- minibuffer left to the terminal's pair, and the modeline the
-- terminal's pair reversed, as `default-dark` paints it.
local bare = {
  ["ui.modeline"] = { reverse = true },
  ["ui.menu"] = { fg = 15, bg = 4 },
  ["ui.menu.selected"] = { fg = 0, bg = 3 },
  ["ui.menu.separator"] = { fg = 7 },
  ["ui.popup"] = { fg = 15, bg = 4 },
}
pmacs.theme.define("pair-modeline", { faces = bare })
local itself = {}
for k, v in pairs(bare) do itself[k] = v end
itself["ui.modeline"] = { fg = 4, bg = 4 }
pmacs.theme.define("planted-bare-fg-on-itself", { faces = itself })
"#;

/// The mode line's row as each surface on it is painted: the mode line,
/// a segment's face, and the divider handle above a lower window.
const MODELINE_ROW: [&str; 3] = ["modeline", "modeline, a segment's face", "divider"];

/// Each clause fails by name for the planted theme that breaks it, on
/// both paths, on the mode line's row (the mode line, a segment drawn in
/// its face and the divider handle on it) and nowhere else; the theme
/// that breaks none passes, so the check can.
#[test]
fn e10g6_a_planted_theme_fails_each_clause_by_name() {
    for path in [Path16::InProcess, Path16::Attached] {
        assert_eq!(
            violations("legible", path),
            Vec::<String>::new(),
            "{path:?}"
        );
        assert_eq!(
            violations("reversed", path),
            Vec::<String>::new(),
            "{path:?}: reverse video applied"
        );
        let same = violations("planted-same-as-buffer", path);
        assert_eq!(
            same.iter()
                .map(|v| v.split(": ").next().unwrap())
                .collect::<Vec<_>>(),
            MODELINE_ROW,
            "{path:?}: {same:#?}"
        );
        assert!(
            same.iter()
                .all(|v| v.contains(": does not differ from the buffer surface")),
            "{path:?}: {same:#?}"
        );
        let itself = violations("planted-fg-on-itself", path);
        assert_eq!(
            itself,
            MODELINE_ROW.map(|surface| format!(
                "{surface}: its foreground does not differ from its background (Index(4) on Index(4))"
            )),
            "{path:?}"
        );
        let black = violations("planted-black-on-default", path);
        assert!(
            black.contains(
                &"modeline: its foreground does not differ from its background (Index(0) on DefaultBg)"
                    .to_owned()
            ),
            "{path:?}: Indexed(0) against a Default background: {black:#?}"
        );
    }
}

/// Without a buffer face only the second clause judges, the terminal's
/// pair two colors against each other: a theme whose modeline is that
/// pair reversed passes (the first clause would fail it, the buffer
/// surface being the same pair), and one whose modeline is 4 on 4 still
/// fails the second clause by name, on both paths. Either reports
/// #428's lines too, as every theme without a buffer face does.
#[test]
fn e10g6_without_a_buffer_face_only_the_second_clause_judges() {
    for path in [Path16::InProcess, Path16::Attached] {
        assert_eq!(
            beyond_428(violations("pair-modeline", path), "pair-modeline"),
            Vec::<String>::new(),
            "{path:?}: the terminal's pair, no clause one"
        );
        assert_eq!(
            beyond_428(
                violations("planted-bare-fg-on-itself", path),
                "planted-bare-fg-on-itself"
            ),
            MODELINE_ROW.map(|surface| format!(
                "{surface}: its foreground does not differ from its background (Index(4) on Index(4))"
            )),
            "{path:?}"
        );
    }
}

/// Every shipped theme's chrome at 16 colors passes on both paths, as
/// the owner ruled on 2026-10-28, every grid surface painted from a
/// `ui.*` face sampled, as ruled on 2026-10-29: `default-dark`, which
/// sets no `ui.default`, by the second clause alone, but for #428's
/// lines, pinned. What the first clause no longer sees for it, its menu
/// and popups filled with color 0 at 16 colors, is #424.
#[test]
fn e10g6_every_shipped_theme_is_legible_at_16_colors_but_for_428() {
    let shipped: Vec<String> = eval(&fresh(), "return pmacs.theme.names()");
    assert!(shipped.iter().any(|n| n == "default-dark"), "{shipped:?}");
    for path in [Path16::InProcess, Path16::Attached] {
        for theme in &shipped {
            let mut s = fresh();
            exec(&s, &format!("pmacs.theme.load({theme:?})"));
            let found = legibility(&mut s, path);
            if theme == "default-dark" {
                assert_eq!(found, ISSUE_428, "{path:?} {theme}: #428, pinned");
            } else {
                assert!(found.is_empty(), "{path:?} {theme}: {found:#?}");
            }
        }
    }
}
