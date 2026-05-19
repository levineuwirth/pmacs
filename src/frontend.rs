// frontend.rs --- crossterm-driven TUI backend.

//! TUI frontend: terminal raw-mode setup, input parsing, escape-sequence
//! emission for `InstanceMessage` deltas. crossterm directly --- no ratatui
//! (its widget model competes with the cell grid).
//!
//! Per T M5.2 (spec §sec:v01-remote-scope deliverable 1), the cell-buffer
//! diff happens *instance-side* in
//! [`crate::instance_render::RenderState`], not here. The frontend is a
//! transport sink: it consumes [`InstanceMessage`] values and emits
//! escape sequences. The same [`InstanceMessage`] stream that drives
//! the local TUI today drives the SSH transport in T M5.7.
//!
//! # Lifecycle
//!
//! [`Frontend::new`] enables raw mode, enters the alternate screen, hides
//! the cursor, and enables bracketed paste. [`Frontend::drop`] tears all of
//! that down even on panic. A panic hook installed at process start
//! (`install_panic_hook`) ensures the terminal is restored before the panic
//! message hits stderr.
//!
//! # Frames
//!
//! [`Frontend::present_messages`] wraps the supplied
//! [`InstanceMessage`]s in DEC mode 2026 synchronized output and applies
//! each. [`InstanceMessage::CellDelta`] emits escape sequences for the
//! span; [`InstanceMessage::Cursor`] moves and shows/hides the cursor;
//! the `ModeLine`, `Signal`, and `Goodbye` variants are reserved for
//! v0.3 and ignored by the v0.1 TUI.
//!
//! # Threading
//!
//! Main thread only.
//!
//! # Status overlay (T M5.8)
//!
//! [`Frontend::draw_status_overlay`] paints a one-row banner across the
//! bottom of the screen in reverse video --- used by the attach
//! reconnect loop to indicate "disconnected, reconnecting in 4s,
//! Ctrl-C to exit". It is *not* an [`InstanceMessage`]: the daemon has
//! no opinion about reconnect status, since reconnect is a frontend
//! concern. The overlay is cleared by [`Frontend::clear_status_overlay`]
//! once a reattach succeeds; the subsequent full-grid resync repaints
//! whatever the daemon's view places in that row.

use std::io::{self, BufWriter, Stdout, Write};
use std::time::Duration;

#[cfg(feature = "crdt")]
use crossterm::{cursor::MoveLeft, style::Print};
use crossterm::{
    cursor::{self, MoveTo},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    queue,
    style::{
        Attribute, Color as CtColor, ResetColor, SetAttribute, SetBackgroundColor,
        SetForegroundColor,
    },
    terminal::{
        BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen,
        LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size as terminal_size,
    },
};

use crate::cell::{CellSize, Color, DiffSpan, Glyph, Style};
use crate::protocol::InstanceMessage;

// Re-export the input event types so callers don't depend on crossterm
// directly. M2's keymap will translate these into normalized commands.
pub use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};

// ---------------------------------------------------------------------------
// Panic hook
// ---------------------------------------------------------------------------

/// Install a panic hook that restores the terminal before forwarding to the
/// previous hook.
///
/// Call this once at process start, before constructing the [`Frontend`].
/// Idempotent: calling it twice replaces the previous hook with one that
/// still chains to the original.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal_on_panic();
        previous(info);
    }));
}

fn restore_terminal_on_panic() {
    let mut out = io::stdout();
    // Pop the keyboard enhancement first --- terminals that ignored the
    // push will likewise ignore the pop, so this is unconditionally
    // safe even if `Frontend::new` never ran or never reached the push.
    let _ = queue!(
        out,
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        cursor::Show,
        LeaveAlternateScreen,
    );
    let _ = out.flush();
    let _ = disable_raw_mode();
}

// ---------------------------------------------------------------------------
// Frontend
// ---------------------------------------------------------------------------

/// TUI backend wrapping crossterm.
//
// The four lifecycle bools each track an independent terminal state we may
// have entered (or not) at init time. We need them separately so teardown
// only undoes what setup actually did. Collapsing them into a single state
// machine would be ceremony without benefit, since the transitions are not
// ordered and the flags are private to the struct.
#[allow(
    clippy::struct_excessive_bools,
    reason = "lifecycle flags are independent"
)]
pub struct Frontend {
    out: BufWriter<Stdout>,
    size: CellSize,
    /// Whether raw mode was entered (for teardown).
    raw_mode: bool,
    /// Whether alternate screen was entered (for teardown).
    alt_screen: bool,
    /// Whether bracketed paste was enabled (for teardown).
    bracketed_paste: bool,
    /// Whether mouse capture was enabled (for teardown).
    mouse: bool,
    /// Whether the kitty keyboard-enhancement flags were pushed.
    /// Disambiguates `Ctrl+/`, `Shift+Tab`, etc., that the legacy
    /// terminal protocol mangles. Best-effort: terminals that don't
    /// support it ignore the CSI; we still record the push so we know
    /// to balance with a Pop on teardown.
    keyboard_enhancement: bool,
}

impl Frontend {
    /// Construct a frontend, taking over the controlling terminal.
    ///
    /// On error the terminal is left in its original state.
    pub fn new() -> io::Result<Self> {
        let (cols, rows) = terminal_size()?;
        let size = CellSize::new(u32::from(rows), u32::from(cols));

        let stdout = io::stdout();
        let out = BufWriter::new(stdout);
        enable_raw_mode()?;
        let mut me = Self {
            out,
            size,
            raw_mode: true,
            alt_screen: false,
            bracketed_paste: false,
            mouse: false,
            keyboard_enhancement: false,
        };
        // Best-effort sequence of init steps. Each step that succeeds
        // is recorded so teardown can skip steps that never ran.
        if let Err(e) = queue!(
            me.out,
            EnterAlternateScreen,
            Clear(ClearType::All),
            cursor::Hide,
            EnableBracketedPaste,
            EnableMouseCapture,
        ) {
            me.teardown();
            return Err(e);
        }
        me.alt_screen = true;
        me.bracketed_paste = true;
        me.mouse = true;

        // Kitty keyboard protocol (best-effort). Terminals that support
        // it (kitty, foot, WezTerm, alacritty, modern xterm, ...) start
        // delivering disambiguated key events: `Ctrl+/` arrives as the
        // literal `/` with CONTROL instead of the byte-roulette legacy
        // protocols produce. Terminals that don't ignore the CSI; we
        // push the flag anyway so the Pop on teardown is balanced.
        //
        // We deliberately do NOT push `REPORT_ALL_KEYS_AS_ESCAPE_CODES`.
        // That flag tells the terminal to send every key (including
        // printable letters) as a CSI sequence carrying the unshifted
        // base key plus modifier bits, e.g. `Shift+a` arrives as
        // `Char('a') + SHIFT` rather than `Char('A')`. Pmacs has no
        // keyboard-layout knowledge to translate `9 + SHIFT` into `(`
        // on a US layout (or `É` on a French layout, etc.); the
        // terminal does. Letting the terminal apply layout-aware shift
        // translation is correct; receiving the post-shift character
        // is what every typing-driven path (self-insert, minibuffer,
        // search) expects. `DISAMBIGUATE_ESCAPE_CODES` alone still
        // gives us the C-i/Tab and C-m/Enter disambiguation we want.
        let kitty_flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
        if queue!(me.out, PushKeyboardEnhancementFlags(kitty_flags)).is_ok() {
            me.keyboard_enhancement = true;
        }

        if let Err(e) = me.out.flush() {
            me.teardown();
            return Err(e);
        }
        Ok(me)
    }

    /// Current size of the terminal in cells.
    #[must_use]
    pub fn size(&self) -> CellSize {
        self.size
    }

    /// Apply a sequence of [`InstanceMessage`]s, wrapped in DEC mode 2026
    /// synchronized output so partial frames never appear when the
    /// terminal supports it. Terminals that don't understand the
    /// brackets silently ignore them.
    pub fn present_messages(&mut self, msgs: &[InstanceMessage]) -> io::Result<()> {
        queue!(self.out, BeginSynchronizedUpdate)?;
        for m in msgs {
            self.apply_message(m)?;
        }
        queue!(self.out, EndSynchronizedUpdate)?;
        self.out.flush()
    }

    /// T M10.10 Day 3 step 5 Path β — paint an optimistic insert.
    ///
    /// The character is written at the terminal's current cursor
    /// position; the terminal advances the cursor by one column.
    /// This is the visual half of the optimistic-apply path:
    /// `BufferMirror::apply_local_insert` updated the CRDT mirror;
    /// this method updates the user-visible display in the same
    /// keystroke.
    ///
    /// Called only when the cursor is at end-of-line for the active
    /// buffer (per `BufferMirror::cursor_at_end_of_line`). End-of-
    /// line is the dominant typing case and the only case where the
    /// daemon's eventual `CellDelta` matches a single-Print
    /// optimistic paint exactly (no cells right of cursor to shift).
    ///
    /// # Post-audit round 2 (F15): style-blindness
    ///
    /// This paint is **default-style only**. We explicitly reset
    /// terminal attributes before the `Print` so the painted glyph
    /// is deterministic and doesn't inherit leftover SGR state from
    /// a prior `emit_span`. The `emit_span` epilogue already issues
    /// `ResetColor + SetAttribute(Attribute::Reset)`, but the
    /// invariant is fragile across crossterm versions and we'd
    /// rather pay one extra reset than re-flash whatever style the
    /// previous span set.
    ///
    /// **Honest scope**: if the cell the daemon will eventually
    /// paint into has a non-default style (e.g., a diagnostic
    /// region, a syntax-highlighted token in a future milestone),
    /// the optimistic glyph briefly renders default-styled until the
    /// authoritative `CellDelta` arrives (within one frame target).
    /// For v0.1 there is no syntax-highlighting pipeline; styled
    /// regions are restricted to diagnostics squiggles, completion
    /// popups, and overlays — none of which typically sit on the
    /// end-of-line cell that Path β paints into. A future milestone
    /// that introduces in-buffer styled content should track the
    /// cursor-cell's pending style from the previous `CellDelta` and
    /// apply it here, or suppress the optimistic paint on styled
    /// cells altogether. The right fix needs per-cell style memory
    /// the attach loop doesn't carry today.
    #[cfg(feature = "crdt")]
    pub fn paint_optimistic_insert(&mut self, c: char) -> io::Result<()> {
        queue!(
            self.out,
            ResetColor,
            SetAttribute(Attribute::Reset),
            Print(c)
        )?;
        self.out.flush()
    }

    /// T M10.10 Day 3 step 5 Path β — paint an optimistic
    /// delete-back.
    ///
    /// Sequence: move cursor one column left, overwrite the cell
    /// with a space, retreat cursor one column to its final
    /// position. Matches what the daemon's eventual `CellDelta`
    /// will carry: the last char of the line becomes a space at
    /// the cursor's pre-edit column.
    ///
    /// Called only when the cursor is at end-of-line and there's a
    /// previous character to erase. Mid-line backspace falls
    /// through to v0.1 round-trip per Path β scope.
    ///
    /// # Post-audit round 2 (F15): style-blindness
    ///
    /// The space is painted with default style (explicit reset
    /// before `Print`). For end-of-line backspace this is correct
    /// in nearly all v0.1 cases: the cell becomes empty / cleared,
    /// and the daemon's eventual `CellDelta` for an empty cell is
    /// itself default-styled. Same scope caveat as
    /// [`Self::paint_optimistic_insert`] for any future milestone
    /// where the post-erase cell might re-render with a non-default
    /// background or syntax style.
    #[cfg(feature = "crdt")]
    pub fn paint_optimistic_delete_back(&mut self) -> io::Result<()> {
        queue!(
            self.out,
            MoveLeft(1),
            ResetColor,
            SetAttribute(Attribute::Reset),
            Print(' '),
            MoveLeft(1)
        )?;
        self.out.flush()
    }

    /// Apply a single [`InstanceMessage`] to the terminal.
    ///
    /// `CellDelta` emits one cursor-move + run-of-glyphs sequence per
    /// span (via [`emit_span`]). `Cursor` moves the terminal cursor
    /// and toggles its visibility. `ModeLine`, `Signal`, and `Goodbye`
    /// are reserved for v0.3 (GUI / multi-frontend) and ignored here.
    pub fn apply_message(&mut self, msg: &InstanceMessage) -> io::Result<()> {
        match msg {
            InstanceMessage::CellDelta { spans, .. } => {
                for span in spans {
                    emit_span(&mut self.out, span)?;
                }
            }
            InstanceMessage::Cursor(state) => match state {
                Some(cs) if cs.visible => {
                    queue!(
                        self.out,
                        MoveTo(cs.coord.col as u16, cs.coord.row as u16),
                        cursor::Show
                    )?;
                }
                _ => {
                    queue!(self.out, cursor::Hide)?;
                }
            },
            InstanceMessage::ModeLine(_)
            | InstanceMessage::Signal(_)
            | InstanceMessage::Goodbye(_)
            // T M10.5: CrdtOp's wire shape exists; the v1.0 TUI doesn't
            // maintain a local CRDT state yet (M10.8 wires that). A v2
            // daemon shouldn't send CrdtOp to this frontend because our
            // FrontendCapabilities advertise crdt_replica: false. If one
            // arrives anyway, drop it silently — same v0.1-ignored
            // category as ModeLine / Signal / Goodbye for now.
            | InstanceMessage::CrdtOp { .. }
            // T M10.6: PresenceUpdate joins the v0.1-ignored category.
            // The peer-cursor overlay renderer is M10.8 work; until
            // then any incoming PresenceUpdate is dropped silently.
            | InstanceMessage::PresenceUpdate { .. }
            // T M10.10: BufferSnapshot is consumed by the BufferMirror
            // layer on M10.10-aware frontends (gated by negotiated
            // `crdt_replica`). The legacy TUI render path here doesn't
            // maintain a BufferMirror, so the variant drops silently
            // in this path. The M10.10 frontend wiring intercepts
            // BufferSnapshot in the attach.rs message loop BEFORE it
            // reaches apply_message.
            | InstanceMessage::BufferSnapshot { .. }
            // T M10.10: CursorByte is paired with Cursor for replica
            // frontends. The cursor's grid position (consumed by the
            // legacy render path above via Cursor) drives paint; the
            // byte position (consumed by BufferMirror's cursor tracker
            // in attach.rs) drives optimistic-apply. The legacy path
            // here only needs grid; the byte variant drops silently.
            | InstanceMessage::CursorByte { .. }
            // T M11.1: the semantic-frontend projection family. This
            // is the grid TUI — it advertises `semantic_render: false`,
            // so a v3 daemon never sends these here (the per-session
            // outgoing filter, M11.2, gates the family). If one
            // arrives anyway it drops silently, same v0.1-ignored
            // category as CrdtOp / PresenceUpdate. A semantic
            // frontend (M11.5) consumes them via its own layout path,
            // not this cell-grid path.
            | InstanceMessage::StyleSpans { .. }
            | InstanceMessage::Decorations { .. }
            | InstanceMessage::InlineAdornments { .. }
            | InstanceMessage::BlockAdornments { .. }
            | InstanceMessage::FoldState { .. }
            | InstanceMessage::FileStyleSummary { .. }
            | InstanceMessage::ResourceOffer { .. } => {
                // v0.1 TUI ignores these; v0.3 GUI consumes them.
            }
        }
        Ok(())
    }

    /// Paint a one-row status banner across the bottom of the screen in
    /// reverse video and hide the cursor.
    ///
    /// Used during attach reconnect (T M5.8) to surface that the
    /// session is disconnected and a reconnect is in flight. The
    /// banner is sanitized (control chars replaced with spaces),
    /// truncated to terminal width, and right-padded with spaces so
    /// the entire bottom row is overwritten --- no leftover cells
    /// from the previous frame or a prior overlay can bleed through.
    /// Subsequent calls overwrite the previous overlay in place.
    ///
    /// The cursor is hidden because input is suppressed during
    /// reconnect: a visible cursor would lie about where the user's
    /// keystrokes land. [`Frontend::clear_status_overlay`] does not
    /// re-show it; the daemon's next [`InstanceMessage::Cursor`] in
    /// the post-reattach resync restores cursor visibility.
    ///
    /// Wrapped in DEC mode 2026 synchronized output so the banner
    /// never appears half-painted on terminals that support the
    /// brackets.
    ///
    /// # Errors
    /// Returns the underlying [`io::Error`] if writing to stdout
    /// fails.
    pub fn draw_status_overlay(&mut self, text: &str) -> io::Result<()> {
        queue!(self.out, BeginSynchronizedUpdate)?;
        emit_status_overlay(&mut self.out, self.size, text)?;
        queue!(self.out, cursor::Hide, EndSynchronizedUpdate)?;
        self.out.flush()
    }

    /// Repaint the bottom row in the default style to remove a status
    /// overlay drawn by [`Frontend::draw_status_overlay`].
    ///
    /// Does not restore cursor visibility; the daemon's next
    /// [`InstanceMessage::Cursor`] in the resync will. Cells outside
    /// the bottom row are untouched --- the post-reattach full-grid
    /// resync repaints the bottom row content the daemon's view
    /// places there.
    ///
    /// # Errors
    /// Returns the underlying [`io::Error`] if writing to stdout
    /// fails.
    pub fn clear_status_overlay(&mut self) -> io::Result<()> {
        queue!(self.out, BeginSynchronizedUpdate)?;
        emit_clear_status_overlay(&mut self.out, self.size)?;
        queue!(self.out, EndSynchronizedUpdate)?;
        self.out.flush()
    }

    /// Wait for the next input event, up to `timeout`.
    ///
    /// Returns `Ok(None)` on timeout, `Ok(Some(event))` on event,
    /// `Err(_)` on terminal error. A `Resize` event updates the
    /// frontend's known size and reallocates the cell buffers.
    pub fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if !crossterm::event::poll(timeout)? {
            return Ok(None);
        }
        let event = crossterm::event::read()?;
        if let Event::Resize(cols, rows) = event {
            self.handle_resize(CellSize::new(u32::from(rows), u32::from(cols)));
        }
        Ok(Some(event))
    }

    /// Block until the next input event.
    ///
    /// Returns the event, updating the frontend's size on `Resize`.
    pub fn read_event(&mut self) -> io::Result<Event> {
        let event = crossterm::event::read()?;
        if let Event::Resize(cols, rows) = event {
            self.handle_resize(CellSize::new(u32::from(rows), u32::from(cols)));
        }
        Ok(event)
    }

    fn handle_resize(&mut self, new_size: CellSize) {
        self.size = new_size;
        // Cell-buffer reallocation lives on
        // [`crate::instance_render::RenderState`] now (T M5.2). The
        // run loop is responsible for forwarding the new size there;
        // the frontend just updates its own view of the terminal
        // dimensions for input-event coordinates.
    }

    /// Tear down the terminal state. Idempotent. Called from [`Drop`] and
    /// on init failure.
    fn teardown(&mut self) {
        if self.keyboard_enhancement {
            // Always pop, even if the original push was a no-op for
            // this terminal: a terminal that didn't enter the enhanced
            // mode will silently ignore the pop.
            let _ = queue!(self.out, PopKeyboardEnhancementFlags);
            self.keyboard_enhancement = false;
        }
        if self.bracketed_paste {
            let _ = queue!(self.out, DisableBracketedPaste);
            self.bracketed_paste = false;
        }
        if self.mouse {
            let _ = queue!(self.out, DisableMouseCapture);
            self.mouse = false;
        }
        if self.alt_screen {
            let _ = queue!(self.out, cursor::Show, ResetColor, LeaveAlternateScreen);
            self.alt_screen = false;
        }
        let _ = self.out.flush();
        if self.raw_mode {
            let _ = disable_raw_mode();
            self.raw_mode = false;
        }
    }
}

impl Drop for Frontend {
    fn drop(&mut self) {
        self.teardown();
    }
}

// ---------------------------------------------------------------------------
// Span emission (pure; testable)
// ---------------------------------------------------------------------------

/// Emit a diff span as escape sequences to `w`.
///
/// Pure: the same span produces the same byte output, regardless of any
/// process state. Tested below by capturing into a `Vec<u8>`.
fn emit_span<W: Write>(w: &mut W, span: &DiffSpan) -> io::Result<()> {
    if span.cells.is_empty() {
        return Ok(());
    }

    queue!(w, MoveTo(span.start.col as u16, span.start.row as u16))?;

    let mut last_style: Option<Style> = None;
    for cell in &span.cells {
        if last_style.as_ref() != Some(&cell.style) {
            apply_style(w, &cell.style)?;
            last_style = Some(cell.style);
        }
        match &cell.glyph {
            Glyph::Char(c) => {
                // Defense in depth: a control character in a cell would
                // jump the cursor (`\n`, `\r`) or emit a CSI ESC
                // sequence (`\x1b`), corrupting subsequent paint
                // operations. Sanitize at the seam so a single bad
                // status line cannot shred the frame.
                let printable = if c.is_control() { ' ' } else { *c };
                write!(w, "{printable}")?;
            }
            Glyph::Cluster(bytes) => w.write_all(bytes)?,
            Glyph::Continuation => {
                // Wide-char continuation: the previous cell's glyph occupies
                // both columns. Skip; the terminal already drew over this
                // cell when it rendered the wide glyph.
            }
        }
    }
    queue!(w, ResetColor, SetAttribute(Attribute::Reset))?;
    Ok(())
}

/// Emit the bottom-row status banner as escape sequences to `w`.
///
/// Pure: same `(size, text)` produces the same bytes. The caller
/// supplies framing (synchronized-update brackets, cursor hide,
/// flush). A zero-area terminal is a no-op so the function is safe
/// to call before the first resize in unusual init paths.
///
/// Truncation is by `char` count, which equals column count for
/// the v0.1 banner texts (ASCII + em-dash). A terminal narrow
/// enough to truncate the message is already showing the user
/// "something is wrong"; precision wide-char width accounting
/// would be ceremony with no user-visible improvement at v0.1.
fn emit_status_overlay<W: Write>(w: &mut W, size: CellSize, text: &str) -> io::Result<()> {
    let cols = size.cols as usize;
    if cols == 0 || size.rows == 0 {
        return Ok(());
    }
    let bottom_row = (size.rows - 1) as u16;

    queue!(
        w,
        MoveTo(0, bottom_row),
        SetAttribute(Attribute::Reset),
        SetAttribute(Attribute::Reverse),
    )?;

    let mut painted: usize = 0;
    for c in text.chars() {
        if painted >= cols {
            break;
        }
        // Sanitize controls so a stray newline / CR / ESC in the
        // banner text cannot scroll the screen or inject SGR
        // sequences. Mirrors the defense in `emit_span`.
        let printable = if c.is_control() { ' ' } else { c };
        write!(w, "{printable}")?;
        painted += 1;
    }
    for _ in painted..cols {
        write!(w, " ")?;
    }
    queue!(w, SetAttribute(Attribute::Reset))?;
    Ok(())
}

/// Emit a bottom-row clear (default-style spaces) as escape sequences
/// to `w`.
///
/// Counterpart to [`emit_status_overlay`]. Same purity contract.
fn emit_clear_status_overlay<W: Write>(w: &mut W, size: CellSize) -> io::Result<()> {
    let cols = size.cols as usize;
    if cols == 0 || size.rows == 0 {
        return Ok(());
    }
    let bottom_row = (size.rows - 1) as u16;

    queue!(w, MoveTo(0, bottom_row), SetAttribute(Attribute::Reset))?;
    for _ in 0..cols {
        write!(w, " ")?;
    }
    Ok(())
}

fn apply_style<W: Write>(w: &mut W, style: &Style) -> io::Result<()> {
    queue!(w, SetAttribute(Attribute::Reset))?;
    queue!(w, SetForegroundColor(to_ct_color(style.fg)))?;
    queue!(w, SetBackgroundColor(to_ct_color(style.bg)))?;
    if style.bold {
        queue!(w, SetAttribute(Attribute::Bold))?;
    }
    if style.italic {
        queue!(w, SetAttribute(Attribute::Italic))?;
    }
    match style.underline {
        crate::cell::UnderlineStyle::None => {}
        crate::cell::UnderlineStyle::Single
        | crate::cell::UnderlineStyle::Double
        | crate::cell::UnderlineStyle::Curly
        | crate::cell::UnderlineStyle::Dotted
        | crate::cell::UnderlineStyle::Dashed => {
            // crossterm exposes Underlined; richer styles need a manual
            // CSI 4:N m emit, which most terminals fall back to plain
            // underline on. M2 can wire CSI 4:N m for kitty/iTerm.
            queue!(w, SetAttribute(Attribute::Underlined))?;
        }
    }
    if style.reverse {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    Ok(())
}

fn to_ct_color(c: Color) -> CtColor {
    match c {
        Color::Default => CtColor::Reset,
        Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
        Color::Indexed(n) => CtColor::AnsiValue(n),
    }
}

// ---------------------------------------------------------------------------
// Tests (pure parts only --- the lifecycle machinery requires a TTY)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{Cell, CellCoord, Glyph, Style};

    fn ch(c: char) -> Cell {
        Cell {
            glyph: Glyph::Char(c),
            style: Style::default(),
            attachment: None,
        }
    }

    #[test]
    fn emit_span_writes_cursor_move_then_chars() {
        let span = DiffSpan {
            start: CellCoord::new(2, 5),
            cells: vec![ch('a'), ch('b'), ch('c')],
        };
        let mut out = Vec::new();
        emit_span(&mut out, &span).unwrap();
        let s = String::from_utf8_lossy(&out);
        // Cursor move uses CSI <row+1> ; <col+1> H. Both 1-based.
        assert!(s.contains("\x1b[3;6H"), "missing cursor move in {s:?}");
        // Glyphs emitted in order.
        assert!(s.contains("abc"), "missing glyphs in {s:?}");
    }

    #[test]
    fn emit_span_handles_continuation() {
        // Wide char span: leading 中 + Continuation. Only the leading char
        // should appear in the output bytes.
        let span = DiffSpan {
            start: CellCoord::new(0, 0),
            cells: vec![
                Cell {
                    glyph: Glyph::Char('中'),
                    style: Style::default(),
                    attachment: None,
                },
                Cell {
                    glyph: Glyph::Continuation,
                    style: Style::default(),
                    attachment: None,
                },
            ],
        };
        let mut out = Vec::new();
        emit_span(&mut out, &span).unwrap();
        let s = String::from_utf8_lossy(&out);
        // 中 appears, but no extra char follows (continuation contributes
        // nothing of its own --- the wide glyph occupies both columns).
        assert!(s.contains('中'));
        assert!(!s.contains("中中"));
    }

    #[test]
    fn emit_span_emits_cluster_bytes() {
        let span = DiffSpan {
            start: CellCoord::new(0, 0),
            cells: vec![Cell {
                glyph: Glyph::Cluster(b"\xC3\xA9".to_vec().into_boxed_slice()),
                style: Style::default(),
                attachment: None,
            }],
        };
        let mut out = Vec::new();
        emit_span(&mut out, &span).unwrap();
        // Cluster is the UTF-8 bytes of é.
        assert!(out.windows(2).any(|w| w == b"\xC3\xA9"));
    }

    #[test]
    fn empty_span_emits_nothing() {
        let span = DiffSpan {
            start: CellCoord::new(0, 0),
            cells: vec![],
        };
        let mut out = Vec::new();
        emit_span(&mut out, &span).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn style_transitions_are_emitted() {
        // First cell has bold style; second cell drops it. The output must
        // contain both the bold-on sequence and a reset between them.
        let span = DiffSpan {
            start: CellCoord::new(0, 0),
            cells: vec![
                Cell {
                    glyph: Glyph::Char('A'),
                    style: Style {
                        bold: true,
                        ..Style::default()
                    },
                    attachment: None,
                },
                ch('b'),
            ],
        };
        let mut out = Vec::new();
        emit_span(&mut out, &span).unwrap();
        let s = String::from_utf8_lossy(&out);
        // Bold uses SGR 1.
        assert!(s.contains("\x1b[1m"), "missing bold-on in {s:?}");
    }

    // ---- status overlay (T M5.8) -----------------------------------------

    #[test]
    fn status_overlay_moves_to_bottom_and_emits_reverse_attr_text_and_padding() {
        // 10 rows, 20 cols → bottom row index 9, ANSI-1-based row 10.
        let size = CellSize::new(10, 20);
        let mut out = Vec::new();
        emit_status_overlay(&mut out, size, "hi").unwrap();
        let s = String::from_utf8_lossy(&out);

        // CSI <row+1> ; <col+1> H — both 1-based.
        assert!(s.contains("\x1b[10;1H"), "missing bottom-row move in {s:?}");
        // SGR 7 — reverse video.
        assert!(s.contains("\x1b[7m"), "missing reverse attr in {s:?}");
        // Banner text painted.
        assert!(s.contains("hi"), "missing banner text in {s:?}");
        // Right-padded to full width: 2 chars of text + 18 spaces.
        let space_count = s.matches(' ').count();
        assert!(
            space_count >= 18,
            "expected at least 18 padding spaces, got {space_count} in {s:?}"
        );
        // Trailing reset so subsequent content isn't reverse-video.
        assert!(s.ends_with("\x1b[0m"), "missing trailing reset in {s:?}");
    }

    #[test]
    fn status_overlay_truncates_text_to_terminal_width() {
        // 4 cols → only the first 4 characters of the input survive.
        let size = CellSize::new(5, 4);
        let mut out = Vec::new();
        emit_status_overlay(&mut out, size, "abcdefgh").unwrap();
        let s = String::from_utf8_lossy(&out);

        assert!(s.contains("abcd"), "missing prefix in {s:?}");
        assert!(!s.contains("efgh"), "did not truncate: {s:?}");
        // No padding when text fills the row exactly.
        let space_count = s.matches(' ').count();
        assert_eq!(space_count, 0, "unexpected padding in {s:?}");
    }

    #[test]
    fn status_overlay_sanitizes_control_chars() {
        // Newline / CR / ESC inside the banner text would scroll the
        // screen or inject SGR sequences. They must be replaced with
        // spaces before reaching the terminal.
        let size = CellSize::new(5, 20);
        let mut out = Vec::new();
        emit_status_overlay(&mut out, size, "a\nb\rc\x1bd").unwrap();
        let s = String::from_utf8_lossy(&out);

        // No raw newline / CR survives in the painted output.
        assert!(!s.contains('\n'), "raw \\n leaked through: {s:?}");
        assert!(!s.contains('\r'), "raw \\r leaked through: {s:?}");
        // Each control char became a single space, so the painted text
        // is "a b c d".
        assert!(s.contains("a b c d"), "expected sanitized run in {s:?}");
    }

    #[test]
    fn status_overlay_zero_dimensions_is_noop() {
        let mut out = Vec::new();
        emit_status_overlay(&mut out, CellSize::new(0, 0), "x").unwrap();
        assert!(out.is_empty());

        let mut out = Vec::new();
        emit_status_overlay(&mut out, CellSize::new(5, 0), "x").unwrap();
        assert!(out.is_empty());

        let mut out = Vec::new();
        emit_status_overlay(&mut out, CellSize::new(0, 80), "x").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn status_overlay_empty_text_paints_full_row_of_spaces() {
        let size = CellSize::new(3, 6);
        let mut out = Vec::new();
        emit_status_overlay(&mut out, size, "").unwrap();
        let s = String::from_utf8_lossy(&out);

        assert!(s.contains("\x1b[3;1H"));
        assert_eq!(s.matches(' ').count(), 6, "expected 6 spaces in {s:?}");
    }

    #[test]
    fn clear_status_overlay_emits_move_then_default_style_spaces() {
        let size = CellSize::new(10, 8);
        let mut out = Vec::new();
        emit_clear_status_overlay(&mut out, size).unwrap();
        let s = String::from_utf8_lossy(&out);

        // Move to bottom row.
        assert!(s.contains("\x1b[10;1H"), "missing bottom-row move in {s:?}");
        // Reset attribute before painting so we don't inherit reverse
        // from a still-resident overlay.
        assert!(s.contains("\x1b[0m"), "missing attr reset in {s:?}");
        // Exactly `cols` spaces.
        assert_eq!(s.matches(' ').count(), 8);
        // No reverse attribute.
        assert!(!s.contains("\x1b[7m"), "unexpected reverse attr in {s:?}");
    }

    #[test]
    fn clear_status_overlay_zero_dimensions_is_noop() {
        let mut out = Vec::new();
        emit_clear_status_overlay(&mut out, CellSize::new(0, 0)).unwrap();
        assert!(out.is_empty());

        let mut out = Vec::new();
        emit_clear_status_overlay(&mut out, CellSize::new(5, 0)).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn color_conversion() {
        assert!(matches!(to_ct_color(Color::Default), CtColor::Reset));
        assert!(matches!(
            to_ct_color(Color::Rgb(1, 2, 3)),
            CtColor::Rgb { r: 1, g: 2, b: 3 }
        ));
        assert!(matches!(
            to_ct_color(Color::Indexed(42)),
            CtColor::AnsiValue(42)
        ));
    }
}
