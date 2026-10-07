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

/// Hard cap on how many candidates a session holds after filtering
/// and sorting. Most prompts have far fewer; the cap bounds what the
/// band paints and what accept resolves against. It is applied AFTER
/// [`filter_and_sort`] has scored and ordered the whole pool (E4.1):
/// until then it truncated a custom source's pool at 1024 entries
/// BEFORE the needle was applied, so a match past the 1024th entry of
/// a 20k-file listing was unreachable by typing, and `filter_and_sort`
/// took its first 1024 survivors before sorting them, so the best
/// match could be dropped in favor of a worse one that came earlier.
pub const CANDIDATE_LIMIT: usize = 1024;

/// Canonical name of the minibuffer's backing buffer.
pub const MINIBUFFER_NAME: &str = "*minibuffer*";

/// How many candidates a frontend shows at once: the grid's band above
/// the prompt (E6.2) and the wire's windowed slice for a semantic
/// frontend share this, so both paint the same window around the
/// selection.
pub const MB_VISIBLE: usize = 10;

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

    /// The active session's accept policy, if a session is active.
    #[must_use]
    pub fn accept_policy(&self) -> Option<AcceptPolicy> {
        self.session.as_ref().map(|s| s.accept)
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

    /// TAB (E6.3). Three steps, the first that changes the field wins,
    /// leaving the session active so the user can keep typing or
    /// accept:
    ///
    /// 1. The longest common prefix of every candidate, when it is
    ///    longer than what is typed: `alp` over `alpha-one.txt` and
    ///    `alpha-two.txt` becomes `alpha-`. A single candidate is its
    ///    own prefix, so a unique match completes whole.
    /// 2. Otherwise the selected candidate, when it differs from what is
    ///    typed (D18: "TAB completes to the selection"), which is also
    ///    what a fuzzy match reaches when no prefix extends.
    /// 3. Otherwise, in a files prompt, when the typed name is a
    ///    directory, a trailing `/`: the listing descends into it on
    ///    the recompute that follows. So `su TAB TAB` is `sub/` with
    ///    sub's entries as the candidates.
    ///
    /// In a files prompt every step edits the part after the last `/`
    /// and keeps the directory part. No-op with no candidates. The
    /// prefix is compared case-sensitively, so a case split in the
    /// candidates (`Makefile`, `main.rs`) has no common prefix and
    /// falls to the selection.
    pub fn complete(&mut self) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        if s.candidates.is_empty() {
            return;
        }
        let contents = self.contents();
        let files_root = match &s.source {
            CompletionSource::Files { root } => Some(root.clone()),
            _ => None,
        };
        let (dir_part, base) = if files_root.is_some() {
            split_dir_base(&contents)
        } else {
            ("", contents.as_str())
        };
        let prefix = common_prefix(&s.candidates);
        let selected = s
            .selected
            .and_then(|i| s.candidates.get(i))
            .map(String::as_str);
        let next_base = if prefix.len() > base.len() {
            prefix
        } else if let Some(pick) = selected
            && pick != base
        {
            pick.to_owned()
        } else if let Some(root) = files_root.as_deref()
            && !base.is_empty()
            && listing_dir(root, dir_part).join(base).is_dir()
        {
            format!("{base}/")
        } else {
            return;
        };
        let next = format!("{dir_part}{next_base}");
        self.replace_contents(&next);
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
        self.accept_with(false)
    }

    /// Commit the typed text as written, whatever the session's accept
    /// policy or selection: `C-j` under D18. Otherwise [`Self::accept`].
    pub fn accept_typed(&mut self) -> Option<(Function, String)> {
        self.accept_with(true)
    }

    fn accept_with(&mut self, typed_wins: bool) -> Option<(Function, String)> {
        let session = self.session.take()?;
        let typed = self.contents();
        let resolved = if typed_wins {
            typed.clone()
        } else {
            resolve_accepted_value(&session, &typed)
        };
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
        let candidates = if s.ranked {
            // The source already filtered and ordered against the
            // needle it was handed; only the cap applies.
            collect_pool(&s.source, &needle, commands, registry)?
                .into_iter()
                .take(CANDIDATE_LIMIT)
                .collect()
        } else if matches!(s.source, CompletionSource::Buffers) {
            rank_buffer_names(&needle, &buffer_pool(registry))
        } else {
            let pool = collect_pool(&s.source, &needle, commands, registry)?;
            filter_and_sort(filter_needle(&s.source, &needle), &pool)
        };
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
    /// When set, the source's returned order IS the candidate order:
    /// the session neither filters nor re-sorts, and applies only
    /// [`CANDIDATE_LIMIT`]. For a [`CompletionSource::Custom`] source
    /// that receives the needle and ranks for itself (the project file
    /// finder puts recent files first, which no score can express);
    /// meaningless for the builtin sources, whose pools are unranked.
    pub ranked: bool,
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
    /// What RET resolves to (D18, E6.1). Set by the caller of
    /// `pmacs.minibuffer.read`; `candidate` when omitted.
    pub accept: AcceptPolicy,
}

/// What RET commits: the selected candidate or the typed text (D18);
/// or, under [`Self::Key`], what one printable key commits (E7b.2).
///
/// Every prompt names its policy because the two are different
/// contracts. A picker over a closed set --- `M-x`, `where-is` --- wants
/// the selection, since the typed text is a search query and cannot be
/// a command that does not exist. A prompt over an open set --- a file
/// to create, a buffer name --- wants the text as written, or a new
/// name that happens to be a subsequence of an existing one silently
/// opens the existing entry (`C-x C-f nots RET` opened `notes.md`; audit
/// §3.1). Under either policy `C-j` takes the typed text and TAB
/// completes to the selection, so the policy decides only what RET
/// means. `Key` is the third contract, Emacs's `y-or-n-p`: the first
/// printable key IS the answer and the prompt closes on it, with RET
/// committing whatever was typed (nothing, for a prompt nobody typed
/// into) and `C-g` cancelling as everywhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AcceptPolicy {
    /// RET takes the selected candidate when one exists, else the
    /// typed text. Today's behavior for every prompt that does not say
    /// otherwise.
    #[default]
    Candidate,
    /// RET takes the typed text as written, whatever is selected.
    Typed,
    /// A printable key commits itself, at once, as the typed text
    /// (E7b.2, `pmacs.minibuffer.y_or_n`). The prompt has no
    /// candidates to select, so the key is the whole answer.
    Key,
}

impl AcceptPolicy {
    /// Parse the `accept` field of `pmacs.minibuffer.read`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "candidate" => Some(Self::Candidate),
            "typed" => Some(Self::Typed),
            "key" => Some(Self::Key),
            _ => None,
        }
    }
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
    /// Commit the typed text as written, whatever is selected (C-j).
    AcceptTyped,
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
                    'j' => Self::AcceptTyped,
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
    if matches!(session.source, CompletionSource::None)
        || matches!(session.accept, AcceptPolicy::Typed | AcceptPolicy::Key)
    {
        return typed.to_owned();
    }
    if let Some(idx) = session.selected
        && let Some(cand) = session.candidates.get(idx)
    {
        // A file candidate is a bare entry of the directory the field
        // names; the value is the path, so the directory part is put
        // back in front of it.
        if matches!(session.source, CompletionSource::Files { .. }) {
            let (dir, _) = split_dir_base(typed);
            return format!("{dir}{cand}");
        }
        return cand.clone();
    }
    typed.to_owned()
}

/// The longest prefix every string in `items` shares, by character,
/// case-sensitively; empty for an empty list.
#[must_use]
pub fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for item in &items[1..] {
        let shared = prefix
            .iter()
            .zip(item.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(shared);
        if prefix.is_empty() {
            break;
        }
    }
    prefix.into_iter().collect()
}

/// Split a files-prompt field at its last `/`: `("src/", "ma")` for
/// `src/ma`, `("", "ma")` for `ma`, `("/tmp/", "")` for `/tmp/`. The
/// directory part keeps its trailing slash so `dir + name` is a path.
#[must_use]
pub fn split_dir_base(input: &str) -> (&str, &str) {
    match input.rfind('/') {
        Some(idx) => input.split_at(idx + 1),
        None => ("", input),
    }
}

/// The directory a files prompt lists for the field's directory part:
/// absolute as written, `~` and `~/` under `$HOME`, anything else
/// joined onto the prompt's root, and the root itself for an empty
/// part.
#[must_use]
pub fn listing_dir(root: &Path, dir_part: &str) -> PathBuf {
    if dir_part.is_empty() {
        return root.to_path_buf();
    }
    if dir_part.starts_with('/') {
        return PathBuf::from(dir_part);
    }
    if let Some(rest) = dir_part.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with('/'))
        && let Some(home) = std::env::var_os("HOME")
    {
        let rest = rest.trim_start_matches('/');
        return if rest.is_empty() {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(rest)
        };
    }
    root.join(dir_part)
}

/// The text the candidate filter runs against: the whole field, except
/// for a files prompt, whose pool is the entries of the directory the
/// field names and whose filter is the part after the last `/`.
fn filter_needle<'a>(source: &CompletionSource, needle: &'a str) -> &'a str {
    match source {
        CompletionSource::Files { .. } => split_dir_base(needle).1,
        _ => needle,
    }
}

fn collect_pool(
    source: &CompletionSource,
    needle: &str,
    commands: &CommandRegistry,
    registry: &BufferRegistry,
) -> mlua::Result<Vec<String>> {
    match source {
        CompletionSource::None => Ok(Vec::new()),
        CompletionSource::Commands => Ok(commands.names().to_vec()),
        CompletionSource::Buffers => Ok(buffer_pool(registry)
            .into_iter()
            .map(|(name, _)| name)
            .collect()),
        CompletionSource::Files { root } => {
            let (dir_part, _) = split_dir_base(needle);
            Ok(list_directory(&listing_dir(root, dir_part)))
        }
        CompletionSource::Custom(f) => {
            // The typed text is the source's one argument (E4.1). A
            // source that ignores it behaves exactly as before; one
            // that reads it can narrow or rank its own pool, which is
            // what lets a 20k-entry listing stay a Lua table rather
            // than 20k strings crossing the boundary per keystroke.
            // The whole sequence is collected: the cap is applied by
            // the caller after filtering, never here before it.
            let table: mlua::Table = f.call(needle)?;
            Ok(table.sequence_values::<String>().flatten().collect())
        }
    }
}

/// The `buffers` source's pool: each buffer's name, and whether the
/// buffer holds a file path, which is what makes a `/` in its name a
/// directory (see [`name_tier`]).
fn buffer_pool(registry: &BufferRegistry) -> Vec<(String, bool)> {
    registry
        .ids()
        .iter()
        .filter_map(|id| {
            let b = registry.get(*id).ok()?;
            Some((b.name().to_owned(), b.file_path().is_some()))
        })
        .collect()
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
    rank_candidates(needle, pool, Some(CANDIDATE_LIMIT))
}

/// Filter `pool` to the strings [`fuzzy_score`] accepts for `needle`,
/// sort them best first (ties lexically), and THEN take at most
/// `limit`. The order of those three steps is the contract: taking
/// before sorting kept whichever survivors came first in the pool and
/// could drop the best match (E4.1). Exposed to Lua as
/// `pmacs.minibuffer.rank` so a ranked source scores with the same
/// function the session would have used.
#[must_use]
pub fn rank_candidates(needle: &str, pool: &[String], limit: Option<usize>) -> Vec<String> {
    let mut scored: Vec<(i32, &str)> = pool
        .iter()
        .filter_map(|s| fuzzy_score(needle, s).map(|sc| (sc, s.as_str())))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|(_, s)| s.to_owned())
        .collect()
}

/// [`filter_and_sort`] for the `buffers` source (`C-x b`, `C-x k`), whose
/// file buffers are named by their absolute paths. A path is many
/// components, so a short name is a subsequence of paths it does not
/// name: `b.rs` matches `…/build/…/a.rs` through the `b` of `build` and
/// the `.rs`, and [`fuzzy_score`], which takes each character's first
/// occurrence, scores the two paths alike wherever their shared
/// directories hold the `b`. The lexical tie-break then put `a.rs` first
/// and `C-x b b.rs RET` stayed on it (#318); under this laptop's gate
/// `jeans`, `gate` and `targets` spell `nts` the same way.
///
/// So a name is placed by [`name_tier`] before its score is read: a match
/// within a buffer's own name outranks one spelled through the
/// directories it sits in. A buffer's own name is the basename of one
/// that holds a file path, and the whole name of any other, which sits in
/// no directory however many `/` it holds: a dired buffer is
/// `*dired:<path>*`, and while its `/` were read as directories `C-x b
/// dired RET` reached `dired.lua` (E8b review 1).
///
/// Within a place the match is scored where the place says it lies:
/// inside the own name in place 3, and on the whole name otherwise. Read
/// on the whole name, `notes.txt` and a dired buffer whose directories
/// both spell `nts` tie through those directories, and the lexical order
/// took the dired buffer. The own name is read where it stands, after
/// its `/`, so a basename's first letter is never taken for the name's.
/// Ties are lexical, which puts a `*` name before a `/` one: `*lsp*`
/// before `src/lsp.rs` for `lsp`, and the dired buffer before
/// `dired.lua` for `dired`, whose own names hold the letters as tightly.
/// The display names are untouched.
fn rank_buffer_names(needle: &str, pool: &[(String, bool)]) -> Vec<String> {
    let mut scored: Vec<(u8, i32, &str)> = pool
        .iter()
        .filter_map(|(name, path)| {
            let whole = fuzzy_score(needle, name)?;
            let place = name_tier(needle, name, *path);
            let score = if place == 3 {
                let own = own_name_start(name, *path);
                fuzzy_score(needle, &name[own.saturating_sub(1)..]).unwrap_or(whole)
            } else {
                whole
            };
            Some((place, score, name.as_str()))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(b.2))
    });
    scored
        .into_iter()
        .take(CANDIDATE_LIMIT)
        .map(|(_, _, s)| s.to_owned())
        .collect()
}

/// How `name` meets `needle` before any score, `path` saying whether its
/// buffer holds a file path: `0` when it is the name as written; `1`
/// when the name ends with it at a path component (a basename typed
/// whole, or `lsp/mod.rs` against `…/src/lsp/mod.rs`, so directories
/// still tell two `mod.rs` apart); `2` the same with case folded as
/// [`fuzzy_score`] folds it; `3` when it is a subsequence of the name's
/// own name (`nts` of `notes.txt`, `dired` of `*dired:/p/proj*`); `4`
/// when the match needs the directories. A name whose buffer holds no
/// file is one component, so a match in it is never placed 1 or 4. An
/// empty needle places every name alike.
fn name_tier(needle: &str, name: &str, path: bool) -> u8 {
    fn fold(s: &str) -> String {
        s.chars().flat_map(char::to_lowercase).collect()
    }
    let ends_at_component = |name: &str, needle: &str| {
        name.strip_suffix(needle).is_some_and(|rest| {
            rest.is_empty() || (path && (rest.ends_with('/') || needle.starts_with('/')))
        })
    };
    if needle.is_empty() {
        4
    } else if name == needle {
        0
    } else if ends_at_component(name, needle) {
        1
    } else if ends_at_component(&fold(name), &fold(needle)) {
        2
    } else if fuzzy_score(needle, &name[own_name_start(name, path)..]).is_some() {
        3
    } else {
        4
    }
}

/// Where a buffer's own name begins in `name`: after the last `/` of one
/// whose buffer holds a file path (`path`), and at the start of any
/// other.
fn own_name_start(name: &str, path: bool) -> usize {
    if path {
        name.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1)
    } else {
        0
    }
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
    // Route through the shared state-dir resolver so history honors the
    // `PMACS_STATE_HOME` override too (Arc 3 Q#PS2).
    crate::state::user_state_dir().map(|d| d.join("history"))
}

/// Pure helper for [`user_history_dir`], factored out so tests can
/// inject paths directly without touching the process environment
/// (R55: `unsafe_code = "forbid"` rules out `env::set_var`).
///
/// History lives under the shared editor state dir
/// ([`crate::state::state_dir`], Arc 3 Q#PS2) in a `history/`
/// subdirectory. A blank `XDG_STATE_HOME` now falls through to `HOME`
/// instead of yielding a relative path (the empty-XDG fix).
#[must_use]
pub fn resolve_history_dir(
    xdg_state: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    crate::state::state_dir(xdg_state, home).map(|d| d.join("history"))
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
            ranked: false,
            on_accept: dummy_accept(lua),
            on_cancel: None,
            candidates: Vec::new(),
            selected: None,
            history_index: None,
            typed_before_history_nav: None,
            accept: AcceptPolicy::Candidate,
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

    /// File buffers named by `paths`, as the `buffers` source pools them.
    fn files(paths: &[&String]) -> Vec<(String, bool)> {
        paths.iter().map(|p| ((*p).clone(), true)).collect()
    }

    /// E8b.1, #318's premise measured on the scorer itself: where the
    /// directories two file buffers share hold a `b`, `b.rs` is a
    /// subsequence of both paths and [`fuzzy_score`] gives them one
    /// score, so the order the shared ranker gives is the lexical one,
    /// `a.rs` first. The buffers ranker puts the basename typed whole
    /// first.
    #[test]
    fn a_basename_typed_whole_outranks_a_match_across_directories() {
        let a = "/home/u/build/tmp/.tmpXq9/a.rs".to_owned();
        let b = "/home/u/build/tmp/.tmpXq9/b.rs".to_owned();
        assert_eq!(
            fuzzy_score("b.rs", &a),
            fuzzy_score("b.rs", &b),
            "premise: the scorer cannot tell the two paths apart"
        );
        assert_eq!(
            rank_candidates("b.rs", &[a.clone(), b.clone()], None),
            vec![a.clone(), b.clone()]
        );
        let pool = files(&[&a, &b]);
        assert_eq!(rank_buffer_names("b.rs", &pool), vec![b.clone(), a.clone()]);
        assert_eq!(rank_buffer_names("a.rs", &pool), vec![a.clone()]);
        assert_eq!(rank_buffer_names("B.RS", &pool), vec![b.clone(), a.clone()]);
        assert_eq!(rank_buffer_names(&a, &pool), vec![a]);
        assert_eq!(rank_buffer_names(&b, &pool), vec![b]);
    }

    /// E8b.1: two buffers with one basename are told apart by typing a
    /// directory with it, or the whole path; a basename alone ranks both
    /// above every other match. A suffix tier needs whole components:
    /// `od.rs` only fits inside the last one, and `src/m` needs the
    /// directories.
    #[test]
    fn directories_still_tell_two_alike_basenames_apart() {
        let lsp = "/p/src/lsp/mod.rs".to_owned();
        let lua = "/p/src/lua_bindings/mod.rs".to_owned();
        let other = "/p/src/main.rs".to_owned();
        let pool = files(&[&other, &lua, &lsp]);
        assert_eq!(rank_buffer_names("lsp/mod.rs", &pool)[0], lsp);
        assert_eq!(rank_buffer_names("lua_bindings/mod.rs", &pool)[0], lua);
        assert_eq!(rank_buffer_names("/mod.rs", &pool).len(), 2);
        let both = rank_buffer_names("mod.rs", &pool);
        assert_eq!(both.len(), 2, "{both:?}");
        assert_eq!(rank_buffer_names(&lua, &pool)[0], lua);
        assert_eq!(name_tier("mod.rs", &lsp, true), 1);
        assert_eq!(name_tier("od.rs", &lsp, true), 3);
        assert_eq!(name_tier("src/m", &lsp, true), 4);
        assert_eq!(name_tier("", &lsp, true), 4);
    }

    /// E8b.1 keeps D18's subsequences: `nts` reaches `notes.txt`, `scr`
    /// `*scratch*` and `lsp` `*lsp*` with `src/lsp.rs` open beside it,
    /// and `zzz` matches nothing. Where the directories every file
    /// shares spell the abbreviation (`jeans`, `gate`, `targets` spell
    /// `nts`), the shared ranker scores every file alike and puts
    /// `a.rs` first; the buffers ranker puts the one whose own name
    /// holds it first.
    #[test]
    fn the_buffers_ranker_keeps_d18_s_subsequences() {
        let root = "/home/jeans/build/pmacs-gate-targets/tmp/hand/.tmpXq9/build";
        let paths: Vec<String> = ["a.rs", "b.rs", "notes.txt", "src/lsp.rs"]
            .iter()
            .map(|f| format!("{root}/{f}"))
            .collect();
        let mut pool = files(&paths.iter().collect::<Vec<_>>());
        pool.extend([("*scratch*".to_owned(), false), ("*lsp*".to_owned(), false)]);
        let notes = format!("{root}/notes.txt");
        let names: Vec<String> = pool.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(
            rank_candidates("nts", &names, None)[0],
            format!("{root}/a.rs"),
            "premise: the directories spell `nts` for every file"
        );
        assert_eq!(rank_buffer_names("nts", &pool)[0], notes);
        assert_eq!(rank_buffer_names("scr", &pool)[0], "*scratch*");
        assert_eq!(rank_buffer_names("lsp", &pool)[0], "*lsp*");
        assert!(rank_buffer_names("zzz", &pool).is_empty());
        assert_eq!(
            rank_buffer_names("lsp.rs", &pool)[0],
            format!("{root}/src/lsp.rs")
        );
    }

    /// E8b review 1's Medium 1: a buffer that holds no file is placed by
    /// its whole name, its `/` read as nothing. A dired buffer's name,
    /// `*dired:<path>*`, holds `dired` and every abbreviation of it, so
    /// it is place 3 beside `dired.lua`, ties it, and comes first by the
    /// lexical order; read as a path (the flag alone changed), its own
    /// name would be `proj*` and its match would need the directories.
    #[test]
    fn a_buffer_holding_no_file_is_its_whole_name() {
        let dired = "*dired:/home/u/build/proj*".to_owned();
        let lua = "/home/u/build/proj/dired.lua".to_owned();
        let editor = "/home/u/build/proj/editor.rs".to_owned();
        let mut pool = files(&[&lua, &editor]);
        pool.push((dired.clone(), false));
        assert_eq!(
            rank_buffer_names("dired", &pool),
            vec![dired.clone(), lua.clone()]
        );
        assert_eq!(
            rank_buffer_names("dir", &pool),
            vec![dired.clone(), lua.clone(), editor]
        );
        assert_eq!(name_tier("dired", &dired, false), 3);
        assert_eq!(name_tier("dired", &dired, true), 4);
        assert_eq!(name_tier("dired", &lua, true), 3);
        assert_eq!(name_tier("/proj*", &dired, false), 3, "one component");
        assert_eq!(name_tier("/proj*", &dired, true), 1);
        assert_eq!(name_tier(&dired, &dired, false), 0);
    }

    /// Within place 3 the match is scored inside each own name, read where
    /// it stands. `notes.txt` and a dired buffer of a directory beside it
    /// both hold `nts`, and on their whole names they tie through the
    /// directories that spell it (`jeans`, `gate`, `targets`), which the
    /// lexical order gave the dired buffer. `src/lsp.rs`'s own name holds
    /// `lsp` as tightly as `*lsp*` does, its `l` taken as following a `/`
    /// rather than starting a name, and the lexical order gives `*lsp*`.
    /// The other side of the rule: a dired buffer whose own name holds
    /// `nts` more tightly than `notes.txt` (`bin-tests`, with `-t`'s
    /// boundary) takes it.
    #[test]
    fn place_three_scores_the_match_inside_the_own_name() {
        let root = "/home/jeans/build/pmacs-gate-targets/tmp/hand/.tmpXq9";
        let notes = format!("{root}/proj/notes.txt");
        let lsp_rs = format!("{root}/proj/src/lsp.rs");
        let dired = format!("*dired:{root}/proj*");
        let mut pool = files(&[&notes, &lsp_rs]);
        pool.extend([(dired.clone(), false), ("*lsp*".to_owned(), false)]);
        assert_eq!(
            fuzzy_score("nts", &dired),
            fuzzy_score("nts", &notes),
            "premise: on their whole names the two tie"
        );
        assert_eq!(rank_buffer_names("nts", &pool)[0], notes);
        assert_eq!(
            rank_buffer_names("lsp", &pool)[..2],
            ["*lsp*", lsp_rs.as_str()]
        );
        let tests = "*dired:/tmp/.tmpXq9/bin-tests*".to_owned();
        pool.push((tests.clone(), false));
        assert_eq!(rank_buffer_names("nts", &pool)[0], tests);
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

    /// E4.1: the typed text reaches a custom source as its argument.
    #[test]
    fn custom_source_receives_the_typed_needle() {
        let lua = Lua::new();
        let commands = CommandRegistry::new();
        let registry = BufferRegistry::new();
        let seen = lua.create_table().unwrap();
        lua.globals().set("seen", seen.clone()).unwrap();
        let f: Function = lua
            .load("return function(needle) seen[#seen + 1] = needle return { 'x' } end")
            .eval()
            .unwrap();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::Custom(f), "");
        for c in "ab".chars() {
            mb.insert_char(c);
        }
        mb.recompute_candidates(&commands, &registry).unwrap();
        let last: String = seen.get(seen.len().unwrap()).unwrap();
        assert_eq!(last, "ab", "the source must be called with the needle");
    }

    /// E4.1: a custom source's pool is filtered whole; the cap applies
    /// after the needle, not before it.
    #[test]
    fn custom_source_pool_is_not_truncated_before_filtering() {
        let lua = Lua::new();
        let commands = CommandRegistry::new();
        let registry = BufferRegistry::new();
        let f: Function = lua
            .load(
                "return function()\n\
                   local t = {}\n\
                   for i = 1, 3000 do t[i] = string.format('entry%04d', i) end\n\
                   t[#t + 1] = 'needle-match'\n\
                   return t\n\
                 end",
            )
            .eval()
            .unwrap();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::Custom(f), "");
        for c in "needle".chars() {
            mb.insert_char(c);
        }
        mb.recompute_candidates(&commands, &registry).unwrap();
        let cands = &mb.session.as_ref().unwrap().candidates;
        assert_eq!(
            cands.as_slice(),
            ["needle-match".to_owned()],
            "the 3001st entry must be reachable by typing"
        );
    }

    /// E4.1: `filter_and_sort` sorts before it takes, so the best match
    /// survives even when it is the last of more than the cap's worth
    /// of survivors.
    #[test]
    fn filter_and_sort_sorts_before_it_takes() {
        let mut pool: Vec<String> = (0..1500).map(|i| format!("zzab{i}")).collect();
        pool.push("ab".to_owned());
        let out = filter_and_sort("ab", &pool);
        assert_eq!(out.len(), CANDIDATE_LIMIT);
        assert_eq!(out[0], "ab", "the best-scoring entry must lead");
    }

    /// The `Files` source's pool is cut at [`CANDIDATE_LIMIT`] entries in
    /// `read_dir` order BEFORE the needle is applied, so in a directory
    /// of more than 1024 entries a name past the cut cannot be completed
    /// to by `find-file` or `write-file`, whatever is typed --- the same
    /// reachability defect E4.1 removed from `Custom`. It stays, and the
    /// reason is stronger than scope: `Custom`'s cut could go because a
    /// custom source can rank for itself (the finder does, through
    /// `pmacs.minibuffer.rank`, and returns at most the cap), while
    /// `Files` has no counterpart, so deleting the `break` in
    /// `list_directory` trades this defect for an unbounded `read_dir` in
    /// a hostile directory, and what replaces the cap is a design
    /// decision no row has authorized. Pinned here so the phase that
    /// lifts it does so on purpose: when this fails because the pool is
    /// whole, replace it with the reachability assertion `Custom` has.
    #[test]
    fn files_source_pool_is_still_cut_at_the_candidate_limit_before_filtering() {
        let dir = tempfile::TempDir::new().unwrap();
        for i in 0..(CANDIDATE_LIMIT + 200) {
            std::fs::write(dir.path().join(format!("entry{i:04}")), b"").unwrap();
        }
        let commands = CommandRegistry::new();
        let registry = BufferRegistry::new();
        let source = CompletionSource::Files {
            root: dir.path().to_path_buf(),
        };
        let pool = collect_pool(&source, "", &commands, &registry).unwrap();
        assert_eq!(
            pool.len(),
            CANDIDATE_LIMIT,
            "the Files pool is cut at the cap before filtering; a whole pool \
             means the cut was lifted --- replace this pin with the \
             reachability assertion"
        );
    }

    /// E4.1: a ranked session keeps the source's order and filters
    /// nothing away itself.
    #[test]
    fn ranked_session_keeps_the_source_order() {
        let lua = Lua::new();
        let commands = CommandRegistry::new();
        let registry = BufferRegistry::new();
        let f: Function = lua
            .load("return function() return { 'b', 'a', 'c' } end")
            .eval()
            .unwrap();
        let mut mb = Minibuffer::new();
        open(&mut mb, &lua, CompletionSource::Custom(f), "");
        mb.session.as_mut().unwrap().ranked = true;
        for c in "zzz".chars() {
            mb.insert_char(c);
        }
        mb.recompute_candidates(&commands, &registry).unwrap();
        let cands = &mb.session.as_ref().unwrap().candidates;
        assert_eq!(
            cands.as_slice(),
            ["b".to_owned(), "a".to_owned(), "c".to_owned()],
            "a ranked source's order is the candidate order"
        );
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
