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

#[path = "common/iso.rs"]
mod iso;

fn fresh() -> EditorState {
    EditorState::new_with_roots(&iso::roots())
}

/// The whole screen of one frame painted at `depth`, as a row-major
/// grid: the first frame diffs against a blank grid, so a cell no span
/// names is a default cell.
fn frame_at(s: &EditorState, size: CellSize, depth: ColorDepth) -> Vec<Vec<Cell>> {
    let mut r = RenderState::new(size);
    r.set_color_depth(depth);
    let mut grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
    for message in r.render_frame(s, FrontendId::LOCAL, &HashMap::new(), &[]) {
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
