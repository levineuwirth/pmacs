// completion_framework.rs --- T M4.11 unified completion framework:
// pluggable providers, priority, dedup, snippets, Lua-defineable
// custom sources.

//! Unified completion provider framework.
//!
//! Per spec §M4.11: "aggregate completion sources (LSP, dabbrev,
//! snippets, project symbols) through a unified provider
//! interface."
//!
//! # Concepts
//!
//! * [`CompletionContext`] is the input every provider sees: the
//!   typed prefix, cursor line/col, the buffer's current text, the
//!   language tag, and the project root (if any). It is owned and
//!   `Clone`, so providers cannot mutate it.
//! * [`CompletionCandidate`] is a [`crate::completion::CompletionItem`]
//!   plus three pieces of framework metadata: the source name (which
//!   provider produced it), the provider's current priority, and a
//!   score derived from the prefix match.
//! * A [`ProviderFn`] is a closure
//!   `Fn(&CompletionContext) -> Vec<CompletionItem>`. Built-in
//!   providers ([`dabbrev_provider`], [`snippet_provider`],
//!   [`project_symbols_provider`], [`lsp_completion_provider`])
//!   are constructed by free functions in this module. Lua-defined
//!   providers go through the same [`ProviderFn`] type --- the Lua
//!   binding wraps a `mlua::Function` in a closure.
//! * [`CompletionRegistry`] owns the providers, a monotonic id
//!   counter, and a single sort order. [`CompletionRegistry::collect`]
//!   walks every enabled provider, tags each item with framework
//!   metadata, dedups by `(label, effective_insert_text)`, and
//!   sorts by `(score desc, priority desc, label asc)`.
//!
//! # Acceptance hooks
//!
//! * **"Multiple sources combine without duplicates"**: dedup keeps
//!   the highest-priority winner; ties broken by score.
//! * **"Source priority configurable"**:
//!   [`CompletionRegistry::set_priority`] adjusts a registered
//!   provider in-place; subsequent `collect` calls observe the new
//!   value.
//! * **"Custom sources defineable from Lua"**: [`ProviderFn`] is a
//!   plain closure; the Lua binding (`pmacs.completion.register`)
//!   wraps a Lua function as one.

use std::borrow::Cow;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::buffer::BufferId;
use crate::completion::{CompletionItem, CompletionItemKind};

// ---------------------------------------------------------------------------
// Provider id
// ---------------------------------------------------------------------------

/// Stable identifier for one registered provider. Allocated in
/// monotonic order across the whole process.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ProviderId(u64);

impl ProviderId {
    /// Mint a fresh id.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw counter value. Used at the Lua boundary.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProviderId({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// Context + trigger
// ---------------------------------------------------------------------------

/// What kicked off a completion request. Providers can use this to
/// short-circuit (e.g. dabbrev does nothing on an explicit
/// keystroke trigger if the prefix is empty).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionTrigger {
    /// User explicitly invoked completion (e.g. M-x or `C-SPC`).
    Invoked,
    /// A trigger character was typed (e.g. `.`, `:` for LSP).
    Char(char),
    /// The previous response was incomplete; refresh.
    Incomplete,
}

/// The buffer a request completes in, read by byte range (E10k.2).
///
/// A built-in provider reads the stretch it needs ([`Self::bytes`]); the
/// whole text is copied only when something asks for it
/// ([`Self::whole`]), at most once a request, and only a Lua provider
/// does: its fourth argument is the whole text, as it always was. Before
/// E10k.2 every key that collected copied an 18 MiB buffer into Lua and
/// again into the context (#413). Every byte handed out is counted
/// ([`Self::read_bytes`]), which is how
/// [`CompletionRegistry::last_read_bytes`] says what a request read.
#[derive(Clone)]
pub struct CompletionText {
    len: usize,
    source: TextSource,
    whole: Rc<OnceCell<Rc<str>>>,
    read: Rc<Cell<usize>>,
}

/// Where a [`CompletionText`] reads from.
#[derive(Clone)]
enum TextSource {
    /// The whole text, handed over by whoever built the context.
    Owned(Rc<str>),
    /// The live buffer, read by byte range.
    Reader(Rc<dyn Fn(usize, usize) -> Vec<u8>>),
}

impl CompletionText {
    /// A text handed over whole (a context built from a string).
    #[must_use]
    pub fn owned(text: impl Into<Rc<str>>) -> Self {
        let text = text.into();
        Self {
            len: text.len(),
            source: TextSource::Owned(text),
            whole: Rc::default(),
            read: Rc::default(),
        }
    }

    /// A buffer of `len` bytes, read by `read(start, end)` a range at a
    /// time.
    #[must_use]
    pub fn reader(len: usize, read: impl Fn(usize, usize) -> Vec<u8> + 'static) -> Self {
        Self {
            len,
            source: TextSource::Reader(Rc::new(read)),
            whole: Rc::default(),
            read: Rc::default(),
        }
    }

    /// The text's length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes in `start..end`, clamped to the text, and counted.
    #[must_use]
    pub fn bytes(&self, start: usize, end: usize) -> Cow<'_, [u8]> {
        let end = end.min(self.len);
        let start = start.min(end);
        self.read.set(self.read.get() + (end - start));
        match &self.source {
            TextSource::Owned(text) => Cow::Borrowed(&text.as_bytes()[start..end]),
            TextSource::Reader(read) => Cow::Owned(read(start, end)),
        }
    }

    /// The whole text: read the first time a request asks, counted then,
    /// and shared by every later ask in the request.
    #[must_use]
    pub fn whole(&self) -> Rc<str> {
        self.whole
            .get_or_init(|| {
                self.read.set(self.read.get() + self.len);
                match &self.source {
                    TextSource::Owned(text) => text.clone(),
                    TextSource::Reader(read) => {
                        let bytes = read(0, self.len);
                        Rc::from(
                            String::from_utf8(bytes).unwrap_or_else(|e| {
                                String::from_utf8_lossy(e.as_bytes()).into_owned()
                            }),
                        )
                    }
                }
            })
            .clone()
    }

    /// Bytes handed out so far, by [`Self::bytes`] and [`Self::whole`].
    #[must_use]
    pub fn read_bytes(&self) -> usize {
        self.read.get()
    }
}

impl std::fmt::Debug for CompletionText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionText")
            .field("len", &self.len)
            .field("read_bytes", &self.read.get())
            .finish_non_exhaustive()
    }
}

/// All input a provider sees on a completion request. Owned so
/// providers can't reach back into editor state.
#[derive(Clone, Debug)]
pub struct CompletionContext {
    /// What the user has typed so far (the substring the popup is
    /// filtering against).
    pub prefix: String,
    /// Zero-based line of the cursor.
    pub line: u32,
    /// Zero-based column of the cursor.
    pub col: u32,
    /// The active buffer at request time, read by range.
    pub text: CompletionText,
    /// The cursor's byte offset in [`Self::text`]; `None` reads as its
    /// end, where a context built from a string types.
    pub cursor: Option<usize>,
    /// The buffer being completed, when the context names one: a
    /// buffer-local setting (`completion.dabbrev-window`) is read
    /// through it.
    pub buffer: Option<BufferId>,
    /// Language tag (`"rust"`, `"lua"`, …) if known.
    pub language: Option<String>,
    /// Project root, if the active buffer belongs to one.
    pub project_root: Option<PathBuf>,
    /// What kicked off the request.
    pub trigger: CompletionTrigger,
    /// Document URI of the buffer being completed (Q#C8 scoping).
    /// When set, URI-keyed providers (LSP) surface only this
    /// document's entries; when `None` they fall back to the legacy
    /// global drain across every cached key.
    pub uri: Option<String>,
}

impl CompletionContext {
    /// Construct a context with the most common defaults filled in: the
    /// whole text handed over, the cursor at its end.
    #[must_use]
    pub fn new(prefix: impl Into<String>, buffer_text: impl Into<Rc<str>>) -> Self {
        Self {
            prefix: prefix.into(),
            line: 0,
            col: 0,
            text: CompletionText::owned(buffer_text),
            cursor: None,
            buffer: None,
            language: None,
            project_root: None,
            trigger: CompletionTrigger::Invoked,
            uri: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Candidate
// ---------------------------------------------------------------------------

/// A completion candidate together with the framework metadata
/// added at collection time.
#[derive(Clone, Debug)]
pub struct CompletionCandidate {
    /// The underlying item.
    pub item: CompletionItem,
    /// Name of the provider that produced this item.
    pub source: String,
    /// Provider priority at collection time.
    pub priority: i32,
    /// Match score against [`CompletionContext::prefix`]. Higher = better.
    pub score: i32,
    /// The item's place in its provider's own output, for a provider
    /// registered ranked ([`CompletionRegistry::register_ranked`]):
    /// among candidates of equal score and priority it orders before the
    /// label does. `None` for every other provider's items.
    pub rank: Option<usize>,
}

impl CompletionCandidate {
    /// Effective insert text (`item.insert_text` or fallback to
    /// `item.label`). Convenience accessor.
    #[must_use]
    pub fn insert_text(&self) -> &str {
        self.item.effective_insert_text()
    }

    /// Dedup key: `(label, effective_insert_text)`.
    #[must_use]
    pub fn dedup_key(&self) -> (String, String) {
        (self.item.label.clone(), self.insert_text().to_owned())
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Type alias for the closure each provider registers. Owned, so
/// the registry holds the closure and the provider's name+priority
/// independently.
pub type ProviderFn = Box<dyn Fn(&CompletionContext) -> Vec<CompletionItem>>;

/// One slot in the [`CompletionRegistry`].
pub struct RegisteredProvider {
    /// Stable id.
    pub id: ProviderId,
    /// Display name (the dedup key callers see in candidate.source).
    pub name: String,
    /// Higher = more important; ties broken by score then label.
    pub priority: i32,
    /// Disabled providers contribute nothing to `collect`.
    pub enabled: bool,
    /// Whether the provider's items come best first, so that its order
    /// breaks ties before the label does (dabbrev's nearest first).
    pub ranked: bool,
    source: ProviderFn,
}

impl std::fmt::Debug for RegisteredProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredProvider")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("priority", &self.priority)
            .field("enabled", &self.enabled)
            .field("ranked", &self.ranked)
            .finish_non_exhaustive()
    }
}

/// Owns every registered completion provider.
#[derive(Default)]
pub struct CompletionRegistry {
    providers: Vec<RegisteredProvider>,
    /// What the last [`Self::collect`]'s providers read of the buffer.
    last_read: Cell<usize>,
}

impl CompletionRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether the registry has no providers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Register a provider. Returns the new id.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        priority: i32,
        source: ProviderFn,
    ) -> ProviderId {
        self.register_with(name.into(), priority, source, false)
    }

    /// Register a provider whose items come best first: among
    /// candidates of equal score and priority, [`Self::collect`] keeps
    /// its order rather than sorting them by label. dabbrev registers
    /// this way, so the nearer of two words is offered first.
    pub fn register_ranked(
        &mut self,
        name: impl Into<String>,
        priority: i32,
        source: ProviderFn,
    ) -> ProviderId {
        self.register_with(name.into(), priority, source, true)
    }

    fn register_with(
        &mut self,
        name: String,
        priority: i32,
        source: ProviderFn,
        ranked: bool,
    ) -> ProviderId {
        let id = ProviderId::next();
        self.providers.push(RegisteredProvider {
            id,
            name,
            priority,
            enabled: true,
            ranked,
            source,
        });
        id
    }

    /// How many bytes of the buffer the last [`Self::collect`]'s
    /// providers read through its context, the whole text counted when
    /// one asked for it (E10k.2).
    #[must_use]
    pub fn last_read_bytes(&self) -> usize {
        self.last_read.get()
    }

    /// Drop a provider. Returns whether something was removed.
    pub fn unregister(&mut self, id: ProviderId) -> bool {
        let before = self.providers.len();
        self.providers.retain(|p| p.id != id);
        self.providers.len() != before
    }

    /// Adjust a provider's priority. Returns whether the id was found.
    pub fn set_priority(&mut self, id: ProviderId, priority: i32) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == id) {
            p.priority = priority;
            true
        } else {
            false
        }
    }

    /// Enable or disable a provider. Disabled providers contribute
    /// nothing to `collect` but stay registered.
    pub fn set_enabled(&mut self, id: ProviderId, enabled: bool) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == id) {
            p.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Borrow a registered provider by id.
    #[must_use]
    pub fn get(&self, id: ProviderId) -> Option<&RegisteredProvider> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// All providers, in registration order.
    #[must_use]
    pub fn providers(&self) -> &[RegisteredProvider] {
        &self.providers
    }

    /// Run every enabled provider against `ctx`, dedup, and sort by
    /// score, then priority, then a ranked provider's own order, then
    /// label. Higher-priority providers' duplicates win.
    #[must_use]
    pub fn collect(&self, ctx: &CompletionContext) -> Vec<CompletionCandidate> {
        let read_before = ctx.text.read_bytes();
        // Sort providers by priority descending so that when we
        // walk them in order, the first hit on a dedup key is also
        // the highest-priority hit.
        let mut order: Vec<&RegisteredProvider> =
            self.providers.iter().filter(|p| p.enabled).collect();
        order.sort_by_key(|p| std::cmp::Reverse(p.priority));

        let mut by_key: HashMap<(String, String), usize> = HashMap::new();
        let mut out: Vec<CompletionCandidate> = Vec::new();
        for p in order {
            let items = (p.source)(ctx);
            for (place, item) in items.into_iter().enumerate() {
                let score = score_match(&item, &ctx.prefix);
                let cand = CompletionCandidate {
                    source: p.name.clone(),
                    priority: p.priority,
                    score,
                    rank: p.ranked.then_some(place),
                    item,
                };
                let key = cand.dedup_key();
                if let Some(&idx) = by_key.get(&key) {
                    let existing = &out[idx];
                    let replace = cand.priority > existing.priority
                        || (cand.priority == existing.priority && cand.score > existing.score);
                    if replace {
                        out[idx] = cand;
                    }
                } else {
                    by_key.insert(key, out.len());
                    out.push(cand);
                }
            }
        }
        out.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.rank.cmp(&b.rank))
                .then_with(|| a.item.label.cmp(&b.item.label))
        });
        self.last_read.set(ctx.text.read_bytes() - read_before);
        out
    }
}

/// Cheaply-cloneable shared registry.
pub type SharedCompletionRegistry = Rc<RefCell<CompletionRegistry>>;

// ---------------------------------------------------------------------------
// Scoring (shared with project_index in shape, kept local in body)
// ---------------------------------------------------------------------------

const SCORE_EXACT: i32 = 1000;
const SCORE_PREFIX: i32 = 600;
const SCORE_WORD_BOUNDARY: i32 = 300;
const SCORE_SUBSTRING: i32 = 100;
const SCORE_NO_PREFIX: i32 = 0;

/// Score a candidate against a prefix. Empty prefix returns
/// [`SCORE_NO_PREFIX`] for everything (the framework still
/// surfaces all candidates; the UI is responsible for filtering
/// at the empty-prefix case).
fn score_match(item: &CompletionItem, prefix: &str) -> i32 {
    if prefix.is_empty() {
        return SCORE_NO_PREFIX;
    }
    // Use filter_text if the LSP supplied one (it's what the
    // server *wants* us to filter by), else the label.
    let haystack = item.filter_text.as_deref().unwrap_or(&item.label);
    let lower = haystack.to_lowercase();
    let needle = prefix.to_lowercase();
    if lower == needle {
        return SCORE_EXACT;
    }
    if lower.starts_with(&needle) {
        return SCORE_PREFIX;
    }
    let Some(pos) = lower.find(&needle) else {
        return SCORE_NO_PREFIX - 1;
    };
    let at_boundary = pos > 0 && {
        let prev = lower.as_bytes()[pos - 1];
        !prev.is_ascii_alphanumeric()
    };
    let mut score = if at_boundary {
        SCORE_WORD_BOUNDARY
    } else {
        SCORE_SUBSTRING
    };
    score -= i32::try_from(lower.len().saturating_sub(needle.len())).unwrap_or(0);
    score
}

// ---------------------------------------------------------------------------
// Snippet store + provider
// ---------------------------------------------------------------------------

/// One reusable snippet template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snippet {
    /// Display name; surfaced as the completion `label`.
    pub name: String,
    /// Trigger prefix the user types. Snippets only fire when the
    /// completion-context prefix matches this (case-insensitive).
    pub prefix: String,
    /// Body of the snippet --- the text that's inserted on accept.
    pub body: String,
    /// Optional one-line description (rendered as `detail`).
    pub description: Option<String>,
    /// Optional language scope (`Some("rust")` only fires when
    /// the active buffer's language matches; [`None`] = all scopes).
    pub scope: Option<String>,
}

impl Snippet {
    /// Convert this snippet to a [`CompletionItem`] for the framework.
    #[must_use]
    pub fn to_completion_item(&self) -> CompletionItem {
        CompletionItem {
            label: self.name.clone(),
            kind: CompletionItemKind::Snippet,
            detail: self.description.clone(),
            documentation: Some(self.body.clone()),
            insert_text: Some(self.body.clone()),
            sort_text: None,
            filter_text: Some(self.prefix.clone()),
            carry: crate::completion::CompletionEditsCarry::default(),
        }
    }
}

/// Snippet storage. Indexed by snippet name, so adding a snippet
/// with an existing name replaces the prior entry.
#[derive(Clone, Debug, Default)]
pub struct SnippetRegistry {
    snippets: HashMap<String, Snippet>,
}

impl SnippetRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored snippets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.snippets.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snippets.is_empty()
    }

    /// Insert (or replace) a snippet.
    pub fn add(&mut self, snippet: Snippet) {
        self.snippets.insert(snippet.name.clone(), snippet);
    }

    /// Drop a snippet by name. Returns whether something was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.snippets.remove(name).is_some()
    }

    /// Borrow a snippet by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Snippet> {
        self.snippets.get(name)
    }

    /// All snippets in name order.
    #[must_use]
    pub fn list(&self) -> Vec<Snippet> {
        let mut out: Vec<Snippet> = self.snippets.values().cloned().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Snippets whose `prefix` starts with `prefix` (case-
    /// insensitive). Used by [`snippet_provider`].
    #[must_use]
    pub fn find(&self, prefix: &str, language: Option<&str>) -> Vec<Snippet> {
        let lower = prefix.to_lowercase();
        let mut out: Vec<Snippet> = self
            .snippets
            .values()
            .filter(|s| {
                let scope_ok = match (&s.scope, language) {
                    (None, _) => true,
                    (Some(scope), Some(lang)) => scope == lang,
                    (Some(_), None) => false,
                };
                if !scope_ok {
                    return false;
                }
                if lower.is_empty() {
                    return true;
                }
                s.prefix.to_lowercase().starts_with(&lower)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// Cheaply-cloneable shared snippet registry.
pub type SharedSnippetRegistry = Rc<RefCell<SnippetRegistry>>;

// ---------------------------------------------------------------------------
// Built-in providers
// ---------------------------------------------------------------------------

/// How many bytes either side of the cursor dabbrev reads when no
/// setting says otherwise: `completion.dabbrev-window`'s default
/// (E10k.2). Set from E10k.1's measurement: at the end of an 18 MiB
/// buffer, in process, a key that collected cost the base about 90 ms
/// more than one that did not (letters against punctuation at the
/// median, two runs), about 5 ms a MiB read. 256 KiB either side reads
/// 512 KiB, 2.5 ms at the base's rate and less on this path, a sixth of
/// the 16 ms a key may take, and holds about 3,700 lines of 70 bytes
/// either side of the cursor.
pub const DABBREV_WINDOW_DEFAULT: usize = 256 * 1024;

/// The most words dabbrev offers for one prefix.
const DABBREV_MAX: usize = 64;

/// dabbrev: words of the buffer that start with the prefix (other
/// than the prefix itself, case-insensitively), the nearest the cursor
/// first, as Emacs's dabbrev searches outward from point; a word as
/// near before the cursor as another after it comes first. It reads at
/// most `window(buffer)` bytes either side of the cursor, and the one byte
/// beyond each edge the window draws, and offers no word beyond them, nor
/// one the window's edge cuts (E10k.2: before it, every key that collected
/// read the whole buffer, #413). A word wholly inside the window that
/// touches its edge is offered: the byte beyond tells it from a cut one
/// (E10k review 1's Low 4: until fix round 1 every word touching a drawn
/// edge was dropped). Empty prefix
/// returns nothing. Registered ranked
/// ([`CompletionRegistry::register_ranked`]), so its order survives the
/// registry's sort among equal scores.
#[must_use]
pub fn dabbrev_provider(window: impl Fn(Option<BufferId>) -> usize + 'static) -> ProviderFn {
    Box::new(move |ctx: &CompletionContext| -> Vec<CompletionItem> {
        if ctx.prefix.is_empty() {
            return Vec::new();
        }
        let len = ctx.text.len();
        let cursor = ctx.cursor.unwrap_or(len).min(len);
        let reach = window(ctx.buffer);
        let lo = cursor.saturating_sub(reach);
        let hi = cursor.saturating_add(reach).min(len);
        let bytes = ctx.text.bytes(lo, hi);
        // Whether the window's edge cuts a word: the byte beyond each edge
        // it drew is a word byte.
        let word_at = |at: usize| {
            ctx.text
                .bytes(at, at + 1)
                .first()
                .is_some_and(|&b| is_word_byte(b))
        };
        let cut_before = lo > 0 && word_at(lo - 1);
        let cut_after = hi < len && word_at(hi);
        let needle = ctx.prefix.to_lowercase();
        // Every matching word with its distance from the cursor and
        // whether it stands after it.
        let mut found: Vec<(usize, bool, &str)> = Vec::new();
        for (start, end) in word_spans(&bytes) {
            // A word the window's edge cuts goes on past it.
            if (start == 0 && cut_before) || (end == bytes.len() && cut_after) {
                continue;
            }
            // Word bytes are ASCII, so the span is a `str`.
            let Ok(token) = std::str::from_utf8(&bytes[start..end]) else {
                continue;
            };
            if token == ctx.prefix
                || token.len() < needle.len()
                || !token.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
            {
                continue;
            }
            let (start, end) = (lo + start, lo + end);
            let place = if end <= cursor {
                (cursor - end, false)
            } else if start >= cursor {
                (start - cursor, true)
            } else {
                (0, false)
            };
            found.push((place.0, place.1, token));
        }
        found.sort_by_key(|&(distance, after, _)| (distance, after));
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        found
            .into_iter()
            .filter(|&(_, _, token)| seen.insert(token))
            .take(DABBREV_MAX)
            .map(|(_, _, token)| CompletionItem {
                label: token.to_owned(),
                kind: CompletionItemKind::Text,
                detail: None,
                documentation: None,
                insert_text: None,
                sort_text: None,
                filter_text: None,
                carry: crate::completion::CompletionEditsCarry::default(),
            })
            .collect()
    })
}

/// Snippet provider: surface snippets whose trigger prefix matches
/// `ctx.prefix`. Filters by `ctx.language` against `Snippet::scope`.
#[must_use]
pub fn snippet_provider(snippets: SharedSnippetRegistry) -> ProviderFn {
    Box::new(move |ctx: &CompletionContext| -> Vec<CompletionItem> {
        let snips = snippets.borrow();
        snips
            .find(&ctx.prefix, ctx.language.as_deref())
            .into_iter()
            .map(|s| s.to_completion_item())
            .collect()
    })
}

/// Project-symbols provider over a [`crate::project_index::ProjectIndexer`].
/// Looks up symbols whose name matches `ctx.prefix` in the index
/// rooted at `ctx.project_root`.
#[must_use]
pub fn project_symbols_provider(indexer: crate::lua_bindings::SharedProjectIndexer) -> ProviderFn {
    Box::new(move |ctx: &CompletionContext| -> Vec<CompletionItem> {
        let Some(root) = ctx.project_root.as_deref() else {
            return Vec::new();
        };
        if ctx.prefix.is_empty() {
            return Vec::new();
        }
        let ix_ref = indexer.borrow();
        let Some(idx) = ix_ref.get(root) else {
            return Vec::new();
        };
        let hits = idx.search(&ctx.prefix, 64);
        hits.iter()
            .map(|h| {
                let detail = format!(
                    "{}  {}:{}",
                    h.kind.tag(),
                    h.relative_path.display(),
                    h.line + 1,
                );
                CompletionItem {
                    label: h.name.clone(),
                    kind: project_kind_to_completion_kind(&h.kind),
                    detail: Some(detail),
                    documentation: None,
                    insert_text: None,
                    sort_text: None,
                    filter_text: None,
                    carry: crate::completion::CompletionEditsCarry::default(),
                }
            })
            .collect()
    })
}

fn project_kind_to_completion_kind(k: &crate::project_index::SymbolKind) -> CompletionItemKind {
    use crate::project_index::SymbolKind as K;
    match k {
        K::Function => CompletionItemKind::Function,
        K::Method => CompletionItemKind::Method,
        K::Struct => CompletionItemKind::Struct,
        K::Class => CompletionItemKind::Class,
        K::Trait => CompletionItemKind::Interface,
        K::Enum => CompletionItemKind::Enum,
        K::Variable => CompletionItemKind::Variable,
        K::Constant => CompletionItemKind::Constant,
        K::Field => CompletionItemKind::Field,
        K::Module => CompletionItemKind::Module,
        K::Macro => CompletionItemKind::Snippet,
        K::TypeAlias => CompletionItemKind::TypeParameter,
        K::Other(_) => CompletionItemKind::Text,
    }
}

/// LSP completion provider: surface whatever's currently cached in
/// the LSP completion store. The framework does **not** drive a
/// fresh `textDocument/completion` request --- that's the editor's
/// job; we just read whatever the async pipeline has produced so
/// far. Strictly scoped to `ctx.uri` (Q#C8): only that document's
/// entries surface, across all servers keyed to it. **No URI → no
/// LSP candidates** --- an unattached/scratch buffer must never show
/// another file's cached completions (the original global drain did
/// exactly that). The registry's dedup collapses identical entries;
/// the prefix score ranks them.
#[must_use]
pub fn lsp_completion_provider(lsp: crate::lsp::SharedLspManager) -> ProviderFn {
    Box::new(move |ctx: &CompletionContext| -> Vec<CompletionItem> {
        let Some(uri) = ctx.uri.clone() else {
            return Vec::new();
        };
        let store_handle = {
            let mgr = lsp.borrow();
            mgr.completion_store()
        };
        let Ok(store) = store_handle.lock() else {
            return Vec::new();
        };
        let mut out: Vec<CompletionItem> = Vec::new();
        let keys: Vec<_> = store.keys().filter(|k| k.uri == uri).cloned().collect();
        for key in keys {
            for item in store.items(&key) {
                out.push(item.clone());
                if out.len() >= 256 {
                    return out;
                }
            }
        }
        out
    })
}

// ---------------------------------------------------------------------------
// Word tokeniser (used by dabbrev)
// ---------------------------------------------------------------------------

/// A word byte: an ASCII letter, digit or `_`.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The `[start, end)` of every run of word bytes ([`is_word_byte`]) in
/// `bytes`. A byte of a multi-byte character is never a
/// word byte, so these are the words splitting the text at every other
/// character gives.
fn word_spans(bytes: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let is_word = |b: &u8| is_word_byte(*b);
    let mut at = 0;
    std::iter::from_fn(move || {
        let start = at + bytes[at..].iter().position(is_word)?;
        let end = start
            + bytes[start..]
                .iter()
                .position(|b| !is_word(b))
                .unwrap_or(bytes.len() - start);
        at = end;
        Some((start, end))
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, kind: CompletionItemKind) -> CompletionItem {
        CompletionItem {
            label: label.to_owned(),
            kind,
            detail: None,
            documentation: None,
            insert_text: None,
            sort_text: None,
            filter_text: None,
            carry: crate::completion::CompletionEditsCarry::default(),
        }
    }

    fn ctx(prefix: &str, buffer_text: &str) -> CompletionContext {
        CompletionContext::new(prefix, buffer_text)
    }

    fn provider_const(items: Vec<CompletionItem>) -> ProviderFn {
        Box::new(move |_| items.clone())
    }

    #[test]
    fn registry_register_and_collect_passes_through_items() {
        let mut reg = CompletionRegistry::new();
        reg.register(
            "static",
            10,
            provider_const(vec![item("alpha", CompletionItemKind::Function)]),
        );
        let cands = reg.collect(&ctx("al", ""));
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].item.label, "alpha");
        assert_eq!(cands[0].source, "static");
        assert_eq!(cands[0].priority, 10);
        assert!(cands[0].score >= SCORE_PREFIX);
    }

    #[test]
    fn registry_dedups_identical_label_and_insert_text() {
        let mut reg = CompletionRegistry::new();
        let id_a = reg.register(
            "a",
            5,
            provider_const(vec![item("foo", CompletionItemKind::Function)]),
        );
        let _id_b = reg.register(
            "b",
            10,
            provider_const(vec![item("foo", CompletionItemKind::Function)]),
        );
        let cands = reg.collect(&ctx("foo", ""));
        assert_eq!(cands.len(), 1, "duplicate labels must collapse");
        // Higher-priority provider wins.
        assert_eq!(cands[0].source, "b");
        assert_eq!(cands[0].priority, 10);
        // The lower-priority registration is still in the registry,
        // it just lost the dedup race.
        assert_eq!(reg.len(), 2);
        assert!(reg.get(id_a).is_some());
    }

    #[test]
    fn registry_priority_change_takes_effect() {
        let mut reg = CompletionRegistry::new();
        let lo = reg.register(
            "lo",
            1,
            provider_const(vec![item("dup", CompletionItemKind::Function)]),
        );
        let hi = reg.register(
            "hi",
            100,
            provider_const(vec![item("dup", CompletionItemKind::Function)]),
        );
        let cands = reg.collect(&ctx("dup", ""));
        assert_eq!(cands[0].source, "hi");

        reg.set_priority(lo, 1000);
        reg.set_priority(hi, 0);
        let cands2 = reg.collect(&ctx("dup", ""));
        assert_eq!(
            cands2[0].source, "lo",
            "after re-prioritising, lo should win the dedup race"
        );
    }

    #[test]
    fn registry_disabled_provider_contributes_nothing() {
        let mut reg = CompletionRegistry::new();
        let id = reg.register(
            "off",
            10,
            provider_const(vec![item("hidden", CompletionItemKind::Function)]),
        );
        reg.set_enabled(id, false);
        let cands = reg.collect(&ctx("hi", ""));
        assert!(cands.is_empty());
        // Re-enable and we see it again.
        reg.set_enabled(id, true);
        let cands2 = reg.collect(&ctx("hi", ""));
        assert_eq!(cands2.len(), 1);
    }

    #[test]
    fn registry_unregister_removes_provider() {
        let mut reg = CompletionRegistry::new();
        let id = reg.register(
            "x",
            0,
            provider_const(vec![item("foo", CompletionItemKind::Function)]),
        );
        assert_eq!(reg.len(), 1);
        assert!(reg.unregister(id));
        assert_eq!(reg.len(), 0);
        assert!(!reg.unregister(id));
    }

    #[test]
    fn registry_sort_order_is_score_then_priority_then_label() {
        let mut reg = CompletionRegistry::new();
        reg.register(
            "a",
            10,
            provider_const(vec![
                item("zeta_match", CompletionItemKind::Function),
                item("alpha_match", CompletionItemKind::Function),
            ]),
        );
        reg.register(
            "b",
            1,
            provider_const(vec![item("match_yes", CompletionItemKind::Function)]),
        );
        let cands = reg.collect(&ctx("match", ""));
        // "match_yes" is a *prefix* match; the others are
        // word-boundary or substring matches, so it must rank
        // first regardless of provider priority.
        assert_eq!(cands[0].item.label, "match_yes");
    }

    #[test]
    fn dabbrev_finds_buffer_words_with_matching_prefix() {
        let f = dabbrev_provider(|_| DABBREV_WINDOW_DEFAULT);
        let buf = "let parser = Parser::new();\nfn parse_helper() {}\n";
        let cands = f(&ctx("par", buf));
        let names: Vec<_> = cands.iter().map(|i| i.label.as_str()).collect();
        assert!(names.contains(&"parser"));
        assert!(names.contains(&"Parser"));
        assert!(names.contains(&"parse_helper"));
        // The exact prefix itself is filtered out (a dabbrev pop-up
        // is useless if the user types `par<TAB>` and gets `par`).
        assert!(!names.contains(&"par"));
    }

    /// E10k.2: the nearer of two words with the prefix comes first,
    /// before the cursor or after it, and the registry keeps that order
    /// among equal scores where it would otherwise sort by label.
    #[test]
    fn dabbrev_offers_the_nearer_word_first_through_the_registry() {
        let mut reg = CompletionRegistry::new();
        reg.register_ranked("dabbrev", 20, dabbrev_provider(|_| DABBREV_WINDOW_DEFAULT));
        // `zqaa` is far before the cursor, `zqzz` three bytes before it,
        // `zqmm` four after it: nearest first is zqzz, zqmm, zqaa; by
        // label it would be zqaa, zqmm, zqzz.
        let text = format!("zqaa {} zqzz zq    zqmm", "filler ".repeat(100));
        let mut c = ctx("zq", text.as_str());
        c.cursor = Some(text.find("zq    ").expect("fixture") + 2);
        let labels: Vec<String> = reg
            .collect(&c)
            .into_iter()
            .map(|cand| cand.item.label)
            .collect();
        assert_eq!(labels, ["zqzz", "zqmm", "zqaa"]);
    }

    /// E10k.2: dabbrev reads at most its window either side of the
    /// cursor, offers no word beyond it, and none the window's edge cuts.
    #[test]
    fn dabbrev_reads_its_window_and_offers_nothing_beyond_it() {
        let window = 64;
        let f = dabbrev_provider(move |_| window);
        // `zqfar` x50 is [0, 300), an `x` run [300, 380), then `zqcut`
        // [381, 386), `zqnear` [387, 393) and the typed `zq`; the cursor
        // at 396 puts the window at [332, 460), inside the `x` run.
        let text = format!(
            "{}{} zqcut zqnear zq{}",
            "zqfar ".repeat(50),
            "x".repeat(80),
            " y".repeat(200)
        );
        let mut c = ctx("zq", text.as_str());
        c.cursor = Some(396);
        assert_eq!(&text[394..396], "zq", "fixture");
        let labels: Vec<String> = f(&c).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["zqnear", "zqcut"], "zqfar lies beyond the window");
        // The window, and the byte beyond each edge it drew (Low 4).
        assert_eq!(c.text.read_bytes(), 2 * window + 2);
        // The window's end at 8 falls inside `zqlongword`: its first five
        // bytes start with the prefix, and it is still not offered.
        let mut c = ctx("zq", "zq zqlongword");
        c.cursor = Some(2);
        let f = dabbrev_provider(|_| 6);
        assert!(f(&c).is_empty(), "a word the window cuts is not offered");
    }

    /// E10k.2: a context reads nothing until a provider asks, and the
    /// whole text once however often it is asked for.
    #[test]
    fn a_context_reads_what_its_providers_ask_and_the_whole_text_once() {
        let mut reg = CompletionRegistry::new();
        reg.register("quiet", 0, provider_const(Vec::new()));
        let c = ctx("zq", "one two three");
        let _ = reg.collect(&c);
        assert_eq!(reg.last_read_bytes(), 0);
        reg.register(
            "whole",
            0,
            Box::new(|c: &CompletionContext| {
                let _ = c.text.whole();
                let _ = c.text.whole();
                Vec::new()
            }),
        );
        let _ = reg.collect(&c);
        assert_eq!(reg.last_read_bytes(), "one two three".len());
    }

    #[test]
    fn dabbrev_returns_empty_on_empty_prefix() {
        let f = dabbrev_provider(|_| DABBREV_WINDOW_DEFAULT);
        let cands = f(&ctx("", "alpha beta gamma"));
        assert!(cands.is_empty());
    }

    #[test]
    fn snippet_registry_add_remove_list() {
        let mut snips = SnippetRegistry::new();
        snips.add(Snippet {
            name: "fn".into(),
            prefix: "fn".into(),
            body: "fn ${1:name}() {\n  $0\n}".into(),
            description: Some("function".into()),
            scope: Some("rust".into()),
        });
        snips.add(Snippet {
            name: "for".into(),
            prefix: "for".into(),
            body: "for $1 in $2 {\n  $0\n}".into(),
            description: None,
            scope: Some("rust".into()),
        });
        assert_eq!(snips.len(), 2);
        let listed = snips.list();
        assert_eq!(listed[0].name, "fn");
        assert_eq!(listed[1].name, "for");
        assert!(snips.remove("fn"));
        assert_eq!(snips.len(), 1);
        assert!(!snips.remove("fn"));
    }

    #[test]
    fn snippet_registry_find_filters_by_prefix_and_scope() {
        let mut snips = SnippetRegistry::new();
        snips.add(Snippet {
            name: "fn-rust".into(),
            prefix: "fn".into(),
            body: "fn _name_() {}".into(),
            description: None,
            scope: Some("rust".into()),
        });
        snips.add(Snippet {
            name: "function-lua".into(),
            prefix: "fn".into(),
            body: "function _name_() end".into(),
            description: None,
            scope: Some("lua".into()),
        });
        snips.add(Snippet {
            name: "fn-any".into(),
            prefix: "fn".into(),
            body: "fn ()".into(),
            description: None,
            scope: None,
        });
        let rust = snips.find("fn", Some("rust"));
        let names: Vec<_> = rust.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"fn-rust"));
        assert!(names.contains(&"fn-any"));
        assert!(!names.contains(&"function-lua"));

        let none = snips.find("xy", Some("rust"));
        assert!(none.is_empty());
    }

    #[test]
    fn snippet_provider_emits_items() {
        let snips = Rc::new(RefCell::new(SnippetRegistry::new()));
        snips.borrow_mut().add(Snippet {
            name: "fn".into(),
            prefix: "fn".into(),
            body: "fn name() {}".into(),
            description: Some("fn template".into()),
            scope: Some("rust".into()),
        });
        let f = snippet_provider(snips);
        let mut c = ctx("fn", "");
        c.language = Some("rust".into());
        let cands = f(&c);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].label, "fn");
        assert_eq!(cands[0].kind, CompletionItemKind::Snippet);
        assert_eq!(cands[0].insert_text.as_deref(), Some("fn name() {}"));
    }

    #[test]
    fn lua_style_custom_provider_via_closure() {
        // The Lua surface wraps a `mlua::Function` in a closure;
        // here we mimic that with a plain closure to verify the
        // boxing pathway works end-to-end.
        let counter = Rc::new(RefCell::new(0));
        let counter_clone = counter.clone();
        let f: ProviderFn = Box::new(move |c: &CompletionContext| {
            *counter_clone.borrow_mut() += 1;
            vec![CompletionItem {
                label: format!("hello-{}", c.prefix),
                kind: CompletionItemKind::Text,
                detail: None,
                documentation: None,
                insert_text: None,
                sort_text: None,
                filter_text: None,
                carry: crate::completion::CompletionEditsCarry::default(),
            }]
        });
        let mut reg = CompletionRegistry::new();
        let _id = reg.register("custom", 0, f);
        let cands = reg.collect(&ctx("xy", ""));
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].item.label, "hello-xy");
        assert_eq!(*counter.borrow(), 1);
    }

    #[test]
    fn empty_prefix_zero_score_preserves_order() {
        let mut reg = CompletionRegistry::new();
        reg.register(
            "p",
            0,
            provider_const(vec![
                item("zeta", CompletionItemKind::Function),
                item("alpha", CompletionItemKind::Function),
            ]),
        );
        let cands = reg.collect(&ctx("", ""));
        assert_eq!(cands.len(), 2);
        // With equal scores+priority, alphabetical wins.
        assert_eq!(cands[0].item.label, "alpha");
        assert_eq!(cands[1].item.label, "zeta");
    }

    #[test]
    fn substring_match_below_prefix() {
        let mut reg = CompletionRegistry::new();
        reg.register(
            "p",
            0,
            provider_const(vec![
                item("XparseY", CompletionItemKind::Function),
                item("parse_thing", CompletionItemKind::Function),
            ]),
        );
        let cands = reg.collect(&ctx("parse", ""));
        assert_eq!(cands[0].item.label, "parse_thing");
    }
}
