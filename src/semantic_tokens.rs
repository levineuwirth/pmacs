// semantic_tokens.rs --- T M4.5 LSP semantic tokens.

//! `textDocument/semanticTokens/full` response state.
//!
//! Semantic tokens are the server's type-aware classification of every
//! token in a document (this identifier is a *mutable* `variable`,
//! that one a `function.defaultLibrary`, …). The wire format is a flat
//! `data: number[]` whose every 5 ints describe one token **relative**
//! to the previous one:
//!
//! ```text
//!   [deltaLine, deltaStartChar, length, tokenType, tokenModifiers]
//! ```
//!
//! This module decodes that into a flat list of *absolute*
//! [`SemanticToken`]s, and parses the server's
//! `semanticTokensProvider.legend` so callers can resolve the
//! `token_type` / `token_modifiers` indices to names.
//!
//! Scope: this is the **LSP data layer only**. It is deliberately
//! independent of the M11 semantic-render protocol
//! ([`crate::semantic_render`] / [`crate::semantic_client`]), which
//! projects tree-sitter highlighting into the frontend wire families.
//! Wiring LSP tokens into rendering (a second styling authority,
//! priority vs. tree-sitter) is a separate rendering milestone; like
//! the other LSP features, nothing here paints — Lua reads the store.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One decoded, **absolute**-positioned semantic token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticToken {
    /// Zero-based line.
    pub line: u32,
    /// Zero-based start column (UTF-16 code units, per LSP).
    pub start: u32,
    /// Token length in UTF-16 code units.
    pub length: u32,
    /// Index into the legend's `token_types`.
    pub token_type: u32,
    /// Bitset; bit `i` set ⇒ legend's `token_modifiers[i]` applies.
    pub token_modifiers: u32,
}

/// Parsed `textDocument/semanticTokens` (`/full`, `/range`, or a
/// delta-applied `/full/delta`) response.
#[derive(Clone, Debug, Default)]
pub struct SemanticTokensResponse {
    /// Tokens in document order (decoded from the relative encoding).
    pub tokens: Vec<SemanticToken>,
    /// Opaque server cursor. Pass back as `previousResultId` on the
    /// next `/full/delta` request.
    pub result_id: Option<String>,
    /// The raw relative-encoded int stream this response decoded
    /// from. Retained so a subsequent `/full/delta` can splice its
    /// edits against it (the delta is expressed over the *previous
    /// data array*, not the decoded tokens).
    pub raw: Vec<u32>,
}

/// Decode the flat relative-encoded int stream into absolute tokens.
/// A trailing partial (<5) group is ignored rather than panicking.
fn decode(ints: &[u32]) -> Vec<SemanticToken> {
    let mut tokens = Vec::with_capacity(ints.len() / 5);
    let mut line = 0u32;
    let mut start = 0u32;
    for chunk in ints.chunks_exact(5) {
        let (d_line, d_start, length, tt, tm) = (chunk[0], chunk[1], chunk[2], chunk[3], chunk[4]);
        // deltaLine is relative to the previous token's line;
        // deltaStartChar is relative to the previous token's start
        // *iff* on the same line, else absolute from col 0.
        line += d_line;
        start = if d_line == 0 {
            start + d_start
        } else {
            d_start
        };
        tokens.push(SemanticToken {
            line,
            start,
            length,
            token_type: tt,
            token_modifiers: tm,
        });
    }
    tokens
}

fn ints_of(v: &Value, key: &str) -> Vec<u32> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|n| n.as_u64().unwrap_or(0) as u32).collect())
        .unwrap_or_default()
}

impl SemanticTokensResponse {
    /// Parse a `SemanticTokens | null` (the `/full` and `/range`
    /// shape). A `null` / shapeless result yields no tokens.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        let result_id = v.get("resultId").and_then(Value::as_str).map(str::to_owned);
        if v.get("data").and_then(Value::as_array).is_none() {
            return Self {
                tokens: Vec::new(),
                result_id,
                raw: Vec::new(),
            };
        }
        let raw = ints_of(v, "data");
        Self {
            tokens: decode(&raw),
            result_id,
            raw,
        }
    }

    /// Apply a `/full/delta` response against the previous raw int
    /// stream. The server is allowed by the spec to answer a delta
    /// request with a *full* `SemanticTokens` instead — detected by a
    /// `data` array and handled by [`Self::from_lsp_value`].
    ///
    /// A `SemanticTokensDelta` is `{ resultId?, edits: [{ start,
    /// deleteCount, data? }] }`, each edit a splice over the previous
    /// data array. Edits are applied in descending `start` order so
    /// earlier indices stay valid regardless of server ordering, and
    /// bounds are clamped defensively.
    #[must_use]
    pub fn apply_delta(prev_raw: &[u32], v: &Value) -> Self {
        if v.get("data").and_then(Value::as_array).is_some() {
            return Self::from_lsp_value(v);
        }
        let result_id = v.get("resultId").and_then(Value::as_str).map(str::to_owned);
        let mut data = prev_raw.to_vec();
        if let Some(edits) = v.get("edits").and_then(Value::as_array) {
            let mut parsed: Vec<(usize, usize, Vec<u32>)> = edits
                .iter()
                .filter_map(|e| {
                    let start = e.get("start")?.as_u64()? as usize;
                    let delete = e.get("deleteCount")?.as_u64()? as usize;
                    Some((start, delete, ints_of(e, "data")))
                })
                .collect();
            parsed.sort_by_key(|e| std::cmp::Reverse(e.0));
            for (start, delete, ins) in parsed {
                let s = start.min(data.len());
                let end = start.saturating_add(delete).min(data.len());
                data.splice(s..end, ins);
            }
        }
        Self {
            tokens: decode(&data),
            result_id,
            raw: data,
        }
    }

    /// True iff the server returned no tokens.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// The server's `semanticTokensProvider.legend`: the ordered name
/// tables the `token_type` index and `token_modifiers` bits map into.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticTokensLegend {
    /// `token_type` index → name.
    pub token_types: Vec<String>,
    /// Modifier bit position → name.
    pub token_modifiers: Vec<String>,
}

impl SemanticTokensLegend {
    /// Pull the legend out of an `initialize` `ServerCapabilities`
    /// JSON value. Returns `None` if the server advertises no
    /// `semanticTokensProvider` (or it carries no `legend`).
    #[must_use]
    pub fn from_capabilities(caps: &Value) -> Option<Self> {
        let legend = caps.get("semanticTokensProvider")?.get("legend")?;
        let pull = |key: &str| -> Vec<String> {
            legend
                .get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self {
            token_types: pull("tokenTypes"),
            token_modifiers: pull("tokenModifiers"),
        })
    }

    /// Resolve a `token_type` index to its legend name.
    #[must_use]
    pub fn type_name(&self, index: u32) -> Option<&str> {
        self.token_types.get(index as usize).map(String::as_str)
    }

    /// Resolve a `token_modifiers` bitset to the set legend names,
    /// low bit first.
    #[must_use]
    pub fn modifier_names(&self, bits: u32) -> Vec<&str> {
        (0..self.token_modifiers.len())
            .filter(|i| bits & (1 << i) != 0)
            .map(|i| self.token_modifiers[i].as_str())
            .collect()
    }

    /// The one type/modifier style resolver both styling paths read:
    /// `<type>.<first-modifier>` when modifiers are set, else `<type>`.
    /// The theme's dotted-prefix `lookup` walks back to the base when no
    /// refined entry exists, so the suffix is a strict refinement. `None`
    /// when the type index is unknown. The grid's `LspStyleView` and the
    /// wire's `lsp_scoped_style_spans` must agree here or a
    /// modifier-specific face differs between frontends.
    #[must_use]
    pub fn style_name_for(
        &self,
        token_type: u32,
        token_modifiers: u32,
    ) -> Option<std::borrow::Cow<'_, str>> {
        let name = self.type_name(token_type)?;
        let mods = self.modifier_names(token_modifiers);
        match mods.first() {
            Some(m) => Some(std::borrow::Cow::Owned(format!("{name}.{m}"))),
            None => Some(std::borrow::Cow::Borrowed(name)),
        }
    }
}

/// One edit to a document, in the byte coordinates of the text before
/// it: `[start, old_end)` was replaced by `inserted_len` bytes. This is
/// exactly what [`crate::rope::Edit`] broadcasts to every buffer view
/// (`range` and `inserted_len`), so the recorder copies and never
/// derives. E6b.1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentEdit {
    /// First byte of the replaced range, in the pre-edit text.
    pub start: u64,
    /// One past the last replaced byte, in the pre-edit text; equal to
    /// `start` for a pure insert.
    pub old_end: u64,
    /// Bytes inserted at `start` in the post-edit text.
    pub inserted_len: u64,
}

impl DocumentEdit {
    /// Map a pre-edit position that begins a range: a position inside
    /// the replaced text snaps to the replacement's end, so the range
    /// keeps its bytes past the edit and claims none of the new text.
    fn translate_start(self, pos: u64) -> u64 {
        if self.old_end == self.start {
            if pos >= self.start {
                pos.saturating_add(self.inserted_len)
            } else {
                pos
            }
        } else if pos <= self.start {
            pos
        } else if pos >= self.old_end {
            self.shift(pos)
        } else {
            self.start.saturating_add(self.inserted_len)
        }
    }

    /// Map a pre-edit position that ends a range. A range ending
    /// exactly at a pure insert EXTENDS over the inserted text (the
    /// GPU's own rule, `translate_range_end` in `pmacs-gpu`: typing at
    /// the end of a token is the dominant editing case, and the new
    /// character inheriting the token's color is what keeps it from
    /// blinking); a range cut by a replace keeps only what stood before
    /// the replaced text.
    fn translate_end(self, pos: u64) -> u64 {
        if self.old_end == self.start {
            if pos >= self.start {
                pos.saturating_add(self.inserted_len)
            } else {
                pos
            }
        } else if pos <= self.start {
            pos
        } else if pos >= self.old_end {
            self.shift(pos)
        } else {
            self.start
        }
    }

    fn shift(self, pos: u64) -> u64 {
        pos.saturating_sub(self.old_end - self.start)
            .saturating_add(self.inserted_len)
    }
}

/// A semantic token resolved to byte coordinates of the text it now
/// describes: the server's answer converted against the text the
/// server saw, then carried through every edit recorded since. Both
/// merge sites read this shape and neither converts a column again
/// (E6b.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedToken {
    /// First byte covered.
    pub start: u64,
    /// One past the last byte covered.
    pub end: u64,
    /// Index into the legend's `token_types`.
    pub token_type: u32,
    /// Bitset over the legend's `token_modifiers`.
    pub token_modifiers: u32,
}

impl PositionedToken {
    /// Carry the token across `edit`, or drop it when nothing of it
    /// survives (a token wholly inside a deleted range). Tokens are
    /// translated in place and in document order, and the mapping is
    /// monotone, so a sorted, disjoint list stays sorted and disjoint:
    /// a pure insert at a boundary between two tokens extends the
    /// left one and shifts the right one by the same amount.
    fn translated(self, edit: DocumentEdit) -> Option<Self> {
        let start = edit.translate_start(self.start);
        let end = edit.translate_end(self.end);
        (start < end).then_some(Self { start, end, ..self })
    }
}

/// Resolve a response's `(line, character, length)` tokens to byte
/// ranges of `text`, the document the server computed them against,
/// in the server's negotiated `encoding`. Tokens on a line past the
/// end of `text` are skipped (a response for text the store no longer
/// has a copy of would have been refused before reaching here).
/// Single-line by the LSP grammar, so the per-line conversion is exact;
/// the same conversion the two merge sites did per frame before E6b.2,
/// done once per response instead.
fn resolve_tokens(
    tokens: &[SemanticToken],
    text: &str,
    encoding: crate::lsp::PositionEncoding,
) -> Vec<PositionedToken> {
    let mut line_starts: Vec<usize> = vec![0];
    line_starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
    );
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        let Some(&ls) = line_starts.get(t.line as usize) else {
            continue;
        };
        let le = line_starts
            .get(t.line as usize + 1)
            .map_or(text.len(), |&n| n.saturating_sub(1));
        let line = &text[ls..le];
        let start = ls + crate::lsp::char_to_byte(line, t.start, encoding);
        let end = ls + crate::lsp::char_to_byte(line, t.start.saturating_add(t.length), encoding);
        if end <= start {
            continue;
        }
        out.push(PositionedToken {
            start: start as u64,
            end: end as u64,
            token_type: t.token_type,
            token_modifiers: t.token_modifiers,
        });
    }
    out
}

/// How many recorded edits a URI's log keeps. A server that stops
/// answering under continuous typing would otherwise grow the log
/// without bound; past the cap the oldest edits are forgotten, and a
/// response computed against text older than the oldest kept edit is
/// stored as unalignable rather than misaligned.
const EDIT_LOG_CAP: usize = 4096;

/// The edits a document has taken since its text was last sent to the
/// server, in order, each numbered so a response can say which of them
/// it already reflects (E6b.1).
struct EditLog {
    /// Number the next recorded edit takes; every edit so far has a
    /// smaller one.
    next_seq: u64,
    /// `next_seq` at the last `didOpen` / `didChange`: the server's
    /// copy of the document reflects every edit numbered below it.
    synced_seq: u64,
    /// Edits numbered below this were forgotten (the cap, a clear, or
    /// a token absorb's prune), so a response with an older base cannot
    /// be aligned, an incremental `didChange` cannot be built from
    /// them, and a completion carry from before them is refused rather
    /// than placed against a log that no longer reaches its base.
    dropped_below: u64,
    /// Each edit with the text it inserted (empty for a delete), which
    /// is what an incremental `didChange` carries for it (E6d.1); the
    /// byte range alone cannot name the bytes a later edit may have
    /// changed again.
    edits: VecDeque<(u64, DocumentEdit, Arc<str>)>,
    /// The edit number the newest completion answer for this document
    /// was answered against (E7.4). A token answer's absorb prunes the
    /// log below its own base, tokens being the log's first consumer;
    /// an accepted completion's `additionalTextEdits` are carried from
    /// this number, so the prune keeps everything from here on while
    /// an answer is held. `u64::MAX` when none is.
    completion_floor: u64,
}

impl Default for EditLog {
    fn default() -> Self {
        Self {
            next_seq: 0,
            synced_seq: 0,
            dropped_below: 0,
            edits: VecDeque::new(),
            completion_floor: u64::MAX,
        }
    }
}

/// One server's answer for one document, held in both the server's
/// coordinates (`response`, for the Lua surface and delta splicing)
/// and the document's current bytes (`spans`).
struct Entry {
    response: SemanticTokensResponse,
    /// `response.tokens` resolved against the text the server saw and
    /// carried through every edit since; sorted, disjoint. Empty when
    /// `unalignable`.
    spans: Vec<PositionedToken>,
    /// The edit number the server's text was current at: every edit
    /// numbered at or above it has been applied to `spans`.
    base_seq: u64,
    /// Edits applied to `spans` since the server's text; zero means the
    /// tokens are exactly the server's.
    pending: u64,
    /// The response was computed against text whose edits the log no
    /// longer holds; nothing of it is painted until the next response.
    unalignable: bool,
}

/// Per-server, per-uri semantic-token state.
#[derive(Default)]
pub struct SemanticTokenStore {
    by_key: HashMap<SemanticTokenKey, Entry>,
    /// URIs a caller declared stale without saying where the edit was
    /// (`mark_stale`), for documents no recorder tracks. A URI with an
    /// edit log never lands here: the log is the authority for it.
    stale_uris: HashSet<String>,
    /// Per-URI edit logs, opened by the recorder that feeds them
    /// (`SemanticEditRecorder`) and by nothing else, so a log's
    /// existence says the document's edits reach this store.
    logs: HashMap<String, EditLog>,
    /// Bumped on every mutation. The semantic-render style gate keys
    /// on it (E5.6): a grammar-backed buffer's wire spans merge LSP
    /// tokens over tree-sitter captures, so a token response must flip
    /// the gate the parse bundle alone used to key.
    version: u64,
}

/// Key into [`SemanticTokenStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct SemanticTokenKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI the request was made on.
    pub uri: String,
}

impl SemanticTokenKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl SemanticTokenStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a response computed against `text` (the document as the
    /// server saw it, in its negotiated `encoding`), current as of
    /// edit number `base_seq` of the URI's log: every edit recorded at
    /// or after `base_seq` is applied to the resolved tokens, so they
    /// describe the document as it is now, and the log forgets the
    /// edits before it. A response older than the one already held
    /// (`base_seq` below the entry's) is refused, since it describes
    /// text the entry has already moved past; a response whose base
    /// the log no longer covers is held unalignable. Returns whether
    /// the entry changed.
    pub fn set(
        &mut self,
        key: SemanticTokenKey,
        response: SemanticTokensResponse,
        text: &str,
        encoding: crate::lsp::PositionEncoding,
        base_seq: u64,
    ) -> bool {
        if self
            .by_key
            .get(&key)
            .is_some_and(|e| !e.unalignable && e.base_seq > base_seq)
        {
            return false;
        }
        self.stale_uris.remove(&key.uri);
        let mut entry = Entry {
            spans: resolve_tokens(&response.tokens, text, encoding),
            response,
            base_seq,
            pending: 0,
            unalignable: false,
        };
        if let Some(log) = self.logs.get_mut(&key.uri) {
            if base_seq < log.dropped_below {
                entry.unalignable = true;
                entry.spans.clear();
            } else {
                for &(seq, edit, _) in &log.edits {
                    if seq >= base_seq {
                        translate_spans(&mut entry.spans, edit);
                        entry.pending += 1;
                    }
                }
                let keep_from = base_seq.min(log.completion_floor);
                log.edits.retain(|(seq, ..)| *seq >= keep_from);
                log.dropped_below = log.dropped_below.max(keep_from);
            }
        }
        self.by_key.insert(key, entry);
        self.version += 1;
        true
    }

    /// Absorb a `semanticTokens/range` answer for the positions
    /// `range` --- `((start_line, start_col), (end_line, end_col))` in
    /// byte columns, end exclusive --- of `text` (E6b.4): the held
    /// tokens inside that extent are replaced by the answer's, the
    /// ones outside are kept, the Lua surface's decoded tokens become
    /// the answer's, and the entry's `raw` and `result_id` stay as the
    /// last whole answer put them so the delta chain is not broken by
    /// a range's `resultId`; an entry created by a range answer holds
    /// no `raw` and no `result_id`, so the next pull asks for the
    /// whole document. The requested lines
    /// are carried across the same edits as the tokens, so the region
    /// replaced is where those lines are now. Refused and held
    /// unalignable on the same terms as [`Self::set`].
    pub fn set_range(
        &mut self,
        key: SemanticTokenKey,
        response: &SemanticTokensResponse,
        text: &str,
        encoding: crate::lsp::PositionEncoding,
        base_seq: u64,
        range: ((u32, u32), (u32, u32)),
    ) -> bool {
        if self
            .by_key
            .get(&key)
            .is_some_and(|e| !e.unalignable && e.base_seq > base_seq)
        {
            return false;
        }
        // The byte extent of the requested range on the server's text,
        // as one token so it rides the same translation.
        let mut line_starts = vec![0usize];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
        );
        let byte_at = |(line, col): (u32, u32)| -> usize {
            line_starts.get(line as usize).map_or(text.len(), |&ls| {
                ls.saturating_add(col as usize).min(text.len())
            })
        };
        let extent_start = byte_at(range.0);
        let extent_end = byte_at(range.1).max(extent_start);
        let mut extent = vec![PositionedToken {
            start: extent_start as u64,
            end: extent_end.max(extent_start + 1) as u64,
            token_type: 0,
            token_modifiers: 0,
        }];
        let mut fresh = resolve_tokens(&response.tokens, text, encoding);
        let mut pending = 0;
        let mut unalignable = false;
        if let Some(log) = self.logs.get(&key.uri) {
            if base_seq < log.dropped_below {
                unalignable = true;
            } else {
                for &(seq, edit, _) in &log.edits {
                    if seq >= base_seq {
                        translate_spans(&mut fresh, edit);
                        translate_spans(&mut extent, edit);
                        pending += 1;
                    }
                }
            }
        }
        self.stale_uris.remove(&key.uri);
        let entry = self.by_key.entry(key).or_insert_with(|| Entry {
            response: SemanticTokensResponse::default(),
            spans: Vec::new(),
            base_seq,
            pending: 0,
            unalignable: false,
        });
        if unalignable {
            entry.unalignable = true;
            entry.spans.clear();
        } else {
            let (ext_start, ext_end) = extent
                .first()
                .map_or((u64::MAX, u64::MAX), |e| (e.start, e.end));
            entry
                .spans
                .retain(|t| t.end <= ext_start || t.start >= ext_end);
            entry.spans.extend(fresh);
            entry.spans.sort_unstable_by_key(|t| (t.start, t.end));
            entry.unalignable = false;
        }
        entry.response.tokens.clone_from(&response.tokens);
        entry.base_seq = base_seq;
        entry.pending = pending;
        self.version += 1;
        true
    }

    /// [`Self::set`] for a response that describes the document as it
    /// stands now (`base_seq` = the next edit number): the shape a
    /// test seeds with, and a server answering a document no edit has
    /// touched since it was sent.
    pub fn set_current(
        &mut self,
        key: SemanticTokenKey,
        response: SemanticTokensResponse,
        text: &str,
        encoding: crate::lsp::PositionEncoding,
    ) -> bool {
        let base_seq = self.next_seq(&key.uri);
        self.set(key, response, text, encoding, base_seq)
    }

    /// A counter that changes on every mutation, so a consumer can key
    /// a cache on "the tokens might differ".
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Drop the entry at `key`. Also clears the stale flag for that
    /// URI when no other server has token data for it. The URI's edit
    /// log stays open --- its recorder is still attached --- but its
    /// edits are forgotten, so a response already in flight for the
    /// cleared entry lands unalignable rather than misaligned.
    pub fn clear(&mut self, key: &SemanticTokenKey) {
        self.by_key.remove(key);
        if !self.by_key.keys().any(|k| k.uri == key.uri) {
            self.stale_uris.remove(&key.uri);
            if let Some(log) = self.logs.get_mut(&key.uri) {
                log.edits.clear();
                log.dropped_below = log.next_seq;
            }
        }
        self.version += 1;
    }

    /// Open the edit log for `uri`, declaring that a recorder will
    /// report every edit the document takes from now on. Idempotent.
    /// From here `mark_stale` is a no-op for `uri` and staleness is
    /// what the log says.
    pub fn open_log(&mut self, uri: impl Into<String>) {
        self.logs.entry(uri.into()).or_default();
    }

    /// Record one edit to `uri`'s document and carry every held token
    /// set for it across the edit, so what the store hands out stays
    /// aligned with the text at every keystroke (E6b.2). Opens the log
    /// if the recorder has not yet. No-op edits (nothing removed,
    /// nothing inserted) are ignored: buffers broadcast them
    /// deliberately, and translating by zero would only bump the
    /// version.
    ///
    /// `inserted` is the text the edit put at `edit.start`, kept so an
    /// incremental `didChange` can ship the edit itself instead of the
    /// document (E6d.1); it is `edit.inserted_len` bytes long, and
    /// empty for a delete.
    pub fn record_edit(&mut self, uri: &str, edit: DocumentEdit, inserted: &str) {
        if edit.old_end == edit.start && edit.inserted_len == 0 {
            return;
        }
        let log = self.logs.entry(uri.to_owned()).or_default();
        let seq = log.next_seq;
        log.next_seq += 1;
        log.edits.push_back((seq, edit, Arc::from(inserted)));
        if log.edits.len() > EDIT_LOG_CAP
            && let Some((dropped, ..)) = log.edits.pop_front()
        {
            log.dropped_below = dropped + 1;
        }
        for (k, entry) in &mut self.by_key {
            if k.uri == uri && !entry.unalignable {
                translate_spans(&mut entry.spans, edit);
                entry.pending += 1;
            }
        }
        self.version += 1;
    }

    /// Note that a completion answer for `uri` was answered against
    /// the text at edit number `base` (E7.4): until a newer answer
    /// replaces it, a token answer's absorb keeps the log from `base`
    /// on, so the accepted item's `additionalTextEdits` can be carried
    /// to the current text. Nothing to note for a URI with no log.
    pub fn note_completion_base(&mut self, uri: &str, base: u64) {
        if let Some(log) = self.logs.get_mut(uri) {
            log.completion_floor = base;
        }
    }

    /// Note that the document's current text has just been sent to the
    /// server (`didOpen` / `didChange`): a response to a request sent
    /// from now on reflects every edit recorded so far. Nothing to note
    /// for a URI with no log.
    pub fn note_synced(&mut self, uri: &str) {
        if let Some(log) = self.logs.get_mut(uri) {
            log.synced_seq = log.next_seq;
        }
    }

    /// The edit number a request sent now is answered against: the
    /// log's `synced_seq`, or `0` for a URI with no log.
    #[must_use]
    pub fn synced_seq(&self, uri: &str) -> u64 {
        self.logs.get(uri).map_or(0, |l| l.synced_seq)
    }

    /// The number the next recorded edit for `uri` takes.
    #[must_use]
    pub fn next_seq(&self, uri: &str) -> u64 {
        self.logs.get(uri).map_or(0, |l| l.next_seq)
    }

    /// The edits recorded for `uri` that no held response reflects yet,
    /// oldest first. The unit rows read this; production reads spans.
    #[must_use]
    pub fn pending_edits(&self, uri: &str) -> Vec<DocumentEdit> {
        self.logs
            .get(uri)
            .map(|l| l.edits.iter().map(|(_, e, _)| *e).collect())
            .unwrap_or_default()
    }

    /// The edits `uri` has taken since its text was last sent to the
    /// server, oldest first, each with the text it inserted: what an
    /// incremental `didChange` ships (E6d.1). `None` when the URI has
    /// no log, or when an edit since the last sync has been forgotten
    /// (the cap, a clear, or a prune), in which case only the whole
    /// document can bring the server up to date. An empty vector means
    /// the server already holds the current text.
    #[must_use]
    pub fn unsynced_edits(&self, uri: &str) -> Option<Vec<(DocumentEdit, Arc<str>)>> {
        let log = self.logs.get(uri)?;
        if log.dropped_below > log.synced_seq {
            return None;
        }
        Some(
            log.edits
                .iter()
                .filter(|(seq, ..)| *seq >= log.synced_seq)
                .map(|(_, edit, inserted)| (*edit, Arc::clone(inserted)))
                .collect(),
        )
    }

    /// Carry a byte range of `uri`'s text as it stood at edit number
    /// `base` across every edit recorded since, to the coordinates of
    /// the current text (E7.4: a completion item's `additionalTextEdits`
    /// at accept time, answered for the text the server held when the
    /// request went out). `None` when the URI has no log, or when an
    /// edit since `base` has been forgotten (the cap, a clear, or a
    /// token absorb's prune under an older answer's base), in which
    /// case nothing can place the range and a caller applies nothing.
    /// A range's start snaps past a replacement that swallowed it and
    /// its end keeps only what stood before one, as tokens do; an
    /// insertion point at a pure insert moves past the inserted text.
    #[must_use]
    pub fn translate_range_since(
        &self,
        uri: &str,
        base: u64,
        start: u64,
        end: u64,
    ) -> Option<(u64, u64)> {
        let log = self.logs.get(uri)?;
        if log.dropped_below > base {
            return None;
        }
        let (mut start, mut end) = (start, end);
        for (seq, edit, _) in &log.edits {
            if *seq >= base {
                start = edit.translate_start(start);
                end = edit.translate_end(end);
            }
        }
        Some((start, end.max(start)))
    }

    /// Declare `uri`'s tokens stale without saying where the edit was.
    /// For a document with an edit log this is a no-op --- the log
    /// carries the edit itself and the tokens are shifted, not
    /// dropped; a URI no recorder tracks has no better information and
    /// paints nothing until the next response, as before E6b.
    pub fn mark_stale(&mut self, uri: impl Into<String>) {
        let uri = uri.into();
        if self.logs.contains_key(&uri) {
            return;
        }
        self.stale_uris.insert(uri);
        self.version += 1;
    }

    /// `true` iff `uri`'s held tokens are not exactly the server's
    /// answer for the current text: either declared stale with no
    /// edit log to shift by, or carried across edits the server has
    /// not answered yet. A consumer that can paint shifted tokens
    /// reads [`Self::positioned_tokens`] instead and never asks.
    #[must_use]
    pub fn is_stale(&self, uri: &str) -> bool {
        self.stale_uris.contains(uri)
            || self
                .entry_for_uri(uri)
                .is_some_and(|(_, e)| e.pending > 0 || e.unalignable)
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &SemanticTokenKey) -> Option<&SemanticTokensResponse> {
        self.by_key.get(key).map(|e| &e.response)
    }

    /// Look up the entry for `uri` regardless of which server keyed
    /// it, returning `(server, response)`. The store keys by
    /// `(server, uri)` but the semantic-render producer only knows the
    /// document; this is the URI-only view the diagnostics store
    /// (`DiagnosticStore`) offers natively.
    ///
    /// When more than one server has tokens for the same URI the
    /// **lowest server id** wins, chosen by numeric value so the
    /// result is deterministic across `HashMap` iteration order
    /// (server ids are assigned monotonically, so the lowest is the
    /// oldest / primary attachment). Blending styling from multiple
    /// servers on one buffer is a deliberately deferred open question
    /// — see `docs/semantic-frontend-protocol.md`.
    #[must_use]
    pub fn for_uri(&self, uri: &str) -> Option<(&str, &SemanticTokensResponse)> {
        self.entry_for_uri(uri).map(|(s, e)| (s, &e.response))
    }

    /// The tokens to paint for `uri`, in the document's current byte
    /// coordinates, from the same server [`Self::for_uri`] picks: the
    /// server's answer carried across every edit recorded since it was
    /// computed. `None` when no server has tokens for the URI, when the
    /// URI was declared stale with no log to shift by, or when the held
    /// response could not be aligned; then nothing is painted.
    #[must_use]
    pub fn positioned_tokens(&self, uri: &str) -> Option<(&str, &[PositionedToken])> {
        if self.stale_uris.contains(uri) {
            return None;
        }
        let (server, entry) = self.entry_for_uri(uri)?;
        (!entry.unalignable).then_some((server, entry.spans.as_slice()))
    }

    fn entry_for_uri(&self, uri: &str) -> Option<(&str, &Entry)> {
        self.by_key
            .iter()
            .filter(|(k, _)| k.uri == uri)
            .min_by_key(|(k, _)| server_sort_key(&k.server))
            .map(|(k, v)| (k.server.as_str(), v))
    }
}

/// Carry a sorted, disjoint token list across one edit in place.
fn translate_spans(spans: &mut Vec<PositionedToken>, edit: DocumentEdit) {
    spans.retain_mut(|t| match t.translated(edit) {
        Some(moved) => {
            *t = moved;
            true
        }
        None => false,
    });
}

/// Order key for picking a representative server: numeric if the id
/// parses (the normal case — ids are decimal `LspServerId::raw`),
/// else a max sentinel so unparsable ids sort last but the lookup
/// still yields *something* rather than nothing.
fn server_sort_key(server: &str) -> (u64, &str) {
    match server.parse::<u64>() {
        Ok(n) => (n, ""),
        Err(_) => (u64::MAX, server),
    }
}

/// Buffer-attached recorder that reports every edit a document takes
/// to the shared store's edit log (E6b.1).
///
/// Attached to the buffer, not to a window, for the reason
/// [`crate::overlay::BufferStyleSpanTranslator`] states: buffer views
/// receive `on_edit` on every mutation path --- the typed self-insert,
/// an intercept-skipping Lua write, undo and redo, and a remote CRDT
/// op imported from another peer (a GPU user's optimistic keystroke
/// among them) --- exactly once per edit and whether or not the buffer
/// is displayed anywhere. That broadcast is the one stage every edit
/// passes through, one level below the `run_rope_edit_and_broadcast`
/// E6.4 cut its undo group at (which remote ops bypass through
/// `run_remote_rope_stages`). The `buffer.after-edit` Lua hook that
/// marks the other LSP stores stale fires once per command with no
/// positions, so it could not feed this.
///
/// Not the CRDT's op log, though one exists in CRDT mode: a buffer no
/// semantic frontend has attached to is a v0.1 rope with no loro
/// document at all, and the grid flashes on those too; and loro's ops
/// index the text in its own units and would need converting back to
/// the bytes the tokens are shifted in. The `Edit` the buffer already
/// broadcasts carries the byte range and the inserted length in every
/// build, which is the whole of what shifting needs, so this log is
/// the one edit log the token store has and not a second one beside
/// the CRDT's.
///
/// The edit is logged under the buffer's file URI as it stands at the
/// edit, the same URI both merge sites derive from the path, so a
/// renamed buffer's later edits reach the new URI's log without the
/// recorder being told.
pub struct SemanticEditRecorder {
    store: SharedSemanticTokenStore,
}

impl SemanticEditRecorder {
    /// The `View::kind` this recorder reports, used to attach it at
    /// most once per buffer.
    pub const KIND: &'static str = "semantic-edit-recorder";

    /// A recorder feeding `store`.
    #[must_use]
    pub fn new(store: SharedSemanticTokenStore) -> Self {
        Self { store }
    }
}

impl crate::view::View for SemanticEditRecorder {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    fn on_edit(
        &mut self,
        buf: &crate::buffer::Buffer,
        edit: &crate::rope::Edit,
    ) -> Result<(), crate::buffer::BufferError> {
        let Some(path) = buf.file_path() else {
            return Ok(());
        };
        let uri = crate::lsp::path_to_file_uri(path);
        // The inserted bytes sit at `range.start` of the post-edit
        // rope; an incremental `didChange` ships them (E6d.1). A
        // buffer holds UTF-8 and an insert is a whole string, so the
        // lossy conversion is a formality.
        let inserted_len = usize::try_from(edit.inserted_len).unwrap_or(usize::MAX);
        let mut inserted = vec![0u8; inserted_len];
        if inserted_len > 0 {
            edit.new_rope.slice(
                edit.range.start,
                edit.range.start + edit.inserted_len,
                &mut inserted,
            );
        }
        let inserted = String::from_utf8_lossy(&inserted);
        self.store
            .lock()
            .expect("semantic token store mutex poisoned")
            .record_edit(
                &uri,
                DocumentEdit {
                    start: edit.range.start,
                    old_end: edit.range.end,
                    inserted_len: edit.inserted_len,
                },
                &inserted,
            );
        Ok(())
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedSemanticTokenStore = Arc<Mutex<SemanticTokenStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedSemanticTokenStore {
    Arc::new(Mutex::new(SemanticTokenStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const UTF16: crate::lsp::PositionEncoding = crate::lsp::PositionEncoding::Utf16;

    fn tok(line: u32, start: u32, length: u32) -> SemanticToken {
        SemanticToken {
            line,
            start,
            length,
            token_type: 0,
            token_modifiers: 0,
        }
    }

    fn resp(tokens: Vec<SemanticToken>) -> SemanticTokensResponse {
        SemanticTokensResponse {
            tokens,
            result_id: None,
            raw: Vec::new(),
        }
    }

    fn edit(start: u64, old_end: u64, inserted_len: u64) -> DocumentEdit {
        DocumentEdit {
            start,
            old_end,
            inserted_len,
        }
    }

    fn ranges(store: &SemanticTokenStore, uri: &str) -> Option<Vec<(u64, u64)>> {
        store
            .positioned_tokens(uri)
            .map(|(_, t)| t.iter().map(|t| (t.start, t.end)).collect())
    }

    /// E6b.1 --- the log fills per recorded edit, in order, and a
    /// response computed against the text as of a given edit number
    /// forgets the edits before it and keeps the ones after.
    #[test]
    fn e6b_1_log_fills_per_edit_and_resets_on_a_fresh_response() {
        let mut s = SemanticTokenStore::new();
        let uri = "file:///a.rs";
        s.open_log(uri);
        assert_eq!(s.next_seq(uri), 0);
        s.record_edit(uri, edit(0, 0, 1), "x");
        s.record_edit(uri, edit(4, 6, 0), "");
        s.record_edit(uri, edit(2, 2, 0), ""); // a no-op broadcast is not an edit
        assert_eq!(s.next_seq(uri), 2);
        assert_eq!(
            s.pending_edits(uri),
            vec![edit(0, 0, 1), edit(4, 6, 0)],
            "every edit is kept in order"
        );
        // The text was sent to the server after the first edit.
        // (Numbering: edit 0 is in the server's copy, edit 1 is not.)
        // A response for that copy keeps only edit 1 pending.
        let key = SemanticTokenKey::new("1", uri);
        assert!(s.set(
            key.clone(),
            resp(vec![tok(0, 1, 3)]),
            "xabc def\n",
            UTF16,
            1
        ));
        assert_eq!(s.pending_edits(uri), vec![edit(4, 6, 0)]);
        assert!(s.is_stale(uri), "one edit the server has not seen");
        // A response current as of now leaves nothing pending.
        s.note_synced(uri);
        assert_eq!(s.synced_seq(uri), 2);
        assert!(s.set(key, resp(vec![tok(0, 1, 3)]), "xabc f\n", UTF16, 2));
        assert!(s.pending_edits(uri).is_empty(), "reset on a fresh response");
        assert!(!s.is_stale(uri));
    }

    /// E6b.2 --- the shift rule at the store: tokens before an edit are
    /// untouched, tokens after it move by the delta, a token cut by a
    /// deletion keeps what stood outside it, and a pure insert at a
    /// token's end extends it (the GPU's own rule).
    #[test]
    fn e6b_2_recorded_edits_shift_held_tokens() {
        let mut s = SemanticTokenStore::new();
        let uri = "file:///a.rs";
        s.open_log(uri);
        let key = SemanticTokenKey::new("1", uri);
        // "fn main() { foo(bar); }" --- tokens main [3,7), foo [12,15), bar [16,19).
        let text = "fn main() { foo(bar); }\n";
        s.set_current(
            key,
            resp(vec![tok(0, 3, 4), tok(0, 12, 3), tok(0, 16, 3)]),
            text,
            UTF16,
        );
        assert_eq!(ranges(&s, uri), Some(vec![(3, 7), (12, 15), (16, 19)]));

        // Insert two bytes at 0: everything shifts.
        s.record_edit(uri, edit(0, 0, 2), "xx");
        assert_eq!(ranges(&s, uri), Some(vec![(5, 9), (14, 17), (18, 21)]));
        assert!(s.is_stale(uri), "shifted is not fresh");

        // Type one byte at the end of `main` (now [5,9)): it extends;
        // the later tokens move.
        s.record_edit(uri, edit(9, 9, 1), "x");
        assert_eq!(ranges(&s, uri), Some(vec![(5, 10), (15, 18), (19, 22)]));

        // Type inside `foo` (now [15,18)) at 16: the token extends over
        // the inserted byte.
        s.record_edit(uri, edit(16, 16, 1), "x");
        assert_eq!(ranges(&s, uri), Some(vec![(5, 10), (15, 19), (20, 23)]));

        // Delete the last byte of `bar` (now [20,23)): it shrinks.
        s.record_edit(uri, edit(22, 23, 0), "");
        assert_eq!(ranges(&s, uri), Some(vec![(5, 10), (15, 19), (20, 22)]));

        // Delete across the end of `foo` and the start of `bar`: each
        // keeps what stood outside the deletion, and they stay disjoint.
        s.record_edit(uri, edit(17, 21, 0), "");
        assert_eq!(ranges(&s, uri), Some(vec![(5, 10), (15, 17), (17, 18)]));

        // Delete a whole token: it is dropped.
        s.record_edit(uri, edit(5, 10, 0), "");
        assert_eq!(ranges(&s, uri), Some(vec![(10, 12), (12, 13)]));

        // A replace over the boundary claims none of the new text for
        // either neighbor.
        s.record_edit(uri, edit(11, 13, 3), "xxx");
        assert_eq!(ranges(&s, uri), Some(vec![(10, 11)]));
    }

    /// Edits recorded between a request and its response are applied
    /// to the response; a response older than the entry held is
    /// refused; a response whose base the log no longer covers is held
    /// unalignable and paints nothing.
    #[test]
    fn e6b_1_a_response_in_flight_is_shifted_by_the_edits_it_missed() {
        let mut s = SemanticTokenStore::new();
        let uri = "file:///a.rs";
        s.open_log(uri);
        let key = SemanticTokenKey::new("1", uri);
        // didChange sent with the text "ab\n"; the request goes out.
        s.note_synced(uri);
        let base = s.synced_seq(uri);
        // The user types "xy" at 0 while the server thinks.
        s.record_edit(uri, edit(0, 0, 1), "x");
        s.record_edit(uri, edit(1, 1, 1), "x");
        // The answer for "ab\n" lands: `ab` is [0,2) there, [2,4) now.
        assert!(s.set(key.clone(), resp(vec![tok(0, 0, 2)]), "ab\n", UTF16, base));
        assert_eq!(ranges(&s, uri), Some(vec![(2, 4)]));
        assert!(s.is_stale(uri));

        // The next didChange carries "xyab\n"; its answer is fresh.
        s.note_synced(uri);
        assert!(s.set(
            key.clone(),
            resp(vec![tok(0, 0, 4)]),
            "xyab\n",
            UTF16,
            s.synced_seq(uri)
        ));
        assert_eq!(ranges(&s, uri), Some(vec![(0, 4)]));
        assert!(!s.is_stale(uri));

        // A straggler for the older text is refused.
        assert!(!s.set(key.clone(), resp(vec![tok(0, 0, 2)]), "ab\n", UTF16, base));
        assert_eq!(ranges(&s, uri), Some(vec![(0, 4)]));

        // Clearing forgets the log's edits; a response for text before
        // the clear cannot be aligned and paints nothing.
        s.record_edit(uri, edit(0, 0, 1), "x");
        let stale_base = s.synced_seq(uri);
        s.clear(&key);
        assert!(s.set(key, resp(vec![tok(0, 0, 4)]), "xyab\n", UTF16, stale_base));
        assert_eq!(ranges(&s, uri), None, "unalignable paints nothing");
        assert!(s.is_stale(uri));
    }

    /// The census's version-only history `Edit` (an undo or redo in
    /// CRDT mode whose text delta is empty: `start == old_end` and
    /// nothing inserted) leaves the store byte-identical --- no edit
    /// number consumed, no span moved, no version bump --- so the
    /// recorder is inert on it, like the two translators before it.
    /// Deleting the early return in `record_edit` fires this: the
    /// version bumps and an edit is logged.
    #[test]
    fn a_version_only_history_edit_leaves_the_store_untouched() {
        let mut s = SemanticTokenStore::new();
        let uri = "file:///a.rs";
        s.open_log(uri);
        s.set_current(
            SemanticTokenKey::new("1", uri),
            resp(vec![tok(0, 3, 4)]),
            "fn main()\n",
            UTF16,
        );
        let before = (
            s.version(),
            s.next_seq(uri),
            ranges(&s, uri),
            s.pending_edits(uri),
        );
        s.record_edit(uri, edit(5, 5, 0), ""); // strictly inside the token
        s.record_edit(uri, edit(9, 9, 0), ""); // at the buffer end, where undo puts it
        let after = (
            s.version(),
            s.next_seq(uri),
            ranges(&s, uri),
            s.pending_edits(uri),
        );
        assert_eq!(after, before, "a 0→0 edit is not an edit to this store");
        assert!(!s.is_stale(uri));
    }

    /// E6b.4 --- a range answer replaces the held tokens on its lines
    /// and keeps the rest; the whole-document answer after it is not
    /// refused; an entry made by a range answer leaves no `result_id`
    /// for a delta to chain on.
    #[test]
    fn e6b_4_a_range_answer_replaces_its_lines_and_keeps_the_rest() {
        let mut s = SemanticTokenStore::new();
        let uri = "file:///a.rs";
        s.open_log(uri);
        let key = SemanticTokenKey::new("1", uri);
        // Three lines, one word each: aa / bb / cc.
        let text = "aa\nbb\ncc\n";
        s.set_current(
            key.clone(),
            resp(vec![tok(0, 0, 2), tok(1, 0, 2), tok(2, 0, 2)]),
            text,
            UTF16,
        );
        assert_eq!(ranges(&s, uri), Some(vec![(0, 2), (3, 5), (6, 8)]));
        // The middle line changes to `b` and the server answers the
        // range for line 1 alone: [3,4) now, the others untouched.
        s.record_edit(uri, edit(4, 5, 0), "");
        s.note_synced(uri);
        let base = s.synced_seq(uri);
        assert!(s.set_range(
            key.clone(),
            &resp(vec![tok(1, 0, 1)]),
            "aa\nb\ncc\n",
            UTF16,
            base,
            ((1, 0), (2, 0))
        ));
        assert_eq!(ranges(&s, uri), Some(vec![(0, 2), (3, 4), (5, 7)]));
        assert!(!s.is_stale(uri));
        // The whole-document answer for the same base lands after it.
        assert!(s.set(
            key.clone(),
            resp(vec![tok(0, 0, 2), tok(1, 0, 1), tok(2, 0, 2)]),
            "aa\nb\ncc\n",
            UTF16,
            base
        ));
        assert_eq!(ranges(&s, uri), Some(vec![(0, 2), (3, 4), (5, 7)]));
        // A range answer for a URI with no entry yet creates one with
        // no delta cursor.
        let other = SemanticTokenKey::new("1", "file:///b.rs");
        assert!(s.set_range(
            other.clone(),
            &resp(vec![tok(0, 0, 2)]),
            "aa\n",
            UTF16,
            0,
            ((0, 0), (1, 0))
        ));
        assert_eq!(ranges(&s, "file:///b.rs"), Some(vec![(0, 2)]));
        assert!(s.get(&other).expect("entry").result_id.is_none());
    }

    /// A URI with an edit log ignores `mark_stale`: the log is the
    /// authority, and a declared-stale flag would drop what the log
    /// can shift. A URI without one keeps the pre-E6b drop.
    #[test]
    fn mark_stale_is_a_no_op_for_a_logged_uri() {
        let mut s = SemanticTokenStore::new();
        let logged = "file:///logged.rs";
        let bare = "file:///bare.rs";
        s.open_log(logged);
        s.set_current(
            SemanticTokenKey::new("1", logged),
            resp(vec![tok(0, 0, 2)]),
            "ab\n",
            UTF16,
        );
        s.set_current(
            SemanticTokenKey::new("1", bare),
            resp(vec![tok(0, 0, 2)]),
            "ab\n",
            UTF16,
        );
        s.mark_stale(logged);
        s.mark_stale(bare);
        assert!(!s.is_stale(logged));
        assert_eq!(ranges(&s, logged), Some(vec![(0, 2)]));
        assert!(s.is_stale(bare));
        assert_eq!(ranges(&s, bare), None, "no log, no shift: nothing paints");
    }

    /// Resolution against the anchor text honors the negotiated
    /// encoding: a UTF-16 column past a non-BMP character lands on the
    /// right byte.
    #[test]
    fn positioned_tokens_resolve_columns_in_the_server_encoding() {
        let mut s = SemanticTokenStore::new();
        let uri = "file:///u.rs";
        // "𝄞ab": the clef is 4 bytes and 2 UTF-16 units; `ab` starts at
        // unit 2, byte 4.
        s.set_current(
            SemanticTokenKey::new("1", uri),
            resp(vec![tok(0, 2, 2)]),
            "𝄞ab\n",
            UTF16,
        );
        assert_eq!(ranges(&s, uri), Some(vec![(4, 6)]));
        s.set_current(
            SemanticTokenKey::new("1", uri),
            resp(vec![tok(0, 4, 2)]),
            "𝄞ab\n",
            crate::lsp::PositionEncoding::Utf8,
        );
        assert_eq!(ranges(&s, uri), Some(vec![(4, 6)]));
    }

    #[test]
    fn decodes_relative_encoding_across_lines() {
        // Three tokens:
        //  - line 0, char 0, len 3, type 1, mods 0
        //  - same line, +5 chars  → char 5, len 2, type 2, mods 0
        //  - +2 lines, char 4 (absolute, deltaLine!=0), len 6, type 0,
        //    mods 0b101
        let v = json!({
            "resultId": "1",
            "data": [
                0, 0, 3, 1, 0,
                0, 5, 2, 2, 0,
                2, 4, 6, 0, 5
            ]
        });
        let r = SemanticTokensResponse::from_lsp_value(&v);
        assert_eq!(r.result_id.as_deref(), Some("1"));
        assert_eq!(r.tokens.len(), 3);
        assert_eq!(
            r.tokens[0],
            SemanticToken {
                line: 0,
                start: 0,
                length: 3,
                token_type: 1,
                token_modifiers: 0
            }
        );
        assert_eq!(
            r.tokens[1],
            SemanticToken {
                line: 0,
                start: 5,
                length: 2,
                token_type: 2,
                token_modifiers: 0
            }
        );
        assert_eq!(
            r.tokens[2],
            SemanticToken {
                line: 2,
                start: 4,
                length: 6,
                token_type: 0,
                token_modifiers: 5
            }
        );
    }

    #[test]
    fn trailing_partial_group_is_ignored() {
        let v = json!({ "data": [0, 0, 3, 1, 0, 9, 9] });
        let r = SemanticTokensResponse::from_lsp_value(&v);
        assert_eq!(r.tokens.len(), 1);
    }

    #[test]
    fn null_response_is_empty() {
        let r = SemanticTokensResponse::from_lsp_value(&Value::Null);
        assert!(r.is_empty());
        assert!(r.result_id.is_none());
    }

    #[test]
    fn legend_parses_and_resolves() {
        let caps = json!({
            "semanticTokensProvider": {
                "legend": {
                    "tokenTypes": ["namespace", "type", "function"],
                    "tokenModifiers": ["declaration", "readonly", "static"]
                },
                "full": true
            }
        });
        let legend = SemanticTokensLegend::from_capabilities(&caps).unwrap();
        assert_eq!(legend.type_name(2), Some("function"));
        assert_eq!(legend.type_name(99), None);
        // bits 0b101 = declaration + static
        assert_eq!(legend.modifier_names(0b101), vec!["declaration", "static"]);
        assert_eq!(legend.modifier_names(0), Vec::<&str>::new());
    }

    #[test]
    fn no_provider_yields_no_legend() {
        assert!(
            SemanticTokensLegend::from_capabilities(&json!({ "hoverProvider": true })).is_none()
        );
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = SemanticTokenStore::new();
        let key = SemanticTokenKey::new("1", "file:///a");
        s.set_current(
            key.clone(),
            SemanticTokensResponse {
                tokens: vec![SemanticToken {
                    line: 0,
                    start: 0,
                    length: 1,
                    token_type: 0,
                    token_modifiers: 0,
                }],
                result_id: None,
                raw: vec![0, 0, 1, 0, 0],
            },
            "x\n",
            UTF16,
        );
        assert_eq!(s.get(&key).unwrap().tokens.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }

    #[test]
    fn stale_flag_clears_on_set_and_final_clear() {
        let mut s = SemanticTokenStore::new();
        let key = SemanticTokenKey::new("1", "file:///a");
        let response = SemanticTokensResponse {
            tokens: vec![SemanticToken {
                line: 0,
                start: 0,
                length: 1,
                token_type: 0,
                token_modifiers: 0,
            }],
            result_id: None,
            raw: vec![0, 0, 1, 0, 0],
        };

        s.set_current(key.clone(), response.clone(), "x\n", UTF16);
        s.mark_stale("file:///a");
        assert!(s.is_stale("file:///a"));

        s.set_current(key.clone(), response, "x\n", UTF16);
        assert!(
            !s.is_stale("file:///a"),
            "fresh semantic tokens clear stale flag"
        );

        s.mark_stale("file:///a");
        s.clear(&key);
        assert!(
            !s.is_stale("file:///a"),
            "clearing final token entry clears stale flag"
        );
    }

    #[test]
    fn for_uri_filters_by_uri_and_picks_lowest_server() {
        let mk = |tok_type: u32| SemanticTokensResponse {
            tokens: vec![SemanticToken {
                line: 0,
                start: 0,
                length: 1,
                token_type: tok_type,
                token_modifiers: 0,
            }],
            result_id: None,
            raw: vec![0, 0, 1, tok_type, 0],
        };
        let mut s = SemanticTokenStore::new();
        // Same URI under two servers; ids deliberately inserted so
        // numeric (not lexicographic: "10" < "9" as strings) ordering
        // is what makes the test meaningful.
        s.set_current(
            SemanticTokenKey::new("10", "file:///a"),
            mk(10),
            "x\n",
            UTF16,
        );
        s.set_current(SemanticTokenKey::new("9", "file:///a"), mk(9), "x\n", UTF16);
        s.set_current(SemanticTokenKey::new("2", "file:///b"), mk(2), "x\n", UTF16);

        let (server, resp) = s.for_uri("file:///a").expect("entry for /a");
        assert_eq!(server, "9", "lowest *numeric* server id wins");
        assert_eq!(resp.tokens[0].token_type, 9);

        let (server_b, _) = s.for_uri("file:///b").expect("entry for /b");
        assert_eq!(server_b, "2");

        assert!(s.for_uri("file:///nope").is_none());
    }

    #[test]
    fn from_lsp_value_retains_raw() {
        let v = json!({ "resultId": "r1", "data": [0, 0, 4, 1, 1, 0, 5, 3, 2, 0] });
        let r = SemanticTokensResponse::from_lsp_value(&v);
        assert_eq!(r.raw, vec![0, 0, 4, 1, 1, 0, 5, 3, 2, 0]);
        assert_eq!(r.tokens.len(), 2);
        assert_eq!(r.result_id.as_deref(), Some("r1"));
    }

    #[test]
    fn apply_delta_splices_previous_raw() {
        // prev: two tokens. Delta replaces the 2nd group (indices
        // 5..10) with a different 5-int group and bumps resultId.
        let prev = [0u32, 0, 4, 1, 1, 0, 5, 3, 2, 0];
        let delta = json!({
            "resultId": "r2",
            "edits": [{ "start": 5, "deleteCount": 5, "data": [1, 2, 6, 0, 0] }]
        });
        let r = SemanticTokensResponse::apply_delta(&prev, &delta);
        assert_eq!(r.result_id.as_deref(), Some("r2"));
        assert_eq!(r.raw, vec![0, 0, 4, 1, 1, 1, 2, 6, 0, 0]);
        // 2nd token: deltaLine 1 ⇒ line 1, start absolute 2, len 6.
        assert_eq!(
            r.tokens[1],
            SemanticToken {
                line: 1,
                start: 2,
                length: 6,
                token_type: 0,
                token_modifiers: 0
            }
        );
    }

    #[test]
    fn apply_delta_multi_edit_descending_safe() {
        // Two edits given in ascending order; applying ascending
        // would invalidate the second's indices. Delete first group,
        // insert a group after the (original) second.
        let prev = [0u32, 0, 1, 0, 0, 0, 1, 1, 0, 0];
        let delta = json!({ "edits": [
            { "start": 0, "deleteCount": 5, "data": [] },
            { "start": 10, "deleteCount": 0, "data": [2, 0, 3, 0, 0] }
        ]});
        let r = SemanticTokensResponse::apply_delta(&prev, &delta);
        assert_eq!(r.raw, vec![0, 1, 1, 0, 0, 2, 0, 3, 0, 0]);
    }

    #[test]
    fn apply_delta_accepts_full_fallback() {
        // Server answered a delta request with a full result.
        let r = SemanticTokensResponse::apply_delta(
            &[9, 9, 9, 9, 9],
            &json!({ "resultId": "f", "data": [0, 0, 2, 1, 0] }),
        );
        assert_eq!(r.raw, vec![0, 0, 2, 1, 0]);
        assert_eq!(r.tokens.len(), 1);
        assert_eq!(r.result_id.as_deref(), Some("f"));
    }
}
