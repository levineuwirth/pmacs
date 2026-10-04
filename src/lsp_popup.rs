// lsp_popup.rs --- E8 the hover and signature popup at the caret.

//! The one popup the language-server affordances float at the caret:
//! the server's hover documentation, or the signature of the call the
//! caret sits in with its active parameter marked (E8).
//!
//! # Ownership
//!
//! [`crate::editor_core::EditorCore`] holds the popup, as it holds the
//! completion popup, behind a shared handle the grid's
//! [`crate::hover::HoverView`] and [`crate::signature::SignatureView`]
//! read and the semantic producer projects onto the wire
//! ([`pmacs_protocol::InstanceMessage::Popup`]). It is populated from
//! the LSP hover and signature stores when a response lands, by the
//! `pmacs.lsp._popup_*` bindings, and is stamped with the window it was
//! asked for in, so only that window's frontend shows it.
//!
//! # Life
//!
//! A popup describes one revision of one buffer shown in one focused
//! window with the caret inside one byte range. It is **closed**, not
//! hidden, as soon as any of those stops holding
//! ([`crate::editor_core::EditorCore::lsp_popup_validate`]): an edit by
//! any means (the buffer's revision moves), the caret leaving the range,
//! the window losing focus or showing another buffer, a kill. Closing
//! rather than hiding is what makes motion out of the range a dismissal:
//! a hidden popup would come back when the caret returned. `C-g` closes
//! it too.
//!
//! # The bound
//!
//! Hover text is arbitrary server markdown. [`popup_lines`] cuts it to
//! the wire's bound ([`pmacs_protocol::MAX_POPUP_LINES`] and its
//! siblings) and counts the lines it left out, which both frontends
//! show as a closing "more lines" row naming `*lsp-help*`, where the
//! whole text stays.

use std::sync::{Arc, Mutex};

use pmacs_protocol::{
    MAX_POPUP_LINE_BYTES, MAX_POPUP_LINES, MAX_POPUP_TEXT_BYTES, PopupActiveRange, PopupFrame,
    PopupKind, TAB_STOP_COLUMNS,
};

use crate::buffer::BufferId;
use crate::window::WindowId;

/// The open popup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LspPopup {
    /// What it shows.
    pub kind: PopupKind,
    /// The buffer it describes.
    pub buffer_id: BufferId,
    /// The window it was asked for in; only that window shows it.
    pub window_id: WindowId,
    /// The byte it floats at.
    pub anchor: u64,
    /// The bytes the caret may move within, both ends included.
    pub range: (u64, u64),
    /// The buffer revision it describes.
    pub revision: u64,
    /// The buffer's length at that revision, so a carried typed
    /// character can move the range's end by what it inserted.
    pub len: u64,
    /// The text, bounded ([`popup_lines`]).
    pub lines: Vec<String>,
    /// The active parameter, for a signature.
    pub active_range: Option<PopupActiveRange>,
    /// Lines left out at the bound.
    pub omitted_lines: u32,
}

impl LspPopup {
    /// The wire frame this popup projects to.
    #[must_use]
    pub fn frame(&self) -> PopupFrame {
        PopupFrame {
            buffer_id: self.buffer_id,
            anchor_byte: self.anchor,
            kind: self.kind,
            lines: self.lines.clone(),
            active_range: self.active_range,
            omitted_lines: self.omitted_lines,
        }
    }
}

/// Shared handle: the core writes, the grid overlays and the semantic
/// producer read.
pub type SharedLspPopup = Arc<Mutex<Option<LspPopup>>>;

/// A fresh, closed popup handle.
#[must_use]
pub fn make_shared_popup() -> SharedLspPopup {
    Arc::new(Mutex::new(None))
}

pub use pmacs_protocol::popup::more_lines_label;

/// Server text as bounded popup lines, and how many lines were left out.
///
/// Split on line breaks (`\r` dropped); a markdown code-fence delimiter
/// line (` ``` `, with or without a language) dropped, since it is
/// markup and not text, while the code inside it is kept; tabs expanded
/// to [`TAB_STOP_COLUMNS`]; any other control character shown as
/// U+FFFD rather than dropped silently; runs of blank lines collapsed to
/// one and blank lines at either end trimmed. A line past
/// [`MAX_POPUP_LINE_BYTES`] is cut at a character boundary and ends in
/// `…`. Lines are then taken in order until the next would pass
/// [`MAX_POPUP_LINES`] or [`MAX_POPUP_TEXT_BYTES`]; the rest are
/// counted, not shipped.
#[must_use]
pub fn popup_lines(text: &str) -> (Vec<String>, u32) {
    let mut cleaned: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.trim_start().starts_with("```") {
            continue;
        }
        let line = sanitize_line(raw);
        let blank = line.trim().is_empty();
        if blank && cleaned.last().is_none_or(|l: &String| l.is_empty()) {
            continue;
        }
        cleaned.push(if blank { String::new() } else { line });
    }
    while cleaned.last().is_some_and(String::is_empty) {
        cleaned.pop();
    }
    let mut lines = Vec::new();
    let mut total = 0usize;
    for (i, line) in cleaned.iter().enumerate() {
        let line = cap_line(line);
        if lines.len() == MAX_POPUP_LINES || total + line.len() > MAX_POPUP_TEXT_BYTES {
            return (lines, u32::try_from(cleaned.len() - i).unwrap_or(u32::MAX));
        }
        total += line.len();
        lines.push(line);
    }
    (lines, 0)
}

/// One line with tabs expanded and control characters made visible.
fn sanitize_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut col = 0usize;
    for ch in raw.chars() {
        if ch == '\t' {
            let stop = TAB_STOP_COLUMNS as usize;
            let pad = stop - col % stop;
            out.extend(std::iter::repeat_n(' ', pad));
            col += pad;
        } else if ch.is_control() {
            out.push('\u{FFFD}');
            col += 1;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// A line cut to [`MAX_POPUP_LINE_BYTES`], ending in `…` when cut.
fn cap_line(line: &str) -> String {
    if line.len() <= MAX_POPUP_LINE_BYTES {
        return line.to_owned();
    }
    let mut end = MAX_POPUP_LINE_BYTES - '…'.len_utf8();
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let mut cut = line[..end].to_owned();
    cut.push('…');
    cut
}

/// The active signature as popup lines: its label first, then the
/// active parameter's documentation, then the signature's. `None` when
/// the help carries no signature.
#[must_use]
pub fn signature_lines(
    help: &crate::signature::SignatureHelp,
) -> Option<(Vec<String>, Option<PopupActiveRange>, u32)> {
    let sig = help.active()?;
    let param = help
        .active_parameter_index()
        .and_then(|i| sig.parameters.get(i as usize));
    let mut text = sig.label.clone();
    for doc in [
        param.and_then(|p| p.documentation.as_deref()),
        sig.documentation.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        text.push_str("\n\n");
        text.push_str(doc);
    }
    let (lines, omitted) = popup_lines(&text);
    if lines.is_empty() {
        return None;
    }
    Some((lines, None, omitted))
}

/// The bytes a signature popup holds the caret within, on the caret's
/// line: from just after the innermost `(` left of the caret that no
/// `)` closes, to its matching `)` right of the caret, or to the line's
/// end when it has none. With no open `(` the range is the caret alone.
/// Also returns the anchor, the open parenthesis, or the caret.
#[must_use]
pub fn call_range(line: &[u8], line_start: u64, caret_in_line: usize) -> ((u64, u64), u64) {
    let caret_in_line = caret_in_line.min(line.len());
    let mut depth = 0i32;
    let mut open = None;
    for i in (0..caret_in_line).rev() {
        match line[i] {
            b')' => depth += 1,
            b'(' if depth == 0 => {
                open = Some(i);
                break;
            }
            b'(' => depth -= 1,
            _ => {}
        }
    }
    let caret = line_start + caret_in_line as u64;
    let Some(open) = open else {
        return ((caret, caret), caret);
    };
    let mut depth = 0i32;
    let mut close = line.len();
    for (i, b) in line.iter().enumerate().skip(caret_in_line) {
        match b {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                close = i;
                break;
            }
            b')' => depth -= 1,
            _ => {}
        }
    }
    (
        (line_start + open as u64 + 1, line_start + close as u64),
        line_start + open as u64,
    )
}

/// The word around `caret_in_line` (`[A-Za-z0-9_]` and any non-ASCII
/// byte, so an identifier in any script counts), as absolute bytes;
/// the caret alone when it touches none.
#[must_use]
pub fn word_range(line: &[u8], line_start: u64, caret_in_line: usize) -> (u64, u64) {
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    let caret_in_line = caret_in_line.min(line.len());
    let mut start = caret_in_line;
    while start > 0 && is_word(line[start - 1]) {
        start -= 1;
    }
    let mut end = caret_in_line;
    while end < line.len() && is_word(line[end]) {
        end += 1;
    }
    (line_start + start as u64, line_start + end as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_go_code_stays_blank_runs_collapse_and_ends_trim() {
        let text = "\n```rust\nfn f()\n```\n\n\n---\n\nDocs\tline\r\n\n";
        let (lines, omitted) = popup_lines(text);
        assert_eq!(lines, vec!["fn f()", "", "---", "", "Docs    line"]);
        assert_eq!(omitted, 0);
    }

    #[test]
    fn a_control_character_shows_as_a_replacement() {
        let (lines, _) = popup_lines("a\u{7}b");
        assert_eq!(lines, vec!["a\u{FFFD}b"]);
    }

    #[test]
    fn two_hundred_lines_fit_and_the_bound_counts_the_rest() {
        let text: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        let (lines, omitted) = popup_lines(&text.join("\n"));
        assert_eq!((lines.len(), omitted), (200, 0));

        let text: Vec<String> = (0..1000).map(|i| format!("line {i}")).collect();
        let (lines, omitted) = popup_lines(&text.join("\n"));
        assert_eq!(lines.len(), MAX_POPUP_LINES);
        assert_eq!(omitted as usize, 1000 - MAX_POPUP_LINES);
        assert_eq!(
            lines.last().unwrap(),
            &format!("line {}", MAX_POPUP_LINES - 1)
        );

        // The byte bound binds before the line bound on long lines.
        let long = "x".repeat(MAX_POPUP_LINE_BYTES);
        let text = vec![long; 20].join("\n");
        let (lines, omitted) = popup_lines(&text);
        assert_eq!(lines.len(), MAX_POPUP_TEXT_BYTES / MAX_POPUP_LINE_BYTES);
        assert_eq!(omitted as usize, 20 - lines.len());
        let frame = PopupFrame {
            buffer_id: BufferId::from_raw(1),
            anchor_byte: 0,
            kind: PopupKind::Hover,
            lines,
            active_range: None,
            omitted_lines: omitted,
        };
        assert_eq!(frame.validate(), Ok(()), "the cut always validates");
    }

    #[test]
    fn a_line_past_its_bound_is_cut_at_a_character_and_says_so() {
        let line = "é".repeat(MAX_POPUP_LINE_BYTES);
        let (lines, _) = popup_lines(&line);
        assert!(lines[0].len() <= MAX_POPUP_LINE_BYTES);
        assert!(lines[0].ends_with('…'));
    }

    #[test]
    fn call_range_spans_the_open_call_and_word_range_the_identifier() {
        let line = b"    foo(a, bar(b), c) + x";
        // Caret after `a, ` : inside foo's call, past a closed inner call.
        let caret = "    foo(a, bar(b), ".len();
        let ((s, e), anchor) = call_range(line, 100, caret);
        assert_eq!(anchor, 100 + 7, "the open parenthesis");
        assert_eq!((s, e), (100 + 8, 100 + 20), "to foo's own `)`");
        // No open call: the caret alone.
        assert_eq!(call_range(line, 0, 2), ((2, 2), 2));
        assert_eq!(word_range("let größe = 1".as_bytes(), 10, 6), (14, 21));
        assert_eq!(word_range(b"a + b", 0, 2), (2, 2));
    }

    #[test]
    fn the_more_lines_label_names_the_count_and_the_persistent_form() {
        assert_eq!(
            more_lines_label(1),
            "… 1 more line · C-c H opens *lsp-help*"
        );
        assert!(more_lines_label(412).starts_with("… 412 more lines"));
    }
}
