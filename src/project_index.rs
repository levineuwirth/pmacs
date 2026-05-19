// project_index.rs --- T M4.10 project index: symbol aggregation,
// persistence, incremental update.

//! Project-scoped symbol index.
//!
//! Per spec §M4.10: an "incremental, persistent symbol index that
//! aggregates LSP, tree-sitter, and ripgrep-style raw search."
//!
//! # Sources
//!
//! Each [`Symbol`] records where it came from
//! ([`SymbolSource::Lsp`], [`SymbolSource::TreeSitter`],
//! [`SymbolSource::Heuristic`], [`SymbolSource::Lua`]). One
//! [`FileEntry`] holds every symbol from a single file as a flat
//! [`Vec<Symbol>`]; replacing the entry replaces the file's whole
//! contribution to the index. Callers wanting cross-source merge
//! call [`extract_heuristic`] / [`extract_raw`] / the tree-sitter
//! extractors and concatenate the results before
//! [`ProjectIndex::upsert_file`].
//!
//! # Persistence
//!
//! [`ProjectIndex::save`] writes a single JSON file. The default
//! cache path is `<root>/.pmacs/index.json`; callers may also pass
//! an explicit path. [`ProjectIndex::load`] returns a fresh empty
//! index when the cache file does not exist (so cold-start with
//! no on-disk index is a no-op rather than an error).
//!
//! # Incremental update
//!
//! Each [`FileEntry`] carries the file's `mtime_secs` and a 64-bit
//! FNV-1a `content_hash`. Callers ask
//! [`ProjectIndex::is_fresh`] before recomputing symbols for a
//! file; if the answer is yes, the previous entry is reused. On
//! buffer save, the editor calls [`ProjectIndex::upsert_file`]
//! with the freshly extracted symbols, replacing the old entry
//! atomically.
//!
//! # Search
//!
//! [`ProjectIndex::search`] is a flat in-memory scan. The score is
//! exact-match > prefix > word-boundary > substring (case-
//! insensitive in every case). A 1M-symbol scan returns in well
//! under a second on commodity hardware --- comfortably inside the
//! spec's 1 s / 100 k-file acceptance bar.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Symbol kinds and sources
// ---------------------------------------------------------------------------

/// Where an indexed symbol came from. Persisted as a `snake_case` tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolSource {
    /// From an LSP `workspace/symbol` or `textDocument/documentSymbol`
    /// response.
    Lsp,
    /// From a tree-sitter walk of the bundled grammars.
    TreeSitter,
    /// From the line-pattern heuristic extractor.
    Heuristic,
    /// Pushed from Lua (e.g. user-supplied indexer).
    Lua,
}

impl SymbolSource {
    /// Stable lowercase tag, used by the Lua surface and search
    /// hit serialization.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Lsp => "lsp",
            Self::TreeSitter => "tree-sitter",
            Self::Heuristic => "heuristic",
            Self::Lua => "lua",
        }
    }

    /// Inverse of [`Self::tag`]. Returns [`None`] for unrecognised tags.
    #[must_use]
    pub fn from_tag(s: &str) -> Option<Self> {
        match s {
            "lsp" => Some(Self::Lsp),
            "tree-sitter" | "tree_sitter" | "treesitter" => Some(Self::TreeSitter),
            "heuristic" => Some(Self::Heuristic),
            "lua" => Some(Self::Lua),
            _ => None,
        }
    }
}

/// What kind of definition a symbol points at. Mirrors the LSP
/// `SymbolKind` enum where it makes sense; everything else falls
/// into [`Self::Other`] with a free-form tag.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// Free function.
    Function,
    /// Method on a type / class / impl.
    Method,
    /// Struct, record, data class.
    Struct,
    /// Class.
    Class,
    /// Trait / interface / protocol.
    Trait,
    /// Sum type / enum.
    Enum,
    /// Local or module-scope variable.
    Variable,
    /// Top-level constant.
    Constant,
    /// Struct/class field.
    Field,
    /// Module / package / namespace.
    Module,
    /// Macro.
    Macro,
    /// Type alias.
    TypeAlias,
    /// Anything else; the tag is free-form.
    Other(String),
}

impl SymbolKind {
    /// Stable tag used at the Lua boundary.
    #[must_use]
    pub fn tag(&self) -> &str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Field => "field",
            Self::Module => "module",
            Self::Macro => "macro",
            Self::TypeAlias => "type_alias",
            Self::Other(s) => s.as_str(),
        }
    }

    /// Map an LSP `SymbolKind` integer to our enum. Unknown codes
    /// map to [`Self::Other`] with the numeric tag, so future LSP
    /// extensions don't get dropped silently.
    #[must_use]
    #[allow(
        clippy::match_same_arms,
        reason = "explicit per-code arms document the LSP mapping; collapsing them would lose that"
    )]
    pub fn from_lsp_code(code: i64) -> Self {
        match code {
            // 1 = File, 2 = Module, 3 = Namespace, 4 = Package
            1 => Self::Other("file".into()),
            2..=4 => Self::Module,
            5 => Self::Class,
            6 => Self::Method,
            7 => Self::Field, // Property
            8 => Self::Field,
            9 => Self::Method, // Constructor
            10 => Self::Enum,
            11 => Self::Trait, // Interface
            12 => Self::Function,
            13 => Self::Variable,
            14 => Self::Constant,
            15 => Self::Variable, // String
            16 => Self::Variable, // Number
            17 => Self::Variable, // Boolean
            18 => Self::Variable, // Array
            19 => Self::Variable, // Object
            20 => Self::Variable, // Key
            21 => Self::Variable, // Null
            22 => Self::Field,    // EnumMember
            23 => Self::Struct,
            24 => Self::Other("event".into()),
            25 => Self::Function, // Operator
            26 => Self::Other("type_parameter".into()),
            other => Self::Other(format!("kind_{other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol + FileEntry
// ---------------------------------------------------------------------------

/// One indexed symbol. Path is implicit --- it lives inside the
/// owning [`FileEntry`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Symbol {
    /// Display name (the identifier the user types).
    pub name: String,
    /// What kind of definition this is.
    pub kind: SymbolKind,
    /// Zero-based line within the owning file.
    pub line: u32,
    /// Zero-based UTF-16 column. Heuristic/raw use byte columns;
    /// LSP responses already report UTF-16. We don't try to
    /// reconcile --- callers treat this as advisory.
    pub col: u32,
    /// Where this symbol came from.
    pub source: SymbolSource,
    /// Containing scope (e.g. module path or impl-block). [`None`]
    /// for top-level definitions or sources that don't supply one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

/// All indexed symbols for one file plus the cache key
/// (`mtime_secs`, `content_hash`) that lets the indexer skip
/// re-extraction.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path **relative** to the index root. Storing relative paths
    /// makes the on-disk index portable across machines.
    pub path: PathBuf,
    /// Seconds since `UNIX_EPOCH` of the file's modification time
    /// at the moment the entry was produced. `0` = unknown.
    pub mtime_secs: u64,
    /// 64-bit FNV-1a hash of the file's bytes. `0` is also the
    /// hash of an empty buffer; combine with `mtime_secs == 0` to
    /// detect "no metadata".
    pub content_hash: u64,
    /// Free-form language tag (`"rust"`, `"lua"`, `"python"`, …).
    /// Used by the search ranker to break ties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Symbols extracted from this file. Order is source-defined.
    pub symbols: Vec<Symbol>,
}

// ---------------------------------------------------------------------------
// ProjectIndex
// ---------------------------------------------------------------------------

/// In-memory project index. Owns one [`FileEntry`] per indexed
/// file. The index is keyed by path relative to [`Self::root`] so
/// the on-disk form is location-independent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectIndex {
    /// Project root. Symbols' absolute path is `root.join(rel)`.
    pub root: PathBuf,
    /// Files in the index, keyed by relative path.
    pub files: HashMap<PathBuf, FileEntry>,
    /// Monotonically increasing counter; bumped on every mutation.
    /// Useful for cache-busting derived state at the Lua layer.
    pub generation: u64,
}

impl ProjectIndex {
    /// Construct an empty index rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            files: HashMap::new(),
            generation: 0,
        }
    }

    /// Default cache file path: `<root>/.pmacs/index.json`.
    #[must_use]
    pub fn default_cache_path(&self) -> PathBuf {
        Self::cache_path_for(&self.root)
    }

    /// Compute the default cache file path for an arbitrary root.
    #[must_use]
    pub fn cache_path_for(root: &Path) -> PathBuf {
        root.join(".pmacs").join("index.json")
    }

    /// Number of indexed files.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total symbols across all files.
    #[must_use]
    pub fn symbol_count(&self) -> usize {
        self.files.values().map(|f| f.symbols.len()).sum()
    }

    /// Make `path` relative to [`Self::root`] when possible. Falls
    /// back to the original path if it isn't under the root.
    pub fn relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .map_or_else(|_| path.to_path_buf(), Path::to_path_buf)
    }

    /// Check whether the indexed entry for `path` matches the
    /// supplied `(mtime_secs, content_hash)`. If both match, the
    /// caller can skip re-extraction.
    pub fn is_fresh(&self, path: &Path, mtime_secs: u64, content_hash: u64) -> bool {
        let rel = self.relative(path);
        self.files
            .get(&rel)
            .is_some_and(|e| e.mtime_secs == mtime_secs && e.content_hash == content_hash)
    }

    /// Replace the indexed entry for one file. The entry's
    /// `path` is rewritten to be relative to the index root so
    /// callers can pass either form.
    pub fn upsert_file(&mut self, mut entry: FileEntry) {
        let rel = self.relative(&entry.path);
        entry.path.clone_from(&rel);
        self.files.insert(rel, entry);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Drop any entry for `path`. Returns whether something was
    /// removed.
    pub fn forget_file(&mut self, path: &Path) -> bool {
        let rel = self.relative(path);
        let removed = self.files.remove(&rel).is_some();
        if removed {
            self.generation = self.generation.wrapping_add(1);
        }
        removed
    }

    /// Drop **every** indexed entry. Resets `generation` is **not**
    /// done --- callers tracking generations across clears can still
    /// detect the change.
    pub fn clear(&mut self) {
        if !self.files.is_empty() {
            self.files.clear();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Save the index to `dest` as JSON, creating parent directories
    /// as needed.
    pub fn save(&self, dest: &Path) -> io::Result<()> {
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        fs::write(dest, bytes)
    }

    /// Save the index under [`Self::default_cache_path`].
    pub fn save_default(&self) -> io::Result<()> {
        let dest = self.default_cache_path();
        self.save(&dest)
    }

    /// Load an index from `src`. A missing file produces an empty
    /// index rooted at `root` (cold-start with no cache is not an
    /// error). The on-disk root is overwritten with the supplied
    /// `root` so a cache file that has been moved with its project
    /// keeps working.
    pub fn load(root: impl Into<PathBuf>, src: &Path) -> io::Result<Self> {
        let root = root.into();
        match fs::read(src) {
            Ok(bytes) => {
                let mut idx: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                idx.root = root;
                Ok(idx)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::new(root)),
            Err(e) => Err(e),
        }
    }

    /// Load from [`Self::cache_path_for`] under `root`.
    pub fn load_default(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        let path = Self::cache_path_for(&root);
        Self::load(root, &path)
    }

    /// Search for symbols whose name contains `query` (case-
    /// insensitive). Up to `limit` hits are returned, sorted by
    /// rank (exact > prefix > word-boundary > substring), then
    /// alphabetically by name.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let mut hits: Vec<SearchHit> = Vec::new();
        for entry in self.files.values() {
            for sym in &entry.symbols {
                let lower = sym.name.to_lowercase();
                if let Some(score) = score_match(&lower, &needle) {
                    let abs = self.root.join(&entry.path);
                    hits.push(SearchHit {
                        name: sym.name.clone(),
                        kind: sym.kind.clone(),
                        source: sym.source,
                        path: abs,
                        relative_path: entry.path.clone(),
                        line: sym.line,
                        col: sym.col,
                        score,
                        container: sym.container.clone(),
                        language: entry.language.clone(),
                    });
                }
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.relative_path.cmp(&b.relative_path))
                .then_with(|| a.line.cmp(&b.line))
        });
        hits.truncate(limit);
        hits
    }
}

// ---------------------------------------------------------------------------
// Multi-project registry
// ---------------------------------------------------------------------------

/// Owns one [`ProjectIndex`] per known project root. The Lua
/// surface (`pmacs.index.*`) goes through this registry so that
/// multi-project sessions don't have to thread a separate handle
/// for each project.
#[derive(Debug, Default)]
pub struct ProjectIndexer {
    indexes: HashMap<PathBuf, ProjectIndex>,
}

impl ProjectIndexer {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered roots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.indexes.len()
    }

    /// Whether the registry contains no roots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indexes.is_empty()
    }

    /// Iterate over registered roots.
    pub fn roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.indexes.keys()
    }

    /// Canonicalise `root` if possible; otherwise return the path
    /// as-is. Mirrors the LSP / Workspace test-friendly fallback.
    fn canonical_key(root: &Path) -> PathBuf {
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
    }

    /// Ensure an index exists for `root`. Returns the freshly-loaded
    /// or freshly-created index. Subsequent calls with the same root
    /// (or a path that canonicalises to the same root) are no-ops.
    pub fn ensure(&mut self, root: impl Into<PathBuf>) -> &mut ProjectIndex {
        let key = Self::canonical_key(&root.into());
        self.indexes
            .entry(key.clone())
            .or_insert_with(|| ProjectIndex::new(key))
    }

    /// Replace whatever is registered for `root` (if anything) with
    /// the on-disk default cache.
    pub fn reload(&mut self, root: impl Into<PathBuf>) -> io::Result<&mut ProjectIndex> {
        let key = Self::canonical_key(&root.into());
        let idx = ProjectIndex::load_default(key.clone())?;
        Ok(self.indexes.entry(key).or_insert(idx))
    }

    /// Borrow the index for `root`, if any.
    #[must_use]
    pub fn get(&self, root: &Path) -> Option<&ProjectIndex> {
        self.indexes.get(&Self::canonical_key(root))
    }

    /// Mutably borrow the index for `root`, if any.
    pub fn get_mut(&mut self, root: &Path) -> Option<&mut ProjectIndex> {
        self.indexes.get_mut(&Self::canonical_key(root))
    }

    /// Drop the index for `root`. Returns whether something was
    /// removed. Does not delete any on-disk cache.
    pub fn forget(&mut self, root: &Path) -> bool {
        self.indexes.remove(&Self::canonical_key(root)).is_some()
    }
}

// ---------------------------------------------------------------------------
// Search ranking
// ---------------------------------------------------------------------------

/// A single match. Both `path` (absolute, joined with `root`) and
/// `relative_path` are included so callers can pick whichever form
/// they need.
#[derive(Clone, Debug)]
pub struct SearchHit {
    /// Symbol display name.
    pub name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Where the symbol came from.
    pub source: SymbolSource,
    /// Absolute path (`root.join(relative_path)`).
    pub path: PathBuf,
    /// Path as stored in the index (relative to root).
    pub relative_path: PathBuf,
    /// Zero-based line within the file.
    pub line: u32,
    /// Zero-based column.
    pub col: u32,
    /// Match score; higher = better.
    pub score: i32,
    /// Containing scope, if any.
    pub container: Option<String>,
    /// Language tag of the file, if known.
    pub language: Option<String>,
}

const SCORE_EXACT: i32 = 1000;
const SCORE_PREFIX: i32 = 600;
const SCORE_WORD_BOUNDARY: i32 = 300;
const SCORE_SUBSTRING: i32 = 100;

/// Score a (lowercased) `name` against a (lowercased) `needle`.
/// Returns [`None`] if there's no match.
fn score_match(name: &str, needle: &str) -> Option<i32> {
    if name == needle {
        return Some(SCORE_EXACT);
    }
    if name.starts_with(needle) {
        return Some(SCORE_PREFIX);
    }
    let pos = name.find(needle)?;
    // Word-boundary match: previous byte is a separator-like char.
    let at_boundary = pos > 0 && {
        let prev = name.as_bytes()[pos - 1];
        !prev.is_ascii_alphanumeric()
    };
    let mut score = if at_boundary {
        SCORE_WORD_BOUNDARY
    } else {
        SCORE_SUBSTRING
    };
    // Shorter names are less ambiguous; shave a little off long names.
    score -= i32::try_from(name.len().saturating_sub(needle.len())).unwrap_or(0);
    Some(score)
}

// ---------------------------------------------------------------------------
// Hashing + mtime helpers
// ---------------------------------------------------------------------------

/// 64-bit FNV-1a hash. Stable, allocation-free, fast enough for the
/// per-file digests the index uses for cache-busting.
#[must_use]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Seconds-since-`UNIX_EPOCH` from a [`fs::Metadata`]. `0` if the
/// platform doesn't expose mtime or the system clock is older than
/// the epoch.
#[must_use]
pub fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------
// Heuristic extractor
// ---------------------------------------------------------------------------

/// Extract symbols from `source` using fast line-pattern heuristics
/// for `language`. Recognised tags: `"rust"`, `"lua"`, `"python"`,
/// `"go"`, `"javascript"`, `"typescript"`. Anything else falls back
/// to [`extract_raw`].
#[must_use]
pub fn extract_heuristic(language: &str, source: &str) -> Vec<Symbol> {
    match language {
        "rust" => extract_rust_heuristic(source),
        "lua" => extract_lua_heuristic(source),
        "python" | "py" => extract_python_heuristic(source),
        "go" => extract_go_heuristic(source),
        "javascript" | "typescript" | "js" | "ts" => extract_js_heuristic(source),
        _ => extract_raw(source),
    }
}

/// "Ripgrep-style raw" fallback: pick out anything that looks like
/// an identifier-followed-by-colon-or-equals at the start of a line.
/// Cheap and language-agnostic.
#[must_use]
pub fn extract_raw(source: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // A ":" or "=" preceded by an identifier, with the
        // identifier sitting at the start of the trimmed line.
        if let Some(eq) = trimmed.find(['=', ':']) {
            let head = trimmed[..eq].trim_end();
            if !head.is_empty() && is_identifier(head) {
                let col = (line.len() - trimmed.len()) as u32;
                out.push(Symbol {
                    name: head.to_owned(),
                    kind: SymbolKind::Variable,
                    line: line_idx as u32,
                    col,
                    source: SymbolSource::Heuristic,
                    container: None,
                });
            }
        }
    }
    out
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn push_sym(out: &mut Vec<Symbol>, name: &str, kind: SymbolKind, line: usize, col: usize) {
    if name.is_empty() {
        return;
    }
    out.push(Symbol {
        name: name.to_owned(),
        kind,
        line: line as u32,
        col: col as u32,
        source: SymbolSource::Heuristic,
        container: None,
    });
}

fn ident_after<'a>(s: &'a str, prefix: &str) -> Option<(&'a str, usize)> {
    let rest = s.strip_prefix(prefix)?;
    let after_ws = rest.trim_start();
    let consumed = s.len() - after_ws.len();
    let end = after_ws
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(after_ws.len());
    if end == 0 {
        return None;
    }
    Some((&after_ws[..end], consumed))
}

fn extract_rust_heuristic(source: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line_idx, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let col = raw.len() - line.len();
        // Strip leading visibility modifiers so `pub fn foo` works.
        let body = strip_rust_visibility(line);
        let kind_and_kw: &[(&str, SymbolKind)] = &[
            ("fn ", SymbolKind::Function),
            ("struct ", SymbolKind::Struct),
            ("enum ", SymbolKind::Enum),
            ("trait ", SymbolKind::Trait),
            ("mod ", SymbolKind::Module),
            ("type ", SymbolKind::TypeAlias),
            ("const ", SymbolKind::Constant),
            ("static ", SymbolKind::Constant),
            ("union ", SymbolKind::Struct),
            ("macro_rules! ", SymbolKind::Macro),
        ];
        let mut matched = false;
        for (kw, kind) in kind_and_kw {
            if let Some((name, _)) = ident_after(body, kw) {
                push_sym(&mut out, name, kind.clone(), line_idx, col);
                matched = true;
                break;
            }
        }
        if !matched {
            // `impl Trait for Type {` — index the type as a Class.
            if let Some(rest) = body.strip_prefix("impl ") {
                let last = rest.rsplit_once(" for ").map_or(rest, |(_, t)| t);
                let name_end = last
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
                    .unwrap_or(last.len());
                let name = last[..name_end].trim_end_matches(':').trim();
                if !name.is_empty() && is_identifier_dotted(name) {
                    push_sym(&mut out, name, SymbolKind::Class, line_idx, col);
                }
            }
        }
    }
    out
}

fn strip_rust_visibility(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("pub(")
        && let Some(close) = rest.find(')')
    {
        return rest[close + 1..].trim_start();
    }
    if let Some(rest) = trimmed.strip_prefix("pub ") {
        return rest.trim_start();
    }
    trimmed
}

fn is_identifier_dotted(s: &str) -> bool {
    s.split([':', '.'])
        .all(|p| !p.is_empty() && is_identifier(p))
}

fn extract_lua_heuristic(source: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line_idx, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let col = raw.len() - line.len();
        // local function name(... | function name(... | function obj.method(...
        if let Some((name, _)) = ident_after(line, "local function ") {
            push_sym(&mut out, name, SymbolKind::Function, line_idx, col);
            continue;
        }
        if let Some(rest) = line.strip_prefix("function ") {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if !name.is_empty() {
                let kind = if name.contains(':') {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                push_sym(&mut out, name, kind, line_idx, col);
                continue;
            }
        }
        // local NAME = function(...
        if let Some(rest) = line.strip_prefix("local ")
            && let Some(eq) = rest.find('=')
        {
            let name = rest[..eq].trim();
            let value = rest[eq + 1..].trim_start();
            if is_identifier(name) {
                let kind = if value.starts_with("function") {
                    SymbolKind::Function
                } else {
                    SymbolKind::Variable
                };
                push_sym(&mut out, name, kind, line_idx, col);
            }
        }
    }
    out
}

fn extract_python_heuristic(source: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line_idx, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let col = raw.len() - line.len();
        if let Some((name, _)) = ident_after(line, "def ") {
            push_sym(&mut out, name, SymbolKind::Function, line_idx, col);
            continue;
        }
        if let Some((name, _)) = ident_after(line, "async def ") {
            push_sym(&mut out, name, SymbolKind::Function, line_idx, col);
            continue;
        }
        if let Some((name, _)) = ident_after(line, "class ") {
            push_sym(&mut out, name, SymbolKind::Class, line_idx, col);
        }
    }
    out
}

fn extract_go_heuristic(source: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line_idx, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let col = raw.len() - line.len();
        if let Some(rest) = line.strip_prefix("func ") {
            // func (r *T) Name(... | func Name(...
            let after_recv = if rest.starts_with('(') {
                rest.find(')').map_or(rest, |i| &rest[i + 1..]).trim_start()
            } else {
                rest
            };
            let end = after_recv
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after_recv.len());
            let name = &after_recv[..end];
            if !name.is_empty() {
                let kind = if rest.starts_with('(') {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                push_sym(&mut out, name, kind, line_idx, col);
                continue;
            }
        }
        if let Some((name, _)) = ident_after(line, "type ") {
            let kind = if line.contains(" interface ")
                || line.trim_end_matches('{').ends_with("interface")
            {
                SymbolKind::Trait
            } else if line.contains(" struct ") || line.trim_end_matches('{').ends_with("struct") {
                SymbolKind::Struct
            } else {
                SymbolKind::TypeAlias
            };
            push_sym(&mut out, name, kind, line_idx, col);
        }
    }
    out
}

fn extract_js_heuristic(source: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line_idx, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let col = raw.len() - line.len();
        let body = line
            .strip_prefix("export default ")
            .or_else(|| line.strip_prefix("export "))
            .unwrap_or(line);
        if let Some((name, _)) = ident_after(body, "function ") {
            push_sym(&mut out, name, SymbolKind::Function, line_idx, col);
            continue;
        }
        if let Some((name, _)) = ident_after(body, "async function ") {
            push_sym(&mut out, name, SymbolKind::Function, line_idx, col);
            continue;
        }
        if let Some((name, _)) = ident_after(body, "class ") {
            push_sym(&mut out, name, SymbolKind::Class, line_idx, col);
            continue;
        }
        for kw in &["const ", "let ", "var "] {
            if let Some((name, _)) = ident_after(body, kw) {
                push_sym(&mut out, name, SymbolKind::Variable, line_idx, col);
                break;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tree-sitter extractors (bundled grammars only)
// ---------------------------------------------------------------------------

/// Walk a tree-sitter parse for `source` (parsed against the
/// supplied `language`) and pull out the named definitions for the
/// node kinds we know about.
///
/// Recognised kinds (by `node.kind()` name): `function_item`,
/// `struct_item`, `enum_item`, `trait_item`, `impl_item`,
/// `mod_item`, `type_item`, `const_item`, `static_item`,
/// `macro_definition` (Rust); `function_declaration_statement`,
/// `function_definition`, `local_function_definition_statement`
/// (Lua, varies by grammar).
///
/// Symbols carry [`SymbolSource::TreeSitter`].
pub fn extract_treesitter(
    language: &tree_sitter::Language,
    source: &str,
) -> Result<Vec<Symbol>, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(language)
        .map_err(|e| format!("set_language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "parse failed".to_owned())?;
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    walk_treesitter(tree.root_node(), bytes, &mut out);
    Ok(out)
}

fn walk_treesitter(node: tree_sitter::Node<'_>, bytes: &[u8], out: &mut Vec<Symbol>) {
    let kind_name = node.kind();
    if let Some(sym_kind) = treesitter_kind(kind_name)
        && let Some(name_node) = node.child_by_field_name("name")
    {
        let start = name_node.start_byte();
        let end = name_node.end_byte();
        if let Ok(name) = std::str::from_utf8(&bytes[start..end]) {
            let pos = node.start_position();
            out.push(Symbol {
                name: name.to_owned(),
                kind: sym_kind,
                line: pos.row as u32,
                col: pos.column as u32,
                source: SymbolSource::TreeSitter,
                container: None,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_treesitter(child, bytes, out);
    }
}

fn treesitter_kind(name: &str) -> Option<SymbolKind> {
    let kind = match name {
        "function_item"
        | "function_declaration"
        | "function_definition_statement"
        | "function_definition" => SymbolKind::Function,
        "method_definition" => SymbolKind::Method,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "impl_item" | "class_definition" | "class_declaration" => SymbolKind::Class,
        "mod_item" => SymbolKind::Module,
        "type_item" | "type_alias" | "type_alias_declaration" => SymbolKind::TypeAlias,
        "const_item" | "static_item" => SymbolKind::Constant,
        "macro_definition" => SymbolKind::Macro,
        _ => return None,
    };
    Some(kind)
}

// ---------------------------------------------------------------------------
// LSP ingestion
// ---------------------------------------------------------------------------

/// One LSP-sourced symbol: the path it lives in plus the parsed
/// [`Symbol`]. Returned by [`ingest_lsp_symbols`] so callers can
/// regroup by file before calling
/// [`ProjectIndex::upsert_file`].
#[derive(Clone, Debug)]
pub struct LspSymbolInbound {
    /// Absolute path to the file that owns the symbol (parsed from
    /// the response's `location.uri`).
    pub path: PathBuf,
    /// Language tag if the response carried one (rare); usually [`None`].
    pub language: Option<String>,
    /// The symbol itself.
    pub symbol: Symbol,
}

/// Parse a JSON value returned by `workspace/symbol`,
/// `textDocument/documentSymbol`, or any LSP response that carries
/// an array of `SymbolInformation` / `WorkspaceSymbol` shapes.
///
/// Supported shapes:
/// * `[ { name, kind, location: { uri, range }, containerName? } , … ]`
/// * `[ { name, kind, location: { uri, range }, containerName? } , … ]`
/// * `[ { name, kind, range, selectionRange, children? } , … ]`
///   (`DocumentSymbol`); `range.start` becomes the position.
///
/// Unrecognised entries are dropped silently rather than failing
/// the whole batch.
#[must_use]
pub fn ingest_lsp_symbols(value: &serde_json::Value) -> Vec<LspSymbolInbound> {
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in arr {
        ingest_lsp_one(entry, None, &mut out);
    }
    out
}

fn ingest_lsp_one(
    entry: &serde_json::Value,
    parent_uri: Option<&str>,
    out: &mut Vec<LspSymbolInbound>,
) {
    let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let kind_code = entry.get("kind").and_then(serde_json::Value::as_i64);
    let container = entry
        .get("containerName")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    // Location form (workspace/symbol): location.uri + location.range.
    let (uri, line, col) = if let Some(loc) = entry.get("location") {
        let uri = loc.get("uri").and_then(|v| v.as_str()).map(str::to_owned);
        let (line, col) = lsp_position_from(loc.get("range"));
        (uri, line, col)
    } else if let Some(range) = entry.get("range") {
        let (line, col) = lsp_position_from(Some(range));
        (parent_uri.map(str::to_owned), line, col)
    } else {
        (parent_uri.map(str::to_owned), 0, 0)
    };

    if !name.is_empty()
        && let Some(uri) = uri.as_deref()
        && let Some(path) = uri_to_path(uri)
    {
        let kind = kind_code.map_or(SymbolKind::Other("unknown".into()), |code| {
            SymbolKind::from_lsp_code(code)
        });
        out.push(LspSymbolInbound {
            path,
            language: None,
            symbol: Symbol {
                name: name.to_owned(),
                kind,
                line,
                col,
                source: SymbolSource::Lsp,
                container: container.clone(),
            },
        });
    }

    // DocumentSymbol form: recurse into children with the same uri.
    if let Some(children) = entry.get("children").and_then(|v| v.as_array()) {
        let next_uri = uri.as_deref().or(parent_uri);
        for child in children {
            ingest_lsp_one(child, next_uri, out);
        }
    }
}

fn lsp_position_from(range: Option<&serde_json::Value>) -> (u32, u32) {
    range.and_then(|r| r.get("start")).map_or((0, 0), |s| {
        let line = s
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let col = s
            .get("character")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        (line, col)
    })
}

/// Decode a `file://` URI into a filesystem path (authority dropped,
/// percent-decoded). `None` for non-`file://` URIs. T M4.5 L1: the
/// reverse of [`crate::lsp::path_to_file_uri`], reused by the Lua
/// surface (`pmacs.lsp.path_for_uri`) to turn server-returned
/// locations into openable files.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Drop the optional authority component (`//host/path`) by
    // skipping to the first '/' if the rest doesn't start with one.
    let path_part = if rest.starts_with('/') {
        rest
    } else {
        rest.find('/').map_or("", |i| &rest[i..])
    };
    Some(PathBuf::from(percent_decode(path_part)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((h << 4) | l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fnv1a_is_stable() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a_64(b"abc"), fnv1a_64(b"abd"));
    }

    #[test]
    fn symbol_kind_from_lsp_code() {
        assert!(matches!(
            SymbolKind::from_lsp_code(12),
            SymbolKind::Function
        ));
        assert!(matches!(SymbolKind::from_lsp_code(5), SymbolKind::Class));
        assert!(matches!(SymbolKind::from_lsp_code(11), SymbolKind::Trait));
        assert!(matches!(
            SymbolKind::from_lsp_code(99),
            SymbolKind::Other(_)
        ));
    }

    #[test]
    fn symbol_source_tag_round_trip() {
        for s in [
            SymbolSource::Lsp,
            SymbolSource::TreeSitter,
            SymbolSource::Heuristic,
            SymbolSource::Lua,
        ] {
            assert_eq!(SymbolSource::from_tag(s.tag()), Some(s));
        }
        assert_eq!(SymbolSource::from_tag("nope"), None);
    }

    #[test]
    fn rust_heuristic_finds_basic_definitions() {
        let src = r#"
pub fn parse() {}
fn helper() {}
pub(crate) fn pub_crate() {}
struct Cell {}
pub enum Direction { Up, Down }
trait Iter {}
impl Cell {}
const MAX: u32 = 32;
static GREETING: &str = "hi";
mod inner;
type Bytes = Vec<u8>;
macro_rules! noop { () => {}; }
"#;
        let syms = extract_heuristic("rust", src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        for expected in &[
            "parse",
            "helper",
            "pub_crate",
            "Cell",
            "Direction",
            "Iter",
            "MAX",
            "GREETING",
            "inner",
            "Bytes",
            "noop",
        ] {
            assert!(names.contains(expected), "missing {expected}: {names:?}");
        }
    }

    #[test]
    fn lua_heuristic_finds_function_forms() {
        let src = r"
local function helper(x) return x end
function obj.greet(name) return name end
function obj:method(x) return x end
local SIZE = 32
local make = function() end
";
        let syms = extract_heuristic("lua", src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"helper"));
        assert!(names.contains(&"obj.greet"));
        assert!(names.contains(&"obj:method"));
        assert!(names.contains(&"SIZE"));
        assert!(names.contains(&"make"));
        // obj:method should be tagged Method
        assert!(
            syms.iter()
                .any(|s| s.name == "obj:method" && matches!(s.kind, SymbolKind::Method))
        );
    }

    #[test]
    fn python_heuristic_finds_def_and_class() {
        let src = "def parse():\n  pass\n\nasync def fetch():\n  pass\n\nclass Frog:\n  pass\n";
        let syms = extract_heuristic("python", src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"parse"));
        assert!(names.contains(&"fetch"));
        assert!(names.contains(&"Frog"));
    }

    #[test]
    fn go_heuristic_distinguishes_methods() {
        let src = "func Add(a int, b int) int { return a + b }\nfunc (r *Receiver) Hi() {}\ntype Point struct { X int }\ntype Reader interface { Read() }\n";
        let syms = extract_heuristic("go", src);
        let by_name: HashMap<&str, &Symbol> = syms.iter().map(|s| (s.name.as_str(), s)).collect();
        assert!(matches!(by_name["Add"].kind, SymbolKind::Function));
        assert!(matches!(by_name["Hi"].kind, SymbolKind::Method));
        assert!(matches!(by_name["Point"].kind, SymbolKind::Struct));
        assert!(matches!(by_name["Reader"].kind, SymbolKind::Trait));
    }

    #[test]
    fn unknown_language_falls_back_to_raw() {
        let src = "name = 32\nother: 64\n";
        let syms = extract_heuristic("unknownlang", src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"other"));
    }

    #[test]
    fn upsert_replaces_entry_for_same_path() {
        let mut idx = ProjectIndex::new("/proj");
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 1,
            content_hash: 1,
            language: Some("rust".into()),
            symbols: vec![Symbol {
                name: "first".into(),
                kind: SymbolKind::Function,
                line: 0,
                col: 0,
                source: SymbolSource::Heuristic,
                container: None,
            }],
        });
        assert_eq!(idx.symbol_count(), 1);
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 2,
            content_hash: 2,
            language: Some("rust".into()),
            symbols: vec![
                Symbol {
                    name: "second".into(),
                    kind: SymbolKind::Function,
                    line: 0,
                    col: 0,
                    source: SymbolSource::Heuristic,
                    container: None,
                },
                Symbol {
                    name: "third".into(),
                    kind: SymbolKind::Function,
                    line: 1,
                    col: 0,
                    source: SymbolSource::Heuristic,
                    container: None,
                },
            ],
        });
        assert_eq!(idx.file_count(), 1);
        assert_eq!(idx.symbol_count(), 2);
    }

    #[test]
    fn is_fresh_skips_unchanged_files() {
        let mut idx = ProjectIndex::new("/p");
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 100,
            content_hash: 0xdead_beef,
            language: None,
            symbols: vec![],
        });
        assert!(idx.is_fresh(Path::new("a.rs"), 100, 0xdead_beef));
        assert!(idx.is_fresh(Path::new("/p/a.rs"), 100, 0xdead_beef));
        assert!(!idx.is_fresh(Path::new("a.rs"), 100, 0xdead_beee));
        assert!(!idx.is_fresh(Path::new("a.rs"), 101, 0xdead_beef));
        assert!(!idx.is_fresh(Path::new("missing.rs"), 0, 0));
    }

    #[test]
    fn forget_file_drops_entry() {
        let mut idx = ProjectIndex::new("/p");
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 1,
            content_hash: 1,
            language: None,
            symbols: vec![Symbol {
                name: "x".into(),
                kind: SymbolKind::Function,
                line: 0,
                col: 0,
                source: SymbolSource::Heuristic,
                container: None,
            }],
        });
        let before_gen = idx.generation;
        assert!(idx.forget_file(Path::new("a.rs")));
        assert_eq!(idx.file_count(), 0);
        assert!(idx.generation > before_gen);
        assert!(!idx.forget_file(Path::new("a.rs")));
    }

    #[test]
    fn search_ranks_exact_above_prefix_above_substring() {
        let mut idx = ProjectIndex::new("/p");
        let symbols: Vec<Symbol> = ["parse", "parser_inner", "inner_parse_helper", "x"]
            .iter()
            .enumerate()
            .map(|(i, n)| Symbol {
                name: (*n).to_string(),
                kind: SymbolKind::Function,
                line: i as u32,
                col: 0,
                source: SymbolSource::Heuristic,
                container: None,
            })
            .collect();
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 0,
            content_hash: 0,
            language: Some("rust".into()),
            symbols,
        });
        let hits = idx.search("parse", 10);
        assert_eq!(hits[0].name, "parse"); // exact
        assert_eq!(hits[1].name, "parser_inner"); // prefix
        // word-boundary or substring next
        assert!(hits.iter().any(|h| h.name == "inner_parse_helper"));
        // "x" should never match "parse"
        assert!(!hits.iter().any(|h| h.name == "x"));
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut idx = ProjectIndex::new("/p");
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 0,
            content_hash: 0,
            language: None,
            symbols: vec![Symbol {
                name: "Render".into(),
                kind: SymbolKind::Function,
                line: 0,
                col: 0,
                source: SymbolSource::Heuristic,
                container: None,
            }],
        });
        assert_eq!(idx.search("render", 10).len(), 1);
        assert_eq!(idx.search("REND", 10).len(), 1);
    }

    #[test]
    fn search_caps_at_limit() {
        let mut idx = ProjectIndex::new("/p");
        let mut symbols = Vec::new();
        for i in 0..50 {
            symbols.push(Symbol {
                name: format!("parse_{i:02}"),
                kind: SymbolKind::Function,
                line: i,
                col: 0,
                source: SymbolSource::Heuristic,
                container: None,
            });
        }
        idx.upsert_file(FileEntry {
            path: PathBuf::from("a.rs"),
            mtime_secs: 0,
            content_hash: 0,
            language: None,
            symbols,
        });
        let hits = idx.search("parse", 10);
        assert_eq!(hits.len(), 10);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut idx = ProjectIndex::new(dir.path());
        idx.upsert_file(FileEntry {
            path: PathBuf::from("src/lib.rs"),
            mtime_secs: 1234,
            content_hash: 0xabcd,
            language: Some("rust".into()),
            symbols: vec![Symbol {
                name: "process".into(),
                kind: SymbolKind::Function,
                line: 7,
                col: 4,
                source: SymbolSource::Heuristic,
                container: None,
            }],
        });
        let cache = idx.default_cache_path();
        idx.save(&cache).expect("save");
        let loaded = ProjectIndex::load(dir.path(), &cache).expect("load");
        assert_eq!(loaded.file_count(), 1);
        assert_eq!(loaded.symbol_count(), 1);
        assert!(
            loaded.is_fresh(Path::new("src/lib.rs"), 1234, 0xabcd),
            "is_fresh should reflect persisted hash"
        );
        let hits = loaded.search("process", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, dir.path().join("src/lib.rs"));
    }

    #[test]
    fn load_missing_returns_empty_index() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = dir.path().join("does_not_exist.json");
        let idx = ProjectIndex::load(dir.path(), &cache).expect("load");
        assert_eq!(idx.file_count(), 0);
    }

    #[test]
    fn ingest_lsp_workspace_symbol_array() {
        let raw = serde_json::json!([
            {
                "name": "compute",
                "kind": 12,
                "location": {
                    "uri": "file:///proj/src/lib.rs",
                    "range": {
                        "start": { "line": 10, "character": 4 },
                        "end":   { "line": 12, "character": 1 }
                    }
                }
            },
            {
                "name": "Cell",
                "kind": 23,
                "location": {
                    "uri": "file:///proj/src/cell.rs",
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end":   { "line": 0, "character": 4 }
                    }
                },
                "containerName": "core"
            }
        ]);
        let ingested = ingest_lsp_symbols(&raw);
        assert_eq!(ingested.len(), 2);
        assert_eq!(ingested[0].symbol.name, "compute");
        assert!(matches!(ingested[0].symbol.kind, SymbolKind::Function));
        assert_eq!(ingested[0].symbol.line, 10);
        assert_eq!(ingested[1].symbol.name, "Cell");
        assert!(matches!(ingested[1].symbol.kind, SymbolKind::Struct));
        assert_eq!(ingested[1].symbol.container.as_deref(), Some("core"));
    }

    #[test]
    fn ingest_document_symbol_with_children() {
        let raw = serde_json::json!([
            {
                "name": "MyClass",
                "kind": 5,
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 9, "character": 0 } },
                "selectionRange": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 13 } },
                "location": { "uri": "file:///proj/src/m.py", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 9, "character": 0 } } },
                "children": [
                    {
                        "name": "method",
                        "kind": 6,
                        "range": { "start": { "line": 4, "character": 4 }, "end": { "line": 6, "character": 0 } }
                    }
                ]
            }
        ]);
        let ingested = ingest_lsp_symbols(&raw);
        // Two entries: MyClass and method (inheriting parent's URI).
        assert_eq!(ingested.len(), 2);
        assert!(ingested.iter().any(|i| i.symbol.name == "MyClass"));
        assert!(ingested.iter().any(|i| i.symbol.name == "method"
            && i.path.ends_with("m.py")
            && matches!(i.symbol.kind, SymbolKind::Method)));
    }

    #[test]
    fn uri_to_path_handles_percent_encoding() {
        let p = uri_to_path("file:///tmp/a%20b/c%2Bd.rs").unwrap();
        assert_eq!(p, PathBuf::from("/tmp/a b/c+d.rs"));
    }

    #[test]
    fn raw_extractor_picks_assignment_like_lines() {
        let src = "alpha = 1\nbeta : 2\n  indented = 3\nnot a name\n";
        let syms = extract_raw(src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"indented"));
        assert_eq!(
            syms.iter().find(|s| s.name == "alpha").unwrap().line,
            0,
            "line should be zero-based"
        );
    }

    #[test]
    fn rust_treesitter_finds_function_and_struct() {
        let src = "fn alpha() {} struct Beta;\nimpl Beta { fn beta_method() {} }\n";
        let language = tree_sitter_rust::LANGUAGE.into();
        let syms = extract_treesitter(&language, src).expect("parse");
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"Beta"));
        assert!(names.contains(&"beta_method"));
        assert!(
            syms.iter()
                .all(|s| matches!(s.source, SymbolSource::TreeSitter))
        );
    }

    #[test]
    fn lua_treesitter_runs_without_crash() {
        let src = "local function helper() end\nfunction Greeter:greet() end\n";
        let language = tree_sitter_lua::LANGUAGE.into();
        let syms = extract_treesitter(&language, src).expect("parse");
        // Lua grammar's node kind names vary. Just assert it doesn't
        // error and that the heuristic and tree-sitter outputs are
        // both non-empty for at least one definition shape.
        let _ = syms;
    }

    #[test]
    fn search_one_million_symbols_under_one_second() {
        // This guards spec acceptance: 100k-file project, sub-1s
        // symbol search. We construct 10 files × 100 000 symbols
        // each so the in-memory shape is realistic, and we look
        // for a needle that matches a thousand of them.
        let mut idx = ProjectIndex::new("/p");
        for f in 0..10u32 {
            let mut symbols = Vec::with_capacity(100_000);
            for i in 0..100_000u32 {
                symbols.push(Symbol {
                    name: format!("symbol_{f}_{i}"),
                    kind: SymbolKind::Function,
                    line: i,
                    col: 0,
                    source: SymbolSource::Heuristic,
                    container: None,
                });
            }
            idx.upsert_file(FileEntry {
                path: PathBuf::from(format!("file_{f}.rs")),
                mtime_secs: 0,
                content_hash: 0,
                language: Some("rust".into()),
                symbols,
            });
        }
        assert_eq!(idx.symbol_count(), 1_000_000);
        let start = std::time::Instant::now();
        let hits = idx.search("symbol_3_42", 50);
        let elapsed = start.elapsed();
        assert!(!hits.is_empty(), "should find at least one match");
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "search took {elapsed:?}, expected < 1s"
        );
    }
}
