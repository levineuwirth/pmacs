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
    /// E10g.5: what a terminal shows, from its `TERM`: sixteen only
    /// where the name positively has no 256 colors --- `linux`, a `vt`
    /// and a model number (`vt100`, `vt220`), a bare `xterm` or
    /// `screen` --- and truecolor otherwise, which is what was always
    /// sent, wherever the name says nothing for certain: `COLORTERM`
    /// is not read, since `ssh` does not forward it by default and its
    /// absence proves nothing. A terminal that shows 256 is named so by
    /// `ui.color-depth`.
    #[must_use]
    pub fn detect(term: Option<&str>) -> Self {
        match term {
            Some("linux" | "xterm" | "screen") => Self::Sixteen,
            Some(t)
                if t.strip_prefix("vt")
                    .is_some_and(|model| model.starts_with(|c: char| c.is_ascii_digit())) =>
            {
                Self::Sixteen
            }
            _ => Self::TrueColor,
        }
    }

    /// [`Self::detect`] from this process's `TERM`.
    #[must_use]
    pub fn detect_env() -> Self {
        Self::detect(std::env::var("TERM").ok().as_deref())
    }

    /// `ui.color-depth`'s value as a ceiling: `None` for `auto` (and
    /// anything unknown), which leaves the detected depth alone.
    #[must_use]
    pub fn from_setting(value: &str) -> Option<Self> {
        match value {
            "truecolor" => Some(Self::TrueColor),
            "256" => Some(Self::TwoFiftySix),
            "16" => Some(Self::Sixteen),
            _ => None,
        }
    }

    /// The depth a terminal is painted at: the lower of the setting and
    /// what was detected.
    #[must_use]
    pub fn effective(setting: Option<Self>, detected: Self) -> Self {
        setting.map_or(detected, |s| s.min(detected))
    }

    /// The depth the daemon paints an attached terminal at: the lower of
    /// the setting and what its frontend detected, at the granularity
    /// the wire carries (`FrontendCapabilities.true_color`): truecolor,
    /// or 256 for anything lower. A client that detected 16 reduces the
    /// 256 itself ([`reduce_spans_to_16`]); an authored 16-color
    /// fallback on attach would need a message of its own.
    #[must_use]
    pub fn attached(setting: Option<Self>, true_color: bool) -> Self {
        let detected = if true_color {
            Self::TrueColor
        } else {
            Self::TwoFiftySix
        };
        match Self::effective(setting, detected) {
            Self::TrueColor => Self::TrueColor,
            _ => Self::TwoFiftySix,
        }
    }

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

/// E10g.5: a frame's spans as a 16-color terminal is sent them by a
/// frontend that knows no theme (the attach client): every index
/// `16..256` through [`index_to_16`], every RGB still arriving (from a
/// daemon that paints truecolor regardless) to the nearest of the
/// sixteen.
#[must_use]
pub fn reduce_spans_to_16(spans: &[crate::cell::DiffSpan]) -> Vec<crate::cell::DiffSpan> {
    spans
        .iter()
        .map(|span| crate::cell::DiffSpan {
            start: span.start,
            cells: span
                .cells
                .iter()
                .map(|cell| crate::cell::Cell {
                    style: Style {
                        fg: reduce(cell.style.fg, ColorDepth::Sixteen),
                        bg: reduce(cell.style.bg, ColorDepth::Sixteen),
                        underline_color: reduce(cell.style.underline_color, ColorDepth::Sixteen),
                        ..cell.style
                    },
                    ..cell.clone()
                })
                .collect(),
        })
        .collect()
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

/// E10g.6: one ink of a surface painted at 16 colors: a palette index,
/// or the terminal's own default foreground or background, which no
/// frame names, or an RGB a 16-color frame should not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// A palette index.
    Index(u8),
    /// The terminal's default foreground.
    DefaultFg,
    /// The terminal's default background.
    DefaultBg,
    /// An RGB at 16 colors: known to no rule here.
    Unknown,
}

/// A painted surface's `(foreground, background)` inks, reverse video
/// applied: a `Default` foreground is the terminal's default foreground
/// wherever reverse video then puts it, and likewise its background.
fn painted_inks(style: &Style) -> (Ink, Ink) {
    let ink = |c: Color, default: Ink| match c {
        Color::Indexed(n) => Ink::Index(n),
        Color::Default => default,
        Color::Rgb(..) => Ink::Unknown,
    };
    let (fg, bg) = (ink(style.fg, Ink::DefaultFg), ink(style.bg, Ink::DefaultBg));
    if style.reverse { (bg, fg) } else { (fg, bg) }
}

/// Whether two inks are known to differ (E10g.6 as ruled on 2026-10-28):
/// two unequal indices do. With `pair` (the second clause and the
/// `Indexed(0)` rule) the terminal's default foreground and background
/// also count as two different colors against each other; a default is
/// unknown against anything else, and without `pair` (the first clause)
/// a default is unknown against everything.
fn known_to_differ(a: Ink, b: Ink, pair: bool) -> bool {
    match (a, b) {
        (Ink::Index(x), Ink::Index(y)) => x != y,
        (Ink::DefaultFg, Ink::DefaultBg) | (Ink::DefaultBg, Ink::DefaultFg) => pair,
        _ => false,
    }
}

/// E10g.6's two clauses for chrome painted at 16 colors, each surface as
/// actually painted (`surfaces`, reverse video applied here), as the
/// owner ruled on 2026-10-28. The first, that a surface differs from the
/// `buffer` surface in foreground or background, applies only where the
/// theme sets `ui.default` (`buffer_face_set`), a default ink unknown in
/// it. The second, that a surface's foreground differs from its own
/// background, applies to every theme, with the terminal's default pair
/// counted as two different colors against each other and as unknown
/// against anything else, so `Indexed(0)` against a `Default` background
/// fails it. One line per failure, naming the surface, the clause and
/// the inks.
#[must_use]
pub fn legibility_violations(
    buffer: &Style,
    surfaces: &[(String, Style)],
    buffer_face_set: bool,
) -> Vec<String> {
    let (buffer_fg, buffer_bg) = painted_inks(buffer);
    let mut out = Vec::new();
    for (name, style) in surfaces {
        let (fg, bg) = painted_inks(style);
        if buffer_face_set
            && !known_to_differ(fg, buffer_fg, false)
            && !known_to_differ(bg, buffer_bg, false)
        {
            out.push(format!(
                "{name}: does not differ from the buffer surface ({fg:?}/{bg:?} against {buffer_fg:?}/{buffer_bg:?})"
            ));
        }
        if !known_to_differ(fg, bg, true) {
            out.push(format!(
                "{name}: its foreground does not differ from its background ({fg:?} on {bg:?})"
            ));
        }
    }
    out
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

    /// E10g.5's detection table: sixteen only where `TERM` positively
    /// has no 256 colors, truecolor wherever it is unsure.
    #[test]
    fn e10g5_detection_is_sixteen_only_where_term_positively_says_so() {
        for term in ["linux", "vt100", "vt220", "vt52", "xterm", "screen"] {
            assert_eq!(
                ColorDepth::detect(Some(term)),
                ColorDepth::Sixteen,
                "{term}"
            );
        }
        for term in [
            "xterm-256color",
            "screen-256color",
            "tmux-256color",
            "xterm-kitty",
            "alacritty",
            "foot",
            "vte-256color",
            "vt",
            "dumb",
            "",
        ] {
            assert_eq!(
                ColorDepth::detect(Some(term)),
                ColorDepth::TrueColor,
                "{term}"
            );
        }
        assert_eq!(ColorDepth::detect(None), ColorDepth::TrueColor, "unset");
    }

    /// E10g.5: the effective depth is the lower of the setting and the
    /// detection; an attached terminal's, as the daemon paints it, is
    /// truecolor or 256, the wire's granularity.
    #[test]
    fn e10g5_the_setting_caps_the_detection_at_the_wire_s_granularity() {
        use ColorDepth::{Sixteen, TrueColor, TwoFiftySix};
        let setting = |v| ColorDepth::from_setting(v);
        assert_eq!(setting("auto"), None);
        assert_eq!(ColorDepth::effective(setting("auto"), TrueColor), TrueColor);
        assert_eq!(
            ColorDepth::effective(setting("256"), TrueColor),
            TwoFiftySix
        );
        assert_eq!(ColorDepth::effective(setting("16"), TrueColor), Sixteen);
        assert_eq!(
            ColorDepth::effective(setting("truecolor"), Sixteen),
            Sixteen
        );
        assert_eq!(ColorDepth::attached(None, true), TrueColor);
        assert_eq!(ColorDepth::attached(None, false), TwoFiftySix);
        assert_eq!(ColorDepth::attached(setting("256"), true), TwoFiftySix);
        assert_eq!(
            ColorDepth::attached(setting("16"), true),
            TwoFiftySix,
            "the wire carries no 16"
        );
        assert_eq!(
            ColorDepth::attached(setting("truecolor"), false),
            TwoFiftySix
        );
    }

    /// E10g.5: the attach client's reduction at 16: an index past 15 by
    /// the fixed table, an RGB to the nearest of the sixteen, `Default`
    /// and the sixteen themselves untouched.
    #[test]
    fn e10g5_the_client_reduces_an_index_and_an_rgb_to_the_sixteen() {
        use crate::cell::{Cell, CellCoord, DiffSpan};
        let cell = |fg, bg| Cell {
            style: Style {
                fg,
                bg,
                ..Style::default()
            },
            ..Cell::default()
        };
        let spans = vec![DiffSpan {
            start: CellCoord::new(0, 0),
            cells: vec![
                cell(Color::Indexed(252), Color::Indexed(236)),
                cell(Color::Rgb(205, 49, 49), Color::Default),
                cell(Color::Indexed(13), Color::Rgb(0, 0, 0)),
            ],
        }];
        let reduced = reduce_spans_to_16(&spans);
        let styles: Vec<(Color, Color)> = reduced[0]
            .cells
            .iter()
            .map(|c| (c.style.fg, c.style.bg))
            .collect();
        assert_eq!(
            styles,
            vec![
                (Color::Indexed(7), Color::Indexed(0)),
                (Color::Indexed(1), Color::Default),
                (Color::Indexed(13), Color::Indexed(0)),
            ]
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
