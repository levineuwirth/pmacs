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
