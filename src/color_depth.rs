// color_depth.rs --- E10g: what a color becomes at a terminal's depth.

//! Color depth (E10g.1 and E10g.5).
//!
//! A terminal shows truecolor, 256 colors or 16. A color that its
//! terminal cannot show is reduced here, by one rule in three steps:
//!
//! 1. A color that is a component of a face of the active theme takes
//!    that face's authored fallback at the depth
//!    ([`crate::highlight::Fallbacks`]).
//! 2. An index already in the depth's range is its own: every index at
//!    256, `0..16` at 16. This is why `default-dark`, whose syntax colors
//!    are indices 1 to 14, looks the same at every depth.
//! 3. Anything else goes to the nearest palette color: an RGB to the
//!    nearest of the 6×6×6 cube and the grey ramp at 256 (never `0..16`,
//!    which every terminal colors its own way), or to the nearest of the
//!    sixteen at 16; an index `16..256` at 16 through [`index_to_16`], a
//!    fixed table.
//!
//! The sixteen system colors' RGB values are the GPU frontend's own
//! table (`indexed_to_glyphon`, `pmacs-gpu/src/main.rs`), so "nearest of
//! sixteen" means nearest to what the GPU shows for that index.
//! Distances are squared Euclidean in RGB.

use std::collections::HashMap;

use crate::cell::{Color, Style};
use crate::highlight::{DepthColors, Fallbacks, Theme};

/// How many colors a terminal shows. Ordered, so the lower of two is
/// [`Ord::min`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorDepth {
    /// The sixteen system colors.
    Sixteen,
    /// The xterm 256-color palette.
    TwoFiftySix,
    /// 24-bit RGB.
    TrueColor,
}

impl ColorDepth {
    /// The largest index a color may carry at this depth, or `None` at
    /// truecolor, where nothing is reduced.
    #[must_use]
    pub fn max_index(self) -> Option<u8> {
        match self {
            Self::Sixteen => Some(15),
            Self::TwoFiftySix => Some(255),
            Self::TrueColor => None,
        }
    }
}

/// The sixteen system colors as the GPU frontend draws them.
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
];

/// The 6×6×6 cube's channel levels (indices 16 to 231).
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The RGB an xterm-palette index shows.
#[must_use]
pub fn index_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => ANSI16[usize::from(index)],
        16..=231 => {
            let i = index - 16;
            (
                CUBE[usize::from(i / 36)],
                CUBE[usize::from((i / 6) % 6)],
                CUBE[usize::from(i % 6)],
            )
        }
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            (level, level, level)
        }
    }
}

fn distance((r1, g1, b1): (u8, u8, u8), (r2, g2, b2): (u8, u8, u8)) -> u32 {
    let d = |a: u8, b: u8| u32::from(a.abs_diff(b)).pow(2);
    d(r1, r2) + d(g1, g2) + d(b1, b2)
}

fn nearest_in(rgb: (u8, u8, u8), indices: std::ops::RangeInclusive<u8>) -> u8 {
    indices
        .min_by_key(|&i| distance(rgb, index_rgb(i)))
        .expect("a non-empty palette range")
}

/// The nearest of the sixteen system colors to `rgb`.
#[must_use]
pub fn nearest_16(rgb: (u8, u8, u8)) -> u8 {
    nearest_in(rgb, 0..=15)
}

/// The nearest 256-palette color to `rgb`, from the cube and the grey
/// ramp only (`16..=255`).
#[must_use]
pub fn nearest_256(rgb: (u8, u8, u8)) -> u8 {
    nearest_in(rgb, 16..=255)
}

/// The fixed table that takes a 256-palette index to the sixteen: its
/// own value below 16, else the nearest of the sixteen to the color it
/// shows. The attach client's reduction at 16 (E10g.5).
#[must_use]
pub fn index_to_16(index: u8) -> u8 {
    if index < 16 {
        index
    } else {
        nearest_16(index_rgb(index))
    }
}

/// Reduce `color` to `depth` by steps 2 and 3 of the module's rule: an
/// index in range is its own, anything else the nearest. `Default`
/// stays `Default` at every depth.
#[must_use]
pub fn reduce(color: Color, depth: ColorDepth) -> Color {
    match (color, depth) {
        (Color::Default, _)
        | (_, ColorDepth::TrueColor)
        | (Color::Indexed(_), ColorDepth::TwoFiftySix) => color,
        (Color::Rgb(r, g, b), ColorDepth::TwoFiftySix) => Color::Indexed(nearest_256((r, g, b))),
        (Color::Rgb(r, g, b), ColorDepth::Sixteen) => Color::Indexed(nearest_16((r, g, b))),
        (Color::Indexed(n), ColorDepth::Sixteen) => Color::Indexed(index_to_16(n)),
    }
}

/// A style's three color components, named, for walking a face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    /// The foreground.
    Fg,
    /// The background.
    Bg,
    /// The underline color.
    UnderlineColor,
}

impl Component {
    /// All three, in a fixed order.
    pub const ALL: [Self; 3] = [Self::Fg, Self::Bg, Self::UnderlineColor];

    /// This component of `style`.
    #[must_use]
    pub fn of(self, style: &Style) -> Color {
        match self {
            Self::Fg => style.fg,
            Self::Bg => style.bg,
            Self::UnderlineColor => style.underline_color,
        }
    }

    fn of_depth(self, colors: DepthColors) -> Option<u8> {
        match self {
            Self::Fg => colors.fg,
            Self::Bg => colors.bg,
            Self::UnderlineColor => colors.underline_color,
        }
    }

    /// The name a theme's `fallback` table uses for this component.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fg => "fg",
            Self::Bg => "bg",
            Self::UnderlineColor => "underline_color",
        }
    }
}

/// Step 1 and the "own" half of step 2: the index a face's `component`
/// takes at `depth` without approximation, `None` when nothing was
/// authored and the main color is not already an index in range (or
/// when the component is `Default`, which has nothing to fall back).
/// What E10g.1's shipped-theme row asks of every face.
#[must_use]
pub fn authored_index(
    main: Color,
    fallbacks: &Fallbacks,
    component: Component,
    depth: ColorDepth,
) -> Option<u8> {
    let max = depth.max_index()?;
    let written = match depth {
        ColorDepth::Sixteen => component.of_depth(fallbacks.c16),
        _ => component.of_depth(fallbacks.c256),
    };
    match (written, main) {
        (_, Color::Default) => None,
        (Some(index), _) => Some(index),
        (None, Color::Indexed(n)) if n <= max => Some(n),
        (None, _) => None,
    }
}

/// What every color of a frame becomes at one depth, given the active
/// theme: each face's colors to their fallbacks (step 1), anything else
/// by [`reduce`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepthMap {
    depth: ColorDepth,
    /// Keyed by [`color_key`], since the wire's `Color` has no `Hash`.
    faces: HashMap<u32, Color>,
}

/// A color as a map key: `Default` has none (it is never mapped), an
/// RGB is `0x01RRGGBB`, an index `0x0200_00II`.
fn color_key(color: Color) -> Option<u32> {
    match color {
        Color::Default => None,
        Color::Rgb(r, g, b) => Some(0x0100_0000 | u32::from_be_bytes([0, r, g, b])),
        Color::Indexed(n) => Some(0x0200_0000 | u32::from(n)),
    }
}

impl DepthMap {
    /// The map for `theme` at `depth`. Entries are walked in name order,
    /// so where two give one color different fallbacks (which no shipped
    /// theme does, a row asserts) the first name wins, deterministically.
    #[must_use]
    pub fn for_theme(theme: &Theme, depth: ColorDepth) -> Self {
        let mut faces = HashMap::new();
        if depth == ColorDepth::TrueColor {
            return Self { depth, faces };
        }
        let mut names: Vec<&String> = theme.by_capture.keys().collect();
        names.sort_unstable();
        for name in names {
            let style = &theme.by_capture[name];
            let fallbacks = theme.fallbacks.get(name).copied().unwrap_or_default();
            for component in Component::ALL {
                let main = component.of(style);
                let Some(key) = color_key(main) else {
                    continue;
                };
                let target = authored_index(main, &fallbacks, component, depth)
                    .map_or_else(|| reduce(main, depth), Color::Indexed);
                faces.entry(key).or_insert(target);
            }
        }
        Self { depth, faces }
    }

    /// The depth this map reduces to.
    #[must_use]
    pub fn depth(&self) -> ColorDepth {
        self.depth
    }

    /// `color` at this map's depth.
    #[must_use]
    pub fn map(&self, color: Color) -> Color {
        color_key(color)
            .and_then(|key| self.faces.get(&key))
            .copied()
            .unwrap_or_else(|| reduce(color, self.depth))
    }

    /// `style` with each of its colors at this map's depth.
    #[must_use]
    pub fn map_style(&self, style: Style) -> Style {
        Style {
            fg: self.map(style.fg),
            bg: self.map(style.bg),
            underline_color: self.map(style.underline_color),
            ..style
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_matches_the_xterm_cube_and_ramp() {
        assert_eq!(index_rgb(16), (0, 0, 0));
        assert_eq!(index_rgb(231), (255, 255, 255));
        assert_eq!(index_rgb(24), (0, 95, 135));
        assert_eq!(index_rgb(232), (8, 8, 8));
        assert_eq!(index_rgb(255), (238, 238, 238));
    }

    #[test]
    fn nearest_256_never_answers_a_system_color() {
        for rgb in [(0, 0, 0), (255, 255, 255), (205, 49, 49), (18, 18, 18)] {
            assert!(nearest_256(rgb) >= 16, "{rgb:?}");
        }
        assert_eq!(nearest_256((0x12, 0x12, 0x12)), 233);
    }

    #[test]
    fn the_16_table_keeps_the_sixteen_and_sends_the_rest_to_the_nearest() {
        for i in 0..16 {
            assert_eq!(index_to_16(i), i);
        }
        assert_eq!(index_to_16(196), 1, "the cube's pure red is red");
        assert_eq!(index_to_16(236), 0, "a dark grey is black");
        assert_eq!(index_to_16(252), 7, "a light grey is white");
        for i in 16..=255 {
            assert!(index_to_16(i) < 16);
        }
    }

    #[test]
    fn reduce_keeps_default_and_in_range_indices() {
        for depth in [ColorDepth::Sixteen, ColorDepth::TwoFiftySix] {
            assert_eq!(reduce(Color::Default, depth), Color::Default);
            assert_eq!(reduce(Color::Indexed(13), depth), Color::Indexed(13));
        }
        assert_eq!(
            reduce(Color::Indexed(236), ColorDepth::TwoFiftySix),
            Color::Indexed(236)
        );
        assert_eq!(
            reduce(Color::Rgb(1, 2, 3), ColorDepth::TrueColor),
            Color::Rgb(1, 2, 3)
        );
    }

    #[test]
    fn an_authored_fallback_beats_the_nearest() {
        let mut theme = Theme::empty();
        theme.by_capture.insert(
            "ui.default".into(),
            Style {
                fg: Color::Rgb(0xd4, 0xd0, 0xc8),
                bg: Color::Rgb(0x12, 0x12, 0x12),
                ..Style::default()
            },
        );
        theme.fallbacks.insert(
            "ui.default".into(),
            Fallbacks {
                c256: DepthColors {
                    fg: Some(252),
                    bg: Some(233),
                    underline_color: None,
                },
                c16: DepthColors {
                    fg: Some(15),
                    bg: Some(0),
                    underline_color: None,
                },
            },
        );
        let at16 = DepthMap::for_theme(&theme, ColorDepth::Sixteen);
        assert_eq!(at16.map(Color::Rgb(0xd4, 0xd0, 0xc8)), Color::Indexed(15));
        assert_eq!(
            reduce(Color::Rgb(0xd4, 0xd0, 0xc8), ColorDepth::Sixteen),
            Color::Indexed(7),
            "the control: unauthored, the same color is the nearest, white"
        );
        let at256 = DepthMap::for_theme(&theme, ColorDepth::TwoFiftySix);
        assert_eq!(at256.map(Color::Rgb(0x12, 0x12, 0x12)), Color::Indexed(233));
        assert_eq!(
            at256.map(Color::Rgb(250, 0, 0)),
            Color::Indexed(196),
            "a color no face carries goes to the nearest"
        );
    }
}
