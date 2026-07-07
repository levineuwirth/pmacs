// minibuffer.rs --- Minibuffer state, completion, and history (T M2.7).

//! The minibuffer is a regular [`Buffer`] with a regular [`TextView`].
//! Universality (spec §3) is the load-bearing constraint: prompts,
//! M-x, file pickers all reuse rope and view machinery rather than a
//! special path.
//!
//! # Lifecycle
//!
//! * The editor builds a single [`Minibuffer`] at startup; it lives
//!   inside [`crate::editor_core::EditorCore`].
//! * A *session* opens via [`Minibuffer::begin`]: the caller supplies
//!   a prompt, a [`CompletionSource`], history bucket, and Lua
//!   callbacks for accept/cancel.
//! * The dispatcher routes keys to minibuffer-specific handlers
//!   ([`crate::editor::EditorState::dispatch_key`]) which mutate the
//!   minibuffer's buffer and recompute candidates.
//! * Accept invokes `on_accept(contents)` with the chosen string;
//!   cancel invokes `on_cancel()` (if provided). Either way, the
//!   session is cleared and the buffer's contents are blanked.
//!
//! # History
//!
//! Each session names a history *bucket* (e.g. `"command"`,
//! `"file"`). Accepted entries are appended to a per-bucket file
//! under `$XDG_STATE_HOME/pmacs/history/<bucket>` (or
//! `~/.local/state/pmacs/history/<bucket>`). Up/down navigates
//! in-memory entries; the typed-but-not-accepted prefix is preserved
//! when the user navigates back to the front of history.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use mlua::Function;

use crate::buffer::{Buffer, BufferId, EditOp};
use crate::buffer_registry::BufferRegistry;
use crate::command::CommandRegistry;
use crate::key::Chord;
use crate::rope::{Position, Range};
use crate::text_view::TextView;
use crate::view::View;

/// Hard cap on per-bucket history length. Bounded so the in-memory
/// deque cannot grow unboundedly across a long session.
pub const HISTORY_MAX: usize = 500;

/// Hard cap on how many candidates we display / iterate against.
/// Most prompts have far fewer; the cap protects against pathological
/// custom sources that return millions of strings.
pub const CANDIDATE_LIMIT: usize = 1024;

/// Canonical name of the minibuffer's backing buffer.
pub const MINIBUFFER_NAME: &str = "*minibuffer*";

// ---------------------------------------------------------------------------
// Minibuffer
// ---------------------------------------------------------------------------

/// Minibuffer state: a real buffer + view + per-bucket history.
pub struct Minibuffer {
    /// Backing rope buffer. Held inline (not in the buffer registry)
    /// to mirror the ownership shape of the main buffer in
    /// [`crate::editor_core::EditorCore`].
    pub buffer: Buffer,
    /// Plain-text view of the minibuffer's contents.
    pub text_view: TextView,
    /// Byte position of the cursor within the minibuffer.
    pub cursor: Position,
    /// The active prompt session, if any.
    pub session: Option<MinibufferSession>,
    /// In-memory history per bucket.
    pub history: HashMap<String, History>,
    /// Directory under which history files are persisted. `None`
    /// means "do not persist" (the default for the lib's own test
    /// suite; the editor binary configures this at startup).
    pub history_dir: Option<PathBuf>,
}

impl Minibuffer {
    /// A fresh minibuffer with an empty buffer, no session, and no
    /// history-on-disk root.
    #[must_use]
    pub fn new() -> Self {
        let buffer = Buffer::new(BufferId::next(), MINIBUFFER_NAME);
        let text_view = TextView::new(&buffer);
        Self {
            buffer,
            text_view,
            cursor: 0,
            session: None,
            history: HashMap::new(),
            history_dir: None,
        }
    }

    /// True iff a prompt session is currently active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    /// Open a prompt session. Replaces any existing session, replacing
    /// the buffer contents with `initial` and seeding history from
    /// disk if a history bucket is named.
    pub fn begin(&mut self, mut session: MinibufferSession) {
        self.replace_contents(&session.initial.clone());
        self.cursor = self.buffer.len();
        // Lazy-load history once per bucket.
        if !session.history_bucket.is_empty() && !self.history.contains_key(&session.history_bucket)
        {
            let entries = self
                .history_dir
                .as_deref()
                .and_then(|dir| load_history_file(dir, &session.history_bucket).ok())
                .unwrap_or_default();
            self.history.insert(
                session.history_bucket.clone(),
                History::with_entries(entries),
            );
        }
        session.history_index = None;
        session.typed_before_history_nav = None;
        self.session = Some(session);
    }

    /// Returns the minibuffer's contents as an owned String.
    #[must_use]
    pub fn contents(&self) -> String {
        let len = self.buffer.len();
        let mut out = vec![0u8; len as usize];
        if len > 0 {
            self.buffer.snapshot_rope().slice(0, len, &mut out);
        }
        String::from_utf8(out).unwrap_or_default()
    }

    /// Replace the buffer's contents with `s`. Notifies the text view
    /// of the resulting edit so cursor coordinates stay valid.
    pub fn replace_contents(&mut self, s: &str) {
        let len = self.buffer.len();
        if len > 0 {
            let edit = self
                .buffer
                .apply_edit(EditOp::Delete {
                    range: Range::new(0, len),
                })
                .expect("delete in minibuffer");
            let _ = self.text_view.on_edit(&self.buffer, &edit);
        }
        if !s.is_empty() {
            let edit = self
                .buffer
                .apply_edit(EditOp::Insert {
                    pos: 0,
                    bytes: s.as_bytes(),
                })
                .expect("insert in minibuffer");
            let _ = self.text_view.on_edit(&self.buffer, &edit);
        }
        self.cursor = self.buffer.len();
    }

    fn apply(&mut self, op: EditOp<'_>) {
        let Ok(edit) = self.buffer.apply_edit(op) else {
            return;
        };
        let _ = self.text_view.on_edit(&self.buffer, &edit);
    }

    /// Insert a single character at the cursor.
    pub fn insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let bytes = s.as_bytes();
        let pos = self.cursor;
        self.apply(EditOp::Insert { pos, bytes });
        self.cursor += bytes.len() as u64;
    }

    /// Delete the codepoint immediately before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = prev_codepoint(&self.buffer, self.cursor);
        let range = Range::new(prev, self.cursor);
        self.apply(EditOp::Delete { range });
        self.cursor = prev;
    }

    /// Delete the codepoint at the cursor (forward delete).
    pub fn delete_forward(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = next_codepoint(&self.buffer, self.cursor);
        let range = Range::new(self.cursor, next);
        self.apply(EditOp::Delete { range });
    }

    /// Move the cursor one codepoint left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = prev_codepoint(&self.buffer, self.cursor);
        }
    }

    /// Move the cursor one codepoint right.
    pub fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor = next_codepoint(&self.buffer, self.cursor);
        }
    }

    /// Move the cursor to the beginning of the buffer.
    pub fn move_line_start(&mut self) {
        self.cursor = 0;
    }

    /// Move the cursor to the end of the buffer.
    pub fn move_line_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    /// Cycle the selected candidate forward by `delta` (negative
    /// allowed). No-op when no candidates exist.
    pub fn scroll_candidate(&mut self, delta: i32) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        if s.candidates.is_empty() {
            s.selected = None;
            return;
        }
        let len = i32::try_from(s.candidates.len()).unwrap_or(i32::MAX);
        let cur = s
            .selected
            .map_or(0, |i| i32::try_from(i).unwrap_or(i32::MAX));
        let mut next = (cur + delta) % len;
        if next < 0 {
            next += len;
        }
        s.selected = Some(usize::try_from(next).unwrap_or(0));
    }

    /// Whether a completion dropdown is currently showing (the active
    /// session has at least one candidate). Drives whether the Up/Down
    /// arrows navigate the dropdown or step through command history.
    #[must_use]
    pub fn has_candidates(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| !s.candidates.is_empty())
    }

    /// Replace the buffer contents with the currently-selected
    /// candidate, leaving the session active so the user can continue
    /// editing or accept. No-op when nothing is selected.
    pub fn complete(&mut self) {
        let pick = self
            .session
            .as_ref()
            .and_then(|s| s.selected.and_then(|i| s.candidates.get(i).cloned()));
        let Some(pick) = pick else { return };
        self.replace_contents(&pick);
    }

    /// Step backwards through history, replacing the buffer contents.
    /// First call stashes the in-progress text so [`Self::history_next`]
    /// can return to it.
    pub fn history_prev(&mut self) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let Some(history) = self.history.get(&s.history_bucket) else {
            return;
        };
        if history.entries.is_empty() {
            return;
        }
        let new_idx = match s.history_index {
            None => {
                s.typed_before_history_nav = Some(self_contents(&self.buffer));
                history.entries.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        s.history_index = Some(new_idx);
        let pick = history.entries[new_idx].clone();
        self.replace_contents(&pick);
    }

    /// Step forwards through history. At the front, restores the
    /// originally-typed text.
    pub fn history_next(&mut self) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let Some(history) = self.history.get(&s.history_bucket) else {
            return;
        };
        let Some(idx) = s.history_index else {
            return;
        };
        if idx + 1 >= history.entries.len() {
            // Past the end: restore typed input (if any).
            s.history_index = None;
            let stash = s.typed_before_history_nav.take().unwrap_or_default();
            self.replace_contents(&stash);
            return;
        }
        let new_idx = idx + 1;
        s.history_index = Some(new_idx);
        let pick = history.entries[new_idx].clone();
        self.replace_contents(&pick);
    }

    /// Commit the current contents: pushes onto the bucket history,
    /// persists if a history dir is configured, clears the session,
    /// and returns the on-accept callback paired with the resolved
    /// value.
    ///
    /// **Resolution.** If the session has a non-empty completion
    /// source and a selected candidate, the selected candidate is
    /// returned --- the typed text is treated as a search query.
    /// Sources of [`CompletionSource::None`], or sessions with no
    /// matches, fall through to the literal typed contents (so
    /// free-form prompts like "search: " behave naturally).
    ///
    /// The caller is expected to invoke the callback (firing user
    /// code from inside the minibuffer would re-enter the registry).
    pub fn accept(&mut self) -> Option<(Function, String)> {
        let session = self.session.take()?;
        let typed = self.contents();
        let resolved = resolve_accepted_value(&session, &typed);
        if !session.history_bucket.is_empty() && !resolved.is_empty() {
            let history = self
                .history
                .entry(session.history_bucket.clone())
                .or_default();
            history.push(resolved.clone());
            if let Some(dir) = self.history_dir.as_deref() {
                let _ = append_history_file(dir, &session.history_bucket, &resolved);
            }
        }
        self.replace_contents("");
        Some((session.on_accept, resolved))
    }

    /// Discard the session, returning the on-cancel callback (if any).
    pub fn cancel(&mut self) -> Option<Function> {
        let session = self.session.take()?;
        self.replace_contents("");
        session.on_cancel
    }

    /// Recompute the candidate list against the current buffer
    /// contents. Resets the selected index to the top match.
    pub fn recompute_candidates(
        &mut self,
        commands: &CommandRegistry,
        registry: &BufferRegistry,
    ) -> mlua::Result<()> {
        let Some(s) = self.session.as_mut() else {
            return Ok(());
        };
        let needle = self_contents(&self.buffer);
        let pool = collect_pool(&s.source, commands, registry)?;
        let candidates = filter_and_sort(&needle, &pool);
        s.candidates = candidates;
        s.selected = if s.candidates.is_empty() {
            None
        } else {
            Some(0)
        };
        Ok(())
    }
}

impl Default for Minibuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// One prompt session.
pub struct MinibufferSession {
    /// Display prompt, e.g. `"M-x "` or `"Find file: "`.
    pub prompt: String,
    /// Initial buffer contents (often empty).
    pub initial: String,
    /// History bucket name. Empty string means "no history".
    pub history_bucket: String,
    /// Where candidates come from.
    pub source: CompletionSource,
    /// Lua callback invoked with the accepted contents.
    pub on_accept: Function,
    /// Optional Lua callback invoked on cancel (no args).
    pub on_cancel: Option<Function>,
    /// Currently-rendered candidate list (recomputed on input change).
    pub candidates: Vec<String>,
    /// Index into `candidates` of the selected entry, if any.
    pub selected: Option<usize>,
    /// Position in `history.entries`. `None` = "at front, editing
    /// fresh input".
    pub history_index: Option<usize>,
    /// Stash of typed input when entering history navigation, so
    /// stepping forward to the front restores it.
    pub typed_before_history_nav: Option<String>,
}

// ---------------------------------------------------------------------------
// Hardcoded key handler
// ---------------------------------------------------------------------------

/// Decoded action for a chord delivered to an active minibuffer
/// session. The dispatcher translates the chord, then mutates the
/// minibuffer accordingly. This keeps the key→action mapping in one
/// declarative place; new bindings (e.g. C-r for incremental search)
/// only need a new variant.
#[derive(Copy, Clone, Debug)]
pub enum MinibufferAction {
    /// Commit the current contents (RET / C-m).
    Accept,
    /// Discard the session (C-g).
    Cancel,
    /// Replace the buffer with the selected candidate (TAB / C-i).
    Complete,
    /// Step backward through history (C-p).
    HistoryPrev,
    /// Step forward through history (C-n).
    HistoryNext,
    /// Cycle the selected candidate forward (M-n).
    ScrollNext,
    /// Cycle the selected candidate backward (M-p).
    ScrollPrev,
    /// Up arrow: move to the previous completion candidate when a
    /// dropdown is showing, else step back through history. Resolved in
    /// the dispatcher, which has the session state `from_chord` lacks.
    PrevCandidateOrHistory,
    /// Down arrow: move to the next completion candidate when a dropdown
    /// is showing, else step forward through history.
    NextCandidateOrHistory,
    /// Backspace.
    Backspace,
    /// Forward delete (DEL / C-d).
    DeleteForward,
    /// Cursor left (Left / C-b).
    Left,
    /// Cursor right (Right / C-f).
    Right,
    /// Cursor to start (Home / C-a).
    LineStart,
    /// Cursor to end (End / C-e).
    LineEnd,
    /// Insert the bare codepoint (printable char with no Ctrl/Alt).
    SelfInsert(char),
    /// Unhandled --- swallow without complaint.
    Ignore,
}

impl MinibufferAction {
    /// Decode `chord` into a minibuffer action. The mapping is
    /// hardcoded; the rationale (R51 keeps the Lua surface curated)
    /// is that the minibuffer's bindings are not user-configurable in
    /// v0.1 --- changes happen by extending this enum and the
    /// matcher below.
    #[must_use]
    pub fn from_chord(chord: Chord) -> Self {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = chord.modifiers.contains(KeyModifiers::CONTROL);
        let alt = chord.modifiers.contains(KeyModifiers::ALT);

        if !ctrl && !alt {
            match chord.code {
                KeyCode::Enter => return Self::Accept,
                KeyCode::Esc => return Self::Cancel,
                KeyCode::Tab => return Self::Complete,
                KeyCode::Up => return Self::PrevCandidateOrHistory,
                KeyCode::Down => return Self::NextCandidateOrHistory,
                KeyCode::Left => return Self::Left,
                KeyCode::Right => return Self::Right,
                KeyCode::Home => return Self::LineStart,
                KeyCode::End => return Self::LineEnd,
                KeyCode::Backspace => return Self::Backspace,
                KeyCode::Delete => return Self::DeleteForward,
                KeyCode::Char(ch) => return Self::SelfInsert(ch),
                _ => return Self::Ignore,
            }
        }
        if ctrl && !alt {
            if let KeyCode::Char(c) = chord.code {
                return match c {
                    'g' => Self::Cancel,
                    'm' => Self::Accept,
                    'i' => Self::Complete,
                    'a' => Self::LineStart,
                    'e' => Self::LineEnd,
                    'b' => Self::Left,
                    'f' => Self::Right,
                    'p' => Self::HistoryPrev,
                    'n' => Self::HistoryNext,
                    'd' => Self::DeleteForward,
                    _ => Self::Ignore,
                };
            }
            return Self::Ignore;
        }
        if alt
            && !ctrl
            && let KeyCode::Char(c) = chord.code
        {
            return match c {
                'n' => Self::ScrollNext,
                'p' => Self::ScrollPrev,
                _ => Self::Ignore,
            };
        }
        Self::Ignore
    }
}

// ---------------------------------------------------------------------------
// CompletionSource
// ---------------------------------------------------------------------------

/// Where candidate strings come from.
pub enum CompletionSource {
    /// No candidates --- a free-form prompt.
    None,
    /// Every command name registered in the [`CommandRegistry`].
    Commands,
    /// Every buffer name in the [`BufferRegistry`].
    Buffers,
    /// Filenames in `root` (non-recursive).
    Files {
        /// Directory to list.
        root: PathBuf,
    },
    /// A Lua function returning a sequence (list-table) of strings.
    Custom(Function),
}

impl CompletionSource {
    /// Stable identifier for diagnostics.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Commands => "commands",
            Self::Buffers => "buffers",
            Self::Files { .. } => "files",
            Self::Custom(_) => "custom",
        }
    }
}

fn resolve_accepted_value(session: &MinibufferSession, typed: &str) -> String {
    if matches!(session.source, CompletionSource::None) {
        return typed.to_owned();
    }
    if let Some(idx) = session.selected
        && let Some(cand) = session.candidates.get(idx)
    {
        return cand.clone();
    }
    typed.to_owned()
}

fn collect_pool(
    source: &CompletionSource,
    commands: &CommandRegistry,
    registry: &BufferRegistry,
) -> mlua::Result<Vec<String>> {
    match source {
        CompletionSource::None => Ok(Vec::new()),
        CompletionSource::Commands => Ok(commands.names().to_vec()),
        CompletionSource::Buffers => Ok(registry
            .ids()
            .iter()
            .filter_map(|id| registry.get(*id).ok().map(|b| b.name().to_owned()))
            .collect()),
        CompletionSource::Files { root } => Ok(list_directory(root)),
        CompletionSource::Custom(f) => {
            let table: mlua::Table = f.call(())?;
            let mut out = Vec::new();
            for (i, item) in table.sequence_values::<String>().enumerate() {
                if i >= CANDIDATE_LIMIT {
                    break;
                }
                if let Ok(s) = item {
                    out.push(s);
                }
            }
            Ok(out)
        }
    }
}

fn list_directory(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            out.push(name.to_owned());
            if out.len() >= CANDIDATE_LIMIT {
                break;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Fuzzy scoring
// ---------------------------------------------------------------------------

/// Score `haystack` against `needle`. Returns `None` if `haystack`
/// does not contain `needle` as a case-insensitive subsequence.
///
/// Higher is better. Bonuses:
/// * Match at position 0 (`+10`).
/// * Match immediately after `.`, `-`, `_`, ` ` (`+5`).
/// * Consecutive matches (`+3` per chained character).
///
/// Penalties:
/// * Each gap byte between matches (`-1`).
#[must_use]
pub fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let n: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let h: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0i32;
    let mut i = 0usize;
    let mut prev_match: Option<usize> = None;
    for (j, &hc) in h.iter().enumerate() {
        if i >= n.len() {
            break;
        }
        if n[i] == hc {
            if j == 0 {
                score += 10;
            } else if let Some(prev_h) = h.get(j - 1)
                && matches!(*prev_h, '.' | '-' | '_' | ' ')
            {
                score += 5;
            }
            if let Some(p) = prev_match {
                if p + 1 == j {
                    score += 3;
                } else {
                    score -= i32::try_from(j - p - 1).unwrap_or(i32::MAX);
                }
            }
            prev_match = Some(j);
            i += 1;
        }
    }
    if i < n.len() { None } else { Some(score) }
}

fn filter_and_sort(needle: &str, pool: &[String]) -> Vec<String> {
    let mut scored: Vec<(i32, &str)> = pool
        .iter()
        .filter_map(|s| fuzzy_score(needle, s).map(|sc| (sc, s.as_str())))
        .take(CANDIDATE_LIMIT)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().map(|(_, s)| s.to_owned()).collect()
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// One bucket of history entries. Bounded to [`HISTORY_MAX`]; oldest
/// entries are evicted on push.
#[derive(Debug, Default, Clone)]
pub struct History {
    /// Entries, oldest at the front, newest at the back.
    pub entries: VecDeque<String>,
}

impl History {
    /// Build a history pre-seeded with `entries` (most-recent last).
    #[must_use]
    pub fn with_entries(entries: Vec<String>) -> Self {
        let mut h = Self::default();
        for e in entries {
            h.push(e);
        }
        h
    }

    /// Append `entry`. De-duplicates against the most-recent entry to
    /// avoid history thrash from repeated commands. Bounds the deque
    /// at [`HISTORY_MAX`].
    pub fn push(&mut self, entry: String) {
        if entry.is_empty() {
            return;
        }
        if self.entries.back().is_some_and(|e| *e == entry) {
            return;
        }
        self.entries.push_back(entry);
        while self.entries.len() > HISTORY_MAX {
            self.entries.pop_front();
        }
    }
}

/// Resolve the user's history directory.
///
/// Order: `$XDG_STATE_HOME/pmacs/history`, then
/// `$HOME/.local/state/pmacs/history`. Returns `None` if neither env
/// var is set.
#[must_use]
pub fn user_history_dir() -> Option<PathBuf> {
    resolve_history_dir(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Pure helper for [`user_history_dir`], factored out so tests can
/// inject paths directly without touching the process environment
/// (R55: `unsafe_code = "forbid"` rules out `env::set_var`).
#[must_use]
pub fn resolve_history_dir(
    xdg_state: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg_state {
        return Some(PathBuf::from(xdg).join("pmacs").join("history"));
    }
    home.map(|h| {
        PathBuf::from(h)
            .join(".local")
            .join("state")
            .join("pmacs")
            .join("history")
    })
}

fn history_path(dir: &Path, bucket: &str) -> PathBuf {
    dir.join(bucket)
}

/// Read all entries for a bucket. Missing file is a successful empty
/// read; unreadable file returns the IO error.
pub fn load_history_file(dir: &Path, bucket: &str) -> std::io::Result<Vec<String>> {
    let path = history_path(dir, bucket);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Append `entry` to the bucket file, creating parents as needed.
pub fn append_history_file(dir: &Path, bucket: &str, entry: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = history_path(dir, bucket);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{entry}")
}

// ---------------------------------------------------------------------------
// Codepoint helpers (mirror editor_core.rs)
// ---------------------------------------------------------------------------

fn self_contents(buf: &Buffer) -> String {
    let len = buf.len();
    let mut out = vec![0u8; len as usize];
    if len > 0 {
        buf.snapshot_rope().slice(0, len, &mut out);
    }
    String::from_utf8(out).unwrap_or_default()
}

fn prev_codepoint(buf: &Buffer, pos: Position) -> Position {
    if pos == 0 {
        return 0;
    }
    let rope = buf.snapshot_rope();
    let mut p = pos - 1;
    while p > 0 {
        let b = rope.byte_at(p).unwrap_or(0);
        if (b & 0xC0) != 0x80 {
            return p;
        }
        p -= 1;
    }
    0
}

fn next_codepoint(buf: &Buffer, pos: Position) -> Position {
    let len = buf.len();
    if pos >= len {
        return len;
    }
    let rope = buf.snapshot_rope();
    let lead = rope.byte_at(pos).unwrap_or(0);
    let advance = utf8_codepoint_len(lead);
    (pos + advance as u64).min(len)
}

fn utf8_codepoint_len(lead: u8) -> usize {
    if lead < 0xC0 {
        // ASCII (< 0x80) or stray continuation byte (< 0xC0):
        // advance by 1 in either case so a malformed leader doesn't
        // wedge the caller in an infinite loop.
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn dummy_accept(lua: &Lua) -> Function {
        lua.create_function(|_, _: String| Ok(())).unwrap()
    }

    fn open(mb: &mut Minibuffer, lua: &Lua, source: CompletionSource, bucket: &str) {
        mb.begin(MinibufferSession {
            prompt: "P: ".into(),
            initial: String::new(),
            history_bucket: bucket.into(),
            source,
            on_accept: dummy_accept(lua),
            on_cancel: None,
            candidates: Vec::new(),
            selected: None,
            history_index: None,
            typed_before_history_nav: None,
        });
    }

    #[test]
    fn buffer_is_real_with_canonical_name() {
        let mb = Minibuffer::new();
        assert_eq!(mb.buffer.name(), MINIBUFFER_NAME);
        assert_eq!(mb.buffer.len(), 0);
    }

    #[test]
    fn from_chord_escape_cancels() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let esc = Chord {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        };
        assert!(matches!(
            MinibufferAction::from_chord(esc),
            MinibufferAction::Cancel
        ));
    }

    #[test]
    fn from_chord_ctrl_g_cancels() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let cg = Chord {
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::CONTROL,
        };
        assert!(matches!(
            MinibufferAction::from_chord(cg),
            MinibufferAction::Cancel
        ));
    }

    #[test]
    fn from_chord_enter_accepts() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ret = Chord {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        };
        assert!(matches!(
            MinibufferAction::from_chord(ret),
            MinibufferAction::Accept
        ));
    }

    #[test]
    fn from_chord_char_self_inserts() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let a = Chord {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
        };
        match MinibufferAction::from_chord(a) {
            MinibufferAction::SelfInsert('a') => {}
            other => panic!("expected SelfInsert('a'), got {other:?}"),
        }
    }

    #[test]
    fn from_chord_arrows_are_candidate_or_history() {
        use crossterm::event::{KeyCode, KeyModifiers};
        // Up/Down resolve to dropdown-or-history in the dispatcher; the
        // chord decode just tags them (was HistoryPrev/HistoryNext, which
        // ignored the completion dropdown entirely).
        assert!(matches!(
            MinibufferAction::from_chord(Chord {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
            }),
            MinibufferAction::PrevCandidateOrHistory
        ));
        assert!(matches!(
            MinibufferAction::from_chord(Chord {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
            }),
            MinibufferAction::NextCandidateOrHistory
        ));
    }

    #[test]
    fn insert_and_backspace_round_trip() {
        let mut mb = Minibuffer::new();
        for c in "hi".chars() {
            mb.insert_char(c);
        }
        assert_eq!(mb.contents(), "hi");
        mb.backspace();
        assert_eq!(mb.contents(), "h");
        mb.backspace();
        mb.backspace();
        assert_eq!(mb.contents(), "");
    }

    #[test]
    fn move_cursor_clamps_at_boundaries() {
        let mut mb = Minibuffer::new();
        for c in "abc".chars() {
            mb.insert_char(c);
        }
        mb.move_line_start();
        assert_eq!(mb.cursor, 0);
        mb.move_left();
        assert_eq!(mb.cursor, 0);
        mb.move_line_end();
        assert_eq!(mb.cursor, 3);
        mb.move_right();
        assert_eq!(mb.cursor, 3);
    }

    #[test]
    fn fuzzy_score_subsequence() {
        assert!(fuzzy_score("buf", "buffer.save").is_some());
        assert!(fuzzy_score("save", "buffer.save").is_some());
        assert!(fuzzy_score("bsave", "buffer.save").is_some());
        assert!(fuzzy_score("xyz", "buffer.save").is_none());
        assert!(fuzzy_score("", "anything").is_some());
    }

    #[test]
    fn fuzzy_score_prefers_word_boundaries() {
        let s_prefix = fuzzy_score("save", "buffer.save").unwrap();
        let s_middle = fuzzy_score("uffe", "buffer.save").unwrap();
        assert!(s_prefix > s_middle, "{s_prefix} > {s_middle}");
    }

    #[test]
    fn filter_and_sort_drops_non_matches() {
        let pool = vec![
            "buffer.save".to_string(),
            "editor.quit".to_string(),
            "buffer.undo".to_string(),
        ];
        let out = filter_and_sort("buf", &pool);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| s.contains("buffer")));
    }

    #[test]
    fn history_push_dedupes_consecutive() {
        let mut h = History::default();
        h.push("a".into());
        h.push("a".into());
        h.push("b".into());
        h.push("a".into());
        assert_eq!(
            h.entries.iter().cloned().collect::<Vec<_>>(),
            vec!["a".to_string(), "b".into(), "a".into()]
        );
    }

    #[test]
    fn history_truncates_to_max() {
        let mut h = History::default();
        for i in 0..(HISTORY_MAX + 50) {
            h.push(format!("entry-{i}"));
        }
        assert_eq!(h.entries.len(), HISTORY_MAX);
        assert_eq!(h.entries.front().unwrap(), &format!("entry-{}", 50));
    }

    #[test]
    fn history_persistence_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        append_history_file(dir.path(), "command", "buffer.save").unwrap();
        append_history_file(dir.path(), "command", "editor.quit").unwrap();
        let entries = load_history_file(dir.path(), "command").unwrap();
        assert_eq!(entries, vec!["buffer.save", "editor.quit"]);
    }

    #[test]
    fn history_load_missing_is_empty_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let entries = load_history_file(dir.path(), "nope").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn history_navigation_steps_through_entries() {
        let lua = Lua::new();
        let mut mb = Minibuffer::new();
        mb.history.insert(
            "test".into(),
            History::with_entries(vec!["one".into(), "two".into(), "three".into()]),
        );
        open(&mut mb, &lua, CompletionSource::None, "test");
        // Type something fresh; up restores entries newest-first.
        for c in "xyz".chars() {
            mb.insert_char(c);
        }
        mb.history_prev();
        assert_eq!(mb.contents(), "three");
        mb.history_prev();
        assert_eq!(mb.contents(), "two");
        mb.history_prev();
        assert_eq!(mb.contents(), "one");
        mb.history_prev();
        assert_eq!(mb.contents(), "one"); // clamps
        mb.history_next();
        assert_eq!(mb.contents(), "two");
        mb.history_next();
        mb.history_next();
        // Past the end: typed prefix restored.
        assert_eq!(mb.contents(), "xyz");
    }

    #[test]
    fn accept_returns_callback_and_clears_buffer() {
        let lua = Lua::new();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::None, "");
        for c in "abc".chars() {
            mb.insert_char(c);
        }
        let result = mb.accept().expect("session was active");
        assert_eq!(result.1, "abc");
        assert!(mb.session.is_none());
        assert_eq!(mb.contents(), "");
    }

    #[test]
    fn cancel_clears_buffer_and_session() {
        let lua = Lua::new();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::None, "");
        for c in "x".chars() {
            mb.insert_char(c);
        }
        let _ = mb.cancel();
        assert!(mb.session.is_none());
        assert_eq!(mb.contents(), "");
    }

    #[test]
    fn recompute_candidates_against_commands() {
        let lua = Lua::new();
        let mut commands = CommandRegistry::new();
        for n in ["buffer.save", "buffer.undo", "editor.quit"] {
            commands
                .define(crate::command::Command {
                    name: n.into(),
                    description: "x".into(),
                    source: crate::command::SourceLocation::default(),
                    body: lua.create_function(|_, ()| Ok(())).unwrap(),
                    predicate: None,
                })
                .unwrap();
        }
        let registry = BufferRegistry::new();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::Commands, "");
        for c in "buf".chars() {
            mb.insert_char(c);
        }
        mb.recompute_candidates(&commands, &registry).unwrap();
        let cands = &mb.session.as_ref().unwrap().candidates;
        assert_eq!(cands.len(), 2);
        assert!(cands.iter().all(|s| s.starts_with("buffer.")));
    }

    #[test]
    fn complete_replaces_buffer_with_selection() {
        let lua = Lua::new();
        let mut commands = CommandRegistry::new();
        commands
            .define(crate::command::Command {
                name: "buffer.save".into(),
                description: "x".into(),
                source: crate::command::SourceLocation::default(),
                body: lua.create_function(|_, ()| Ok(())).unwrap(),
                predicate: None,
            })
            .unwrap();
        let registry = BufferRegistry::new();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::Commands, "");
        for c in "buf".chars() {
            mb.insert_char(c);
        }
        mb.recompute_candidates(&commands, &registry).unwrap();
        mb.complete();
        assert_eq!(mb.contents(), "buffer.save");
    }

    #[test]
    fn resolve_history_dir_prefers_xdg() {
        use std::ffi::OsStr;
        let xdg = OsStr::new("/srv/xdg");
        let home = OsStr::new("/home/u");
        let dir = resolve_history_dir(Some(xdg), Some(home)).unwrap();
        assert_eq!(dir, PathBuf::from("/srv/xdg/pmacs/history"));
    }

    #[test]
    fn resolve_history_dir_falls_back_to_home() {
        use std::ffi::OsStr;
        let dir = resolve_history_dir(None, Some(OsStr::new("/home/u"))).unwrap();
        assert_eq!(dir, PathBuf::from("/home/u/.local/state/pmacs/history"));
    }

    #[test]
    fn resolve_history_dir_returns_none_with_no_env() {
        assert!(resolve_history_dir(None, None).is_none());
    }
}
