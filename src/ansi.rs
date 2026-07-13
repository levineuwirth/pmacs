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

// ---------------------------------------------------------------------------
// Public output
// ---------------------------------------------------------------------------

/// One observable effect of feeding bytes to an [`AnsiParser`].
///
/// One byte may produce zero or more events; one event is emitted
/// at the moment the parser has enough context to commit to it
/// (e.g., `Text` is emitted at every transition out of Ground, not
/// per byte).
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
        Self::with_config(AnsiParserConfig::default())
    }

    /// Construct a parser with custom configuration.
    #[must_use]
    pub fn with_config(config: AnsiParserConfig) -> Self {
        Self {
            state: State::Ground,
            current_style: Style::default(),
            ignore_byte_count: 0,
            text_run: String::new(),
            utf8_buf: Vec::new(),
            alt_screen_active: false,
            csi: CsiCollector::default(),
            osc_body: Vec::new(),
            escape_intermediates: Vec::new(),
            config,
        }
    }

    /// Reset the parser to ground state. The running style is *not*
    /// reset --- callers that want a clean style should pair this
    /// with their own `SetStyle(Style::default())`.
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
        if !self.text_run.is_empty() && !self.alt_screen_active {
            let run = std::mem::take(&mut self.text_run);
            events.push(AnsiEvent::Text(run));
        } else {
            self.text_run.clear();
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
    /// state AND alt-screen suppression included — so a `feed` after
    /// `finish` parses a NEW stream from a clean slate rather than
    /// continuing a pre-EOF escape sequence or staying suppressed
    /// (PR #113 round-2 finding 4). Idempotent once drained. First
    /// consumer: compile-mode's terminal-event path (Q#CM4).
    pub fn finish(&mut self) -> Vec<AnsiEvent> {
        let mut events = Vec::new();
        self.flush_pending_utf8_as_replacement();
        if !self.text_run.is_empty() && !self.alt_screen_active {
            let run = std::mem::take(&mut self.text_run);
            events.push(AnsiEvent::Text(run));
        } else {
            self.text_run.clear();
        }
        self.reset();
        // `reset` deliberately preserves alt-screen suppression (a
        // mid-stream reset must not unhide alt-screen contents); a
        // stream END does end the suppression.
        self.alt_screen_active = false;
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

        // Per-state byte cap. The counter increments for every byte
        // consumed in any non-Ground state, and is reset to zero at
        // every transition into a fresh sequence (ESC-anywhere) or
        // back to Ground (normal dispatch / force-recover). At the
        // limit, the parser drops the in-flight sequence and
        // returns to Ground; the *current* byte is dropped on the
        // floor, but subsequent bytes are processed normally as
        // ordinary text. Spec §sec:ansi-scope: "drops back to ground
        // state at the next ESC or after a bounded number of bytes
        // (1 KiB), whichever comes first."
        if self.state != State::Ground {
            self.ignore_byte_count = self.ignore_byte_count.saturating_add(1);
            if self.ignore_byte_count > self.config.unknown_sequence_byte_limit {
                self.recover_to_ground();
                return;
            }
        }

        match self.state {
            State::Ground => self.feed_ground(b, events),
            State::Escape => self.feed_escape(b),
            State::EscapeIntermediate => self.feed_escape_intermediate(b),
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
        if !self.alt_screen_active {
            events.push(AnsiEvent::Text(run));
        }
    }

    fn emit_set_style(&mut self, events: &mut Vec<AnsiEvent>) {
        if self.alt_screen_active {
            return;
        }
        events.push(AnsiEvent::SetStyle(self.current_style));
    }

    /// Helper: emit `ev` if alt-screen is not active; drop otherwise.
    /// Used for visible side-effects (CR / BS / Erase / Bracketed
    /// paste / `SetTitle`). The alt-screen markers themselves
    /// bypass this.
    fn push_visible(&self, ev: AnsiEvent, events: &mut Vec<AnsiEvent>) {
        if !self.alt_screen_active {
            events.push(ev);
        }
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
        match b {
            // CR: flush text, emit CarriageReturn.
            0x0D => {
                self.flush_text_run(events);
                if !self.alt_screen_active {
                    events.push(AnsiEvent::CarriageReturn);
                }
            }
            // BS: flush text, emit Backspace.
            0x08 => {
                self.flush_text_run(events);
                if !self.alt_screen_active {
                    events.push(AnsiEvent::Backspace);
                }
            }
            // BEL (0x07), VT (0x0B), FF (0x0C), HT (0x09), LF
            // (0x0A): pass through to text alongside printable
            // ASCII (0x20..=0x7E). The REPL view treats LF as a
            // line break in the rope; HT as a literal tab. Other
            // C0 controls (0x00..=0x06, 0x0E..=0x1F) and DEL
            // (0x7F) are dropped silently.
            //
            // 0x80..=0xFF: UTF-8 lead or continuation byte. Goes
            // through `push_text_byte`'s stateful decoder so
            // multi-byte sequences across feeds are buffered until
            // complete.
            //
            // All text bytes route through `push_text_byte` (not
            // just non-ASCII): an ASCII byte arriving while a
            // partial UTF-8 sequence is pending invalidates that
            // sequence (the partial prefix's expected continuation
            // didn't arrive), and `push_text_byte` is the only
            // place that knows to flush the partial as `U+FFFD`.
            // The fast path inside `push_text_byte` keeps the
            // pure-ASCII case allocation-free.
            0x07 | 0x09 | 0x0A | 0x0B | 0x0C | 0x20..=0x7E | 0x80..=0xFF => {
                self.push_text_byte(b);
            }
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

    fn feed_escape(&mut self, b: u8) {
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
            // DCS / SOS / PM / APC introducers --- parse and discard.
            b'P' => {
                self.state = State::DcsEntry;
            }
            b'X' | b'^' | b'_' => {
                self.state = State::SosPmApcString;
            }
            // ESC \ in Escape state is a stray ST; final byte for
            // a bare ESC sequence (0x30..=0x7E) lands here too. We
            // don't dispatch any single-byte ESC commands in v0.1
            // (cursor save/restore `ESC 7`/`ESC 8` are deliberately
            // unsupported per spec); both cases consume and return
            // to Ground.
            b'\\' | 0x30..=0x7E => {
                self.recover_to_ground();
            }
            // C0 controls inside Escape: drop, stay in Escape.
            _ => {}
        }
    }

    fn feed_escape_intermediate(&mut self, b: u8) {
        match b {
            0x20..=0x2F => {
                self.escape_intermediates.push(b);
            }
            // Final byte: drop the sequence (no ESC + intermediate
            // dispatches in v0.1 --- charsets are deliberately
            // unsupported per spec) and return to Ground.
            0x30..=0x7E => {
                self.recover_to_ground();
            }
            _ => {}
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
    fn dispatch_csi(&mut self, final_byte: u8, events: &mut Vec<AnsiEvent>) {
        let private_marker = self.csi.private_marker;
        let params = self.csi.finalize();

        match (private_marker, final_byte) {
            // SGR.
            (None, b'm') => self.dispatch_sgr(&params, events),
            // Erase in line: `CSI [n] K`. n=0 (default) →
            // EraseToEol; n=2 → EraseLine; n=1 (start to cursor)
            // and others: parsed and ignored.
            (None, b'K') => {
                let n = params.first().map_or(0, |p| p.main);
                match n {
                    0 => self.push_visible(AnsiEvent::EraseToEol, events),
                    2 => self.push_visible(AnsiEvent::EraseLine, events),
                    _ => {}
                }
            }
            // Bracketed paste markers: `CSI 200 ~` / `CSI 201 ~`.
            (None, b'~') => {
                let n = params.first().map_or(0, |p| p.main);
                match n {
                    200 => self.push_visible(AnsiEvent::BracketedPasteBegin, events),
                    201 => self.push_visible(AnsiEvent::BracketedPasteEnd, events),
                    _ => {}
                }
            }
            // DEC private mode set / reset: `CSI ? <num> h` / `l`.
            // Of these, only ?1049 (alternate screen) produces an
            // event; mouse modes (?1000, ?1006), bracketed-paste
            // mode (?2004), and the long tail are parsed and
            // discarded per spec §sec:ansi-scope.
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
                        }
                    }
                }
            }
            // Cursor motions (A/B/C/D/E/F/G/H/J/f) and other CSI
            // commands: parsed and discarded for M6.3. The M6.4
            // view layer handles intra-line motion (CR / BS) at
            // its own level; cross-region motion via CSI is in the
            // "parsed but ignored when it would cross region
            // boundaries" bucket from §sec:ansi-scope.
            _ => {}
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
                // 5/6 (blink, rapid blink): mapped to bold per spec
                // §sec:ansi-scope ("blink-as-bold").
                5 | 6 => self.current_style.bold = true,
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
                // 25 (blink off): no-op. Symmetry with 5/6 → bold:
                // we do not unset bold here, since that would also
                // unset bold acquired via SGR 1.
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
            // 0x20..=0x7F: body bytes. The per-state byte cap
            // (enforced at the top of `feed_byte`) bounds how many
            // bytes we'll accept before force-recovering.
            0x20..=0x7F => self.osc_body.push(b),
            // Other C0/C1 controls: drop silently, stay in OSC.
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
        if matches!(num, Some(133)) && !self.alt_screen_active {
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
        if matches!(num, Some(0 | 2)) && !self.alt_screen_active {
            let title = String::from_utf8_lossy(text_part).into_owned();
            events.push(AnsiEvent::SetTitle(title));
        }
    }
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
}
