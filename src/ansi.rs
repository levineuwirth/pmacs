// ansi.rs --- T M6.3 ANSI parser.

//! ANSI/VT100 escape-sequence parser for the REPL streaming pipeline.
//!
//! This module is a pure stream filter: bytes go in, [`AnsiEvent`]s
//! come out. The parser is responsible for the support/unsupport
//! split spelled out in spec §sec:ansi-scope --- SGR + intra-line
//! cursor motion + line-level erase + bracketed paste + OSC
//! title-setting are *supported*; alternate-screen, mouse tracking,
//! character-set switching, scrolling regions, and cursor save/restore
//! are *parsed but discarded*. Unknown sequences recover per the
//! per-state byte cap rule documented on [`AnsiParserConfig`].
//!
//! # State machine
//!
//! Hand-rolled, modeled on the canonical vt100.net / ECMA-48 parser
//! table. Hand-rolling (rather than vendoring a third-party crate
//! like `vte`) keeps the recovery rule from spec §sec:ansi-scope
//! --- 1 KiB cap, drop-to-ground on next `ESC`, never crash ---
//! directly inspectable as code instead of pinned to a dependency's
//! choices about failure modes.
//!
//! # Output stream split
//!
//! Style annotations are emitted as a separate stream from text
//! ([`AnsiEvent::SetStyle`] interleaved with [`AnsiEvent::Text`]):
//! the rope sees only literal text, and the consumer (the M6.4 REPL
//! view) maintains a single piece of state ("the active style is
//! whatever the most recent [`SetStyle`] said") and translates to
//! byte-range style annotations as it appends to the rope.
//!
//! # CSI parameters
//!
//! The parser collects CSI parameters as a list of *parameter
//! groups*, each a list of subparameters. This shape is committed
//! to from the start because retrofitting subparameter support
//! after the SGR mapper is written is painful: the modern variants
//! (`ESC [4:3 m` for curly underline, `ESC [38:2::R:G:B m` for
//! 24-bit color in the colon-separated form) require subparameter
//! parsing that the legacy `;`-only approach can't represent. With
//! [`CsiParam`] in hand both legacy `38;2;R;G;B` and modern
//! `38:2::R:G:B` become two cases of the same SGR loop.

use crate::cell::{Color, Style, UnderlineStyle};

/// Parser compatibility profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnsiParserProfile {
    /// Preserve the compile/REPL byte-stream contract.
    #[default]
    LineOriented,
    /// Emit terminal operations for a stateful full-screen consumer.
    FullScreen,
}

#[allow(missing_docs)]
/// Erase direction for display and line operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraseMode {
    ToEnd,
    ToStart,
    All,
    Saved,
}

#[allow(missing_docs)]
/// DEC alternate-screen selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlternateScreenMode {
    Mode47,
    Mode1047,
    Mode1049,
}

#[allow(missing_docs)]
/// Terminal modes understood by the screen/input core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalMode {
    Insert,
    Origin,
    AutoWrap,
    ApplicationCursor,
    ApplicationKeypad,
    CursorVisible,
    BracketedPaste,
    FocusReporting,
    SynchronizedOutput,
    MouseX10,
    MouseButton,
    MouseAny,
    MouseSgr,
}

#[allow(missing_docs)]
/// G0/G1 designation target and supported character set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterSetSlot {
    G0,
    G1,
}

/// Character set designated into a DEC G0/G1 slot.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterSet {
    Ascii,
    DecSpecialGraphics,
}

#[allow(missing_docs)]
/// Typed terminal query. Only these requests may generate PTY input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceRequest {
    PrimaryAttributes,
    SecondaryAttributes,
    OperatingStatus,
    CursorPosition,
}

// ---------------------------------------------------------------------------
// Public output
// ---------------------------------------------------------------------------

/// One observable effect of feeding bytes to an [`AnsiParser`].
///
/// One byte may produce zero or more events; one event is emitted
/// at the moment the parser has enough context to commit to it
/// (e.g., `Text` is emitted at every transition out of Ground, not
/// per byte).
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnsiEvent {
    /// Append literal text to the consumer's rope. Text never
    /// contains escape bytes (`0x1B`) --- that property is what makes
    /// stream-separation testable.
    Text(String),
    /// Set the active style. Subsequent `Text` events are rendered
    /// at this style until the next `SetStyle`. Spec §sec:ansi-scope.
    SetStyle(Style),
    /// Carriage return: move the input position to the start of the
    /// current input region. The REPL view interprets "input region"
    /// per spec §sec:repl-view.
    CarriageReturn,
    /// Backspace: move the input position one column left within
    /// the current input region.
    Backspace,
    /// Erase from the input position to the end of the line
    /// (`CSI K` or `CSI 0 K`).
    EraseToEol,
    /// Erase the entire current line (`CSI 2 K`).
    EraseLine,
    /// Set the window title (OSC 0 / OSC 2 with terminator).
    /// Exposed as a per-buffer attribute by the M6.4 REPL view.
    SetTitle(String),
    /// `OSC 133;A`: shell prompt begins.
    PromptStart,
    /// `OSC 133;B`: shell prompt ends.
    PromptEnd,
    /// `OSC 133;C`: command input begins.
    CommandStart,
    /// `OSC 133;D`: command output begins / command finished marker.
    OutputStart,
    /// `CSI 200 ~`: a process-emitted bracketed-paste begin marker.
    BracketedPasteBegin,
    /// `CSI 201 ~`: a process-emitted bracketed-paste end marker.
    BracketedPasteEnd,
    /// `CSI ? 1049 h`: alternate-screen entered. Subsequent `Text`
    /// and `SetStyle` events are *suppressed* until
    /// [`AnsiEvent::AlternateScreenExit`] --- the parser still
    /// advances state correctly but emits no text payload, per
    /// spec §sec:ansi-scope.
    AlternateScreenEnter,
    /// `CSI ? 1049 l`: alternate-screen exited. `Text` and
    /// `SetStyle` resume.
    AlternateScreenExit,
    /// Full-screen-only terminal operations.
    Bell,
    LineFeed,
    /// `ESC D`: advance one row, scrolling at the bottom margin.
    Index,
    /// `ESC E`: return to column zero and advance one row.
    NextLine,
    /// `ESC M`: move up one row, scrolling down at the top margin.
    ReverseIndex,
    HorizontalTab,
    SetTabStop,
    ClearTabStop,
    ClearAllTabStops,
    CursorUp(u32),
    CursorDown(u32),
    CursorForward(u32),
    CursorBackward(u32),
    CursorNextLine(u32),
    CursorPreviousLine(u32),
    CursorHorizontalAbsolute(u32),
    CursorVerticalAbsolute(u32),
    CursorPosition {
        row: u32,
        col: u32,
    },
    EraseDisplay(EraseMode),
    EraseLineMode(EraseMode),
    EraseCharacters(u32),
    InsertCharacters(u32),
    DeleteCharacters(u32),
    InsertLines(u32),
    DeleteLines(u32),
    ScrollUp(u32),
    ScrollDown(u32),
    SetScrollingRegion {
        top: u32,
        bottom: Option<u32>,
    },
    SaveCursor,
    RestoreCursor,
    AlternateScreen {
        mode: AlternateScreenMode,
        enabled: bool,
    },
    SetMode {
        mode: TerminalMode,
        enabled: bool,
    },
    DesignateCharacterSet {
        slot: CharacterSetSlot,
        charset: CharacterSet,
    },
    ShiftOut,
    ShiftIn,
    DeviceRequest(DeviceRequest),
}

/// Tunable knobs for [`AnsiParser`].
#[derive(Clone, Copy, Debug)]
pub struct AnsiParserConfig {
    /// Per-state byte budget for `Ignore`/`OscIgnore`/`DcsIgnore`
    /// states. *Reset to zero on every entry to an Ignore state*
    /// --- this is a per-state cap, not a global counter, so a
    /// feed containing N malformed sequences each gets its own
    /// budget. At the limit, the parser force-returns to `Ground`
    /// and discards the malformed sequence; subsequent input is
    /// processed normally as ordinary text. Default 1 KiB per
    /// spec §sec:ansi-scope.
    pub unknown_sequence_byte_limit: usize,
}

impl Default for AnsiParserConfig {
    fn default() -> Self {
        Self {
            unknown_sequence_byte_limit: 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// CSI parameter representation
// ---------------------------------------------------------------------------

/// One parameter group inside a CSI sequence.
///
/// `main` is the primary parameter value (digits before any `:`);
/// `sub` is the sequence of subparameters (digits between `:`s
/// after `main`, in order). Empty digits resolve to `0` per
/// ECMA-48 default-parameter convention. Empty `sub` is the common
/// case (any legacy `;`-only sequence).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CsiParam {
    /// Primary parameter. Empty input resolves to `0`.
    pub main: u32,
    /// Subparameters (the `:`-separated tail). Empty for legacy
    /// `;`-only sequences.
    pub sub: Vec<u32>,
}

/// Sequence of [`CsiParam`]s collected from a single CSI command.
pub type CsiParams = Vec<CsiParam>;

// ---------------------------------------------------------------------------
// Internal state machine
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // DCS sub-states wire up in stage 3; stage 1 only enters DcsEntry.
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    EscapeIgnore,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    OscString,
    OscIgnore,
    /// Sub-state of `OscString`: just saw `ESC` inside an OSC body;
    /// the next byte is the terminator-check (`\` ⇒ ST, else
    /// restart-from-Escape).
    OscEscPending,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    SosPmApcString,
}

/// CSI accumulator. Holds in-progress parameters as they arrive
/// byte-by-byte, plus the private-marker prefix (`?`, `<`, `=`,
/// `>`) and any intermediate bytes seen before the final byte.
#[derive(Default)]
struct CsiCollector {
    params: CsiParams,
    /// In-progress `main` value. `None` until the first digit;
    /// flushed into `params` on `;` or final byte.
    current_main: Option<u32>,
    /// In-progress sub list for the current group.
    current_subs: Vec<u32>,
    /// In-progress sub digit accumulator. `None` after a `:` but
    /// before the next digit; some `(0)` after a `:` if no digits
    /// yet have been seen for that sub (the empty-sub case, e.g.
    /// `38:2::R:G:B`).
    current_sub_acc: Option<u32>,
    /// `true` once we've seen at least one `:` in the current
    /// group --- subsequent digits accumulate into a sub, not main.
    in_subs: bool,
    /// Private marker byte (`?`, `<`, `=`, `>`) if the CSI started
    /// with one; `None` for ordinary CSI.
    private_marker: Option<u8>,
    /// Intermediate bytes (range `0x20..=0x2F`) seen between params
    /// and the final byte.
    intermediates: Vec<u8>,
}

impl CsiCollector {
    fn reset(&mut self) {
        self.params.clear();
        self.current_main = None;
        self.current_subs.clear();
        self.current_sub_acc = None;
        self.in_subs = false;
        self.private_marker = None;
        self.intermediates.clear();
    }

    /// Accumulate a digit into `main` (before any `:`) or into the
    /// current sub (after a `:`).
    fn push_digit(&mut self, d: u32) {
        if self.in_subs {
            let cur = self.current_sub_acc.unwrap_or(0);
            self.current_sub_acc = Some(cur.saturating_mul(10).saturating_add(d));
        } else {
            let cur = self.current_main.unwrap_or(0);
            self.current_main = Some(cur.saturating_mul(10).saturating_add(d));
        }
    }

    /// `:` between subparameters. Flush the in-progress sub digit
    /// (or `0` if empty) into `current_subs`.
    fn push_colon(&mut self) {
        if self.in_subs {
            // Flush prior sub.
            self.current_subs.push(self.current_sub_acc.unwrap_or(0));
            self.current_sub_acc = None;
        } else {
            // First `:` of this group: flush main if started, then
            // switch into sub mode. Note `main` may be `None` if the
            // group started with `:` (rare but legal).
            self.in_subs = true;
        }
    }

    /// `;` between parameter groups. Flush the in-progress group
    /// into `params` and reset for the next group.
    fn push_semicolon(&mut self) {
        // Close the in-progress sub if any.
        if self.in_subs {
            self.current_subs.push(self.current_sub_acc.unwrap_or(0));
            self.current_sub_acc = None;
        }
        let main = self.current_main.unwrap_or(0);
        let subs = std::mem::take(&mut self.current_subs);
        self.params.push(CsiParam { main, sub: subs });
        self.current_main = None;
        self.in_subs = false;
    }

    /// Final byte arrived: flush any in-progress group and return
    /// the collected parameters.
    fn finalize(&mut self) -> CsiParams {
        // Flush the last group, if any digits or subs are pending.
        let pending_main = self.current_main.is_some();
        let pending_subs = self.in_subs || !self.current_subs.is_empty();
        if pending_main || pending_subs {
            if self.in_subs {
                self.current_subs.push(self.current_sub_acc.unwrap_or(0));
                self.current_sub_acc = None;
            }
            let main = self.current_main.unwrap_or(0);
            let subs = std::mem::take(&mut self.current_subs);
            self.params.push(CsiParam { main, sub: subs });
        }
        std::mem::take(&mut self.params)
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Stateful ANSI parser. Feed bytes; collect [`AnsiEvent`]s.
///
/// State persists across `feed` calls --- a CSI started in one
/// chunk continues in the next. M6.2's coalescing makes per-tick
/// chunk boundaries common, so this property is load-bearing for
/// correctness, not just convenience.
pub struct AnsiParser {
    state: State,
    /// Current SGR state. Mutated by SGR parameters; emitted as
    /// [`AnsiEvent::SetStyle`] when it changes.
    current_style: Style,
    /// The style the CONSUMER last received via an emitted
    /// `SetStyle` event. Outside alternate-screen this always equals
    /// `current_style` (every SGR emits immediately); inside, SGR
    /// events are suppressed while `current_style` keeps advancing,
    /// so the two drift apart — and alternate-screen exit (ordinary
    /// or via [`Self::finish`]) must resynchronize the consumer from
    /// this field, not from an internal comparison against default
    /// (PR #113 round-4 finding 2).
    emitted_style: Style,
    /// Byte count consumed in the current Ignore state. Reset to
    /// zero on every entry to an Ignore state: this is a per-state
    /// budget, not a global counter, so each malformed sequence
    /// gets its own [`AnsiParserConfig::unknown_sequence_byte_limit`].
    ignore_byte_count: usize,
    /// In-progress text run accumulator. Flushed as a single
    /// [`AnsiEvent::Text`] when we leave Ground for any non-Ground
    /// state. Holds only complete (valid UTF-8) characters; partial
    /// multi-byte sequences live in `utf8_buf` until they complete.
    text_run: String,
    /// Pending UTF-8 bytes that haven't yet decoded to a complete
    /// scalar. Cross-feed buffer: a multi-byte sequence split across
    /// `feed()` calls accumulates here until the trailing byte
    /// arrives (or until a non-continuation byte invalidates the
    /// sequence, at which point we emit U+FFFD for the malformed
    /// bytes and continue). The buffer holds at most a few bytes
    /// (the longest valid UTF-8 sequence is 4 bytes; we cap at 8
    /// defensively). This replaces the M6.3-stage-1 shortcut where
    /// every non-ASCII byte was emitted as U+FFFD regardless of
    /// whether it was actually malformed.
    utf8_buf: Vec<u8>,
    /// Suppress `Text` and `SetStyle` events while alternate-screen
    /// is active. Spec §sec:ansi-scope: parser advances state
    /// normally but emits no payload.
    alt_screen_active: bool,
    csi: CsiCollector,
    /// In-progress OSC body bytes.
    osc_body: Vec<u8>,
    /// Intermediate bytes for plain ESC sequences (`ESC` + 0x20..=0x2F).
    escape_intermediates: Vec<u8>,
    profile: AnsiParserProfile,
    config: AnsiParserConfig,
}

impl Default for AnsiParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AnsiParser {
    /// Construct a parser with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::with_profile(AnsiParserProfile::LineOriented)
    }

    /// Construct a parser using the selected compatibility profile.
    #[must_use]
    pub fn with_profile(profile: AnsiParserProfile) -> Self {
        Self::with_profile_and_config(profile, AnsiParserConfig::default())
    }

    /// Construct a line-oriented parser with custom configuration.
    #[must_use]
    pub fn with_config(config: AnsiParserConfig) -> Self {
        Self::with_profile_and_config(AnsiParserProfile::LineOriented, config)
    }

    /// Construct a parser with both an explicit profile and configuration.
    #[must_use]
    pub fn with_profile_and_config(profile: AnsiParserProfile, config: AnsiParserConfig) -> Self {
        Self {
            state: State::Ground,
            current_style: Style::default(),
            emitted_style: Style::default(),
            ignore_byte_count: 0,
            text_run: String::new(),
            utf8_buf: Vec::new(),
            alt_screen_active: false,
            csi: CsiCollector::default(),
            osc_body: Vec::new(),
            escape_intermediates: Vec::new(),
            profile,
            config,
        }
    }

    /// Reset the parser to ground state. The running style is *not*
    /// reset --- callers that want a clean style should pair this
    /// with their own `SetStyle(Style::default())`. Neither is
    /// `emitted_style`: a mid-stream reset changes nothing about
    /// what the consumer has already been shown.
    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.ignore_byte_count = 0;
        self.text_run.clear();
        self.utf8_buf.clear();
        self.csi.reset();
        self.osc_body.clear();
        self.escape_intermediates.clear();
    }

    /// Feed a slice of bytes to the parser. Returns every event the
    /// bytes produced in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<AnsiEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            self.feed_byte(b, &mut events);
        }
        // Flush any pending text run at the end of the feed, but
        // only if we're in Ground (mid-CSI/mid-OSC text shouldn't
        // be emitted yet --- the in-flight escape might still
        // resolve and the bytes belong to it). Since we only build
        // text_run while in Ground, this is safe to flush
        // unconditionally as a final step.
        //
        // We do NOT flush `utf8_buf` here: an incomplete multi-byte
        // sequence at the feed boundary is exactly the case the
        // cross-feed buffer was added to handle --- the trailing
        // bytes are expected to arrive in the next feed. The state
        // transition path (flush_text_run) does emit U+FFFD for
        // pending bytes because a non-text byte genuinely
        // interrupts the sequence; feed-boundary doesn't.
        if !self.text_run.is_empty() {
            let run = std::mem::take(&mut self.text_run);
            if !self.suppress_visible() {
                events.push(AnsiEvent::Text(run));
            }
        }
        events
    }

    /// Stream-end finalization. The feed-boundary contract keeps an
    /// incomplete UTF-8 sequence buffered because its trailing bytes
    /// are expected in the next feed — but at process EOF there IS no
    /// next feed, so the pending prefix can never complete. Emit
    /// U+FFFD for it (the same posture `flush_text_run` takes when a
    /// control byte interrupts a sequence) and flush the resulting
    /// text run.
    ///
    /// The parser is then fully reset — in-flight CSI/OSC/escape
    /// state, alt-screen suppression, AND the running SGR style — so
    /// a `feed` after `finish` parses a NEW stream from a clean
    /// slate rather than continuing a pre-EOF escape sequence,
    /// staying suppressed, or inheriting stale color (PR #113
    /// round-2 finding 4; round-3 finding 2). The reset is
    /// OBSERVABLE: consumers that mirror parser state from the event
    /// stream receive balancing events — an `AlternateScreenExit`
    /// for an unclosed enter, a default `SetStyle` whenever the
    /// style the consumer LAST RECEIVED was non-default. The
    /// comparison is against `emitted_style`, not `current_style`:
    /// an SGR reset inside the alternate screen leaves the internal
    /// style default while the consumer still shows pre-enter color
    /// (round-4 finding 2). Idempotent once drained. First consumer:
    /// compile-mode's terminal-event path (Q#CM4).
    pub fn finish(&mut self) -> Vec<AnsiEvent> {
        let mut events = Vec::new();
        self.flush_pending_utf8_as_replacement();
        if !self.text_run.is_empty() {
            let run = std::mem::take(&mut self.text_run);
            if !self.suppress_visible() {
                events.push(AnsiEvent::Text(run));
            }
        }
        if self.profile == AnsiParserProfile::LineOriented {
            if self.alt_screen_active {
                self.alt_screen_active = false;
                events.push(AnsiEvent::AlternateScreenExit);
            }
            if self.emitted_style != Style::default() {
                events.push(AnsiEvent::SetStyle(Style::default()));
            }
        }
        self.alt_screen_active = false;
        self.current_style = Style::default();
        self.emitted_style = Style::default();
        self.reset();
        events
    }

    fn feed_byte(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        // ESC-anywhere rule: ECMA-48 §10.2 ("Cancel"). Aborts any
        // in-progress sequence and starts a fresh Escape state. The
        // exception is OscString, where ESC begins a two-byte ST
        // terminator check (handled in OscString below).
        if b == 0x1B && !matches!(self.state, State::OscString | State::OscEscPending) {
            self.flush_text_run(events);
            self.start_new_sequence();
            return;
        }
        // CAN (0x18) and SUB (0x1A) cancel any in-progress sequence
        // and return to Ground.
        if matches!(b, 0x18 | 0x1A) {
            self.flush_text_run(events);
            self.recover_to_ground();
            return;
        }

        // Bound retained control-string payload without ever exposing its
        // overflow as printable text. Once capped, remain in a zero-storage
        // ignore state until BEL/ST or a fresh ESC sequence provides a safe
        // recovery boundary.
        if self.state != State::Ground
            && !matches!(
                self.state,
                State::EscapeIgnore | State::CsiIgnore | State::OscIgnore | State::DcsIgnore
            )
        {
            self.ignore_byte_count = self.ignore_byte_count.saturating_add(1);
            if self.ignore_byte_count > self.config.unknown_sequence_byte_limit {
                match self.state {
                    State::OscString | State::OscEscPending => {
                        self.osc_body.clear();
                        self.state = State::OscIgnore;
                        self.ignore_byte_count = 0;
                    }
                    State::DcsEntry
                    | State::DcsParam
                    | State::DcsIntermediate
                    | State::DcsPassthrough
                    | State::SosPmApcString => {
                        self.state = State::DcsIgnore;
                        self.ignore_byte_count = 0;
                    }
                    State::Escape | State::EscapeIntermediate => {
                        self.escape_intermediates.clear();
                        self.state = State::EscapeIgnore;
                        self.ignore_byte_count = 0;
                    }
                    State::CsiEntry | State::CsiParam | State::CsiIntermediate => {
                        self.csi.reset();
                        self.state = State::CsiIgnore;
                        self.ignore_byte_count = 0;
                    }
                    _ => self.recover_to_ground(),
                }
                return;
            }
        }

        match self.state {
            State::Ground => self.feed_ground(b, events),
            State::Escape => self.feed_escape(b, events),
            State::EscapeIntermediate => self.feed_escape_intermediate(b, events),
            State::EscapeIgnore => self.feed_escape_ignore(b),
            State::CsiEntry => self.feed_csi_entry(b, events),
            State::CsiParam => self.feed_csi_param(b, events),
            State::CsiIntermediate => self.feed_csi_intermediate(b, events),
            State::CsiIgnore => self.feed_csi_ignore(b),
            State::OscString => self.feed_osc_string(b, events),
            State::OscEscPending => self.feed_osc_esc_pending(b, events),
            State::OscIgnore => self.feed_osc_ignore(b),
            State::DcsEntry
            | State::DcsParam
            | State::DcsIntermediate
            | State::DcsPassthrough
            | State::DcsIgnore
            | State::SosPmApcString => {
                // DCS / SOS / PM / APC are parsed-and-discarded
                // per spec §sec:ansi-scope. The body bytes drop on
                // the floor; the ESC-anywhere rule terminates the
                // sequence on the next ESC, and the per-state byte
                // cap is a backstop.
            }
        }
    }

    // -----------------------------------------------------------------------
    // Text run buffering
    // -----------------------------------------------------------------------

    fn flush_text_run(&mut self, events: &mut Vec<AnsiEvent>) {
        // Any pending UTF-8 prefix at this point is interrupted by
        // a non-text byte (control byte, CSI start, etc.); the
        // sequence can't continue across that boundary. Emit U+FFFD
        // for the incomplete bytes before committing the run.
        self.flush_pending_utf8_as_replacement();
        if self.text_run.is_empty() {
            return;
        }
        let run = std::mem::take(&mut self.text_run);
        if !self.suppress_visible() {
            events.push(AnsiEvent::Text(run));
        }
    }

    fn emit_set_style(&mut self, events: &mut Vec<AnsiEvent>) {
        if self.suppress_visible() {
            return;
        }
        self.emitted_style = self.current_style;
        events.push(AnsiEvent::SetStyle(self.current_style));
    }

    /// Helper: emit `ev` if alt-screen is not active; drop otherwise.
    /// Used for visible side-effects (CR / BS / Erase / Bracketed
    /// paste / `SetTitle`). The alt-screen markers themselves
    /// bypass this.
    fn push_visible(&self, ev: AnsiEvent, events: &mut Vec<AnsiEvent>) {
        if !self.suppress_visible() {
            events.push(ev);
        }
    }

    fn suppress_visible(&self) -> bool {
        self.profile == AnsiParserProfile::LineOriented && self.alt_screen_active
    }

    /// Begin a fresh escape sequence (called from ESC-anywhere).
    /// Resets the byte budget and all in-flight sequence state.
    fn start_new_sequence(&mut self) {
        self.csi.reset();
        self.osc_body.clear();
        self.escape_intermediates.clear();
        self.ignore_byte_count = 0;
        self.state = State::Escape;
    }

    /// Force-recover to Ground. Discards in-flight sequence state
    /// and resets the byte budget. Called on CAN/SUB, on byte-cap
    /// overflow, and after a successful sequence dispatch.
    fn recover_to_ground(&mut self) {
        self.csi.reset();
        self.osc_body.clear();
        self.escape_intermediates.clear();
        self.ignore_byte_count = 0;
        self.state = State::Ground;
    }

    // -----------------------------------------------------------------------
    // Ground
    // -----------------------------------------------------------------------

    fn feed_ground(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        if self.profile == AnsiParserProfile::LineOriented {
            match b {
                0x0D => {
                    self.flush_text_run(events);
                    self.push_visible(AnsiEvent::CarriageReturn, events);
                }
                0x08 => {
                    self.flush_text_run(events);
                    self.push_visible(AnsiEvent::Backspace, events);
                }
                0x07 | 0x09 | 0x0A | 0x0B | 0x0C | 0x20..=0x7E | 0x80..=0xFF => {
                    self.push_text_byte(b);
                }
                0x00..=0x1F | 0x7F => {}
            }
            return;
        }
        match b {
            0x07 => {
                self.flush_text_run(events);
                events.push(AnsiEvent::Bell);
            }
            0x08 => {
                self.flush_text_run(events);
                events.push(AnsiEvent::Backspace);
            }
            0x09 => {
                self.flush_text_run(events);
                events.push(AnsiEvent::HorizontalTab);
            }
            0x0A..=0x0C => {
                self.flush_text_run(events);
                events.push(AnsiEvent::LineFeed);
            }
            0x0D => {
                self.flush_text_run(events);
                events.push(AnsiEvent::CarriageReturn);
            }
            0x0E => {
                self.flush_text_run(events);
                events.push(AnsiEvent::ShiftOut);
            }
            0x0F => {
                self.flush_text_run(events);
                events.push(AnsiEvent::ShiftIn);
            }
            0x20..=0x7E | 0x80..=0xFF => self.push_text_byte(b),
            0x00..=0x1F | 0x7F => {}
        }
    }

    /// Append a single text byte, handling multi-byte UTF-8
    /// correctly across feed boundaries.
    ///
    /// ASCII bytes (0x00..=0x7F) take a fast path directly into
    /// `text_run` when no partial sequence is pending. Non-ASCII
    /// bytes (0x80..=0xFF) and any byte arriving while a partial
    /// sequence is pending go through `utf8_buf`, which is then
    /// greedily decoded by `try_decode_utf8_buf`. Complete scalars
    /// flush to `text_run`; an incomplete trailing sequence stays
    /// in `utf8_buf` for the next byte (whether next-feed or
    /// later in the same feed).
    ///
    /// Malformed sequences (lone continuation, invalid start byte,
    /// non-continuation arriving where one was expected, overlong
    /// encoding) emit `U+FFFD` for the offending bytes only and
    /// resume decoding the rest --- they do not corrupt subsequent
    /// valid UTF-8.
    fn push_text_byte(&mut self, b: u8) {
        if self.utf8_buf.is_empty() && b < 0x80 {
            self.text_run.push(b as char);
            return;
        }
        self.utf8_buf.push(b);
        self.try_decode_utf8_buf();
    }

    /// Greedy UTF-8 decoder over `utf8_buf`. Pushes complete chars
    /// to `text_run`; emits `U+FFFD` for malformed bytes; leaves
    /// the trailing incomplete prefix in `utf8_buf` for the next
    /// byte to complete.
    ///
    /// Caps the buffer at 8 bytes defensively: a valid UTF-8
    /// sequence is at most 4 bytes, so any trailing run longer
    /// than that is malformed (we'd have hit either a complete
    /// scalar or a `from_utf8` error before reaching 8). The cap
    /// bounds the pathological-input memory cost.
    fn try_decode_utf8_buf(&mut self) {
        loop {
            if self.utf8_buf.is_empty() {
                return;
            }
            match std::str::from_utf8(&self.utf8_buf) {
                Ok(s) => {
                    // Whole buffer is valid UTF-8: flush all of it.
                    self.text_run.push_str(s);
                    self.utf8_buf.clear();
                    return;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    if valid > 0 {
                        // Push the valid prefix.
                        let prefix = std::str::from_utf8(&self.utf8_buf[..valid])
                            .expect("invariant: valid_up_to bytes are valid UTF-8");
                        self.text_run.push_str(prefix);
                        self.utf8_buf.drain(..valid);
                    }
                    match e.error_len() {
                        None => {
                            // Trailing incomplete sequence; wait for
                            // more bytes (next push_text_byte or next
                            // feed). The 8-byte defensive cap below
                            // guards against a pathological producer
                            // that never finishes a sequence.
                            if self.utf8_buf.len() >= 8 {
                                self.text_run.push('\u{FFFD}');
                                self.utf8_buf.clear();
                            }
                            return;
                        }
                        Some(n) => {
                            // n bytes after the valid prefix are an
                            // invalid sequence: emit U+FFFD for them
                            // and continue with whatever follows.
                            self.text_run.push('\u{FFFD}');
                            self.utf8_buf.drain(..n);
                            // Loop to retry.
                        }
                    }
                }
            }
        }
    }

    /// Drain any pending UTF-8 prefix as `U+FFFD`. Called from
    /// `flush_text_run` when the text run is being committed
    /// because we're transitioning out of Ground (a control byte,
    /// a CSI start, etc.). At that boundary, an unfinished
    /// multi-byte sequence is genuinely interrupted --- it can't
    /// continue across the non-text bytes --- so we emit the
    /// replacement character and clear.
    ///
    /// Not called at end-of-feed: a sequence interrupted by feed
    /// boundary may legitimately resume in the next feed.
    fn flush_pending_utf8_as_replacement(&mut self) {
        if !self.utf8_buf.is_empty() {
            self.text_run.push('\u{FFFD}');
            self.utf8_buf.clear();
        }
    }

    // -----------------------------------------------------------------------
    // Escape
    // -----------------------------------------------------------------------

    fn feed_escape(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        match b {
            0x20..=0x2F => {
                self.escape_intermediates.push(b);
                self.state = State::EscapeIntermediate;
            }
            b'[' => {
                self.csi.reset();
                self.state = State::CsiEntry;
            }
            b']' => {
                self.osc_body.clear();
                self.state = State::OscString;
            }
            b'P' => self.state = State::DcsEntry,
            b'X' | b'^' | b'_' => self.state = State::SosPmApcString,
            b'7' | b'8' | b'D' | b'E' | b'H' | b'M' | b'=' | b'>'
                if self.profile == AnsiParserProfile::FullScreen =>
            {
                let event = match b {
                    b'7' => AnsiEvent::SaveCursor,
                    b'8' => AnsiEvent::RestoreCursor,
                    b'D' => AnsiEvent::Index,
                    b'E' => AnsiEvent::NextLine,
                    b'H' => AnsiEvent::SetTabStop,
                    b'M' => AnsiEvent::ReverseIndex,
                    b'=' => AnsiEvent::SetMode {
                        mode: TerminalMode::ApplicationKeypad,
                        enabled: true,
                    },
                    _ => AnsiEvent::SetMode {
                        mode: TerminalMode::ApplicationKeypad,
                        enabled: false,
                    },
                };
                events.push(event);
                self.recover_to_ground();
            }
            b'\\' | 0x30..=0x7E => self.recover_to_ground(),
            _ => {}
        }
    }

    fn feed_escape_intermediate(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        match b {
            0x20..=0x2F => self.escape_intermediates.push(b),
            0x30..=0x7E => {
                if self.profile == AnsiParserProfile::FullScreen {
                    let slot = match self.escape_intermediates.as_slice() {
                        [b'('] => Some(CharacterSetSlot::G0),
                        [b')'] => Some(CharacterSetSlot::G1),
                        _ => None,
                    };
                    let charset = match b {
                        b'0' => Some(CharacterSet::DecSpecialGraphics),
                        b'B' => Some(CharacterSet::Ascii),
                        _ => None,
                    };
                    if let (Some(slot), Some(charset)) = (slot, charset) {
                        events.push(AnsiEvent::DesignateCharacterSet { slot, charset });
                    }
                }
                self.recover_to_ground();
            }
            _ => {}
        }
    }

    fn feed_escape_ignore(&mut self, b: u8) {
        if matches!(b, 0x30..=0x7E) {
            self.recover_to_ground();
        }
    }

    // -----------------------------------------------------------------------
    // CSI
    // -----------------------------------------------------------------------

    fn feed_csi_entry(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        match b {
            0x3C..=0x3F => {
                self.csi.private_marker = Some(b);
                self.state = State::CsiParam;
            }
            b'0'..=b'9' => {
                self.csi.push_digit(u32::from(b - b'0'));
                self.state = State::CsiParam;
            }
            b':' => {
                self.csi.push_colon();
                self.state = State::CsiParam;
            }
            b';' => {
                self.csi.push_semicolon();
                self.state = State::CsiParam;
            }
            0x20..=0x2F => {
                self.csi.intermediates.push(b);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7E => {
                self.dispatch_csi(b, events);
                self.recover_to_ground();
            }
            _ => self.state = State::CsiIgnore,
        }
    }

    fn feed_csi_param(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        match b {
            b'0'..=b'9' => self.csi.push_digit(u32::from(b - b'0')),
            b':' => self.csi.push_colon(),
            b';' => self.csi.push_semicolon(),
            0x20..=0x2F => {
                self.csi.intermediates.push(b);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7E => {
                self.dispatch_csi(b, events);
                self.recover_to_ground();
            }
            _ => self.state = State::CsiIgnore,
        }
    }

    fn feed_csi_intermediate(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        match b {
            0x20..=0x2F => self.csi.intermediates.push(b),
            0x40..=0x7E => {
                self.dispatch_csi(b, events);
                self.recover_to_ground();
            }
            _ => self.state = State::CsiIgnore,
        }
    }

    fn feed_csi_ignore(&mut self, b: u8) {
        // The per-state byte cap is enforced at the top of
        // `feed_byte`; here we only watch for the structural
        // terminator.
        if matches!(b, 0x40..=0x7E) {
            self.recover_to_ground();
        }
    }

    /// Dispatch a fully-collected CSI sequence. `final_byte` is the
    /// terminating byte (`0x40..=0x7E`). The collected parameters
    /// are taken from `self.csi`.
    #[allow(clippy::too_many_lines)]
    fn dispatch_csi(&mut self, final_byte: u8, events: &mut Vec<AnsiEvent>) {
        let private = self.csi.private_marker;
        let intermediates = self.csi.intermediates.clone();
        let params = self.csi.finalize();
        if private.is_none() && final_byte == b'm' {
            self.dispatch_sgr(&params, events);
            self.csi.reset();
            return;
        }
        if self.profile == AnsiParserProfile::LineOriented {
            match (private, final_byte) {
                (None, b'K') => match param(&params, 0, 0) {
                    0 => self.push_visible(AnsiEvent::EraseToEol, events),
                    2 => self.push_visible(AnsiEvent::EraseLine, events),
                    _ => {}
                },
                (None, b'~') => match param(&params, 0, 0) {
                    200 => self.push_visible(AnsiEvent::BracketedPasteBegin, events),
                    201 => self.push_visible(AnsiEvent::BracketedPasteEnd, events),
                    _ => {}
                },
                (Some(b'?'), b'h' | b'l') => {
                    let set = final_byte == b'h';
                    for p in &params {
                        if p.main == 1049 {
                            if set && !self.alt_screen_active {
                                self.alt_screen_active = true;
                                events.push(AnsiEvent::AlternateScreenEnter);
                            } else if !set && self.alt_screen_active {
                                self.alt_screen_active = false;
                                events.push(AnsiEvent::AlternateScreenExit);
                                if self.current_style != self.emitted_style {
                                    self.emit_set_style(events);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            self.csi.reset();
            return;
        }

        let count = || param(&params, 0, 1).max(1);
        let event = match (private, intermediates.as_slice(), final_byte) {
            (None, [], b'A') => Some(AnsiEvent::CursorUp(count())),
            (None, [], b'B') => Some(AnsiEvent::CursorDown(count())),
            (None, [], b'C' | b'a') => Some(AnsiEvent::CursorForward(count())),
            (None, [], b'D') => Some(AnsiEvent::CursorBackward(count())),
            (None, [], b'E') => Some(AnsiEvent::CursorNextLine(count())),
            (None, [], b'F') => Some(AnsiEvent::CursorPreviousLine(count())),
            (None, [], b'G' | b'`') => Some(AnsiEvent::CursorHorizontalAbsolute(
                param(&params, 0, 1).max(1),
            )),
            (None, [], b'd') => Some(AnsiEvent::CursorVerticalAbsolute(
                param(&params, 0, 1).max(1),
            )),
            (None, [], b'H' | b'f') => Some(AnsiEvent::CursorPosition {
                row: param(&params, 0, 1).max(1),
                col: param(&params, 1, 1).max(1),
            }),
            (None, [], b'J') => erase_mode(param(&params, 0, 0)).map(AnsiEvent::EraseDisplay),
            (None, [], b'K') => erase_mode(param(&params, 0, 0)).map(AnsiEvent::EraseLineMode),
            (None, [], b'X') => Some(AnsiEvent::EraseCharacters(count())),
            (None, [], b'@') => Some(AnsiEvent::InsertCharacters(count())),
            (None, [], b'P') => Some(AnsiEvent::DeleteCharacters(count())),
            (None, [], b'L') => Some(AnsiEvent::InsertLines(count())),
            (None, [], b'M') => Some(AnsiEvent::DeleteLines(count())),
            (None, [], b'S') => Some(AnsiEvent::ScrollUp(count())),
            (None, [], b'T') => Some(AnsiEvent::ScrollDown(count())),
            (None, [], b'r') => Some(AnsiEvent::SetScrollingRegion {
                top: param(&params, 0, 1).max(1),
                bottom: params.get(1).map(|p| p.main).filter(|&n| n != 0),
            }),
            (None, [], b's') => Some(AnsiEvent::SaveCursor),
            (None, [], b'u') => Some(AnsiEvent::RestoreCursor),
            (None, [], b'g') => match param(&params, 0, 0) {
                0 => Some(AnsiEvent::ClearTabStop),
                3 => Some(AnsiEvent::ClearAllTabStops),
                _ => None,
            },
            (None, [], b'~') => match param(&params, 0, 0) {
                200 => Some(AnsiEvent::BracketedPasteBegin),
                201 => Some(AnsiEvent::BracketedPasteEnd),
                _ => None,
            },
            (None, [], b'h' | b'l') => {
                let enabled = final_byte == b'h';
                for p in &params {
                    if p.main == 4 {
                        events.push(AnsiEvent::SetMode {
                            mode: TerminalMode::Insert,
                            enabled,
                        });
                    }
                }
                None
            }
            (Some(b'?'), [], b'h' | b'l') => {
                let enabled = final_byte == b'h';
                for p in &params {
                    if let Some(ev) = private_mode_event(p.main, enabled) {
                        events.push(ev);
                    }
                }
                None
            }
            (None, [], b'c') => Some(AnsiEvent::DeviceRequest(DeviceRequest::PrimaryAttributes)),
            (Some(b'>'), [], b'c') => {
                Some(AnsiEvent::DeviceRequest(DeviceRequest::SecondaryAttributes))
            }
            (None, [], b'n') => match param(&params, 0, 0) {
                5 => Some(AnsiEvent::DeviceRequest(DeviceRequest::OperatingStatus)),
                6 => Some(AnsiEvent::DeviceRequest(DeviceRequest::CursorPosition)),
                _ => None,
            },
            _ => None,
        };
        if let Some(event) = event {
            events.push(event);
        }
        self.csi.reset();
    }

    // -----------------------------------------------------------------------
    // SGR --- stage 1 subset (reset, bold/italic/reverse, basic 8-color)
    // -----------------------------------------------------------------------

    // Each SGR pattern carries its own per-pattern comment
    // documenting the spec policy (faint, blink-as-bold,
    // strikethrough, etc.); collapsing the no-op patterns into one
    // arm would erase that documentation.
    #[allow(clippy::match_same_arms)]
    fn dispatch_sgr(&mut self, params: &CsiParams, events: &mut Vec<AnsiEvent>) {
        // Empty parameter list = reset (CSI m).
        if params.is_empty() {
            self.current_style = Style::default();
            self.emit_set_style(events);
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = &params[i];
            let mut consumed_extra = 0usize;
            match p.main {
                0 => self.current_style = Style::default(),
                1 => self.current_style.bold = true,
                // 2 (faint): no-op. The cell `Style` has no faint
                // field; advance state without modifying running
                // style. Spec lists faint among supported attrs but
                // our representation collapses it; preserved as a
                // possible future `Style` field per the M11
                // measurement-driven discipline.
                2 => {}
                3 => self.current_style.italic = true,
                4 => {
                    self.current_style.underline = match p.sub.first().copied() {
                        Some(0) => UnderlineStyle::None,
                        None | Some(1) => UnderlineStyle::Single,
                        Some(2) => UnderlineStyle::Double,
                        Some(3) => UnderlineStyle::Curly,
                        Some(4) => UnderlineStyle::Dotted,
                        Some(5) => UnderlineStyle::Dashed,
                        _ => UnderlineStyle::Single,
                    };
                }
                // Line-oriented compile/REPL consumers historically render
                // blink as bold. Full-screen preserves the shared Style
                // contract: blink is unsupported and leaves it unchanged.
                5 | 6 if self.profile == AnsiParserProfile::LineOriented => {
                    self.current_style.bold = true;
                }
                5 | 6 => {}
                7 => self.current_style.reverse = true,
                // 8 (concealed/invisible): no-op. Out of scope.
                8 => {}
                // 9 (strikethrough): no-op. Spec §sec:ansi-scope
                // lists it among supported attrs; the cell `Style`
                // has no strikethrough field, so advance state
                // without modifying running style. Stage 4
                // regression test asserts the running style is
                // *unchanged* (not corrupted) by SGR 9.
                9 => {}
                22 => self.current_style.bold = false,
                23 => self.current_style.italic = false,
                24 => self.current_style.underline = UnderlineStyle::None,
                // 25 (blink off): unsupported. It must not unset bold
                // acquired through SGR 1.
                25 => {}
                27 => self.current_style.reverse = false,
                28 => {}
                29 => {}
                30..=37 => {
                    self.current_style.fg = Color::Indexed(u8::try_from(p.main - 30).unwrap_or(0));
                }
                38 => {
                    if let Some((color, extra)) = parse_extended_color(p, &params[i + 1..]) {
                        self.current_style.fg = color;
                        consumed_extra = extra;
                    }
                }
                39 => self.current_style.fg = Color::Default,
                40..=47 => {
                    self.current_style.bg = Color::Indexed(u8::try_from(p.main - 40).unwrap_or(0));
                }
                48 => {
                    if let Some((color, extra)) = parse_extended_color(p, &params[i + 1..]) {
                        self.current_style.bg = color;
                        consumed_extra = extra;
                    }
                }
                49 => self.current_style.bg = Color::Default,
                // 58/59 (underline color, kitty/mintty extension):
                // same extended-color grammar as 38/48 (T M4.6).
                58 => {
                    if let Some((color, extra)) = parse_extended_color(p, &params[i + 1..]) {
                        self.current_style.underline_color = color;
                        consumed_extra = extra;
                    }
                }
                59 => self.current_style.underline_color = Color::Default,
                90..=97 => {
                    self.current_style.fg =
                        Color::Indexed(u8::try_from(p.main - 90 + 8).unwrap_or(0));
                }
                100..=107 => {
                    self.current_style.bg =
                        Color::Indexed(u8::try_from(p.main - 100 + 8).unwrap_or(0));
                }
                _ => {} // Unknown SGR: drop silently, advance state.
            }
            i += 1 + consumed_extra;
        }
        self.emit_set_style(events);
    }

    // -----------------------------------------------------------------------
    // OSC (stage 2 will fill this in)
    // -----------------------------------------------------------------------

    fn feed_osc_string(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        match b {
            // BEL: terminator (xterm convention).
            0x07 => {
                self.dispatch_osc(events);
                self.recover_to_ground();
            }
            // ESC: begin ST-terminator check (ESC \).
            0x1B => self.state = State::OscEscPending,
            // OSC payload is UTF-8 bytes, not ASCII. Retain printable ASCII,
            // DEL (compatibility), and all high bytes; lossy UTF-8 decoding at
            // dispatch replaces malformed sequences. The per-state cap bounds
            // retained storage.
            0x20..=0xFF => self.osc_body.push(b),
            // Other C0 controls: drop silently, stay in OSC.
            _ => {}
        }
    }

    fn feed_osc_esc_pending(&mut self, b: u8, events: &mut Vec<AnsiEvent>) {
        if b == b'\\' {
            self.dispatch_osc(events);
            self.recover_to_ground();
        } else {
            // Not ST: restart from Escape with this byte as the
            // first byte of a fresh sequence.
            self.osc_body.clear();
            self.ignore_byte_count = 0;
            self.state = State::Escape;
            self.feed_byte(b, events);
        }
    }

    fn feed_osc_ignore(&mut self, b: u8) {
        // Per-state byte cap is enforced at the top of `feed_byte`;
        // here we only watch for the structural terminator.
        match b {
            0x07 => self.recover_to_ground(),
            0x1B => self.state = State::OscEscPending,
            _ => {}
        }
    }

    fn dispatch_osc(&mut self, events: &mut Vec<AnsiEvent>) {
        let body = std::mem::take(&mut self.osc_body);
        // OSC body is `<num>;<text>`. Parse the numeric prefix; the
        // tail (after the first `;`) is the OSC payload.
        let (num_part, text_part) = match body.iter().position(|&b| b == b';') {
            Some(idx) => (&body[..idx], &body[idx + 1..]),
            None => (body.as_slice(), &b""[..]),
        };
        let num: Option<u32> = std::str::from_utf8(num_part)
            .ok()
            .and_then(|s| s.parse().ok());
        if matches!(num, Some(133)) && !self.suppress_visible() {
            match text_part.first().copied() {
                Some(b'A') => events.push(AnsiEvent::PromptStart),
                Some(b'B') => events.push(AnsiEvent::PromptEnd),
                Some(b'C') => events.push(AnsiEvent::CommandStart),
                Some(b'D') => events.push(AnsiEvent::OutputStart),
                _ => {}
            }
            return;
        }

        // Only OSC 0 (set icon name + window title), OSC 2 (set
        // window title), and the OSC 133 shell integration markers
        // above produce events. Other OSC numbers are parsed and
        // discarded per spec §sec:ansi-scope, with the critical
        // guarantee that state alignment is preserved.
        if matches!(num, Some(0 | 2)) && !self.suppress_visible() {
            let title = String::from_utf8_lossy(text_part).into_owned();
            events.push(AnsiEvent::SetTitle(title));
        }
    }
}

fn param(params: &CsiParams, index: usize, default: u32) -> u32 {
    params
        .get(index)
        .map_or(default, |p| if p.main == 0 { default } else { p.main })
}

fn erase_mode(value: u32) -> Option<EraseMode> {
    match value {
        0 => Some(EraseMode::ToEnd),
        1 => Some(EraseMode::ToStart),
        2 => Some(EraseMode::All),
        3 => Some(EraseMode::Saved),
        _ => None,
    }
}

fn private_mode_event(value: u32, enabled: bool) -> Option<AnsiEvent> {
    let mode = match value {
        47 => {
            return Some(AnsiEvent::AlternateScreen {
                mode: AlternateScreenMode::Mode47,
                enabled,
            });
        }
        1047 => {
            return Some(AnsiEvent::AlternateScreen {
                mode: AlternateScreenMode::Mode1047,
                enabled,
            });
        }
        1049 => {
            return Some(AnsiEvent::AlternateScreen {
                mode: AlternateScreenMode::Mode1049,
                enabled,
            });
        }
        1 => TerminalMode::ApplicationCursor,
        6 => TerminalMode::Origin,
        7 => TerminalMode::AutoWrap,
        25 => TerminalMode::CursorVisible,
        66 => TerminalMode::ApplicationKeypad,
        1000 => TerminalMode::MouseX10,
        1002 => TerminalMode::MouseButton,
        1003 => TerminalMode::MouseAny,
        1004 => TerminalMode::FocusReporting,
        1006 => TerminalMode::MouseSgr,
        2004 => TerminalMode::BracketedPaste,
        2026 => TerminalMode::SynchronizedOutput,
        _ => return None,
    };
    Some(AnsiEvent::SetMode { mode, enabled })
}

/// Parse a CSI 38/48 extended-color suffix into a `Color` plus
/// the number of *additional* params consumed (legacy form only;
/// the modern subparam form keeps everything inside `p.sub` so
/// `consumed_extra` is 0).
///
/// - Modern: `38:5:N` → sub = `[5, N]`, `38:2::R:G:B` → sub =
///   `[2, 0, R, G, B]` (the empty colorspace ID at index 1 is the
///   distinguishing feature of the canonical form).
/// - Legacy: `38;5;N` → next param `main=5`, then param `main=N`;
///   `38;2;R;G;B` → next params `main=2`, `R`, `G`, `B`.
///
/// Returns `None` if the suffix is malformed (truncated, unknown
/// kind byte). The caller silently drops the SGR token in that
/// case --- state stays aligned.
fn parse_extended_color(p: &CsiParam, rest: &[CsiParam]) -> Option<(Color, usize)> {
    if !p.sub.is_empty() {
        let kind = p.sub[0];
        return match kind {
            5 => {
                let n = *p.sub.get(1)?;
                Some((Color::Indexed(u8::try_from(n).unwrap_or(255)), 0))
            }
            2 => {
                // 5-element form has the colorspace ID at index 1
                // (typically 0); 4-element form omits it.
                let (r, g, b) = if p.sub.len() >= 5 {
                    (p.sub[2], p.sub[3], p.sub[4])
                } else if p.sub.len() >= 4 {
                    (p.sub[1], p.sub[2], p.sub[3])
                } else {
                    return None;
                };
                Some((
                    Color::Rgb(
                        u8::try_from(r).unwrap_or(255),
                        u8::try_from(g).unwrap_or(255),
                        u8::try_from(b).unwrap_or(255),
                    ),
                    0,
                ))
            }
            _ => None,
        };
    }
    let kind = rest.first()?.main;
    match kind {
        5 => {
            let n = rest.get(1)?.main;
            Some((Color::Indexed(u8::try_from(n).unwrap_or(255)), 2))
        }
        2 => {
            let r = rest.get(1)?.main;
            let g = rest.get(2)?.main;
            let b = rest.get(3)?.main;
            Some((
                Color::Rgb(
                    u8::try_from(r).unwrap_or(255),
                    u8::try_from(g).unwrap_or(255),
                    u8::try_from(b).unwrap_or(255),
                ),
                4,
            ))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: collect every text-payload byte across `Text` events
    /// in `evs`. Used to assert acceptance bullet 3 ("rope stays
    /// clean text") --- the rope-bound stream must never contain
    /// escape bytes.
    fn collect_text(evs: &[AnsiEvent]) -> String {
        let mut out = String::new();
        for ev in evs {
            if let AnsiEvent::Text(s) = ev {
                out.push_str(s);
            }
        }
        out
    }

    /// Helper: locate the indices of all `SetStyle` events in `evs`,
    /// in arrival order, paired with their `Style` payloads.
    fn collect_styles(evs: &[AnsiEvent]) -> Vec<Style> {
        evs.iter()
            .filter_map(|e| {
                if let AnsiEvent::SetStyle(s) = e {
                    Some(*s)
                } else {
                    None
                }
            })
            .collect()
    }

    // -----------------------------------------------------------------
    // Acceptance bullet 1: ls/git diff/grep --color render correctly
    // -----------------------------------------------------------------

    /// `ls --color=always` emits SGR with bold + indexed color
    /// (e.g. directory = bold blue, executable = bold green).
    /// Source pattern captured 2026-05-03 from
    /// `/bin/ls --color=always /tmp` on a stock GNU ls runner.
    #[test]
    fn m6_3_sgr_ls_color_render() {
        let mut p = AnsiParser::new();
        // \x1b[01;34m = bold + blue fg; "dir" text; reset; newline.
        let evs = p.feed(b"\x1b[01;34mdir\x1b[0m\n\x1b[01;31mfile\x1b[0m\n");
        let styles = collect_styles(&evs);
        // Expected order: bold+blue, reset, bold+red, reset.
        assert_eq!(styles.len(), 4, "expected 4 SetStyle events, got {evs:?}");
        assert!(
            styles[0].bold && styles[0].fg == Color::Indexed(4),
            "first style should be bold + blue, got {:?}",
            styles[0]
        );
        assert_eq!(styles[1], Style::default(), "second style should be reset");
        assert!(
            styles[2].bold && styles[2].fg == Color::Indexed(1),
            "third style should be bold + red, got {:?}",
            styles[2]
        );
        assert_eq!(styles[3], Style::default(), "fourth style should be reset");
        assert_eq!(collect_text(&evs), "dir\nfile\n");
    }

    /// `git diff --color` emits red for deletions and green for
    /// additions, with surrounding context in default style.
    #[test]
    fn m6_3_sgr_git_diff_render() {
        let mut p = AnsiParser::new();
        let input = b"\
            \x1b[31m-old line\x1b[0m\n\
            \x1b[32m+new line\x1b[0m\n\
            context\n";
        let evs = p.feed(input);
        let styles = collect_styles(&evs);
        assert!(
            styles.iter().any(|s| s.fg == Color::Indexed(1)),
            "must observe red SGR for deletion line; got {styles:?}"
        );
        assert!(
            styles.iter().any(|s| s.fg == Color::Indexed(2)),
            "must observe green SGR for addition line; got {styles:?}"
        );
        let text = collect_text(&evs);
        assert!(text.contains("-old line"));
        assert!(text.contains("+new line"));
        assert!(text.contains("context"));
    }

    /// `grep --color=always` highlights matches with bold reverse-or-red.
    #[test]
    fn m6_3_sgr_grep_color_render() {
        let mut p = AnsiParser::new();
        // \x1b[01;31m\x1b[Khello\x1b[m = bold red + erase-to-eol +
        // text + SGR reset (the final `\x1b[m` is empty-param SGR).
        let evs = p.feed(b"prefix \x1b[01;31m\x1b[Khello\x1b[m suffix");
        let styles = collect_styles(&evs);
        assert!(
            styles.iter().any(|s| s.bold && s.fg == Color::Indexed(1)),
            "grep match should be bold red; got {styles:?}"
        );
        // The \x1b[K (erase-to-eol) must surface as its own event.
        assert!(
            evs.iter().any(|e| matches!(e, AnsiEvent::EraseToEol)),
            "must observe EraseToEol from \\x1b[K; got {evs:?}"
        );
        let text = collect_text(&evs);
        assert!(text.contains("prefix "));
        assert!(text.contains("hello"));
        assert!(text.contains(" suffix"));
    }

    /// 24-bit truecolor: `\x1b[38;2;R;G;B m`.
    #[test]
    fn m6_3_truecolor_24bit() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[38;2;255;128;0morange");
        let styles = collect_styles(&evs);
        assert_eq!(
            styles.first().map(|s| s.fg),
            Some(Color::Rgb(255, 128, 0)),
            "expected truecolor RGB(255,128,0); got {styles:?}"
        );
        assert_eq!(collect_text(&evs), "orange");
    }

    /// 256-color indexed: `\x1b[38;5;N m`.
    #[test]
    fn m6_3_color_256_indexed() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[38;5;124m"); // 256-color shade of red.
        let styles = collect_styles(&evs);
        assert_eq!(
            styles.first().map(|s| s.fg),
            Some(Color::Indexed(124)),
            "expected 256-color index 124; got {styles:?}"
        );
    }

    /// Underline variants via subparam: bare 4 (Single), 4:2
    /// (Double), 4:3 (Curly), 4:4 (Dotted), 4:5 (Dashed), 24 (off).
    #[test]
    fn m6_3_underline_styles() {
        let cases: &[(&[u8], UnderlineStyle)] = &[
            (b"\x1b[4m", UnderlineStyle::Single),
            (b"\x1b[4:2m", UnderlineStyle::Double),
            (b"\x1b[4:3m", UnderlineStyle::Curly),
            (b"\x1b[4:4m", UnderlineStyle::Dotted),
            (b"\x1b[4:5m", UnderlineStyle::Dashed),
        ];
        for (input, expected) in cases {
            let mut p = AnsiParser::new();
            let evs = p.feed(input);
            let style = collect_styles(&evs);
            assert_eq!(
                style.first().map(|s| s.underline),
                Some(*expected),
                "input {input:?} should produce underline {expected:?}; got {evs:?}"
            );
        }
        // 24 must reset underline to None.
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[4:3m\x1b[24m");
        let styles = collect_styles(&evs);
        assert_eq!(
            styles.last().map(|s| s.underline),
            Some(UnderlineStyle::None)
        );
    }

    /// Underline color via SGR 58 (both colon-subparam and semicolon
    /// grammars, mirroring 38/48), reset via SGR 59 (T M4.6).
    #[test]
    fn m4_6_underline_color() {
        let cases: &[(&[u8], Color)] = &[
            (b"\x1b[58:5:1m", Color::Indexed(1)),
            (b"\x1b[58;5;124m", Color::Indexed(124)),
            (b"\x1b[58:2::255:0:0m", Color::Rgb(255, 0, 0)),
        ];
        for (input, expected) in cases {
            let mut p = AnsiParser::new();
            let evs = p.feed(input);
            let styles = collect_styles(&evs);
            assert_eq!(
                styles.first().map(|s| s.underline_color),
                Some(*expected),
                "input {input:?} should produce underline color {expected:?}; got {evs:?}"
            );
        }
        // 59 resets to follow-text-color.
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[58:5:1m\x1b[59m");
        let styles = collect_styles(&evs);
        assert_eq!(
            styles.last().map(|s| s.underline_color),
            Some(Color::Default)
        );
    }

    // -----------------------------------------------------------------
    // Intra-line motion and erase
    // -----------------------------------------------------------------

    /// `progress\rdone\x1b[K` --- a typical shell progress-bar
    /// pattern: print "progress", carriage-return to overwrite,
    /// print "done", erase rest of line.
    #[test]
    fn m6_3_carriage_return_and_erase_to_eol() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"progress\rdone\x1b[K");
        // Expected event sequence:
        //   Text("progress"), CarriageReturn, Text("done"), EraseToEol
        let mut iter = evs.into_iter();
        match iter.next() {
            Some(AnsiEvent::Text(s)) => assert_eq!(s, "progress"),
            other => panic!("expected Text(\"progress\"), got {other:?}"),
        }
        assert_eq!(iter.next(), Some(AnsiEvent::CarriageReturn));
        match iter.next() {
            Some(AnsiEvent::Text(s)) => assert_eq!(s, "done"),
            other => panic!("expected Text(\"done\"), got {other:?}"),
        }
        assert_eq!(iter.next(), Some(AnsiEvent::EraseToEol));
        assert!(iter.next().is_none(), "no extra events expected");
    }

    // -----------------------------------------------------------------
    // Acceptance bullet 2 + 5: malformed input recovery
    // -----------------------------------------------------------------

    /// An unknown CSI is consumed silently; subsequent legitimate
    /// SGR still parses correctly. Validates ESC-anywhere recovery
    /// without state corruption.
    #[test]
    fn m6_3_unknown_csi_recovers() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[?123x\x1b[31mhi\x1b[0m");
        let text = collect_text(&evs);
        let styles = collect_styles(&evs);
        assert_eq!(text, "hi", "unknown CSI must not leak bytes into text");
        assert!(
            styles.iter().any(|s| s.fg == Color::Indexed(1)),
            "post-unknown SGR red must be observed; got {styles:?}"
        );
    }

    /// A CSI split across two `feed` calls produces the same events
    /// as the single-call version. M6.2 chunk-boundary safety.
    #[test]
    fn m6_3_truncated_csi_resumes_across_feed_calls() {
        let mut p = AnsiParser::new();
        let mut evs = p.feed(b"\x1b[3");
        evs.extend(p.feed(b"1mhi\x1b[0m"));
        let mut p2 = AnsiParser::new();
        let evs_joined = p2.feed(b"\x1b[31mhi\x1b[0m");
        assert_eq!(
            evs, evs_joined,
            "split-feed must produce identical events to single-feed"
        );
    }

    /// An OSC with no terminator beyond the per-state byte cap
    /// triggers force-recovery; subsequent input parses normally.
    /// Validates the spec's per-state 1 KiB cap.
    #[test]
    fn m6_3_osc_no_terminator_recovers_at_byte_cap() {
        let mut p = AnsiParser::new();
        // Open OSC, emit 5 KiB of body, no terminator.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b]0;");
        input.extend(std::iter::repeat_n(b'A', 5 * 1024));
        // Then a fresh SGR sequence after the malformed OSC.
        input.extend_from_slice(b"\x1b[31mhi");
        let evs = p.feed(&input);
        let styles = collect_styles(&evs);
        let text = collect_text(&evs);
        // The OSC produced no SetTitle event (malformed, discarded).
        assert!(
            !evs.iter().any(|e| matches!(e, AnsiEvent::SetTitle(_))),
            "malformed OSC must not produce SetTitle; got {evs:?}"
        );
        // Post-recovery: the SGR red parses; "hi" arrives as Text.
        assert!(
            styles.iter().any(|s| s.fg == Color::Indexed(1)),
            "post-recovery SGR red must be observed; got {styles:?}"
        );
        assert!(text.ends_with("hi"));
    }

    /// OSC 0 (set window title) with BEL terminator → `SetTitle`.
    #[test]
    fn m6_3_osc_title_set() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b]0;mytitle\x07hello");
        let titles: Vec<&str> = evs
            .iter()
            .filter_map(|e| {
                if let AnsiEvent::SetTitle(t) = e {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(titles, vec!["mytitle"]);
        assert_eq!(collect_text(&evs), "hello");
    }

    #[test]
    fn m6_3_osc_133_prompt_markers_are_structured_events() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07");
        let kinds: Vec<&str> = evs
            .iter()
            .map(|ev| match ev {
                AnsiEvent::PromptStart => "prompt_start",
                AnsiEvent::Text(s) if s == "$ " => "text",
                AnsiEvent::PromptEnd => "prompt_end",
                AnsiEvent::CommandStart => "command_start",
                AnsiEvent::OutputStart => "output_start",
                other => panic!("unexpected event: {other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "prompt_start",
                "text",
                "prompt_end",
                "command_start",
                "output_start"
            ]
        );
    }

    // -----------------------------------------------------------------
    // Acceptance bullet 4: alternate-screen suppression
    // -----------------------------------------------------------------

    #[test]
    fn m6_3_alt_screen_suppresses_output() {
        let mut p = AnsiParser::new();
        let mut input = Vec::new();
        input.extend_from_slice(b"before");
        // Enter alt-screen.
        input.extend_from_slice(b"\x1b[?1049h");
        // Garbage TUI output that must NOT surface as events.
        input.extend_from_slice(b"\x1b[31mblah\x1b[2J\x1b[Hsome stuff");
        // Exit alt-screen.
        input.extend_from_slice(b"\x1b[?1049l");
        // Resume normal output.
        input.extend_from_slice(b"after");
        let evs = p.feed(&input);
        let text = collect_text(&evs);
        assert!(
            text.contains("before") && text.contains("after"),
            "outside-alt-screen text must surface; got {text:?}"
        );
        assert!(
            !text.contains("blah") && !text.contains("some stuff"),
            "alt-screen text must be suppressed; got {text:?}"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, AnsiEvent::AlternateScreenEnter)),
            "AlternateScreenEnter event must be emitted"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, AnsiEvent::AlternateScreenExit)),
            "AlternateScreenExit event must be emitted"
        );
    }

    // -----------------------------------------------------------------
    // Bracketed paste markers
    // -----------------------------------------------------------------

    #[test]
    fn m6_3_bracketed_paste_markers_emitted() {
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[200~hello\x1b[201~");
        let mut iter = evs.into_iter();
        assert_eq!(iter.next(), Some(AnsiEvent::BracketedPasteBegin));
        match iter.next() {
            Some(AnsiEvent::Text(s)) => assert_eq!(s, "hello"),
            other => panic!("expected Text(\"hello\"), got {other:?}"),
        }
        assert_eq!(iter.next(), Some(AnsiEvent::BracketedPasteEnd));
        assert!(iter.next().is_none());
    }

    // -----------------------------------------------------------------
    // Acceptance bullet 3: rope stream stays clean text
    // -----------------------------------------------------------------

    /// Across a corpus of valid + malformed sequences, no `Text`
    /// event ever contains an escape byte.
    #[test]
    fn m6_3_text_stream_carries_no_escape_bytes() {
        let corpus: &[&[u8]] = &[
            b"\x1b[31mhello\x1b[0m world",
            b"\x1b[?123x\x1b[m unknown CSI ignored",
            b"\x1b]0;title\x07ordinary",
            b"\x1b[200~paste\x1b[201~",
            b"a\x1b[A\x1b[B\x1b[Cb cursor motions ignored",
            b"trunc\x1b[3", // no terminator; test asserts safety
        ];
        for input in corpus {
            let mut p = AnsiParser::new();
            let evs = p.feed(input);
            for ev in &evs {
                if let AnsiEvent::Text(s) = ev {
                    assert!(
                        !s.bytes().any(|b| b == 0x1B),
                        "Text event leaked an ESC byte for input {input:?}: {ev:?}"
                    );
                }
            }
        }
    }

    /// Pathological inputs all return promptly with the parser in
    /// a sane state (subsequent input parses normally).
    #[test]
    fn m6_3_parser_terminates_on_malformed_input() {
        let pathological: &[Vec<u8>] = &[
            // Bare ESC repeated.
            std::iter::repeat_n(0x1B, 1024).collect(),
            // Half-CSI followed by ESC repeatedly.
            (0..512).flat_map(|_| b"\x1b[3".iter().copied()).collect(),
            // OSC with no terminator and no ESC for 5 KiB.
            {
                let mut v = b"\x1b]0;".to_vec();
                v.extend(std::iter::repeat_n(b'X', 5 * 1024));
                v
            },
        ];
        for input in pathological {
            let mut p = AnsiParser::new();
            let _ = p.feed(input);
            // Post-pathology, force the parser back to Ground via
            // an explicit ST sequence (ESC \), since the pathology
            // may have left it mid-escape (e.g., a trailing bare
            // ESC waits for its final byte). Then assert ordinary
            // text comes through cleanly --- the goal of this test
            // is that the parser doesn't panic, hang, or corrupt
            // future input, not that any one byte aligns with a
            // particular state.
            let evs = p.feed(b"\x1b\\recovered");
            let text = collect_text(&evs);
            assert!(
                text.contains("recovered"),
                "parser failed to recover after pathological input; got {evs:?}"
            );
        }
    }

    // -----------------------------------------------------------------
    // Strikethrough advances state without corrupting running style
    // -----------------------------------------------------------------

    // ---- UTF-8 cross-feed handling (post-M6.10 audit fix) -----------------

    /// Multi-byte UTF-8 in a single feed decodes to the correct
    /// scalar, not U+FFFD. Catches the M6.3-stage-1 shortcut that
    /// emitted U+FFFD for every non-ASCII byte regardless of whether
    /// it was actually malformed.
    #[test]
    fn ansi_utf8_multibyte_in_single_feed_decodes_correctly() {
        let mut p = AnsiParser::new();
        // "café" --- the é is U+00E9, encoded as 0xC3 0xA9 in UTF-8.
        let evs = p.feed("café".as_bytes());
        assert_eq!(collect_text(&evs), "café");
    }

    /// Three-byte UTF-8 (e.g., CJK) decodes correctly. Same gate as
    /// above but exercises the 1110xxxx start byte + two
    /// continuations path.
    #[test]
    fn ansi_utf8_three_byte_sequence_decodes_correctly() {
        let mut p = AnsiParser::new();
        let evs = p.feed("日本語".as_bytes());
        assert_eq!(collect_text(&evs), "日本語");
    }

    /// Four-byte UTF-8 (e.g., emoji past U+FFFF) decodes correctly.
    /// Exercises the 11110xxx start byte + three continuations path.
    #[test]
    fn ansi_utf8_four_byte_sequence_decodes_correctly() {
        let mut p = AnsiParser::new();
        // U+1F600 = 😀 = 0xF0 0x9F 0x98 0x80
        let evs = p.feed("😀".as_bytes());
        assert_eq!(collect_text(&evs), "😀");
    }

    /// A multi-byte sequence split across two feeds completes
    /// correctly. This is the load-bearing cross-feed behavior:
    /// pre-fix, the first feed would emit U+FFFD for the start byte
    /// and the second feed would emit U+FFFDs for the continuation
    /// bytes; post-fix, the partial bytes wait in `utf8_buf` and
    /// flush as one correct char when the trailing byte arrives.
    #[test]
    fn ansi_utf8_split_across_two_feeds() {
        let mut p = AnsiParser::new();
        let bytes = "café".as_bytes();
        // Split between the start byte (0xC3) and continuation
        // (0xA9) of the é.
        let split = bytes.len() - 1;
        let evs1 = p.feed(&bytes[..split]);
        let evs2 = p.feed(&bytes[split..]);
        let combined: String = collect_text(&evs1) + &collect_text(&evs2);
        assert_eq!(combined, "café");
    }

    /// A four-byte sequence split across three feeds completes
    /// correctly. Exercises the cross-feed buffer holding 2 bytes
    /// across one feed and 1 byte across the next.
    #[test]
    fn ansi_utf8_four_byte_split_across_three_feeds() {
        let mut p = AnsiParser::new();
        // U+1F600 = 0xF0 0x9F 0x98 0x80
        let evs1 = p.feed(&[0xF0]);
        let evs2 = p.feed(&[0x9F, 0x98]);
        let evs3 = p.feed(&[0x80]);
        let combined = collect_text(&evs1) + &collect_text(&evs2) + &collect_text(&evs3);
        assert_eq!(combined, "😀");
    }

    /// Mixed ASCII and multi-byte text decodes correctly.
    #[test]
    fn ansi_utf8_mixed_ascii_and_multibyte() {
        let mut p = AnsiParser::new();
        let evs = p.feed("hello, café 日本 😀!".as_bytes());
        assert_eq!(collect_text(&evs), "hello, café 日本 😀!");
    }

    /// A lone continuation byte (no preceding start) emits U+FFFD
    /// for that byte only; subsequent ASCII still decodes to ASCII.
    #[test]
    fn ansi_utf8_lone_continuation_byte_emits_replacement() {
        let mut p = AnsiParser::new();
        // 0x80 alone is a continuation byte with no start.
        // ASCII follows; it should decode normally.
        let evs = p.feed(&[b'a', 0x80, b'b']);
        assert_eq!(collect_text(&evs), "a\u{FFFD}b");
    }

    /// An invalid start byte (0xFF) emits U+FFFD; subsequent ASCII
    /// decodes normally.
    #[test]
    fn ansi_utf8_invalid_start_byte_emits_replacement() {
        let mut p = AnsiParser::new();
        let evs = p.feed(&[b'a', 0xFF, b'b']);
        assert_eq!(collect_text(&evs), "a\u{FFFD}b");
    }

    /// A start byte expecting two continuations followed by ASCII
    /// (one continuation, then ASCII) emits U+FFFD for the
    /// truncated sequence and decodes the ASCII normally.
    #[test]
    fn ansi_utf8_truncated_sequence_then_ascii() {
        let mut p = AnsiParser::new();
        // 0xE6 (start of 3-byte) + 0x97 (continuation) + 'X' (ASCII,
        // not a continuation). The sequence is malformed: 0xE6 0x97
        // followed by ASCII.
        let evs = p.feed(&[0xE6, 0x97, b'X']);
        // The 0xE6 0x97 prefix is malformed; std::str::from_utf8
        // reports error_len() = 2, so we emit one U+FFFD covering
        // both bytes, then ASCII.
        let text = collect_text(&evs);
        assert!(
            text.contains('\u{FFFD}') && text.ends_with('X'),
            "expected U+FFFD then 'X', got {text:?}"
        );
    }

    /// A multi-byte sequence interrupted by a control byte (CSI
    /// start) flushes pending bytes as U+FFFD, then resumes
    /// processing the control byte normally.
    #[test]
    fn ansi_utf8_partial_interrupted_by_csi_emits_replacement() {
        let mut p = AnsiParser::new();
        // 0xC3 (start of 2-byte) then ESC [ 31 m (red SGR).
        // The 0xC3 is interrupted by the ESC; should emit U+FFFD
        // for the partial, then process the SGR.
        let evs = p.feed(&[b'a', 0xC3, 0x1B, b'[', b'3', b'1', b'm', b'b']);
        let text = collect_text(&evs);
        assert!(
            text.starts_with("a\u{FFFD}") && text.ends_with('b'),
            "expected a + U+FFFD + b, got {text:?}"
        );
        // The SGR should still have produced a SetStyle event.
        let styles = collect_styles(&evs);
        assert_eq!(styles.len(), 1, "expected one SetStyle from the SGR");
        assert_eq!(styles[0].fg, Color::Indexed(1));
    }

    /// Pending UTF-8 prefix that exceeds the 8-byte defensive cap
    /// (pathological producer that never finishes a sequence)
    /// flushes as U+FFFD and recovers. Ensures the buffer can't
    /// grow unbounded.
    #[test]
    fn ansi_utf8_pathological_pending_caps_at_8_bytes() {
        let mut p = AnsiParser::new();
        // Feed 8 start bytes in a row. Each is a "start of 2-byte"
        // marker; the next one arriving where a continuation is
        // expected is malformed. The first one accumulates; each
        // subsequent one emits U+FFFD for the malformed prefix.
        // After 8 bytes accumulated without completing, the cap
        // forces a flush.
        let evs = p.feed(&[0xC3; 9]);
        let text = collect_text(&evs);
        // Every byte should have been replaced; no panic, no
        // unbounded growth.
        assert!(
            text.chars().all(|c| c == '\u{FFFD}'),
            "all 9 bytes should be replaced; got {text:?}"
        );
    }

    /// `reset()` clears any pending UTF-8 prefix.
    #[test]
    fn ansi_utf8_reset_clears_pending_buffer() {
        let mut p = AnsiParser::new();
        let _ = p.feed(&[0xC3]); // partial é prefix pending
        p.reset();
        // Subsequent valid input must decode cleanly --- the stale
        // prefix should not contaminate it.
        let evs = p.feed("hello".as_bytes());
        assert_eq!(collect_text(&evs), "hello");
    }

    /// SGR 9 (strikethrough) is a no-op for the running style ---
    /// the cell `Style` has no strikethrough field, so the running
    /// style must be *unchanged* after SGR 9. A future regression
    /// that maps SGR 9 to another field (e.g., flips bold by
    /// accident) is caught here.
    #[test]
    fn m6_3_strikethrough_sgr_9_does_not_corrupt_running_style() {
        let mut p = AnsiParser::new();
        // Set bold + italic + red, then SGR 9.
        let _ = p.feed(b"\x1b[1;3;31m");
        let evs_after_9 = p.feed(b"\x1b[9m");
        let after = collect_styles(&evs_after_9);
        assert_eq!(after.len(), 1, "SGR 9 should still emit a SetStyle");
        let s = after[0];
        assert!(
            s.bold && s.italic && s.fg == Color::Indexed(1),
            "SGR 9 must leave bold + italic + red intact; got {s:?}"
        );
        // Subsequent text uses the unchanged style.
        let evs_text = p.feed(b"hi");
        assert_eq!(collect_text(&evs_text), "hi");
    }

    // -----------------------------------------------------------------
    // Stream-end finish() (compile-mode Q#CM4; PR #113 rounds 1–2)
    // -----------------------------------------------------------------

    #[test]
    fn finish_flushes_truncated_utf8_as_replacement() {
        let mut p = AnsiParser::new();
        // 0xC3 opens a two-byte sequence that never completes.
        let evs = p.feed(b"abc\xC3");
        assert_eq!(collect_text(&evs), "abc", "prefix buffered across feeds");
        let evs = p.finish();
        assert_eq!(
            collect_text(&evs),
            "\u{FFFD}",
            "stream end must surface the pending prefix as U+FFFD"
        );
        // Idempotent once drained.
        assert!(p.finish().is_empty(), "second finish drains nothing");
    }

    #[test]
    fn feed_after_finish_starts_a_fresh_stream() {
        // Mid-CSI at stream end: without the finish-time reset, a
        // subsequent feed would keep consuming bytes as CSI
        // parameters instead of parsing a new stream (PR #113
        // round-2 finding 4).
        let mut p = AnsiParser::new();
        let _ = p.feed(b"\x1b[3"); // incomplete CSI
        let _ = p.finish();
        let evs = p.feed(b"plain");
        assert_eq!(
            collect_text(&evs),
            "plain",
            "post-finish feeds must not continue a pre-EOF escape"
        );
    }

    #[test]
    fn finish_ends_alt_screen_suppression() {
        let mut p = AnsiParser::new();
        let _ = p.feed(b"\x1b[?1049hhidden"); // enter alt screen
        let _ = p.finish();
        let evs = p.feed(b"visible");
        assert_eq!(
            collect_text(&evs),
            "visible",
            "a stream END ends suppression; a new stream starts unsuppressed"
        );
    }

    #[test]
    fn finish_emits_balancing_events_for_consumer_state() {
        // A consumer mirrors parser state from the event stream
        // alone (PR #113 round-3 finding 2): apply every event to a
        // consumer-side mirror and require finish() to unwind it —
        // not merely make subsequent text visible.
        let mut p = AnsiParser::new();
        let mut consumer_alt = false;
        let mut consumer_style = Style::default();
        let apply = |evs: &[AnsiEvent], alt: &mut bool, style: &mut Style| {
            for ev in evs {
                match ev {
                    AnsiEvent::AlternateScreenEnter => *alt = true,
                    AnsiEvent::AlternateScreenExit => *alt = false,
                    AnsiEvent::SetStyle(s) => *style = *s,
                    _ => {}
                }
            }
        };
        let evs = p.feed(b"\x1b[31mred\x1b[?1049h");
        apply(&evs, &mut consumer_alt, &mut consumer_style);
        assert!(consumer_alt, "enter observed");
        assert_ne!(consumer_style, Style::default(), "red observed");
        let evs = p.finish();
        apply(&evs, &mut consumer_alt, &mut consumer_style);
        assert!(
            !consumer_alt,
            "finish must balance the unclosed AlternateScreenEnter"
        );
        assert_eq!(
            consumer_style,
            Style::default(),
            "finish must reset the running style observably"
        );
    }

    /// Consumer mirror for the round-4 alt-screen style-desync
    /// tests: alt flag + last received style, driven purely by
    /// emitted events.
    fn mirror(evs: &[AnsiEvent], alt: &mut bool, style: &mut Style) {
        for ev in evs {
            match ev {
                AnsiEvent::AlternateScreenEnter => *alt = true,
                AnsiEvent::AlternateScreenExit => *alt = false,
                AnsiEvent::SetStyle(s) => *style = *s,
                _ => {}
            }
        }
    }

    #[test]
    fn alt_screen_exit_resyncs_suppressed_style_changes() {
        // SGR events are suppressed inside the alternate screen
        // while `current_style` keeps advancing; an ordinary exit
        // must resynchronize the consumer's effective style (PR #113
        // round-4 finding 2). Both drift directions: a reset the
        // consumer never saw, and a color it never saw.
        let mut p = AnsiParser::new();
        let mut alt = false;
        let mut style = Style::default();
        let evs = p.feed(b"\x1b[31mred\x1b[?1049h\x1b[0m\x1b[?1049l");
        mirror(&evs, &mut alt, &mut style);
        assert!(!alt, "exit observed");
        assert_eq!(
            style,
            Style::default(),
            "the suppressed SGR reset must reach the consumer on exit"
        );

        let mut p = AnsiParser::new();
        let mut alt = false;
        let mut style = Style::default();
        let evs = p.feed(b"\x1b[?1049h\x1b[31m\x1b[?1049lafter");
        mirror(&evs, &mut alt, &mut style);
        assert_ne!(
            style,
            Style::default(),
            "a color set inside the alt screen styles post-exit text"
        );
        // No drift → no spurious resync event.
        let mut p = AnsiParser::new();
        let evs = p.feed(b"\x1b[?1049h\x1b[?1049l");
        assert!(
            !evs.iter().any(|e| matches!(e, AnsiEvent::SetStyle(_))),
            "style untouched inside alt screen emits no resync"
        );
    }

    #[test]
    fn finish_emits_default_style_when_reset_was_suppressed() {
        // The round-4 finding-2 finish() scenario: consumer shows
        // red from before the alt-screen enter; an SGR reset inside
        // makes the INTERNAL style default, so a current_style
        // comparison sees nothing to balance — but the consumer is
        // still red. finish() must compare against what was last
        // EMITTED.
        let mut p = AnsiParser::new();
        let mut alt = false;
        let mut style = Style::default();
        let evs = p.feed(b"\x1b[31mred\x1b[?1049h\x1b[0m");
        mirror(&evs, &mut alt, &mut style);
        assert!(alt, "enter observed");
        assert_ne!(style, Style::default(), "consumer is red pre-finish");
        let evs = p.finish();
        mirror(&evs, &mut alt, &mut style);
        assert!(!alt, "finish balances the enter");
        assert_eq!(
            style,
            Style::default(),
            "finish must emit the default SetStyle the consumer needs \
             even though the internal style is already default"
        );
    }

    #[test]
    fn full_screen_emits_typed_operation_set_across_every_split() {
        let bytes = b"\x07\t\n\x1bD\x1bE\x1bM\x1bH\x1b[2A\x1b[3B\x1b[4C\x1b[5D\
            \x1b[2E\x1b[2F\x1b[7G\x1b[8d\x1b[2;3H\x1b[J\x1b[1K\x1b[2X\
            \x1b[3@\x1b[4P\x1b[2L\x1b[2M\x1b[3S\x1b[2T\x1b[2;20r\x1b[s\
            \x1b[u\x1b[3g\x1b[?1;6;7;25;1000;1002;1003;1004;1006;2004;2026h\
            \x1b[?47h\x1b[?1047h\x1b[?1049h\x1b[c\x1b[>c\x1b[5n\x1b[6n";
        let mut whole = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
        let expected = whole.feed(bytes);
        assert!(expected.contains(&AnsiEvent::Bell));
        assert!(expected.contains(&AnsiEvent::Index));
        assert!(expected.contains(&AnsiEvent::NextLine));
        assert!(expected.contains(&AnsiEvent::ReverseIndex));
        assert!(expected.contains(&AnsiEvent::CursorPosition { row: 2, col: 3 }));
        assert!(expected.contains(&AnsiEvent::SetScrollingRegion {
            top: 2,
            bottom: Some(20)
        }));
        assert!(expected.contains(&AnsiEvent::SetMode {
            mode: TerminalMode::SynchronizedOutput,
            enabled: true
        }));
        assert!(expected.contains(&AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode1049,
            enabled: true
        }));
        assert!(expected.contains(&AnsiEvent::DeviceRequest(DeviceRequest::CursorPosition)));
        for split in 0..=bytes.len() {
            let mut parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
            let mut actual = parser.feed(&bytes[..split]);
            actual.extend(parser.feed(&bytes[split..]));
            assert_eq!(actual, expected, "split {split}");
        }
    }

    #[test]
    fn full_screen_finish_flushes_without_synthetic_balancing() {
        let mut parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
        let events = parser.feed(b"\x1b[?1049h\x1b[31mred");
        assert!(events.contains(&AnsiEvent::AlternateScreen {
            mode: AlternateScreenMode::Mode1049,
            enabled: true,
        }));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AnsiEvent::SetStyle(_)))
        );
        assert!(parser.finish().is_empty());
        assert!(!parser.alt_screen_active);
        assert!(parser.finish().is_empty());
        assert_eq!(parser.feed(b"x"), vec![AnsiEvent::Text("x".into())]);
    }

    #[test]
    fn full_screen_charset_designation_and_shift_are_typed() {
        let mut parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
        assert_eq!(
            parser.feed(b"\x1b(0\x1b)B\x0e\x0f"),
            vec![
                AnsiEvent::DesignateCharacterSet {
                    slot: CharacterSetSlot::G0,
                    charset: CharacterSet::DecSpecialGraphics,
                },
                AnsiEvent::DesignateCharacterSet {
                    slot: CharacterSetSlot::G1,
                    charset: CharacterSet::Ascii,
                },
                AnsiEvent::ShiftOut,
                AnsiEvent::ShiftIn,
            ]
        );
    }

    #[test]
    fn full_screen_capped_control_strings_recover_invisibly() {
        let config = AnsiParserConfig {
            unknown_sequence_byte_limit: 8,
        };
        let mut parser = AnsiParser::with_profile_and_config(AnsiParserProfile::FullScreen, config);
        let events = parser.feed(b"\x1b]52;AAAAAAAABsecret\x1b\\ok\x1bPAAAAAAAAAAAA\x1b\\done");
        let visible: String = events
            .iter()
            .filter_map(|event| match event {
                AnsiEvent::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!visible.contains("secret"));
        assert!(!visible.contains("AAAA"));
        assert!(visible.ends_with("done"));
        assert!(!visible.contains('\x1b'));
    }

    #[test]
    fn capped_csi_and_escape_intermediate_never_leak_payload() {
        let config = AnsiParserConfig {
            unknown_sequence_byte_limit: 8,
        };
        for input in [
            b"\x1b[12345678901234567890mOK".as_slice(),
            b"\x1b[?999999999999999999hOK".as_slice(),
            b"\x1b                    0OK".as_slice(),
        ] {
            let mut parser =
                AnsiParser::with_profile_and_config(AnsiParserProfile::FullScreen, config);
            let visible: String = parser
                .feed(input)
                .into_iter()
                .filter_map(|event| {
                    if let AnsiEvent::Text(text) = event {
                        Some(text)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(visible, "OK", "input {input:?}");
        }
    }

    #[test]
    fn full_screen_unsupported_sgr_attributes_leave_style_unchanged() {
        let mut parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
        parser.feed(b"\x1b[1;3;31m");
        let before = parser.current_style;
        parser.feed(b"\x1b[2;5;6;8;9;25;28;29m");
        assert_eq!(parser.current_style, before);
        assert!(before.bold);

        let mut line = AnsiParser::new();
        line.feed(b"\x1b[5m");
        assert!(
            line.current_style.bold,
            "line-oriented blink-as-bold compatibility"
        );
    }

    #[test]
    fn unicode_osc_titles_survive_every_feed_split() {
        let bytes = "\u{1b}]2;héllo 世界\u{7}".as_bytes();
        let mut whole = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
        let expected = whole.feed(bytes);
        assert_eq!(expected, vec![AnsiEvent::SetTitle("héllo 世界".into())]);
        for split in 0..=bytes.len() {
            let mut parser = AnsiParser::with_profile(AnsiParserProfile::FullScreen);
            let mut actual = parser.feed(&bytes[..split]);
            actual.extend(parser.feed(&bytes[split..]));
            assert_eq!(actual, expected, "split {split}");
        }
    }

    #[test]
    fn malformed_utf8_osc_title_is_replaced_and_bounded() {
        let config = AnsiParserConfig {
            unknown_sequence_byte_limit: 32,
        };
        let mut parser = AnsiParser::with_profile_and_config(AnsiParserProfile::FullScreen, config);
        let events = parser.feed(b"\x1b]0;bad\xfftitle\x07");
        let title = events
            .into_iter()
            .find_map(|event| {
                if let AnsiEvent::SetTitle(title) = event {
                    Some(title)
                } else {
                    None
                }
            })
            .expect("title event");
        assert_eq!(title, "bad\u{fffd}title");
        assert!(title.len() <= config.unknown_sequence_byte_limit);
    }
}
