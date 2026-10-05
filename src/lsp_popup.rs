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
//! it too. The one edit a popup survives is a typed character inside a
//! signature's call, which the runtime carries forward
//! ([`crate::editor_core::EditorCore::lsp_popup_carry`], E8.5) so the
//! user can type the argument the popup describes; a typed `)` closes
//! it.
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

/// The active signature as popup lines: its label first, with the
/// active parameter's byte range in it (E8.5), then the parameter's
/// documentation, then the signature's. `None` when the help carries no
/// signature.
///
/// The range survives only when the label reaches the popup unchanged
/// (no control character, no tab, not cut at the bound), since its
/// offsets index the label as the server sent it.
#[must_use]
pub fn signature_lines(
    help: &crate::signature::SignatureHelp,
) -> Option<(Vec<String>, Option<PopupActiveRange>, u32)> {
    let sig = help.active()?;
    let index = help
        .active_parameter_index()
        .map(|i| i as usize)
        .filter(|&i| i < sig.parameters.len());
    let param = index.map(|i| &sig.parameters[i]);
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
    let range = index
        .and_then(|i| parameter_byte_range(sig, i))
        .filter(|_| lines[0] == sig.label)
        .map(|(start, end)| PopupActiveRange {
            line: 0,
            start: start as u32,
            end: end as u32,
        });
    Some((lines, range, omitted))
}

/// The bytes of parameter `index` in its signature's label: the
/// server's own offsets when it gave them (pmacs declares
/// `labelOffsetSupport`, and [`crate::signature`] converts them to bytes
/// at absorb), else the parameter's label searched for **in order**: each
/// parameter from the first is found after the end of the one before it,
/// starting just after the label's first `(`. So a later parameter is
/// never found inside an earlier one --- `len: usize` inside
/// `max_len: usize`, the second `i32` of `Pair(i32, i32)` --- and a
/// parameter named like its function is not found in the function's
/// name. A server that ignores the capability and sends strings takes
/// this path.
fn parameter_byte_range(sig: &crate::signature::Signature, index: usize) -> Option<(usize, usize)> {
    let label = sig.label.as_str();
    let valid = |(s, e): (u32, u32)| {
        let (s, e) = (s as usize, e as usize);
        (s < e && e <= label.len() && label.is_char_boundary(s) && label.is_char_boundary(e))
            .then_some((s, e))
    };
    let mut from = label.find('(').map_or(0, |i| i + 1);
    for (i, param) in sig.parameters.iter().enumerate().take(index + 1) {
        let found = match param.span {
            Some(span) => valid(span),
            None if param.label.is_empty() => None,
            None => label[from..]
                .find(param.label.as_str())
                .map(|at| (from + at, from + at + param.label.len())),
        };
        if i == index {
            return found;
        }
        // A parameter not found leaves the search where it was: the next
        // is still after every parameter that was.
        if let Some((_, end)) = found {
            from = from.max(end);
        }
    }
    None
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

/// Most columns the grid popup is wide, before the window's own width
/// clamps it: a hover's prose reads at a line length, not a window's.
const GRID_POPUP_MAX_COLS: u32 = 80;

/// One painted row of the grid popup: the text, the bytes of it the
/// active parameter covers, and whether it is the closing "more" row.
struct GridRow {
    text: String,
    active: Option<(usize, usize)>,
    footer: bool,
}

/// `line` broken into rows of at most `cols` display columns, at the
/// last space that fits and by character where none does, as
/// `(start, end)` byte ranges.
fn wrap_columns(line: &str, cols: u32) -> Vec<(usize, usize)> {
    use unicode_width::UnicodeWidthChar;
    let cols = cols.max(1);
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut width = 0u32;
    let mut last_space: Option<usize> = None;
    for (i, ch) in line.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0) as u32;
        if width + w > cols && i > start {
            let cut = last_space.filter(|&s| s > start).unwrap_or(i);
            rows.push((start, cut));
            start = if cut == i { i } else { cut + 1 };
            width = line[start..i]
                .chars()
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u32)
                .sum();
            last_space = None;
        }
        if ch == ' ' {
            last_space = Some(i);
        }
        width += w;
    }
    rows.push((start, line.len()));
    rows
}

/// The popup's rows for a box `cols` wide and at most `max_rows` tall:
/// every line wrapped, then cut to fit with a closing row naming what
/// was left out --- the lines past the wire's bound plus the lines this
/// box could not show --- and where the whole text is.
fn grid_rows(popup: &LspPopup, cols: u32, max_rows: u32) -> Vec<GridRow> {
    let mut rows: Vec<(usize, GridRow)> = Vec::new();
    for (index, line) in popup.lines.iter().enumerate() {
        let active = popup
            .active_range
            .filter(|r| r.line as usize == index)
            .map(|r| (r.start as usize, r.end as usize));
        for (s, e) in wrap_columns(line, cols) {
            let clipped = active.and_then(|(a, b)| {
                let (a, b) = (a.max(s), b.min(e));
                (a < b).then_some((a - s, b - s))
            });
            rows.push((
                index,
                GridRow {
                    text: line[s..e].to_owned(),
                    active: clipped,
                    footer: false,
                },
            ));
        }
    }
    let max_rows = max_rows.max(1) as usize;
    if rows.len() <= max_rows && popup.omitted_lines == 0 {
        return rows.into_iter().map(|(_, r)| r).collect();
    }
    // Keep a row for the closing one; the first line it drops, whole or
    // in part, is how many lines show whole.
    let keep = rows.len().min(max_rows - 1);
    let shown_whole = rows.get(keep).map_or(popup.lines.len(), |&(i, _)| i);
    let left = (popup.lines.len() - shown_whole) as u32 + popup.omitted_lines;
    rows.truncate(keep);
    rows.push((
        usize::MAX,
        GridRow {
            text: more_lines_label(left),
            active: None,
            footer: true,
        },
    ));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// A face from the theme, or the grid's own default for it (also when
/// there is no theme, as in a bare core).
fn face_or(
    theme: Option<&crate::highlight::ThemeHandle>,
    name: &str,
    default: crate::cell::Style,
) -> crate::cell::Style {
    theme
        .and_then(|t| t.lock().expect("theme mutex poisoned").face(name))
        .unwrap_or(default)
}

/// Paint `popup` into a window's cells (E8.4): the completion popup's
/// anchor walk, row window and row painter, placed by the one popup
/// rule ([`pmacs_protocol::place_popup`], E7d.3) in cells --- below the
/// anchor's row when it fits there, else above, else on the roomier
/// side, clamped into the window --- and themed by the `ui.popup` faces.
pub fn paint_grid_popup(
    buf: &crate::buffer::Buffer,
    viewport: crate::view::Viewport<'_>,
    cells: &mut crate::cell::CellGrid<'_>,
    popup: &LspPopup,
    theme: Option<&crate::highlight::ThemeHandle>,
) {
    let (win_rows, win_cols) = (viewport.cell_size.rows, viewport.cell_size.cols);
    if win_rows == 0 || win_cols < 3 {
        return;
    }
    let Some((anchor_row, anchor_col)) =
        crate::completion::anchor_cell(buf, viewport, popup.anchor)
    else {
        return; // the anchor is scrolled out or folded away
    };
    let width = GRID_POPUP_MAX_COLS.min(win_cols);
    // The text sits two columns in, as a completion row's label does.
    let text_cols = width - 2;
    let max_rows = (win_rows / 2).max(3).min(win_rows);
    let rows = grid_rows(popup, text_cols, max_rows);
    let (start, len) = crate::completion::popup_window(rows.len(), 0, max_rows as usize);
    let shown = &rows[start..start + len];
    let widest = shown
        .iter()
        .map(|r| crate::display_width::byte_to_column(r.text.as_bytes(), r.text.len()))
        .max()
        .unwrap_or(0);
    let width = (widest + 3).min(width).max(3);
    let height = shown.len() as u32;
    let (x, y) = pmacs_protocol::place_popup(
        (
            anchor_col as f32,
            anchor_row as f32,
            (anchor_row + 1) as f32,
        ),
        (width as f32, height as f32),
        (win_cols as f32, win_rows as f32),
    );
    let (left, top) = (x as u32, y as u32);
    let base = face_or(theme, "ui.popup", crate::completion::popup_style());
    // The mark is structural and the face is color: a theme that sets
    // only `ui.popup` resolves `ui.popup.active-parameter` to it by the
    // dotted-prefix walk, and the parameter must still stand out.
    let active = crate::cell::Style {
        bold: true,
        underline: crate::cell::UnderlineStyle::Single,
        ..face_or(theme, "ui.popup.active-parameter", base)
    };
    let footer = face_or(theme, "ui.popup.footer", crate::completion::kind_style());
    let origin = viewport.cell_origin;
    for (i, row) in shown.iter().enumerate() {
        let r = origin.row + top + i as u32;
        if top + i as u32 >= win_rows {
            break;
        }
        crate::completion::paint_text_row(
            cells,
            ' ',
            &row.text,
            r,
            origin.col + left,
            width.min(win_cols - left),
            if row.footer { footer } else { base },
            row.active.map(|(s, e)| (s, e, active)),
        );
    }
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

    fn popup_of(lines: &[&str], omitted: u32) -> LspPopup {
        LspPopup {
            kind: PopupKind::Hover,
            buffer_id: BufferId::from_raw(1),
            window_id: crate::window::WindowId::next(),
            anchor: 0,
            range: (0, 0),
            revision: 0,
            len: 0,
            lines: lines.iter().map(|l| (*l).to_owned()).collect(),
            active_range: None,
            omitted_lines: omitted,
        }
    }

    #[test]
    fn grid_rows_wrap_at_spaces_and_close_with_what_they_left_out() {
        // A short text fits whole, no closing row.
        let rows = grid_rows(&popup_of(&["one two", "three"], 0), 20, 5);
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["one two", "three"]);
        // Wrapped at the last space that fits.
        let rows = grid_rows(&popup_of(&["alpha beta gamma"], 0), 11, 5);
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["alpha beta", "gamma"]);
        // Ten lines in a five-row box: four shown, the fifth row says the
        // other six, plus what the wire's bound left out.
        let lines: Vec<String> = (0..10).map(|i| format!("l{i}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let rows = grid_rows(&popup_of(&refs, 7), 20, 5);
        assert_eq!(rows.len(), 5);
        assert!(rows[4].footer);
        assert_eq!(rows[4].text, more_lines_label(6 + 7));
        // A text that fits but was cut at the wire's bound still says so.
        let rows = grid_rows(&popup_of(&["only"], 3), 20, 5);
        assert_eq!(
            rows.last().map(|r| r.text.clone()),
            Some(more_lines_label(3))
        );
    }

    #[test]
    fn the_active_parameter_is_its_span_or_its_label_after_the_paren() {
        use crate::signature::{Signature, SignatureHelp, SignatureParameter};
        let help = |param: SignatureParameter| SignatureHelp {
            signatures: vec![Signature {
                label: "fn x(x: u8, y: u8)".to_owned(),
                documentation: None,
                parameters: vec![param],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: Some(0),
        };
        // By label: found after the `(`, not in the function's name.
        let (_, range, _) = signature_lines(&help(SignatureParameter {
            label: "x".to_owned(),
            span: None,
            documentation: None,
        }))
        .expect("lines");
        assert_eq!(
            range,
            Some(PopupActiveRange {
                line: 0,
                start: 5,
                end: 6
            })
        );
        // By the server's span.
        let (_, range, _) = signature_lines(&help(SignatureParameter {
            label: "y: u8".to_owned(),
            span: Some((12, 17)),
            documentation: None,
        }))
        .expect("lines");
        assert_eq!(
            range,
            Some(PopupActiveRange {
                line: 0,
                start: 12,
                end: 17
            })
        );
    }

    /// E8 fix round 1 (review 1's Medium 1): a server that sends string
    /// labels has them searched in order, so a later parameter is found
    /// after the one before it and never inside it.
    #[test]
    fn a_string_label_is_found_after_the_parameter_before_it() {
        use crate::signature::{Signature, SignatureHelp, SignatureParameter};
        let marked = |label: &str, params: &[&str], active: u32| {
            let help = SignatureHelp {
                signatures: vec![Signature {
                    label: label.to_owned(),
                    documentation: None,
                    parameters: params
                        .iter()
                        .map(|p| SignatureParameter {
                            label: (*p).to_owned(),
                            span: None,
                            documentation: None,
                        })
                        .collect(),
                    active_parameter: None,
                }],
                active_signature: 0,
                active_parameter: Some(active),
            };
            let (_, range, _) = signature_lines(&help).expect("lines");
            range.map(|r| {
                let (s, e) = (r.start as usize, r.end as usize);
                (s, label[s..e].to_owned())
            })
        };
        let pair = "struct Pair(i32, i32)";
        assert_eq!(marked(pair, &["i32", "i32"], 0), Some((12, "i32".into())));
        assert_eq!(marked(pair, &["i32", "i32"], 1), Some((17, "i32".into())));
        let clamp = "fn clamp_len(max_len: usize, len: usize) -> usize";
        let params = ["max_len: usize", "len: usize"];
        assert_eq!(marked(clamp, &params, 1), Some((29, "len: usize".into())));
        // A parameter the label does not hold marks nothing, and the one
        // after it is still found after the parameters that were.
        let three = "fn f(a: u8, b: u8, a: u8)";
        assert_eq!(marked(three, &["a: u8", "zz", "a: u8"], 1), None);
        assert_eq!(
            marked(three, &["a: u8", "zz", "a: u8"], 2),
            Some((19, "a: u8".into()))
        );
    }
}
