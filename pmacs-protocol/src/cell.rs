//! Cell wire types — moved from `pmacs::cell` in session 1 of the
//! `pmacs-gpu` arc. The original `pmacs::cell` module keeps
//! `CellGrid` (borrowed-slice render surface) and `fn diff()`
//! (rendering helper) since those are instance-side rendering
//! machinery, not wire shapes; the data types below all travel on
//! the `InstanceMessage::CellDelta` wire and on the
//! `SemanticFrame` family's `StyleSpan` / `Decoration` shapes.

// ---------------------------------------------------------------------------
// Coordinates
// ---------------------------------------------------------------------------

/// Coordinate in the cell grid (row, col), measured in cells.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CellCoord {
    /// 0-based row.
    pub row: u32,
    /// 0-based column.
    pub col: u32,
}

impl CellCoord {
    /// Construct a cell coordinate.
    #[must_use]
    pub const fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }
}

/// Dimensions of a cell grid, measured in cells.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CellSize {
    /// Number of rows.
    pub rows: u32,
    /// Number of columns.
    pub cols: u32,
}

impl CellSize {
    /// Construct a cell size.
    #[must_use]
    pub const fn new(rows: u32, cols: u32) -> Self {
        Self { rows, cols }
    }

    /// Number of cells in the grid (`rows * cols`).
    #[must_use]
    pub const fn area(self) -> u32 {
        self.rows * self.cols
    }
}

// ---------------------------------------------------------------------------
// Cell content
// ---------------------------------------------------------------------------

/// A glyph in a cell.
///
/// `Char` is the common case (single Unicode codepoint, single column).
/// `Cluster` carries a UTF-8 grapheme cluster spanning multiple codepoints
/// (e.g. emoji with modifiers, combining characters). `Continuation` is the
/// trailing column of a wide character: it has no glyph of its own; the
/// preceding cell's glyph occupies both columns.
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Glyph {
    /// A single Unicode codepoint occupying one column.
    Char(char),
    /// A grapheme cluster (one or more codepoints, encoded as UTF-8).
    Cluster(Box<[u8]>),
    /// The trailing column of a wide character. The preceding cell's glyph
    /// renders into both columns; this cell's `glyph` and `style` are
    /// ignored by frontends.
    Continuation,
}

impl Default for Glyph {
    fn default() -> Self {
        Self::Char(' ')
    }
}

/// A 24-bit RGB color, plus a `Default` sentinel meaning "use terminal
/// foreground/background".
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Color {
    /// Use the terminal's default foreground or background.
    #[default]
    Default,
    /// Truecolor RGB.
    Rgb(u8, u8, u8),
    /// 8-bit indexed terminal color (0..=255).
    Indexed(u8),
}

/// Underline style.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum UnderlineStyle {
    /// No underline.
    #[default]
    None,
    /// Single straight underline.
    Single,
    /// Double underline.
    Double,
    /// Curly (wavy) underline, typical for diagnostics.
    Curly,
    /// Dotted underline.
    Dotted,
    /// Dashed underline.
    Dashed,
}

/// Visual style applied to a cell.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Style {
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: UnderlineStyle,
    /// Reverse video.
    pub reverse: bool,
    /// Underline color (SGR 58/59). `Color::Default` means "follow
    /// the text color": the underline draws in `fg`. Diagnostics set
    /// this per severity so the squiggle color can differ from the
    /// syntax-colored text it underlines (T M4.6, protocol v6).
    pub underline_color: Color,
}

/// A non-text attachment carried in a cell (TUI ignores this).
///
/// The TUI backend never inspects `Attachment`; a GUI backend interprets it
/// to render images, embedded widgets, and the like.
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Attachment {
    /// One cell of an image. The image is identified by `image_id` and the
    /// cell's location within the image is `(sub_x, sub_y)`.
    ImageCell {
        /// Identifier into the frontend's image registry.
        image_id: u32,
        /// Sub-cell X offset.
        sub_x: u16,
        /// Sub-cell Y offset.
        sub_y: u16,
    },
}

/// One cell in the grid.
#[derive(Clone, Eq, PartialEq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    /// What is drawn in the cell.
    pub glyph: Glyph,
    /// How it is drawn.
    pub style: Style,
    /// Frontend-specific attachment (ignored by the TUI).
    pub attachment: Option<Attachment>,
}

// ---------------------------------------------------------------------------
// Diff span (wire shape for `InstanceMessage::CellDelta`)
// ---------------------------------------------------------------------------

/// A run of changed cells starting at one position.
///
/// Frontend translation: emit one cursor-move escape and then write the
/// cells in order. Wide characters appear as a leading `Char(_)` followed
/// by a [`Glyph::Continuation`] in the same span; the frontend consumes
/// both cells but only emits the leading glyph (the terminal handles the
/// width).
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiffSpan {
    /// First cell of the span.
    pub start: CellCoord,
    /// New contents of the cells in the span, in row-major order. The
    /// span occupies a contiguous run on `start.row`.
    pub cells: Vec<Cell>,
}
