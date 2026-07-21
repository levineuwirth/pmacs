use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    MAX_TERMINAL_COLS, MAX_TERMINAL_GRAPHEME_BYTES, MAX_TERMINAL_HISTORY_CELLS,
    MAX_TERMINAL_METADATA_BYTES, MAX_TERMINAL_ROWS, MAX_TERMINAL_VISIBLE_CELLS,
};
use crate::ansi::{
    AlternateScreenMode, AnsiEvent, CharacterSet, CharacterSetSlot, DeviceRequest, EraseMode,
    TerminalMode,
};
use crate::cell::{Cell, CellCoord, CellSize, Glyph, Style};

/// Maximum time a child may hold synchronized-output publication.
pub const SYNCHRONIZED_OUTPUT_WATCHDOG: Duration = Duration::from_secs(1);

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenError {
    InvalidSize(CellSize),
}

impl std::fmt::Display for ScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSize(size) => {
                write!(f, "invalid terminal size {}x{}", size.rows, size.cols)
            }
        }
    }
}

impl std::error::Error for ScreenError {}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseTrackingMode {
    Off,
    X10,
    Button,
    Any,
}

#[allow(missing_docs, clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalModes {
    pub insert: bool,
    pub origin: bool,
    pub autowrap: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub cursor_visible: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub synchronized_output: bool,
    pub mouse_tracking: MouseTrackingMode,
    pub mouse_sgr: bool,
}

impl Default for TerminalModes {
    fn default() -> Self {
        Self {
            insert: false,
            origin: false,
            autowrap: true,
            application_cursor: false,
            application_keypad: false,
            cursor_visible: true,
            bracketed_paste: false,
            focus_reporting: false,
            synchronized_output: false,
            mouse_tracking: MouseTrackingMode::Off,
            mouse_sgr: false,
        }
    }
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRow {
    pub cells: Vec<Cell>,
    pub logical_line_id: u64,
    pub cell_offset: u32,
    pub soft_wrapped: bool,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenSnapshot {
    pub size: CellSize,
    pub cells: Vec<Cell>,
    pub cursor: Option<CellCoord>,
    pub title: Option<String>,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    row: usize,
    col: usize,
    pending_wrap: bool,
}

#[derive(Clone, Copy, Debug)]
struct SavedCursor {
    cursor: Cursor,
    style: Style,
    g0: CharacterSet,
    g1: CharacterSet,
    use_g1: bool,
}

#[derive(Clone, Debug)]
struct Grid {
    rows: Vec<TerminalRow>,
    history: VecDeque<TerminalRow>,
}

#[allow(missing_docs, clippy::struct_excessive_bools)]
pub struct TerminalScreen {
    size: CellSize,
    main: Grid,
    alt: Grid,
    alt_active: bool,
    cursor: Cursor,
    saved_main_cursor: Option<SavedCursor>,
    saved_alt_cursor: Option<SavedCursor>,
    saved_main_1049: Option<SavedCursor>,
    inactive_main_cursor: Cursor,
    inactive_alt_cursor: Cursor,
    style: Style,
    modes: TerminalModes,
    scroll_top: usize,
    scroll_bottom: usize,
    tab_stops: BTreeSet<usize>,
    title: Option<String>,
    generation: u64,
    published: ScreenSnapshot,
    sync_started: Option<Instant>,
    next_line_id: u64,
    scrollback_rows: usize,
    g0: CharacterSet,
    g1: CharacterSet,
    use_g1: bool,
    bell_count: u64,
    mouse_x10: bool,
    mouse_button: bool,
    mouse_any: bool,
    last_grapheme: Option<(usize, usize)>,
}

#[allow(missing_docs)]
impl TerminalScreen {
    pub fn new(size: CellSize, scrollback_rows: usize) -> Result<Self, ScreenError> {
        validate_size(size)?;
        let mut next_line_id = 1;
        let main = Grid::new(size, &mut next_line_id);
        let alt = Grid::new(size, &mut next_line_id);
        let published = ScreenSnapshot {
            size,
            cells: flatten(&main.rows),
            cursor: Some(CellCoord::new(0, 0)),
            title: None,
            generation: 0,
        };
        Ok(Self {
            size,
            main,
            alt,
            alt_active: false,
            cursor: Cursor::default(),
            saved_main_cursor: None,
            saved_alt_cursor: None,
            saved_main_1049: None,
            inactive_main_cursor: Cursor::default(),
            inactive_alt_cursor: Cursor::default(),
            style: Style::default(),
            modes: TerminalModes::default(),
            scroll_top: 0,
            scroll_bottom: size.rows as usize - 1,
            tab_stops: default_tab_stops(size.cols as usize),
            title: None,
            generation: 0,
            published,
            sync_started: None,
            next_line_id,
            scrollback_rows,
            g0: CharacterSet::Ascii,
            g1: CharacterSet::Ascii,
            use_g1: false,
            bell_count: 0,
            mouse_x10: false,
            mouse_button: false,
            mouse_any: false,
            last_grapheme: None,
        })
    }

    /// Apply one parsed terminal operation and return an optional fixed device
    /// reply for the session manager to queue to the child.
    #[allow(clippy::too_many_lines, clippy::let_and_return, clippy::cast_lossless)]
    pub fn apply_event(&mut self, event: AnsiEvent) -> Option<Vec<u8>> {
        if !matches!(&event, AnsiEvent::Text(_)) {
            self.last_grapheme = None;
        }
        let reply = match event {
            AnsiEvent::Text(text) => {
                self.write_text(&text);
                None
            }
            AnsiEvent::SetStyle(style) => {
                self.style = style;
                self.changed();
                None
            }
            AnsiEvent::CarriageReturn => {
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
                self.changed();
                None
            }
            AnsiEvent::Backspace => {
                self.cursor.col = self.cursor.col.saturating_sub(1);
                self.cursor.pending_wrap = false;
                self.changed();
                None
            }
            AnsiEvent::Bell => {
                self.bell_count = self.bell_count.saturating_add(1);
                self.changed();
                None
            }
            AnsiEvent::LineFeed | AnsiEvent::Index => {
                self.line_feed();
                None
            }
            AnsiEvent::NextLine => {
                self.cursor.col = 0;
                self.line_feed();
                None
            }
            AnsiEvent::ReverseIndex => {
                self.reverse_index();
                None
            }
            AnsiEvent::HorizontalTab => {
                self.horizontal_tab();
                None
            }
            AnsiEvent::SetTabStop => {
                self.tab_stops.insert(self.cursor.col);
                self.changed();
                None
            }
            AnsiEvent::ClearTabStop => {
                self.tab_stops.remove(&self.cursor.col);
                self.changed();
                None
            }
            AnsiEvent::ClearAllTabStops => {
                self.tab_stops.clear();
                self.changed();
                None
            }
            AnsiEvent::CursorUp(n) => {
                self.move_vertical(-i64::from(n));
                None
            }
            AnsiEvent::CursorDown(n) => {
                self.move_vertical(i64::from(n));
                None
            }
            AnsiEvent::CursorForward(n) => {
                self.move_horizontal(i64::from(n));
                None
            }
            AnsiEvent::CursorBackward(n) => {
                self.move_horizontal(-i64::from(n));
                None
            }
            AnsiEvent::CursorNextLine(n) => {
                self.move_vertical(i64::from(n));
                self.cursor.col = 0;
                None
            }
            AnsiEvent::CursorPreviousLine(n) => {
                self.move_vertical(-i64::from(n));
                self.cursor.col = 0;
                None
            }
            AnsiEvent::CursorHorizontalAbsolute(col) => {
                self.set_col(col);
                None
            }
            AnsiEvent::CursorVerticalAbsolute(row) => {
                self.set_row(row);
                None
            }
            AnsiEvent::CursorPosition { row, col } => {
                self.set_position(row, col);
                None
            }
            AnsiEvent::EraseDisplay(mode) => {
                self.erase_display(mode);
                None
            }
            AnsiEvent::EraseLineMode(mode) => {
                self.erase_line(mode);
                None
            }
            AnsiEvent::EraseCharacters(n) => {
                self.erase_characters(n);
                None
            }
            AnsiEvent::InsertCharacters(n) => {
                self.insert_characters(n);
                None
            }
            AnsiEvent::DeleteCharacters(n) => {
                self.delete_characters(n);
                None
            }
            AnsiEvent::InsertLines(n) => {
                self.insert_lines(n);
                None
            }
            AnsiEvent::DeleteLines(n) => {
                self.delete_lines(n);
                None
            }
            AnsiEvent::ScrollUp(n) => {
                self.scroll_up(n as usize);
                None
            }
            AnsiEvent::ScrollDown(n) => {
                self.scroll_down(n as usize);
                None
            }
            AnsiEvent::SetScrollingRegion { top, bottom } => {
                self.set_scrolling_region(top, bottom);
                None
            }
            AnsiEvent::SaveCursor => {
                let saved = Some(self.capture_cursor());
                if self.alt_active {
                    self.saved_alt_cursor = saved;
                } else {
                    self.saved_main_cursor = saved;
                }
                None
            }
            AnsiEvent::RestoreCursor => {
                let saved = if self.alt_active {
                    self.saved_alt_cursor
                } else {
                    self.saved_main_cursor
                };
                if let Some(saved) = saved {
                    self.restore_cursor(saved);
                }
                None
            }
            AnsiEvent::AlternateScreen { mode, enabled } => {
                self.set_alternate(mode, enabled);
                None
            }
            AnsiEvent::SetMode { mode, enabled } => {
                self.set_mode(mode, enabled);
                None
            }
            AnsiEvent::DesignateCharacterSet { slot, charset } => {
                match slot {
                    CharacterSetSlot::G0 => self.g0 = charset,
                    CharacterSetSlot::G1 => self.g1 = charset,
                }
                self.changed();
                None
            }
            AnsiEvent::ShiftOut => {
                self.use_g1 = true;
                self.changed();
                None
            }
            AnsiEvent::ShiftIn => {
                self.use_g1 = false;
                self.changed();
                None
            }
            AnsiEvent::DeviceRequest(request) => Some(self.device_reply(request)),
            AnsiEvent::SetTitle(title) => {
                self.title = Some(sanitize_title(&title));
                self.changed();
                None
            }
            AnsiEvent::EraseToEol => {
                self.erase_line(EraseMode::ToEnd);
                None
            }
            AnsiEvent::EraseLine => {
                self.erase_line(EraseMode::All);
                None
            }
            AnsiEvent::AlternateScreenEnter => {
                self.set_alternate(AlternateScreenMode::Mode1049, true);
                None
            }
            AnsiEvent::AlternateScreenExit => {
                self.set_alternate(AlternateScreenMode::Mode1049, false);
                None
            }
            AnsiEvent::BracketedPasteBegin
            | AnsiEvent::BracketedPasteEnd
            | AnsiEvent::PromptStart
            | AnsiEvent::PromptEnd
            | AnsiEvent::CommandStart
            | AnsiEvent::OutputStart => None,
        };
        reply
    }

    /// Release a synchronized-output batch at EOF or session completion.
    pub fn finish_output(&mut self) {
        if self.modes.synchronized_output {
            self.modes.synchronized_output = false;
            self.sync_started = None;
            self.publish();
        }
    }

    pub fn synchronized_watchdog_expired(&mut self, now: Instant) -> bool {
        if self.sync_started.is_some_and(|start| {
            now.saturating_duration_since(start) >= SYNCHRONIZED_OUTPUT_WATCHDOG
        }) {
            self.modes.synchronized_output = false;
            self.sync_started = None;
            self.publish();
            true
        } else {
            false
        }
    }

    pub fn append_process_annotation(&mut self, annotation: &str) {
        self.last_grapheme = None;
        let needs_newline = self.cursor.col != 0
            || self.cursor.pending_wrap
            || self.active().rows[self.cursor.row]
                .cells
                .iter()
                .any(|c| !is_blank(c));
        self.style = Style::default();
        if needs_newline {
            self.cursor.pending_wrap = false;
            self.cursor.col = 0;
            self.line_feed();
        }
        self.write_text(annotation);
        self.cursor.pending_wrap = false;
        let row = self.cursor.row;
        self.active_mut().rows[row].soft_wrapped = false;
        self.break_chain_after(row);
        self.finish_output();
    }

    /// Resize the main screen with soft-wrap reflow and clip/pad the alternate
    /// screen, preserving cursor, history, and application tab-stop state.
    pub fn resize(&mut self, size: CellSize) -> Result<(), ScreenError> {
        validate_size(size)?;
        if size == self.size {
            return Ok(());
        }
        self.last_grapheme = None;
        let old_size = self.size;
        self.reflow_main(size);
        resize_grid_clip(
            &mut self.alt,
            old_size,
            size,
            &mut self.next_line_id,
            self.style,
        );
        self.size = size;
        self.scroll_top = 0;
        self.scroll_bottom = size.rows as usize - 1;
        self.cursor.row = self.cursor.row.min(self.scroll_bottom);
        self.cursor.col = self.cursor.col.min(size.cols as usize - 1);
        self.cursor.pending_wrap = false;
        self.tab_stops.retain(|&col| col < size.cols as usize);
        if size.cols > old_size.cols {
            let first_new_default = (old_size.cols as usize).div_ceil(8) * 8;
            for col in (first_new_default..size.cols as usize).step_by(8) {
                self.tab_stops.insert(col);
            }
        }
        self.enforce_history_budget();
        self.changed();
        Ok(())
    }

    pub fn snapshot(&self) -> ScreenSnapshot {
        if self.modes.synchronized_output {
            self.published.clone()
        } else {
            self.current_snapshot()
        }
    }

    pub fn modes(&self) -> TerminalModes {
        self.modes
    }
    pub fn bell_count(&self) -> u64 {
        self.bell_count
    }
    pub fn history(&self) -> &VecDeque<TerminalRow> {
        &self.main.history
    }
    pub fn visible_rows(&self) -> &[TerminalRow] {
        &self.active().rows
    }

    fn active(&self) -> &Grid {
        if self.alt_active {
            &self.alt
        } else {
            &self.main
        }
    }
    fn active_mut(&mut self) -> &mut Grid {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.main
        }
    }

    fn capture_cursor(&self) -> SavedCursor {
        SavedCursor {
            cursor: self.cursor,
            style: self.style,
            g0: self.g0,
            g1: self.g1,
            use_g1: self.use_g1,
        }
    }

    fn restore_cursor(&mut self, saved: SavedCursor) {
        self.cursor = saved.cursor;
        self.cursor.row = self.cursor.row.min(self.size.rows as usize - 1);
        self.cursor.col = self.cursor.col.min(self.size.cols as usize - 1);
        self.cursor.pending_wrap = false;
        self.style = saved.style;
        self.g0 = saved.g0;
        self.g1 = saved.g1;
        self.use_g1 = saved.use_g1;
        self.changed();
    }

    fn write_text(&mut self, text: &str) {
        for ch in text.chars() {
            let ch = self.map_character(ch);
            if ch.is_control() {
                continue;
            }
            if self.try_extend_previous_grapheme(ch) {
                continue;
            }
            let width = UnicodeWidthChar::width(ch).unwrap_or(0).min(2);
            if width == 0 {
                self.write_leading_combining(ch);
            } else {
                self.write_character(ch, width);
            }
        }
    }

    fn try_extend_previous_grapheme(&mut self, ch: char) -> bool {
        let Some((row, lead)) = self.last_grapheme else {
            return false;
        };
        let previous = glyph_bytes(&self.active().rows[row].cells[lead].glyph);
        let Ok(previous_text) = std::str::from_utf8(&previous) else {
            return false;
        };
        let mut encoded = [0; 4];
        let addition = ch.encode_utf8(&mut encoded);
        let mut candidate = String::with_capacity(previous.len() + addition.len());
        candidate.push_str(previous_text);
        candidate.push_str(addition);
        let mut graphemes = candidate.graphemes(true);
        let joins_previous =
            graphemes.next() == Some(candidate.as_str()) && graphemes.next().is_none();
        if !joins_previous {
            return false;
        }
        if candidate.len() > MAX_TERMINAL_GRAPHEME_BYTES {
            return true;
        }
        self.replace_previous_grapheme(row, lead, candidate);
        true
    }

    fn replace_previous_grapheme(&mut self, row: usize, lead: usize, cluster: String) {
        let cols = self.size.cols as usize;
        let width = UnicodeWidthStr::width(cluster.as_str()).clamp(1, 2);
        let old_width = glyph_width(&self.active().rows[row].cells[lead].glyph);
        let style = self.active().rows[row].cells[lead].style;
        if width == 2 && cols == 1 {
            self.active_mut().rows[row].cells[lead] = cell(Glyph::Char('\u{fffd}'), style);
            self.cursor.row = row;
            self.cursor.col = 0;
            self.cursor.pending_wrap = self.modes.autowrap;
            self.last_grapheme = Some((row, lead));
            self.changed();
            return;
        }
        if width == 2 && lead + 1 >= cols && !self.modes.autowrap {
            self.active_mut().rows[row].cells[lead] = cell(Glyph::Char('\u{fffd}'), style);
            self.cursor.row = row;
            self.cursor.col = lead;
            self.cursor.pending_wrap = false;
            self.last_grapheme = Some((row, lead));
            self.changed();
            return;
        }
        if width == 2 && lead + 1 >= cols {
            self.active_mut().rows[row].cells[lead] = blank(self.style);
            self.cursor.row = row;
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = true;
            self.soft_wrap();
            let new_row = self.cursor.row;
            self.active_mut().rows[new_row].cells[0] = cell(
                Glyph::Cluster(cluster.into_bytes().into_boxed_slice()),
                style,
            );
            self.active_mut().rows[new_row].cells[1] = cell(Glyph::Continuation, style);
            if cols == 2 {
                self.cursor.col = 1;
                self.cursor.pending_wrap = self.modes.autowrap;
            } else {
                self.cursor.col = 2;
                self.cursor.pending_wrap = false;
            }
            self.last_grapheme = Some((new_row, 0));
            self.changed();
            return;
        }
        self.active_mut().rows[row].cells[lead] = cell(
            Glyph::Cluster(cluster.into_bytes().into_boxed_slice()),
            style,
        );
        if width == 2 {
            self.active_mut().rows[row].cells[lead + 1] = cell(Glyph::Continuation, style);
        } else if old_width == 2 && lead + 1 < cols {
            self.active_mut().rows[row].cells[lead + 1] = blank(self.style);
        }
        self.cursor.row = row;
        if lead + width >= cols {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = lead + width;
            self.cursor.pending_wrap = false;
        }
        self.last_grapheme = Some((row, lead));
        self.changed();
    }

    fn write_leading_combining(&mut self, ch: char) {
        let row = self.cursor.row;
        let col = self.cursor.col;
        self.clear_wide_at(row, col);
        let mut cluster = String::from(" ");
        cluster.push(ch);
        if cluster.len() <= MAX_TERMINAL_GRAPHEME_BYTES {
            self.active_mut().rows[row].cells[col] = cell(
                Glyph::Cluster(cluster.into_bytes().into_boxed_slice()),
                self.style,
            );
            self.last_grapheme = Some((row, col));
            self.changed();
        }
    }

    fn map_character(&self, ch: char) -> char {
        let charset = if self.use_g1 { self.g1 } else { self.g0 };
        if charset == CharacterSet::DecSpecialGraphics {
            dec_special_graphics(ch)
        } else {
            ch
        }
    }

    fn write_character(&mut self, mut ch: char, mut width: usize) {
        let cols = self.size.cols as usize;
        if width == 2 && cols == 1 {
            ch = '\u{fffd}';
            width = 1;
        }
        if width == 2 && self.cursor.col + 1 >= cols && !self.modes.autowrap {
            ch = '\u{fffd}';
            width = 1;
        } else if self.cursor.pending_wrap || (width == 2 && self.cursor.col + 1 >= cols) {
            self.soft_wrap();
        }
        if self.modes.insert {
            self.insert_characters(width as u32);
        }
        self.clear_wide_at(self.cursor.row, self.cursor.col);
        if width == 2 {
            self.clear_wide_at(self.cursor.row, self.cursor.col + 1);
        }
        let style = self.style;
        let row = self.cursor.row;
        let col = self.cursor.col;
        let grid = self.active_mut();
        grid.rows[row].cells[col] = cell(Glyph::Char(ch), style);
        if width == 2 {
            grid.rows[row].cells[col + 1] = cell(Glyph::Continuation, style);
        }
        if col + width >= cols {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col += width;
            self.cursor.pending_wrap = false;
        }
        self.last_grapheme = Some((row, col));
        self.changed();
    }

    fn soft_wrap(&mut self) {
        let row = self.cursor.row;
        let id = self.active().rows[row].logical_line_id;
        let offset = self.active().rows[row].cell_offset + self.size.cols;
        self.active_mut().rows[row].soft_wrapped = true;
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
        if row == self.scroll_bottom {
            self.scroll_up_internal(1, Some((id, offset)));
        } else {
            self.cursor.row += 1;
            let next_row = self.cursor.row;
            let next = &mut self.active_mut().rows[next_row];
            next.logical_line_id = id;
            next.cell_offset = offset;
        }
        self.changed();
    }

    fn line_feed(&mut self) {
        self.cursor.pending_wrap = false;
        let row = self.cursor.row;
        self.active_mut().rows[row].soft_wrapped = false;
        self.break_chain_after(row);
        if row == self.scroll_bottom {
            self.scroll_up_internal(1, None);
        } else if row + 1 < self.size.rows as usize {
            self.cursor.row += 1;
        }
        self.changed();
    }

    fn reverse_index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.scroll_top {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.changed();
        }
    }

    fn horizontal_tab(&mut self) {
        let cols = self.size.cols as usize;
        self.cursor.col = self
            .tab_stops
            .range((self.cursor.col + 1)..)
            .next()
            .copied()
            .unwrap_or(cols - 1);
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn move_vertical(&mut self, delta: i64) {
        let (lo, hi) = if self.modes.origin {
            (self.scroll_top, self.scroll_bottom)
        } else {
            (0, self.size.rows as usize - 1)
        };
        let distance = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
        let moved = if delta.is_negative() {
            self.cursor.row.saturating_sub(distance)
        } else {
            self.cursor.row.saturating_add(distance)
        };
        self.cursor.row = moved.clamp(lo, hi);
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn move_horizontal(&mut self, delta: i64) {
        let distance = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
        let moved = if delta.is_negative() {
            self.cursor.col.saturating_sub(distance)
        } else {
            self.cursor.col.saturating_add(distance)
        };
        self.cursor.col = moved.min(self.size.cols as usize - 1);
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn set_col(&mut self, col: u32) {
        self.cursor.col = col.saturating_sub(1).min(self.size.cols - 1) as usize;
        self.cursor.pending_wrap = false;
        self.changed();
    }
    fn set_row(&mut self, row: u32) {
        let base = if self.modes.origin {
            self.scroll_top
        } else {
            0
        };
        let hi = if self.modes.origin {
            self.scroll_bottom
        } else {
            self.size.rows as usize - 1
        };
        self.cursor.row = (base + row.saturating_sub(1) as usize).min(hi);
        self.cursor.pending_wrap = false;
        self.changed();
    }
    fn set_position(&mut self, row: u32, col: u32) {
        self.set_row(row);
        self.set_col(col);
    }

    fn erase_display(&mut self, mode: EraseMode) {
        match mode {
            EraseMode::ToEnd => {
                self.erase_line(EraseMode::ToEnd);
                for row in self.cursor.row + 1..self.size.rows as usize {
                    self.clear_row(row);
                }
            }
            EraseMode::ToStart => {
                for row in 0..self.cursor.row {
                    self.clear_row(row);
                }
                self.erase_line(EraseMode::ToStart);
            }
            EraseMode::All => {
                for row in 0..self.size.rows as usize {
                    self.clear_row(row);
                }
            }
            EraseMode::Saved => {
                if !self.alt_active {
                    self.main.history.clear();
                }
            }
        }
        self.changed();
    }

    fn erase_line(&mut self, mode: EraseMode) {
        let cols = self.size.cols as usize;
        match mode {
            EraseMode::All | EraseMode::Saved => self.clear_row(self.cursor.row),
            EraseMode::ToEnd => self.clear_range(self.cursor.row, self.cursor.col, cols),
            EraseMode::ToStart => self.clear_range(self.cursor.row, 0, self.cursor.col + 1),
        }
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn erase_characters(&mut self, n: u32) {
        let end = (self.cursor.col + n.max(1) as usize).min(self.size.cols as usize);
        self.clear_range(self.cursor.row, self.cursor.col, end);
        self.changed();
    }

    fn insert_characters(&mut self, n: u32) {
        let cols = self.size.cols as usize;
        let cursor_col = self.cursor.col;
        let cursor_row = self.cursor.row;
        let style = self.style;
        let n = (n.max(1) as usize).min(cols - cursor_col);
        self.normalize_row(cursor_row);
        let row = &mut self.active_mut().rows[cursor_row].cells;
        row[cursor_col..].rotate_right(n);
        row[cursor_col..cursor_col + n].fill(blank(style));
        sanitize_row(row, style);
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn delete_characters(&mut self, n: u32) {
        let cols = self.size.cols as usize;
        let cursor_col = self.cursor.col;
        let cursor_row = self.cursor.row;
        let style = self.style;
        let n = (n.max(1) as usize).min(cols - cursor_col);
        self.normalize_row(cursor_row);
        let row = &mut self.active_mut().rows[cursor_row].cells;
        row[cursor_col..].rotate_left(n);
        row[cols - n..].fill(blank(style));
        sanitize_row(row, style);
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn insert_lines(&mut self, n: u32) {
        if self.cursor.row < self.scroll_top || self.cursor.row > self.scroll_bottom {
            return;
        }
        let bottom = self.scroll_bottom;
        let cursor_row = self.cursor.row;
        let count = (n.max(1) as usize).min(bottom - cursor_row + 1);
        for _ in 0..count {
            self.active_mut().rows.remove(bottom);
            let row = self.new_row();
            self.active_mut().rows.insert(cursor_row, row);
        }
        self.changed();
    }

    fn delete_lines(&mut self, n: u32) {
        if self.cursor.row < self.scroll_top || self.cursor.row > self.scroll_bottom {
            return;
        }
        let bottom = self.scroll_bottom;
        let cursor_row = self.cursor.row;
        let count = (n.max(1) as usize).min(bottom - cursor_row + 1);
        for _ in 0..count {
            self.active_mut().rows.remove(cursor_row);
            let row = self.new_row();
            self.active_mut().rows.insert(bottom, row);
        }
        self.changed();
    }

    fn scroll_up(&mut self, n: usize) {
        let cursor = self.cursor;
        self.scroll_up_internal(n.max(1), None);
        self.cursor = cursor;
        self.changed();
    }
    fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let count = n.max(1).min(bottom - top + 1);
        for _ in 0..count {
            self.active_mut().rows.remove(bottom);
            let row = self.new_row();
            self.active_mut().rows.insert(top, row);
        }
        self.changed();
    }

    fn scroll_up_internal(&mut self, n: usize, continuation: Option<(u64, u32)>) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let count = n.min(bottom - top + 1);
        for index in 0..count {
            let removed = self.active_mut().rows.remove(top);
            if !self.alt_active && top == 0 && bottom + 1 == self.size.rows as usize {
                self.main.history.push_back(removed);
            }
            let mut row = self.new_row();
            if index + 1 == count
                && let Some((id, offset)) = continuation
            {
                row.logical_line_id = id;
                row.cell_offset = offset;
            }
            self.active_mut().rows.insert(bottom, row);
        }
        self.cursor.row = bottom;
        self.enforce_history_budget();
    }

    fn set_scrolling_region(&mut self, top: u32, bottom: Option<u32>) {
        let top = top.saturating_sub(1) as usize;
        let bottom = bottom.unwrap_or(self.size.rows).saturating_sub(1) as usize;
        if top < bottom && bottom < self.size.rows as usize {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.cursor.row = if self.modes.origin { top } else { 0 };
            self.cursor.col = 0;
            self.cursor.pending_wrap = false;
            self.changed();
        }
    }

    fn set_alternate(&mut self, mode: AlternateScreenMode, enabled: bool) {
        if enabled == self.alt_active {
            return;
        }
        if enabled {
            self.inactive_main_cursor = self.cursor;
            if mode == AlternateScreenMode::Mode1049 {
                self.saved_main_1049 = Some(self.capture_cursor());
                self.alt = Grid::new(self.size, &mut self.next_line_id);
                self.cursor = Cursor::default();
            } else if mode == AlternateScreenMode::Mode1047 {
                self.alt = Grid::new(self.size, &mut self.next_line_id);
                self.cursor = Cursor::default();
            } else {
                self.cursor = self.inactive_alt_cursor;
            }
            self.alt_active = true;
        } else {
            self.inactive_alt_cursor = self.cursor;
            self.alt_active = false;
            if mode == AlternateScreenMode::Mode1049 {
                if let Some(saved) = self.saved_main_1049.take() {
                    self.restore_cursor(saved);
                } else {
                    self.cursor = self.inactive_main_cursor;
                }
            } else {
                self.cursor = self.inactive_main_cursor;
            }
        }
        self.scroll_top = 0;
        self.scroll_bottom = self.size.rows as usize - 1;
        self.cursor.row = self.cursor.row.min(self.scroll_bottom);
        self.cursor.col = self.cursor.col.min(self.size.cols as usize - 1);
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn set_mode(&mut self, mode: TerminalMode, enabled: bool) {
        match mode {
            TerminalMode::Insert => self.modes.insert = enabled,
            TerminalMode::Origin => {
                self.modes.origin = enabled;
                self.cursor.row = if enabled { self.scroll_top } else { 0 };
                self.cursor.col = 0;
            }
            TerminalMode::AutoWrap => self.modes.autowrap = enabled,
            TerminalMode::ApplicationCursor => self.modes.application_cursor = enabled,
            TerminalMode::ApplicationKeypad => self.modes.application_keypad = enabled,
            TerminalMode::CursorVisible => self.modes.cursor_visible = enabled,
            TerminalMode::BracketedPaste => self.modes.bracketed_paste = enabled,
            TerminalMode::FocusReporting => self.modes.focus_reporting = enabled,
            TerminalMode::SynchronizedOutput => {
                if enabled && !self.modes.synchronized_output {
                    self.publish();
                    self.sync_started = Some(Instant::now());
                }
                self.modes.synchronized_output = enabled;
                if !enabled {
                    self.sync_started = None;
                    self.publish();
                }
            }
            TerminalMode::MouseX10 => {
                self.mouse_x10 = enabled;
                self.resolve_mouse_tracking();
            }
            TerminalMode::MouseButton => {
                self.mouse_button = enabled;
                self.resolve_mouse_tracking();
            }
            TerminalMode::MouseAny => {
                self.mouse_any = enabled;
                self.resolve_mouse_tracking();
            }
            TerminalMode::MouseSgr => self.modes.mouse_sgr = enabled,
        }
        self.cursor.pending_wrap = false;
        self.changed();
    }

    fn resolve_mouse_tracking(&mut self) {
        self.modes.mouse_tracking = if self.mouse_any {
            MouseTrackingMode::Any
        } else if self.mouse_button {
            MouseTrackingMode::Button
        } else if self.mouse_x10 {
            MouseTrackingMode::X10
        } else {
            MouseTrackingMode::Off
        };
    }

    fn device_reply(&self, request: DeviceRequest) -> Vec<u8> {
        match request {
            DeviceRequest::PrimaryAttributes => b"\x1b[?1;2c".to_vec(),
            DeviceRequest::SecondaryAttributes => b"\x1b[>0;0;0c".to_vec(),
            DeviceRequest::OperatingStatus => b"\x1b[0n".to_vec(),
            DeviceRequest::CursorPosition => {
                let row = if self.modes.origin {
                    self.cursor.row.saturating_sub(self.scroll_top) + 1
                } else {
                    self.cursor.row + 1
                };
                format!("\x1b[{row};{}R", self.cursor.col + 1).into_bytes()
            }
        }
    }

    fn clear_range(&mut self, row: usize, start: usize, end: usize) {
        if start >= end {
            return;
        }
        self.clear_wide_at(row, start);
        self.clear_wide_at(row, end.saturating_sub(1));
        let fill = blank(self.style);
        self.active_mut().rows[row].cells[start..end].fill(fill);
    }
    fn clear_row(&mut self, row: usize) {
        if row > 0
            && self.active().rows[row - 1].soft_wrapped
            && self.active().rows[row - 1].logical_line_id
                == self.active().rows[row].logical_line_id
        {
            self.active_mut().rows[row - 1].soft_wrapped = false;
            self.break_chain_after(row - 1);
        }
        let fill = blank(self.style);
        self.active_mut().rows[row].cells.fill(fill);
        self.active_mut().rows[row].soft_wrapped = false;
        self.break_chain_after(row);
    }
    fn clear_wide_at(&mut self, row: usize, col: usize) {
        let cols = self.size.cols as usize;
        if col >= cols {
            return;
        }
        let lead = self.wide_lead(row, col);
        let width = matches!(
            self.active().rows[row]
                .cells
                .get(lead + 1)
                .map(|c| &c.glyph),
            Some(Glyph::Continuation)
        );
        let fill = blank(self.style);
        self.active_mut().rows[row].cells[lead] = fill.clone();
        if width {
            self.active_mut().rows[row].cells[lead + 1] = fill;
        }
    }
    fn wide_lead(&self, row: usize, col: usize) -> usize {
        if matches!(
            self.active().rows[row].cells[col].glyph,
            Glyph::Continuation
        ) && col > 0
        {
            col - 1
        } else {
            col
        }
    }
    fn normalize_row(&mut self, row: usize) {
        let style = self.style;
        sanitize_row(&mut self.active_mut().rows[row].cells, style);
    }
    fn break_chain_after(&mut self, row: usize) {
        let old_id = self.active().rows[row].logical_line_id;
        if row + 1 >= self.active().rows.len()
            || self.active().rows[row + 1].logical_line_id != old_id
        {
            return;
        }
        let cols = self.size.cols;
        let mut assignments = Vec::new();
        let mut id = self.next_line_id;
        self.next_line_id = self.next_line_id.saturating_add(1);
        let mut offset = 0u32;
        for index in row + 1..self.active().rows.len() {
            let next = &self.active().rows[index];
            if next.logical_line_id != old_id {
                break;
            }
            assignments.push((index, id, offset));
            if next.soft_wrapped {
                offset = offset.saturating_add(cols);
            } else {
                id = self.next_line_id;
                self.next_line_id = self.next_line_id.saturating_add(1);
                offset = 0;
            }
        }
        let grid = self.active_mut();
        for (index, id, offset) in assignments {
            grid.rows[index].logical_line_id = id;
            grid.rows[index].cell_offset = offset;
        }
    }
    fn new_row(&mut self) -> TerminalRow {
        let id = self.next_line_id;
        self.next_line_id = self.next_line_id.saturating_add(1);
        TerminalRow::new(self.size.cols as usize, id, self.style)
    }

    fn enforce_history_budget(&mut self) {
        let cols = self.size.cols as usize;
        while self.main.history.len() > self.scrollback_rows
            || self.main.history.len().saturating_mul(cols) > MAX_TERMINAL_HISTORY_CELLS
        {
            self.main.history.pop_front();
        }
    }

    #[allow(clippy::too_many_lines)]
    fn reflow_main(&mut self, size: CellSize) {
        let old_cols = self.size.cols as usize;
        let new_cols = size.cols as usize;
        let main_cursor = if self.alt_active {
            self.inactive_main_cursor
        } else {
            self.cursor
        };
        let cursor_row = main_cursor.row.min(self.main.rows.len() - 1);
        let cursor_id = self.main.rows[cursor_row].logical_line_id;
        let cursor_offset = self.main.rows[cursor_row].cell_offset as usize + main_cursor.col;
        let mut all: Vec<TerminalRow> = self.main.history.drain(..).collect();
        all.append(&mut self.main.rows);
        let mut logical: Vec<(u64, usize, Vec<Cell>)> = Vec::new();
        let mut previous_soft_wrapped = false;
        for row in all {
            let cursor_used = if row.logical_line_id == cursor_id
                && cursor_offset >= row.cell_offset as usize
                && cursor_offset < row.cell_offset as usize + old_cols
            {
                cursor_offset - row.cell_offset as usize + 1
            } else {
                0
            };
            let used = if row.soft_wrapped {
                old_cols
            } else {
                last_nonblank(&row.cells).max(cursor_used)
            };
            if previous_soft_wrapped
                && let Some((id, base, cells)) = logical.last_mut()
                && *id == row.logical_line_id
            {
                let start = row.cell_offset as usize - *base;
                cells.resize(start, blank(self.style));
                cells.extend_from_slice(&row.cells[..used]);
            } else {
                logical.push((
                    row.logical_line_id,
                    row.cell_offset as usize,
                    row.cells[..used].to_vec(),
                ));
            }
            previous_soft_wrapped = row.soft_wrapped;
        }

        let mut rows = Vec::new();
        let mut mapped_cursor_offset = cursor_offset;
        for (id, base, mut cells) in logical {
            sanitize_row(&mut cells, self.style);
            if cells.is_empty() {
                rows.push(TerminalRow::new(new_cols, id, self.style));
                continue;
            }
            let cursor_source = (id == cursor_id).then(|| cursor_offset.saturating_sub(base));
            let mut source = 0usize;
            let mut physical = 0usize;
            let mut col = 0usize;
            let mut part = vec![blank(self.style); new_cols];
            while source < cells.len() {
                if matches!(cells[source].glyph, Glyph::Continuation) {
                    source += 1;
                    continue;
                }
                let source_width = glyph_width(&cells[source].glyph).max(1);
                let mut display_width = source_width.min(2);
                let mut glyph = cells[source].glyph.clone();
                if display_width == 2 && new_cols == 1 {
                    glyph = Glyph::Char('\u{fffd}');
                    display_width = 1;
                }
                if display_width == 2 && col + 2 > new_cols {
                    rows.push(TerminalRow {
                        cells: part,
                        logical_line_id: id,
                        cell_offset: (base + physical) as u32,
                        soft_wrapped: true,
                    });
                    physical += new_cols;
                    col = 0;
                    part = vec![blank(self.style); new_cols];
                }
                if let Some(cursor_source) = cursor_source
                    && cursor_source >= source
                    && cursor_source < source + source_width
                {
                    mapped_cursor_offset =
                        base + physical + col + (cursor_source - source).min(display_width - 1);
                }
                part[col] = cell(glyph, cells[source].style);
                if display_width == 2 {
                    part[col + 1] = cell(Glyph::Continuation, cells[source].style);
                }
                col += display_width;
                source += source_width;
                if col == new_cols && source < cells.len() {
                    rows.push(TerminalRow {
                        cells: part,
                        logical_line_id: id,
                        cell_offset: (base + physical) as u32,
                        soft_wrapped: true,
                    });
                    physical += new_cols;
                    col = 0;
                    part = vec![blank(self.style); new_cols];
                }
            }
            rows.push(TerminalRow {
                cells: part,
                logical_line_id: id,
                cell_offset: (base + physical) as u32,
                soft_wrapped: false,
            });
        }
        while rows.len() < size.rows as usize {
            let id = self.next_line_id;
            self.next_line_id = self.next_line_id.saturating_add(1);
            rows.push(TerminalRow::new(new_cols, id, self.style));
        }
        let split = rows.len().saturating_sub(size.rows as usize);
        self.main.history = rows.drain(..split).collect();
        self.main.rows = rows;
        let mapped = self
            .main
            .rows
            .iter()
            .enumerate()
            .find_map(|(idx, row)| {
                if row.logical_line_id == cursor_id
                    && mapped_cursor_offset >= row.cell_offset as usize
                    && mapped_cursor_offset < row.cell_offset as usize + new_cols
                {
                    Some(Cursor {
                        row: idx,
                        col: mapped_cursor_offset - row.cell_offset as usize,
                        pending_wrap: false,
                    })
                } else {
                    None
                }
            })
            .unwrap_or(Cursor {
                row: size.rows as usize - 1,
                col: 0,
                pending_wrap: false,
            });
        if self.alt_active {
            self.inactive_main_cursor = mapped;
        } else {
            self.cursor = mapped;
        }
    }

    fn changed(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }
    fn current_snapshot(&self) -> ScreenSnapshot {
        ScreenSnapshot {
            size: self.size,
            cells: flatten(&self.active().rows),
            cursor: self
                .modes
                .cursor_visible
                .then(|| CellCoord::new(self.cursor.row as u32, self.cursor.col as u32)),
            title: self.title.clone(),
            generation: self.generation,
        }
    }
    fn publish(&mut self) {
        self.published = self.current_snapshot();
    }
}

impl Grid {
    fn new(size: CellSize, next_id: &mut u64) -> Self {
        let mut rows = Vec::with_capacity(size.rows as usize);
        for _ in 0..size.rows {
            rows.push(TerminalRow::new(
                size.cols as usize,
                *next_id,
                Style::default(),
            ));
            *next_id = next_id.saturating_add(1);
        }
        Self {
            rows,
            history: VecDeque::new(),
        }
    }
}

impl TerminalRow {
    fn new(cols: usize, logical_line_id: u64, style: Style) -> Self {
        Self {
            cells: vec![blank(style); cols],
            logical_line_id,
            cell_offset: 0,
            soft_wrapped: false,
        }
    }
}

fn validate_size(size: CellSize) -> Result<(), ScreenError> {
    let area = size.rows as usize * size.cols as usize;
    if size.rows == 0
        || size.cols == 0
        || size.rows > u32::from(MAX_TERMINAL_ROWS)
        || size.cols > u32::from(MAX_TERMINAL_COLS)
        || area > MAX_TERMINAL_VISIBLE_CELLS
    {
        Err(ScreenError::InvalidSize(size))
    } else {
        Ok(())
    }
}
fn default_tab_stops(cols: usize) -> BTreeSet<usize> {
    (8..cols).step_by(8).collect()
}
fn blank(style: Style) -> Cell {
    cell(Glyph::Char(' '), style)
}
fn cell(glyph: Glyph, style: Style) -> Cell {
    Cell {
        glyph,
        style,
        attachment: None,
    }
}
fn is_blank(cell: &Cell) -> bool {
    matches!(cell.glyph, Glyph::Char(' '))
        && cell.style == Style::default()
        && cell.attachment.is_none()
}
fn flatten(rows: &[TerminalRow]) -> Vec<Cell> {
    rows.iter()
        .flat_map(|row| row.cells.iter().cloned())
        .collect()
}
fn last_nonblank(cells: &[Cell]) -> usize {
    cells
        .iter()
        .rposition(|c| !is_blank(c))
        .map_or(0, |i| i + 1)
}
fn glyph_bytes(glyph: &Glyph) -> Vec<u8> {
    match glyph {
        Glyph::Char(ch) => {
            let mut buf = [0; 4];
            ch.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        Glyph::Cluster(bytes) => bytes.to_vec(),
        Glyph::Continuation => Vec::new(),
    }
}
fn sanitize_row(cells: &mut [Cell], style: Style) {
    for i in 0..cells.len() {
        if matches!(cells[i].glyph, Glyph::Continuation)
            && (i == 0 || glyph_width(&cells[i - 1].glyph) != 2)
        {
            cells[i] = blank(style);
        }
    }
    for i in 0..cells.len() {
        if glyph_width(&cells[i].glyph) == 2
            && (i + 1 == cells.len() || !matches!(cells[i + 1].glyph, Glyph::Continuation))
        {
            cells[i] = blank(style);
        }
    }
}
fn glyph_width(glyph: &Glyph) -> usize {
    match glyph {
        Glyph::Char(ch) => UnicodeWidthChar::width(*ch).unwrap_or(0).min(2),
        Glyph::Cluster(bytes) => std::str::from_utf8(bytes)
            .ok()
            .map_or(1, |cluster| UnicodeWidthStr::width(cluster).min(2)),
        Glyph::Continuation => 0,
    }
}
fn resize_grid_clip(
    grid: &mut Grid,
    old: CellSize,
    new: CellSize,
    next_id: &mut u64,
    style: Style,
) {
    let new_cols = new.cols as usize;
    for row in &mut grid.rows {
        row.cells.truncate(new_cols);
        row.cells.resize(new_cols, blank(style));
        sanitize_row(&mut row.cells, style);
    }
    grid.rows.truncate(new.rows as usize);
    while grid.rows.len() < new.rows as usize {
        grid.rows.push(TerminalRow::new(new_cols, *next_id, style));
        *next_id += 1;
    }
    grid.history.clear();
    let _ = old;
}
fn sanitize_title(value: &str) -> String {
    let mut value: String = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if value.len() > MAX_TERMINAL_METADATA_BYTES {
        let mut end = MAX_TERMINAL_METADATA_BYTES;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}
fn dec_special_graphics(ch: char) -> char {
    match ch {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{AnsiParser, AnsiParserProfile};

    fn screen(rows: u32, cols: u32) -> TerminalScreen {
        TerminalScreen::new(CellSize::new(rows, cols), 10_000).unwrap()
    }
    fn text(snapshot: &ScreenSnapshot) -> String {
        snapshot
            .cells
            .iter()
            .map(|c| match &c.glyph {
                Glyph::Char(ch) => *ch,
                Glyph::Cluster(bytes) => {
                    std::str::from_utf8(bytes).unwrap().chars().next().unwrap()
                }
                Glyph::Continuation => '·',
            })
            .collect()
    }

    #[test]
    fn parser_split_points_produce_identical_screen() {
        let bytes = b"ab\x1b[2;3Hc\x1b[31mD\x1b[2J\x1b[Hdone\x1bE\x1bM\x1bD";
        let mut whole_parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
        let mut whole = screen(3, 8);
        for event in whole_parser.feed(bytes) {
            whole.apply_event(event);
        }
        assert_eq!(whole.snapshot().cursor, Some(CellCoord::new(1, 0)));
        for split in 0..=bytes.len() {
            let mut parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
            let mut split_screen = screen(3, 8);
            for event in parser
                .feed(&bytes[..split])
                .into_iter()
                .chain(parser.feed(&bytes[split..]))
            {
                split_screen.apply_event(event);
            }
            assert_eq!(split_screen.snapshot(), whole.snapshot(), "split {split}");
        }
    }

    #[test]
    fn wide_combining_and_overwrite_keep_invariants() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::Text("中e\u{301}".into()));
        assert!(matches!(s.snapshot().cells[1].glyph, Glyph::Continuation));
        assert!(matches!(s.snapshot().cells[2].glyph, Glyph::Cluster(_)));
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 2 });
        s.apply_event(AnsiEvent::Text("x".into()));
        assert!(matches!(s.snapshot().cells[0].glyph, Glyph::Char(' ')));
        assert!(matches!(s.snapshot().cells[1].glyph, Glyph::Char('x')));
    }

    #[test]
    fn control_characters_never_enter_cells() {
        let mut s = screen(2, 4);
        let before = s.snapshot();
        s.apply_event(AnsiEvent::Text("\u{9b}\n\0".into()));
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn alternate_screen_preserves_main_and_has_no_history() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::Text("main".into()));
        let main = s.snapshot();
        s.apply_event(AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode1049,
            enabled: true,
        });
        s.apply_event(AnsiEvent::Text("alt\nmore".into()));
        assert!(s.history().is_empty());
        s.apply_event(AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode1049,
            enabled: false,
        });
        assert_eq!(&s.snapshot().cells[..4], &main.cells[..4]);
    }

    #[test]
    fn acs_and_device_replies_are_exact() {
        let mut s = screen(4, 8);
        s.apply_event(AnsiEvent::DesignateCharacterSet {
            slot: CharacterSetSlot::G0,
            charset: CharacterSet::DecSpecialGraphics,
        });
        s.apply_event(AnsiEvent::Text("lqk".into()));
        assert!(text(&s.snapshot()).starts_with("┌─┐"));
        assert_eq!(
            s.apply_event(AnsiEvent::DeviceRequest(DeviceRequest::PrimaryAttributes)),
            Some(b"\x1b[?1;2c".to_vec())
        );
        assert_eq!(
            s.apply_event(AnsiEvent::DeviceRequest(DeviceRequest::SecondaryAttributes)),
            Some(b"\x1b[>0;0;0c".to_vec())
        );
        assert_eq!(
            s.apply_event(AnsiEvent::DeviceRequest(DeviceRequest::OperatingStatus)),
            Some(b"\x1b[0n".to_vec())
        );
        assert_eq!(
            s.apply_event(AnsiEvent::DeviceRequest(DeviceRequest::CursorPosition)),
            Some(b"\x1b[1;4R".to_vec())
        );
    }

    #[test]
    fn synchronized_output_gates_snapshot_and_finish_releases() {
        let mut s = screen(2, 4);
        let before = s.snapshot();
        s.apply_event(AnsiEvent::SetMode {
            mode: TerminalMode::SynchronizedOutput,
            enabled: true,
        });
        s.apply_event(AnsiEvent::Text("x".into()));
        assert_eq!(s.snapshot(), before);
        s.finish_output();
        assert_ne!(s.snapshot(), before);
    }

    #[test]
    fn main_reflows_soft_wrap_and_alt_clips() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::Text("abcdef".into()));
        let id = s.visible_rows()[0].logical_line_id;
        s.resize(CellSize::new(3, 3)).unwrap();
        assert_eq!(s.visible_rows()[0].logical_line_id, id);
        assert_eq!(s.visible_rows()[1].logical_line_id, id);
        assert!(s.visible_rows()[0].soft_wrapped);
        s.apply_event(AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode1049,
            enabled: true,
        });
        s.apply_event(AnsiEvent::Text("abcdef".into()));
        s.resize(CellSize::new(3, 2)).unwrap();
        assert_eq!(text(&s.snapshot())[..4].to_string(), "abde");
    }

    #[test]
    fn history_obeys_row_and_cell_caps() {
        let mut s = TerminalScreen::new(CellSize::new(2, 512), 2).unwrap();
        for _ in 0..6 {
            s.apply_event(AnsiEvent::LineFeed);
        }
        assert_eq!(s.history().len(), 2);
        let mut budget = TerminalScreen::new(CellSize::new(2, 512), 20_000).unwrap();
        for _ in 0..8_000 {
            budget.apply_event(AnsiEvent::LineFeed);
        }
        assert!(budget.history().len() * 512 <= MAX_TERMINAL_HISTORY_CELLS);
    }

    #[test]
    fn index_next_line_and_reverse_index_respect_scrolling_margins() {
        let mut s = screen(4, 4);
        for (row, value) in ["aaaa", "bbbb", "cccc", "dddd"].into_iter().enumerate() {
            s.apply_event(AnsiEvent::CursorPosition {
                row: row as u32 + 1,
                col: 1,
            });
            s.apply_event(AnsiEvent::Text(value.into()));
        }
        s.apply_event(AnsiEvent::SetScrollingRegion {
            top: 2,
            bottom: Some(3),
        });
        s.apply_event(AnsiEvent::CursorPosition { row: 2, col: 3 });
        s.apply_event(AnsiEvent::ReverseIndex);
        assert_eq!(&text(&s.snapshot()), "aaaa    bbbbdddd");
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(1, 2)));

        s.apply_event(AnsiEvent::Index);
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(2, 2)));
        s.apply_event(AnsiEvent::NextLine);
        assert_eq!(&text(&s.snapshot()), "aaaabbbb    dddd");
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(2, 0)));
    }

    #[test]
    fn resize_only_adds_default_tab_stops_in_new_columns() {
        let mut s = screen(2, 16);
        s.apply_event(AnsiEvent::ClearAllTabStops);
        s.apply_event(AnsiEvent::CursorHorizontalAbsolute(4));
        s.apply_event(AnsiEvent::SetTabStop);
        s.resize(CellSize::new(2, 32)).unwrap();
        s.apply_event(AnsiEvent::CursorHorizontalAbsolute(1));
        s.apply_event(AnsiEvent::HorizontalTab);
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(0, 3)));
        s.apply_event(AnsiEvent::HorizontalTab);
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(0, 16)));
    }

    #[test]
    fn annotation_is_default_style_hard_line() {
        let mut s = screen(3, 20);
        s.apply_event(AnsiEvent::Text("partial".into()));
        s.append_process_annotation("Process 1 exited normally with code 0");
        let snapshot = s.snapshot();
        assert!(text(&snapshot).contains("Process 1 exited"));
        assert!(!s.visible_rows()[2].soft_wrapped);
        assert_eq!(snapshot.cursor.unwrap().row, 2);
        assert!(matches!(
            snapshot.cells[2 * 20 + 16].glyph,
            Glyph::Char('0')
        ));
    }

    #[test]
    fn annotation_remains_visible_without_trailing_scroll_on_one_row() {
        let mut s = screen(1, 64);
        s.append_process_annotation("Process 7 exited normally with code 0");
        let snapshot = s.snapshot();
        assert!(text(&snapshot).starts_with("Process 7 exited normally with code 0"));
        assert_eq!(snapshot.cursor, Some(CellCoord::new(0, 37)));
        assert!(s.history().is_empty());
        assert!(!s.visible_rows()[0].soft_wrapped);
    }
    #[test]
    fn invalid_resize_is_atomic() {
        let mut s = screen(2, 2);
        let before = s.snapshot();
        assert!(s.resize(CellSize::new(0, 2)).is_err());
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn cursor_erase_insert_delete_scroll_and_margins_mutate_exact_regions() {
        let mut s = screen(4, 6);
        s.apply_event(AnsiEvent::Text("abcdef".into()));
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 3 });
        s.apply_event(AnsiEvent::DeleteCharacters(2));
        assert!(text(&s.snapshot()).starts_with("abef  "));
        s.apply_event(AnsiEvent::InsertCharacters(2));
        assert!(text(&s.snapshot()).starts_with("ab  ef"));
        s.apply_event(AnsiEvent::EraseLineMode(EraseMode::ToStart));
        assert!(text(&s.snapshot()).starts_with("    ef"));
        s.apply_event(AnsiEvent::SetScrollingRegion {
            top: 2,
            bottom: Some(3),
        });
        s.apply_event(AnsiEvent::CursorPosition { row: 2, col: 1 });
        s.apply_event(AnsiEvent::InsertLines(1));
        assert_eq!(s.snapshot().cells.len(), 24);
        s.apply_event(AnsiEvent::ScrollUp(1));
        assert_eq!(s.snapshot().cells.len(), 24);
        assert_eq!(
            s.history().len(),
            0,
            "partial-region scroll never enters history"
        );
    }

    #[test]
    fn alternate_and_dec_saved_cursors_are_independent() {
        let mut s = screen(4, 8);
        s.apply_event(AnsiEvent::CursorPosition { row: 3, col: 4 });
        s.apply_event(AnsiEvent::SaveCursor);
        s.apply_event(AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode47,
            enabled: true,
        });
        s.apply_event(AnsiEvent::CursorPosition { row: 2, col: 2 });
        s.apply_event(AnsiEvent::SaveCursor);
        s.apply_event(AnsiEvent::CursorPosition { row: 4, col: 8 });
        s.apply_event(AnsiEvent::RestoreCursor);
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(1, 1)));
        s.apply_event(AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode47,
            enabled: false,
        });
        s.apply_event(AnsiEvent::RestoreCursor);
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(2, 3)));
    }

    #[test]
    fn watchdog_releases_sync_and_mouse_mode_resets_do_not_clobber_stronger_mode() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::SetMode {
            mode: TerminalMode::MouseAny,
            enabled: true,
        });
        s.apply_event(AnsiEvent::SetMode {
            mode: TerminalMode::MouseX10,
            enabled: true,
        });
        s.apply_event(AnsiEvent::SetMode {
            mode: TerminalMode::MouseX10,
            enabled: false,
        });
        assert_eq!(s.modes().mouse_tracking, MouseTrackingMode::Any);
        s.apply_event(AnsiEvent::SetMode {
            mode: TerminalMode::SynchronizedOutput,
            enabled: true,
        });
        s.apply_event(AnsiEvent::Text("x".into()));
        let start = s.sync_started.unwrap();
        assert!(s.synchronized_watchdog_expired(start + SYNCHRONIZED_OUTPUT_WATCHDOG));
        assert!(!s.modes().synchronized_output);
        assert!(text(&s.snapshot()).starts_with('x'));
    }

    #[test]
    fn split_zwj_cluster_is_bounded_and_keeps_wide_continuation() {
        let mut s = screen(2, 8);
        s.apply_event(AnsiEvent::Text("👩\u{200d}".into()));
        s.apply_event(AnsiEvent::Text("💻".into()));
        let snapshot = s.snapshot();
        assert!(matches!(snapshot.cells[0].glyph, Glyph::Cluster(_)));
        assert!(matches!(snapshot.cells[1].glyph, Glyph::Continuation));
        for _ in 0..MAX_TERMINAL_GRAPHEME_BYTES {
            s.apply_event(AnsiEvent::Text("\u{301}".into()));
        }
        if let Glyph::Cluster(bytes) = &s.snapshot().cells[0].glyph {
            assert!(bytes.len() <= MAX_TERMINAL_GRAPHEME_BYTES);
        } else {
            panic!("cluster expected");
        }
    }

    #[test]
    fn split_regional_indicator_modifier_zwj_and_variation_sequences_are_single_cells() {
        let cases = ["🇺🇸", "👍🏽", "👩\u{200d}💻", "❤\u{fe0f}"];
        for sequence in cases {
            let mut s = screen(2, 8);
            for ch in sequence.chars() {
                s.apply_event(AnsiEvent::Text(ch.to_string()));
            }
            let snapshot = s.snapshot();
            let Glyph::Cluster(bytes) = &snapshot.cells[0].glyph else {
                panic!("cluster expected for {sequence}");
            };
            assert_eq!(std::str::from_utf8(bytes).unwrap(), sequence);
            assert_eq!(glyph_width(&snapshot.cells[0].glyph), 2);
            assert!(matches!(snapshot.cells[1].glyph, Glyph::Continuation));
            assert!(
                !snapshot.cells[2..]
                    .iter()
                    .any(|cell| matches!(cell.glyph, Glyph::Continuation))
            );
        }
    }

    #[test]
    fn reflow_carries_wide_glyph_whole_across_new_boundary() {
        let mut s = screen(2, 6);
        s.apply_event(AnsiEvent::Text("ab中c".into()));
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 5 });
        s.resize(CellSize::new(2, 3)).unwrap();
        let snapshot = s.snapshot();
        assert!(matches!(snapshot.cells[0].glyph, Glyph::Char('中')));
        assert!(matches!(snapshot.cells[1].glyph, Glyph::Continuation));
        assert!(matches!(snapshot.cells[2].glyph, Glyph::Char('c')));
        assert_eq!(s.history().back().unwrap().cells[0].glyph, Glyph::Char('a'));
        assert_eq!(s.history().back().unwrap().cells[1].glyph, Glyph::Char('b'));
    }

    #[test]
    fn reflow_preserves_cursor_in_trailing_blank_cells() {
        let mut s = screen(2, 6);
        let line_id = s.visible_rows()[0].logical_line_id;
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 5 });
        s.resize(CellSize::new(2, 3)).unwrap();
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(0, 1)));
        assert_eq!(s.visible_rows()[0].logical_line_id, line_id);
        s.resize(CellSize::new(2, 6)).unwrap();
        assert_eq!(s.snapshot().cursor, Some(CellCoord::new(0, 4)));
        assert_eq!(s.visible_rows()[0].logical_line_id, line_id);
    }

    #[test]
    fn no_autowrap_wide_at_right_edge_never_scrolls_or_orphans() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::SetMode {
            mode: TerminalMode::AutoWrap,
            enabled: false,
        });
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 4 });
        s.apply_event(AnsiEvent::Text("中".into()));
        let snapshot = s.snapshot();
        assert_eq!(snapshot.cursor, Some(CellCoord::new(0, 3)));
        assert!(matches!(snapshot.cells[3].glyph, Glyph::Char('\u{fffd}')));
        assert!(s.history().is_empty());
        assert!(
            !snapshot
                .cells
                .iter()
                .any(|cell| matches!(cell.glyph, Glyph::Continuation))
        );
    }

    #[test]
    fn variation_selector_width_change_at_right_edge_wraps_atomically() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::Text("abc❤".into()));
        s.apply_event(AnsiEvent::Text("\u{fe0f}".into()));
        let snapshot = s.snapshot();
        assert!(matches!(snapshot.cells[3].glyph, Glyph::Char(' ')));
        assert!(matches!(snapshot.cells[4].glyph, Glyph::Cluster(_)));
        assert!(matches!(snapshot.cells[5].glyph, Glyph::Continuation));
        assert_eq!(snapshot.cursor, Some(CellCoord::new(1, 2)));
    }
    #[test]
    fn title_is_single_line_control_free_and_utf8_bounded() {
        let mut s = screen(2, 4);
        s.apply_event(AnsiEvent::SetTitle(format!(
            "a\r\nb\x1bc{}",
            "é".repeat(600)
        )));
        let title = s.snapshot().title.unwrap();
        assert_eq!(&title[..6], "a  b c");
        assert!(!title.chars().any(char::is_control));
        assert!(title.len() <= MAX_TERMINAL_METADATA_BYTES);
        assert!(title.is_char_boundary(title.len()));
    }

    #[test]
    fn hard_break_splits_a_previous_soft_wrap_chain_before_reflow() {
        let mut s = screen(3, 4);
        s.apply_event(AnsiEvent::Text("abcdefgh".into()));
        let wrapped_id = s.visible_rows()[0].logical_line_id;
        assert_eq!(s.visible_rows()[1].logical_line_id, wrapped_id);
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 1 });
        s.apply_event(AnsiEvent::LineFeed);
        assert!(!s.visible_rows()[0].soft_wrapped);
        assert_ne!(s.visible_rows()[1].logical_line_id, wrapped_id);
        let hard_id = s.visible_rows()[1].logical_line_id;
        s.resize(CellSize::new(3, 8)).unwrap();
        assert_eq!(s.visible_rows()[0].logical_line_id, wrapped_id);
        assert_eq!(s.visible_rows()[1].logical_line_id, hard_id);
        assert_eq!(&text(&s.snapshot())[..12], "abcd    efgh");
    }

    #[test]
    fn styled_trailing_blanks_survive_reflow_and_count_as_content() {
        let mut s = screen(2, 4);
        let style = Style {
            bg: crate::cell::Color::Indexed(4),
            underline: crate::cell::UnderlineStyle::Single,
            ..Style::default()
        };
        s.apply_event(AnsiEvent::SetStyle(style));
        s.apply_event(AnsiEvent::Text("  ".into()));
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 2 });
        s.resize(CellSize::new(2, 2)).unwrap();
        assert_eq!(s.snapshot().cells[0].style, style);
        assert_eq!(s.snapshot().cells[1].style, style);
        s.resize(CellSize::new(2, 4)).unwrap();
        assert_eq!(s.snapshot().cells[0].style, style);
        assert_eq!(s.snapshot().cells[1].style, style);
        s.apply_event(AnsiEvent::CursorPosition { row: 1, col: 1 });
        s.apply_event(AnsiEvent::EraseLineMode(EraseMode::All));
        s.append_process_annotation("exit");
        assert!(text(&s.snapshot()).contains("exit"));
        assert_eq!(s.snapshot().cursor.unwrap().row, 1);
    }
}
