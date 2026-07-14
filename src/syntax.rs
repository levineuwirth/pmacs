// syntax.rs --- T M4.1 tree-sitter integration: parse types, the
// per-buffer ParseView, and the worker-side run_parse function.
// T M4.2 layers bundled grammars; T M4.3 adds highlight-query
// loading and the capture-walk that feeds the highlight view.

//! Tree-sitter integration (T M4.1 -- T M4.3).
//!
//! The async runtime ([`crate::async_runtime`]) already carries the
//! dispatch shape Tree-sitter needs (parse-on-worker, supersede,
//! frame-cadence settle). This module adds:
//!
//! * [`ParseRequest`] / [`ParseTreeBundle`] --- the inputs and outputs
//!   that travel between the main thread and a worker.
//! * [`run_parse`] --- the worker-side body. Synchronous; called from
//!   the parse closure submitted by [`crate::async_runtime::AsyncRuntime::dispatch_parse`].
//! * [`ParseView`] / [`ParseViewHandle`] --- the per-buffer
//!   [`crate::view::View`] implementation that mirrors buffer bytes,
//!   captures every [`Edit`] as a [`tree_sitter::InputEdit`] (with
//!   correct row/col [`tree_sitter::Point`]s), and holds the most
//!   recent [`ParseTreeBundle`]. State lives behind an
//!   [`Arc<Mutex<ParseViewInner>>`] so the buffer-owned `Box<dyn View>`
//!   and the Lua-side glue (which needs to read the tree and feed
//!   back installed bundles) share the same backing store.
//! * [`HighlightSpan`] / [`compute_highlight_spans`] --- T M4.3
//!   capture-walk over a settled tree using a bundled
//!   `highlights.scm`. The resulting spans, sorted "wider first",
//!   feed [`crate::highlight::SyntaxHighlightView`]'s render path.
//!
//! M4.2 wires concrete grammars (`tree-sitter-rust`,
//! `tree-sitter-lua`) on top of this module. M4.1 uses
//! `tree-sitter-rust` only as a `dev-dependency` to drive acceptance
//! tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tree_sitter::StreamingIterator;

use crate::async_runtime::JobId;
use crate::buffer::{Buffer, BufferError, BufferId};
use crate::highlight::{Theme, ThemeHandle};
use crate::rope::Edit;
use crate::view::View;

/// Description of a parse job: the source bytes to parse, the
/// language to parse against, the prior tree (if any) for incremental
/// re-parse, and the [`tree_sitter::InputEdit`] descriptions
/// accumulated since that prior tree was produced.
///
/// All fields are owned ([R31]) so the closure submitted to a worker
/// holds nothing borrowed from the main thread.
#[derive(Clone, Debug)]
pub struct ParseRequest {
    /// Bytes to parse. Materialized from the buffer's rope on the
    /// main thread before dispatch.
    pub source: Arc<[u8]>,
    /// Grammar language. `tree_sitter::Language` is a cheap
    /// pointer-to-static and `Send + Sync + Clone`.
    pub language: tree_sitter::Language,
    /// Human-readable language label, surfaced through Lua and the
    /// `*workers*` buffer ([T M3.7]).
    pub language_name: String,
    /// Tree from a prior parse of the same buffer, or `None` for a
    /// cold parse. The worker calls [`tree_sitter::Tree::edit`] for
    /// every entry in [`Self::edits`] before re-parsing.
    pub prior_tree: Option<tree_sitter::Tree>,
    /// Edits accumulated by the [`ParseView`] since `prior_tree` was
    /// produced. Empty for cold parses; non-empty drives incremental
    /// re-parse.
    pub edits: Vec<tree_sitter::InputEdit>,
}

/// Output of [`run_parse`]. The runtime's parse-handoff side map
/// holds these by [`Arc`]; `Lua` introspection ([`crate::lua_bindings`])
/// resolves a buffer id to its current bundle and walks the tree.
#[derive(Debug)]
pub struct ParseTreeBundle {
    /// The freshly-produced parse tree.
    pub tree: tree_sitter::Tree,
    /// Source bytes the tree was parsed against. Co-owned with the
    /// request so node-byte-range lookups can read the underlying
    /// text (T M4.1 acceptance: "parse tree introspectable via Lua"
    /// implies the source the tree references).
    pub source: Arc<[u8]>,
    /// Language label. Stored alongside the tree so Lua can ask
    /// "what grammar produced this?".
    pub language_name: String,
    /// Wall-clock duration of the parse itself (excludes dispatch
    /// queueing, source materialization, and bus delivery). T M4.1
    /// acceptance criteria are stated in this metric.
    pub parse_duration: Duration,
}

/// Run a parse. This is the worker-side body that the runtime's
/// `dispatch_parse` closure invokes after pulling a job from the
/// queue. Always synchronous --- there is no internal yielding.
///
/// Returns `Err` if the language is rejected by [`tree_sitter::Parser`]
/// (ABI mismatch, almost always a build issue) or if the parser
/// itself returns no tree (cancellation flag flipped, exhausted
/// timeout --- neither wired in M4.1, so under M4.1 contracts this
/// path is unreachable in practice).
pub fn run_parse(req: ParseRequest) -> Result<ParseTreeBundle, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&req.language)
        .map_err(|e| format!("set_language: {e}"))?;
    let mut prior = req.prior_tree;
    if let Some(tree) = prior.as_mut() {
        for edit in &req.edits {
            tree.edit(edit);
        }
    }
    let started = Instant::now();
    let tree = parser
        .parse(req.source.as_ref(), prior.as_ref())
        .ok_or_else(|| "parser produced no tree".to_owned())?;
    let parse_duration = started.elapsed();
    Ok(ParseTreeBundle {
        tree,
        source: req.source,
        language_name: req.language_name,
        parse_duration,
    })
}

/// Convert a byte offset within `source` to a tree-sitter
/// `(row, column)` [`tree_sitter::Point`]. `byte` is clamped to
/// `source.len()`.
///
/// O(byte) on a linear scan. For 5000-line files (~150 KB) this is
/// tens of microseconds per call --- well under the 5 ms incremental
/// budget. A precomputed line-start index would be the obvious
/// follow-up if profiling argues for it.
#[must_use]
pub fn byte_to_point(source: &[u8], byte: usize) -> tree_sitter::Point {
    let bounded = byte.min(source.len());
    let mut row: usize = 0;
    let mut last_nl: Option<usize> = None;
    for (i, b) in source[..bounded].iter().enumerate() {
        if *b == b'\n' {
            row += 1;
            last_nl = Some(i);
        }
    }
    let column = match last_nl {
        Some(nl) => bounded - nl - 1,
        None => bounded,
    };
    tree_sitter::Point::new(row, column)
}

/// Mutable state shared between the buffer-attached [`ParseView`]
/// and any external [`ParseViewHandle`] clones.
struct ParseViewInner {
    language: tree_sitter::Language,
    language_name: String,
    /// Source bytes mirror, kept in sync with the buffer. Updated
    /// inside `on_edit`.
    source: Vec<u8>,
    /// Edits accumulated since `current` was produced. Drained on
    /// `make_request`; cleared on `install`.
    pending: Vec<tree_sitter::InputEdit>,
    /// Most recent settled parse, or `None` if no parse has run yet.
    current: Option<Arc<ParseTreeBundle>>,
}

/// Per-buffer parse-tree state. Attached to a [`Buffer`] as a
/// [`View`]; `on_edit` mirrors the rope edit into a parallel
/// `Vec<u8>` source buffer and pushes a corresponding
/// [`tree_sitter::InputEdit`] onto a pending list.
///
/// Internally a thin wrapper over `Arc<Mutex<ParseViewInner>>` so
/// callers (Lua bindings, dispatch glue) can hold a
/// [`ParseViewHandle`] clone and read/modify the same state without
/// having to detach the view from the buffer.
pub struct ParseView {
    inner: Arc<Mutex<ParseViewInner>>,
}

/// External handle to a [`ParseView`]'s state. Cheap to clone
/// (`Arc` bump). Used by [`crate::lua_bindings`] to (a) build a
/// [`ParseRequest`] before dispatch, (b) install the produced
/// [`ParseTreeBundle`] after settle, (c) introspect the tree from
/// Lua.
#[derive(Clone)]
pub struct ParseViewHandle {
    inner: Arc<Mutex<ParseViewInner>>,
}

impl ParseView {
    /// Construct a view by snapshotting the buffer's current bytes.
    /// The snapshot becomes the view's source mirror, so the first
    /// dispatched parse has byte-accurate input even before any edit
    /// is observed.
    #[must_use]
    pub fn new(buf: &Buffer, language: tree_sitter::Language, language_name: String) -> Self {
        let len = buf.len();
        let mut source = Vec::with_capacity(len as usize);
        if len > 0 {
            for chunk in buf.snapshot_rope().chunks(0, len) {
                source.extend_from_slice(chunk);
            }
        }
        let inner = ParseViewInner {
            language,
            language_name,
            source,
            pending: Vec::new(),
            current: None,
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Cheap clone of the shared state handle.
    #[must_use]
    pub fn handle(&self) -> ParseViewHandle {
        ParseViewHandle {
            inner: self.inner.clone(),
        }
    }
}

impl ParseViewHandle {
    /// Language this view parses against.
    #[must_use]
    pub fn language(&self) -> tree_sitter::Language {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .language
            .clone()
    }

    /// Human-readable language label.
    #[must_use]
    pub fn language_name(&self) -> String {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .language_name
            .clone()
    }

    /// Most recent parse, if any has settled.
    #[must_use]
    pub fn current(&self) -> Option<Arc<ParseTreeBundle>> {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .current
            .clone()
    }

    /// Number of pending edits waiting for the next parse dispatch.
    #[must_use]
    pub fn pending_edit_count(&self) -> usize {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .pending
            .len()
    }

    /// Snapshot of the source mirror's current contents. Test
    /// helper; the worker receives the same bytes as `req.source`
    /// when a parse is dispatched.
    #[must_use]
    pub fn source_snapshot(&self) -> Vec<u8> {
        self.inner
            .lock()
            .expect("ParseView mutex poisoned")
            .source
            .clone()
    }

    /// Build a [`ParseRequest`] reflecting the current state. Drains
    /// the pending-edit list. Caller is expected to dispatch the
    /// request and feed the settled bundle back via [`Self::install`].
    pub fn make_request(&self) -> ParseRequest {
        let mut inner = self.inner.lock().expect("ParseView mutex poisoned");
        let edits = std::mem::take(&mut inner.pending);
        let prior_tree = inner.current.as_ref().map(|b| b.tree.clone());
        ParseRequest {
            source: Arc::from(inner.source.clone()),
            language: inner.language.clone(),
            language_name: inner.language_name.clone(),
            prior_tree,
            edits,
        }
    }

    /// Install a freshly-parsed bundle. The caller is responsible
    /// for matching the bundle to the request that produced it ---
    /// installing a stale bundle would desynchronize the source
    /// mirror from the tree.
    pub fn install(&self, bundle: Arc<ParseTreeBundle>) {
        self.inner.lock().expect("ParseView mutex poisoned").current = Some(bundle);
    }
}

/// One row of the bundled-grammar config (T M4.2). Adding a new
/// grammar is a one-line addition to [`BUILTIN_LANGUAGES`] (plus the
/// matching `tree-sitter-foo` line in `Cargo.toml`).
///
/// The `loader` is a function pointer rather than a pre-materialized
/// [`tree_sitter::Language`] so the C-side grammar object isn't
/// touched until the first buffer of that language is opened ---
/// "load grammar lazily" per the M4.2 acceptance criterion. The
/// [`Self::highlights_query`] fragments ship as `&'static str`
/// constants in the binary; T M4.3 concatenates and compiles them
/// into a [`tree_sitter::Query`] on first highlight attach.
pub struct LanguageEntry {
    /// Canonical language name. Used by [`SyntaxRegistry::language`]
    /// lookups, surfaced through Lua as the grammar label.
    pub name: &'static str,
    /// File extensions (without the leading dot) that should auto-
    /// attach this grammar's [`ParseView`] when a file is opened.
    /// First match wins; ordering inside [`BUILTIN_LANGUAGES`] is
    /// the tiebreaker for ambiguous extensions.
    pub extensions: &'static [&'static str],
    /// Producer for the [`tree_sitter::Language`]. Called at most
    /// once per registry lifetime --- the result is cached under
    /// `name` after the first invocation.
    pub loader: fn() -> tree_sitter::Language,
    /// Bundled `highlights.scm` query fragments (T M4.3), concatenated
    /// in order (base grammar first) to form the effective query. Most
    /// grammars ship one self-contained fragment. A grammar whose
    /// bundled query is a tree-sitter `; inherits: <lang>` delta lists
    /// the inherited base queries ahead of its own, because pmacs does
    /// not resolve `inherits:` directives — CUDA, for instance, ships a
    /// two-capture delta over C++ and must carry the C and C++ queries
    /// explicitly or ordinary C/C++ syntax goes unhighlighted. An empty
    /// slice (or all-empty fragments) means no highlights: the view
    /// runs but emits nothing.
    pub highlights_query: &'static [&'static str],
}

/// Bundled grammars (T M4.2 + M4.3). The order is significant only
/// for extensions that map to multiple languages --- none of the
/// v0.1 entries collide.
///
/// Adding a grammar:
/// 1. Add `tree-sitter-foo = "X.Y"` to `Cargo.toml`.
/// 2. Add one [`LanguageEntry`] here, with
///    `highlights_query: &[tree_sitter_foo::HIGHLIGHTS_QUERY]` (or the
///    inherited base queries ahead of it, if `foo`'s bundled query is
///    a `; inherits:` delta — see the `cuda` entry).
/// 3. (Done.) The Lua side picks up the new grammar through the
///    `buffer.after-load` hook automatically and the highlight
///    overlay attaches in the same step.
pub const BUILTIN_LANGUAGES: &[LanguageEntry] = &[
    LanguageEntry {
        name: "rust",
        extensions: &["rs"],
        loader: || tree_sitter_rust::LANGUAGE.into(),
        highlights_query: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
    },
    LanguageEntry {
        name: "lua",
        extensions: &["lua"],
        loader: || tree_sitter_lua::LANGUAGE.into(),
        highlights_query: &[tree_sitter_lua::HIGHLIGHTS_QUERY],
    },
    // T M9.7: markdown grammar for prompt-result buffers with
    // `_meta.format = "markdown"`. Uses only the block grammar
    // (`tree_sitter_md::LANGUAGE`) — block-level highlighting (headers,
    // lists, fenced code blocks, blockquotes) is the v0.1 floor.
    // Inline highlighting (emphasis, links inside running text) would
    // require the dual-grammar `MarkdownParser` and is M9.8+ work.
    // The `markdown_inline` fixture prompt + matching acceptance test
    // pin this floor: an `**emphasis**` span must not crash, and is
    // expected to render unhighlighted; any future expansion that
    // adds inline coverage is additive, not a regression.
    // Note the constant name: `HIGHLIGHT_QUERY_BLOCK` (singular) is
    // the markdown crate's idiom; `tree-sitter-rust` and
    // `tree-sitter-lua` use `HIGHLIGHTS_QUERY` (plural).
    LanguageEntry {
        name: "markdown",
        extensions: &["md", "markdown"],
        loader: || tree_sitter_md::LANGUAGE.into(),
        highlights_query: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
    },
    // T M_B3 — C / C++. Lexical highlighting (keywords / strings /
    // operators) so the grid TUI shows code-shaped C++ on first open.
    // `LspStyleView` layers on top with semantic refinement
    // (functions / types / macros / namespaces) from clangd's
    // semantic tokens — the two views' styles merge via
    // `crate::overlay::merge_styles`.
    //
    // `.h` is ambiguous C / C++; the `c` entry below claims it
    // (matches the LSP filetype map's default in `lsp.lua`). Users
    // who want `.h` parsed as C++ can override via Lua.
    // Note the const names: `tree-sitter-c` and `tree-sitter-cpp`
    // expose `HIGHLIGHT_QUERY` (singular), matching the `tree-sitter-md`
    // crate's `HIGHLIGHT_QUERY_BLOCK` style; `tree-sitter-rust` and
    // `tree-sitter-lua` use `HIGHLIGHTS_QUERY` (plural). No semantic
    // difference — same bundled `highlights.scm` either way.
    LanguageEntry {
        name: "c",
        extensions: &["c", "h"],
        loader: || tree_sitter_c::LANGUAGE.into(),
        highlights_query: &[tree_sitter_c::HIGHLIGHT_QUERY],
    },
    LanguageEntry {
        name: "cpp",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp", "inl", "cppm"],
        loader: || tree_sitter_cpp::LANGUAGE.into(),
        highlights_query: &[tree_sitter_cpp::HIGHLIGHT_QUERY],
    },
    // CUDA (`.cu` source, `.cuh` header). A dedicated grammar rather
    // than reusing `cpp`: CUDA extends C++ with `__global__`/`__device__`
    // qualifiers, `<<<grid, block>>>` kernel-launch syntax, and builtin
    // types the C++ grammar misparses. Neither extension collides with
    // an entry above, so ordering is irrelevant here. `LspStyleView`
    // layers clangd's CUDA semantic tokens on top, exactly as for C/C++.
    //
    // Note the const name: `tree-sitter-cuda` exposes `HIGHLIGHTS_QUERY`
    // (plural, the `tree-sitter-rust`/`tree-sitter-lua` idiom), NOT the
    // singular `HIGHLIGHT_QUERY` that `tree-sitter-c`/`-cpp`/`-md` use.
    //
    // The CUDA `highlights.scm` opens with `; inherits: cpp` and defines
    // only the CUDA-specific captures (`<<<...>>>` launch brackets, the
    // `__global__`/`__device__` modifiers) — two capture classes on its
    // own. pmacs does not resolve `inherits:`, so the C and C++ base
    // queries are prepended explicitly; the three compile together into
    // ~16 capture classes against the CUDA grammar (which is a superset
    // of C++). Order is base-first (C, then C++, then CUDA) so later
    // fragments refine earlier ones. Without this, ordinary C/C++ syntax
    // in a `.cu` file would go almost entirely unhighlighted.
    LanguageEntry {
        name: "cuda",
        extensions: &["cu", "cuh"],
        loader: || tree_sitter_cuda::LANGUAGE.into(),
        highlights_query: &[
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            tree_sitter_cuda::HIGHLIGHTS_QUERY,
        ],
    },
    // Shell / bash. Lexical highlighting for the shell family; the LSP
    // half (bash-language-server) was already wired in `lsp.lua`. Unlike
    // `cuda`, bash's `highlights.scm` is self-contained (no `; inherits:`
    // delta), so a single fragment suffices. The extension set is wider
    // than the `.sh`/`.bash` the LSP filetype map covered: `.zsh`/`.ksh`/
    // `.ash` are close-enough dialects and `.bats` is bash. None collide
    // with an entry above. Because the language name is `bash` — matching
    // the `pmacs.lsp.config.bash` key — opening any of these also
    // auto-attaches bash-language-server. Extensionless shell scripts are
    // resolved by shebang, and rc dotfiles (`.bashrc`, `PKGBUILD`) by the
    // filename map — both in `builtin/runtime/syntax.lua`.
    LanguageEntry {
        name: "bash",
        extensions: &["sh", "bash", "zsh", "ksh", "ash", "bats"],
        loader: || tree_sitter_bash::LANGUAGE.into(),
        highlights_query: &[tree_sitter_bash::HIGHLIGHT_QUERY],
    },
    // Filename-identified languages. These files usually have no useful
    // extension (`Dockerfile`, `Makefile`, `CMakeLists.txt`), so the bulk
    // of detection is the filename map in `syntax.lua`; the extensions
    // here catch the `.dockerfile`/`.mk`/`.cmake` variants. All three ship
    // self-contained highlights (no `; inherits:`), so single fragments.
    //
    // Dockerfile uses the `tree-sitter-containerfile` crate (the
    // ABI-current grammar; also covers Containerfile); its root node is
    // `source_file`. Make roots at `makefile`, CMake at `source_file`.
    LanguageEntry {
        name: "dockerfile",
        extensions: &["dockerfile", "containerfile"],
        loader: || tree_sitter_containerfile::LANGUAGE.into(),
        highlights_query: &[tree_sitter_containerfile::HIGHLIGHTS_QUERY],
    },
    LanguageEntry {
        name: "make",
        extensions: &["mk", "make"],
        loader: || tree_sitter_make::LANGUAGE.into(),
        highlights_query: &[tree_sitter_make::HIGHLIGHTS_QUERY],
    },
    LanguageEntry {
        name: "cmake",
        extensions: &["cmake"],
        loader: || tree_sitter_cmake::LANGUAGE.into(),
        highlights_query: &[tree_sitter_cmake::HIGHLIGHTS_QUERY],
    },
    // Grammar-gap languages — these already had LSP configs but no
    // grammar, so they rendered without lexical color. Each language name
    // matches its existing `pmacs.lsp.config.<name>` key, so grammar
    // detection (which wins over the filetype map) resolves the same id
    // the server keys off. Root kinds: python `module`, go/zig
    // `source_file`, js/ts family `program`, toml `document`.
    LanguageEntry {
        name: "python",
        extensions: &["py", "pyi"],
        loader: || tree_sitter_python::LANGUAGE.into(),
        highlights_query: &[tree_sitter_python::HIGHLIGHTS_QUERY],
    },
    LanguageEntry {
        name: "go",
        extensions: &["go"],
        loader: || tree_sitter_go::LANGUAGE.into(),
        highlights_query: &[tree_sitter_go::HIGHLIGHTS_QUERY],
    },
    // JavaScript / TypeScript. One `tree-sitter-javascript` grammar parses
    // both `.js` and `.jsx`; `tree-sitter-typescript` ships two grammars
    // (`LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`). Highlights inherit: the TS
    // query is a ~5-capture delta over JavaScript, and JSX is a further
    // `JSX_HIGHLIGHT_QUERY` delta — so the `*react` and `typescript*`
    // entries compose base-first (js → jsx → ts), the same pattern as
    // `cuda` over C/C++. The four names mirror the LSP filetype map
    // (typescriptreact/javascriptreact) so tsserver enables the JSX parser.
    LanguageEntry {
        name: "javascript",
        extensions: &["js", "mjs", "cjs"],
        loader: || tree_sitter_javascript::LANGUAGE.into(),
        highlights_query: &[tree_sitter_javascript::HIGHLIGHT_QUERY],
    },
    LanguageEntry {
        name: "javascriptreact",
        extensions: &["jsx"],
        loader: || tree_sitter_javascript::LANGUAGE.into(),
        highlights_query: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
    },
    LanguageEntry {
        name: "typescript",
        extensions: &["ts", "mts", "cts"],
        loader: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlights_query: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
    },
    LanguageEntry {
        name: "typescriptreact",
        extensions: &["tsx"],
        loader: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlights_query: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
    },
    LanguageEntry {
        name: "toml",
        extensions: &["toml"],
        loader: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights_query: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
    },
    LanguageEntry {
        name: "zig",
        extensions: &["zig", "zon"],
        loader: || tree_sitter_zig::LANGUAGE.into(),
        highlights_query: &[tree_sitter_zig::HIGHLIGHTS_QUERY],
    },
];

/// Registry that the Lua surface ([`crate::lua_bindings::install_parse`])
/// reads to map language names to grammars and buffer ids to attached
/// [`ParseView`] handles. Held by `Rc<...>` --- main-thread state, no
/// cross-thread sharing.
pub struct SyntaxRegistry {
    languages: RefCell<HashMap<String, tree_sitter::Language>>,
    views: RefCell<HashMap<BufferId, ParseViewHandle>>,
    /// Job id → buffer id mapping populated when a parse is dispatched
    /// for a buffer. The Lua-side install path looks up the buffer id
    /// from the settled job's id so it can drain the parse-handoff
    /// bundle into the right view.
    parse_jobs: RefCell<HashMap<JobId, BufferId>>,
    /// Custom (non-builtin) `extension → language name` mappings
    /// registered at runtime. Hosts that want to wire an extra
    /// grammar without touching [`BUILTIN_LANGUAGES`] can call
    /// [`SyntaxRegistry::register_extension`] alongside
    /// [`SyntaxRegistry::register_language`]. Looked up *after*
    /// [`BUILTIN_LANGUAGES`] so users can't accidentally shadow a
    /// builtin extension.
    extra_extensions: RefCell<HashMap<String, String>>,
    /// Compiled `highlights.scm` query per language (T M4.3). Lazy-
    /// compiled from [`LanguageEntry::highlights_query`] on first
    /// access; cached for the registry's lifetime. The result of a
    /// compilation failure (e.g. grammar / query ABI skew) is
    /// cached as `Err(message)` so we don't burn cycles re-trying.
    queries: RefCell<HashMap<String, Result<Arc<tree_sitter::Query>, String>>>,
    /// Active theme (T M4.3). Shared with every
    /// [`crate::highlight::SyntaxHighlightView`] attached through
    /// this registry --- editing the theme through Lua updates all
    /// attached views in lock-step.
    theme: ThemeHandle,
}

/// Cheaply-cloneable shared handle to a [`SyntaxRegistry`]. Same
/// `Rc<RefCell<...>>` shape as the other shared registries in
/// [`crate::lua_bindings`].
pub type SharedSyntaxRegistry = Rc<SyntaxRegistry>;

impl SyntaxRegistry {
    /// Construct an empty registry. Languages are registered by the
    /// host (Rust startup) before Lua scripts run, or lazy-loaded
    /// from [`BUILTIN_LANGUAGES`] on first lookup. Theme starts at
    /// the [`Theme::default_dark`] palette so an opening rust file
    /// gets a usable highlight without any Lua configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            languages: RefCell::new(HashMap::new()),
            views: RefCell::new(HashMap::new()),
            parse_jobs: RefCell::new(HashMap::new()),
            extra_extensions: RefCell::new(HashMap::new()),
            queries: RefCell::new(HashMap::new()),
            theme: Arc::new(Mutex::new(Theme::default_dark())),
        }
    }

    /// Shared theme handle. Cheap clone (an [`Arc`] bump). T M4.3:
    /// every [`crate::highlight::SyntaxHighlightView`] holds a clone
    /// of this same `Arc<Mutex<Theme>>`, so a Lua-driven theme edit
    /// is observable on the next render.
    #[must_use]
    pub fn theme(&self) -> ThemeHandle {
        self.theme.clone()
    }

    /// Register a tree-sitter [`tree_sitter::Language`] under `name`.
    /// Subsequent registrations under the same name overwrite the
    /// prior entry. Called from Rust startup; Lua scripts cannot
    /// construct `tree_sitter::Language` values directly.
    pub fn register_language(&self, name: impl Into<String>, lang: tree_sitter::Language) {
        self.languages.borrow_mut().insert(name.into(), lang);
    }

    /// Register a runtime extension → language mapping. The name
    /// must already exist (either from a manual
    /// [`Self::register_language`] or in [`BUILTIN_LANGUAGES`]).
    /// Custom mappings sit *after* [`BUILTIN_LANGUAGES`] in the
    /// lookup order, so users can't accidentally shadow a builtin.
    /// T M4.2: lets a host wire an out-of-tree grammar without
    /// touching the builtin table.
    pub fn register_extension(&self, ext: impl Into<String>, lang_name: impl Into<String>) {
        self.extra_extensions
            .borrow_mut()
            .insert(ext.into(), lang_name.into());
    }

    /// Look up a language by name. On miss, consults
    /// [`BUILTIN_LANGUAGES`] and lazy-loads the entry on demand
    /// (caching it for the rest of the registry's lifetime).
    #[must_use]
    pub fn language(&self, name: &str) -> Option<tree_sitter::Language> {
        if let Some(lang) = self.languages.borrow().get(name).cloned() {
            return Some(lang);
        }
        let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == name)?;
        let lang = (entry.loader)();
        self.languages
            .borrow_mut()
            .insert(name.to_owned(), lang.clone());
        Some(lang)
    }

    /// True if `name` is a registered or builtin language.
    #[must_use]
    pub fn has_language(&self, name: &str) -> bool {
        self.languages.borrow().contains_key(name)
            || BUILTIN_LANGUAGES.iter().any(|e| e.name == name)
    }

    /// Resolve a file extension to a language name. Checks
    /// [`BUILTIN_LANGUAGES`] first, then runtime-registered extras.
    /// Match is case-sensitive (file extensions traditionally are);
    /// extension is the part *after* the last `.`, with no leading
    /// dot. T M4.2.
    #[must_use]
    pub fn language_name_for_extension(&self, ext: &str) -> Option<&'static str> {
        BUILTIN_LANGUAGES
            .iter()
            .find(|e| e.extensions.contains(&ext))
            .map(|e| e.name)
    }

    /// Resolve a file path to a language name. Strips the path to
    /// its extension and delegates to
    /// [`Self::language_name_for_extension`]. Returns `None` for
    /// extensionless paths and unrecognized extensions.
    #[must_use]
    pub fn language_name_for_path(&self, path: &str) -> Option<String> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|os| os.to_str())?;
        if let Some(name) = self.language_name_for_extension(ext) {
            return Some(name.to_owned());
        }
        self.extra_extensions.borrow().get(ext).cloned()
    }

    /// Record the [`ParseViewHandle`] attached to a buffer.
    pub fn attach_view(&self, buffer: BufferId, handle: ParseViewHandle) {
        self.views.borrow_mut().insert(buffer, handle);
    }

    /// Retrieve the handle for `buffer` if one is attached.
    #[must_use]
    pub fn view(&self, buffer: BufferId) -> Option<ParseViewHandle> {
        self.views.borrow().get(&buffer).cloned()
    }

    /// Forget the handle for `buffer`. Called when a buffer is
    /// removed.
    pub fn detach_view(&self, buffer: BufferId) {
        self.views.borrow_mut().remove(&buffer);
    }

    /// Record that `job_id` was dispatched for `buffer`. The settle
    /// path looks the buffer up from the job id so it can install the
    /// settled bundle into the right view.
    pub fn record_parse_job(&self, job_id: JobId, buffer: BufferId) {
        self.parse_jobs.borrow_mut().insert(job_id, buffer);
    }

    /// Drain the recorded buffer-id for `job_id`.
    #[must_use]
    pub fn take_parse_job(&self, job_id: JobId) -> Option<BufferId> {
        self.parse_jobs.borrow_mut().remove(&job_id)
    }

    /// Number of unsettled parse-job → buffer mappings. Test helper.
    #[must_use]
    pub fn pending_parse_job_count(&self) -> usize {
        self.parse_jobs.borrow().len()
    }

    /// True when a dispatched parse job for `buffer` has not yet been
    /// installed or drained. The syntax Lua glue records jobs here at
    /// dispatch time and removes them in `_install_settled`, so this
    /// is the main-thread "parse in flight" bit for render producers
    /// that need to avoid stale whole-file work while typing.
    #[must_use]
    pub fn has_pending_parse_job_for(&self, buffer: BufferId) -> bool {
        self.parse_jobs.borrow().values().any(|&bid| bid == buffer)
    }

    /// Lazy-compile and cache the bundled `highlights.scm` query for
    /// `lang_name`. Returns `None` if the language is unknown, the
    /// language entry has an empty query (no highlights shipped),
    /// or compilation failed (the failure is cached so subsequent
    /// calls don't re-attempt). T M4.3.
    #[must_use]
    pub fn highlights_query(&self, lang_name: &str) -> Option<Arc<tree_sitter::Query>> {
        if let Some(slot) = self.queries.borrow().get(lang_name) {
            return slot.as_ref().ok().cloned();
        }
        let language = self.language(lang_name)?;
        let entry = BUILTIN_LANGUAGES.iter().find(|e| e.name == lang_name);
        // Fragments are joined with a newline, never bare-concatenated: a
        // fragment can end mid-`; comment` or without a trailing newline,
        // and abutting it against the next fragment's first token would
        // corrupt the query (e.g. `@variable; Functions` swallows the
        // next line into a comment).
        let source = entry.map_or_else(String::new, |e| e.highlights_query.join("\n"));
        if source.trim().is_empty() {
            self.queries
                .borrow_mut()
                .insert(lang_name.to_owned(), Err("no highlights query".to_owned()));
            return None;
        }
        let compiled = tree_sitter::Query::new(&language, &source)
            .map(Arc::new)
            .map_err(|e| format!("compile {lang_name} highlights: {e:?}"));
        let result = compiled.as_ref().ok().cloned();
        self.queries
            .borrow_mut()
            .insert(lang_name.to_owned(), compiled);
        result
    }
}

impl Default for SyntaxRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl View for ParseView {
    fn on_edit(&mut self, _buf: &Buffer, edit: &Edit) -> Result<(), BufferError> {
        let start_byte = edit.range.start as usize;
        let old_end_byte = edit.range.end as usize;
        let new_end_byte = start_byte + edit.inserted_len as usize;

        let mut inner = self.inner.lock().expect("ParseView mutex poisoned");

        // Compute pre-edit Points BEFORE mutating the source mirror.
        let start_position = byte_to_point(&inner.source, start_byte);
        let old_end_position = byte_to_point(&inner.source, old_end_byte);

        // Splice: replace [start..old_end] with the inserted bytes
        // pulled from the new rope. The inserted bytes are at
        // [start_byte, new_end_byte) in the new rope.
        let mut new_bytes = vec![0u8; edit.inserted_len as usize];
        if !new_bytes.is_empty() {
            edit.new_rope
                .slice(start_byte as u64, new_end_byte as u64, &mut new_bytes);
        }
        inner.source.splice(start_byte..old_end_byte, new_bytes);

        // Now compute the new_end Point against the updated source.
        let new_end_position = byte_to_point(&inner.source, new_end_byte);

        inner.pending.push(tree_sitter::InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        });
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// T M4.3: highlight-span extraction
// ---------------------------------------------------------------------------

/// One highlighted byte range produced by a `highlights.scm` query
/// run against a [`ParseTreeBundle`]. The `capture_index` is into
/// [`tree_sitter::Query::capture_names`]; resolving to a style
/// happens inside [`crate::highlight::SyntaxHighlightView`] using
/// the active theme.
///
/// `start_byte`/`end_byte` are byte offsets into the bundle's
/// `source`. Stored as `u32` because pmacs files cap at 4 GiB
/// (rope domain) and downstream coordinates (rows, cols) are also
/// `u32` --- avoids signed/unsigned conversion noise on the render
/// hot path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    /// First byte covered by the span.
    pub start_byte: u32,
    /// One past the last byte covered.
    pub end_byte: u32,
    /// Index into [`tree_sitter::Query::capture_names`] for the
    /// capture that produced this span.
    pub capture_index: u32,
}

/// Walk every capture produced by `query` over `bundle.tree`,
/// collect them as [`HighlightSpan`]s, and sort them so that wider
/// (outer) ranges come *before* narrower (inner) ones at the same
/// start byte. The render path then applies them in order, so inner
/// captures end up overriding outer ones --- matches typical
/// editor highlight precedence ("the most specific node wins").
///
/// O(captures · log captures) for the sort; O(query work) for the
/// capture walk itself (the dominant cost; see M4.3 acceptance).
#[must_use]
pub fn compute_highlight_spans(
    query: &tree_sitter::Query,
    bundle: &ParseTreeBundle,
) -> Vec<HighlightSpan> {
    compute_highlight_spans_in_range(query, bundle, None)
}

/// Like [`compute_highlight_spans`], but restricts the query to nodes
/// intersecting `byte_range` when `Some`. tree-sitter's
/// `QueryCursor::set_byte_range` makes the capture walk proportional to
/// the range, not the whole tree — the semantic producer passes the
/// declared viewport so styling a screenful of a huge file is
/// O(visible), not O(file) (the per-edit typing cost; framing Q#S6).
#[must_use]
pub fn compute_highlight_spans_in_range(
    query: &tree_sitter::Query,
    bundle: &ParseTreeBundle,
    byte_range: Option<std::ops::Range<usize>>,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    if let Some(range) = byte_range {
        cursor.set_byte_range(range);
    }
    let source: &[u8] = bundle.source.as_ref();
    let root = bundle.tree.root_node();
    let mut iter = cursor.captures(query, root, source);
    while let Some((qmatch, capture_idx)) = iter.next() {
        // Fail-closed on the locals property predicate. The capture
        // iterator already applies text predicates (`#eq?`/`#match?`/
        // `#any-of?`), but `#is? local` / `#is-not? local` are *property*
        // predicates (`Query::property_predicates`) that need a scope map
        // built from the grammar's LOCALS_QUERY, which pmacs does not run.
        // Applying such a capture regardless mis-styles shadowed locals —
        // e.g. a local `console`/`require` in JS/TS would still capture as
        // `@variable.builtin`/`@function.builtin`. Until locals processing
        // exists, drop captures whose pattern carries one; the identifier
        // falls back to its non-builtin capture. `#set!` (property
        // *settings*) is a different API and is not consulted here.
        if query
            .property_predicates(qmatch.pattern_index)
            .iter()
            .any(|(prop, _)| &*prop.key == "local")
        {
            continue;
        }
        let cap = qmatch.captures[*capture_idx];
        spans.push(HighlightSpan {
            start_byte: cap.node.start_byte() as u32,
            end_byte: cap.node.end_byte() as u32,
            capture_index: cap.index,
        });
    }
    // Wider-first ordering at equal start: later writes (the
    // narrower / more specific spans) override earlier ones in the
    // overlay merge.
    spans.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then_with(|| b.end_byte.cmp(&a.end_byte))
            .then_with(|| a.capture_index.cmp(&b.capture_index))
    });
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, BufferId, EditOp};

    fn fresh_buffer(name: &str) -> Buffer {
        Buffer::new(BufferId::next(), name)
    }

    fn rust_view(buf: &Buffer) -> (ParseView, ParseViewHandle) {
        let view = ParseView::new(buf, tree_sitter_rust::LANGUAGE.into(), "rust".to_owned());
        let handle = view.handle();
        (view, handle)
    }

    fn parse_synchronously(handle: &ParseViewHandle) -> Arc<ParseTreeBundle> {
        let req = handle.make_request();
        let bundle = Arc::new(run_parse(req).expect("parse succeeds"));
        handle.install(bundle.clone());
        bundle
    }

    #[test]
    fn builtin_languages_include_c_and_cpp() {
        // M_B3 regression guard: a future refactor must not silently
        // drop C / C++ tree-sitter coverage — the dual-authority
        // styling in the TUI depends on these entries existing and
        // claiming their canonical extensions.
        let c = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "c")
            .expect("`c` language entry must be present");
        assert!(c.extensions.contains(&"c"), "`c` claims `.c`");
        assert!(
            c.extensions.contains(&"h"),
            "`c` claims `.h` (matches the LSP filetype map's default)"
        );
        assert!(
            !c.highlights_query.is_empty(),
            "`c` ships a non-empty highlights query"
        );
        let cpp = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "cpp")
            .expect("`cpp` language entry must be present");
        for ext in ["cpp", "cc", "cxx", "hpp", "hh", "hxx"] {
            assert!(cpp.extensions.contains(&ext), "`cpp` claims `.{ext}`");
        }
        assert!(
            !cpp.highlights_query.is_empty(),
            "`cpp` ships a non-empty highlights query"
        );
    }

    #[test]
    fn builtin_languages_include_cuda() {
        // Regression guard mirroring `builtin_languages_include_c_and_cpp`:
        // the CUDA entry must keep claiming its canonical extensions and,
        // because its bundled query is a `; inherits: cpp` delta, prepend
        // the C and C++ base queries explicitly (see the entry comment and
        // `cuda_highlights_resolve_c_and_cpp_captures`).
        let cuda = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "cuda")
            .expect("`cuda` language entry must be present");
        assert!(cuda.extensions.contains(&"cu"), "`cuda` claims `.cu`");
        assert!(cuda.extensions.contains(&"cuh"), "`cuda` claims `.cuh`");
        assert!(
            cuda.highlights_query
                .contains(&tree_sitter_c::HIGHLIGHT_QUERY),
            "`cuda` prepends the C base highlights (it does not resolve `inherits:`)"
        );
        assert!(
            cuda.highlights_query
                .contains(&tree_sitter_cpp::HIGHLIGHT_QUERY),
            "`cuda` prepends the C++ base highlights"
        );
        assert!(
            cuda.highlights_query
                .contains(&tree_sitter_cuda::HIGHLIGHTS_QUERY),
            "`cuda` carries its own CUDA-specific highlights delta"
        );
    }

    #[test]
    fn cuda_highlights_resolve_c_and_cpp_captures() {
        // Finding-2 regression: the bundled CUDA `highlights.scm` is only
        // a two-capture `; inherits: cpp` delta (launch brackets + CUDA
        // modifiers). pmacs does not resolve `inherits:`, so the entry
        // prepends the C and C++ base queries; assert the COMPILED query
        // actually carries ordinary C/C++ captures (the C base's
        // `@variable`) and far more than the delta's two capture classes —
        // not merely that some query is non-empty.
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("cuda")
            .expect("cuda highlights compile");
        let names = query.capture_names();
        assert!(
            names.contains(&"variable"),
            "combined query carries the C base `@variable` capture; got {names:?}"
        );
        assert!(
            names.len() >= 8,
            "combined C+C+++CUDA query resolves many capture classes, not the \
             CUDA delta's two; got {} ({names:?})",
            names.len()
        );
    }

    #[test]
    fn cuda_grammar_loads_and_parses_kernel_launch() {
        // ABI acceptance: a `tree-sitter-cuda` 0.21 grammar must be
        // accepted by our `tree-sitter` 0.26 core — `set_language`
        // succeeds and a tree is produced. This is the runtime check the
        // compile step cannot give us (a too-old grammar ABI fails only
        // here, at parse time). Grammar identity: `<<<grid, block>>>`
        // kernel-launch syntax is CUDA-specific; the C++ grammar parses
        // it as chained comparison/shift operators and flags an error, so
        // an error-free parse proves the entry wired the CUDA grammar,
        // not a C++ fallback.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("cuda")
            .expect("`cuda` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("kernel.cu");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"__global__ void add(int *c) { c[threadIdx.x] = 1; }\n\
                     int main() { add<<<1, 256>>>(0); return 0; }\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "cuda".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.tree.root_node().kind(),
            "translation_unit",
            "CUDA grammar (C-derived) roots at translation_unit"
        );
        assert!(
            !bundle.tree.root_node().has_error(),
            "CUDA grammar parses the `<<<...>>>` kernel launch without error"
        );
    }

    #[test]
    fn language_for_path_resolves_cuda_extensions() {
        // `.cu`/`.cuh` resolve to the CUDA grammar through the same
        // extension-detection path as every other bundled language, so
        // the LSP filetype fallback in `lsp.lua` is never consulted for
        // them in practice.
        let reg = SyntaxRegistry::new();
        assert_eq!(
            reg.language_name_for_path("kernel.cu").as_deref(),
            Some("cuda")
        );
        assert_eq!(
            reg.language_name_for_path("device.cuh").as_deref(),
            Some("cuda")
        );
    }

    #[test]
    fn builtin_languages_include_bash() {
        // Regression guard: the bash entry claims the wider shell family
        // and ships a highlights query, so shell scripts get lexical color
        // (they had LSP via bash-language-server but no grammar before).
        let bash = BUILTIN_LANGUAGES
            .iter()
            .find(|l| l.name == "bash")
            .expect("`bash` language entry must be present");
        for ext in ["sh", "bash", "zsh", "ksh", "ash", "bats"] {
            assert!(bash.extensions.contains(&ext), "`bash` claims `.{ext}`");
        }
        assert!(
            !bash.highlights_query.is_empty(),
            "`bash` ships a highlights query"
        );
    }

    #[test]
    fn bash_grammar_loads_and_parses_script() {
        // ABI acceptance: the `tree-sitter-bash` 0.25 grammar must be
        // accepted by our `tree-sitter` 0.26 core — `set_language`
        // succeeds and a tree is produced. A representative script
        // (shebang, `set`, parameter expansion, function, `if`) parses
        // without error, so the entry wired a working grammar.
        let reg = SyntaxRegistry::new();
        let language = reg
            .language("bash")
            .expect("`bash` language loads from BUILTIN_LANGUAGES");
        let mut buf = fresh_buffer("deploy.sh");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"#!/usr/bin/env bash\nset -euo pipefail\nname=${1:-world}\n\
                     greet() { echo \"hello, $name\"; }\nif [ -n \"$name\" ]; then greet; fi\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "bash".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        assert_eq!(
            bundle.tree.root_node().kind(),
            "program",
            "bash grammar roots at `program`"
        );
        assert!(
            !bundle.tree.root_node().has_error(),
            "bash grammar parses a representative script without error"
        );
    }

    #[test]
    fn bash_highlights_compile_with_captures() {
        // The self-contained bash `highlights.scm` must compile against
        // the grammar and yield real capture classes (a crate bump that
        // drifted query and grammar apart would surface here).
        let reg = SyntaxRegistry::new();
        let query = reg
            .highlights_query("bash")
            .expect("bash highlights compile");
        assert!(
            query.capture_names().len() >= 5,
            "bash highlights resolve several capture classes; got {}",
            query.capture_names().len()
        );
    }

    #[test]
    fn language_for_path_resolves_bash_extensions() {
        // The whole shell family resolves to the `bash` grammar via
        // extension detection; `.zsh`/`.ksh`/`.ash`/`.bats` are new here
        // (only `.sh`/`.bash` were covered by the LSP filetype map before).
        let reg = SyntaxRegistry::new();
        for path in [
            "deploy.sh",
            "lib.bash",
            "prompt.zsh",
            "script.ksh",
            "init.ash",
            "test_cli.bats",
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some("bash"),
                "{path} resolves to bash"
            );
        }
    }

    #[test]
    fn builtin_languages_include_dockerfile_make_cmake() {
        for (name, exts) in [
            ("dockerfile", &["dockerfile", "containerfile"][..]),
            ("make", &["mk", "make"][..]),
            ("cmake", &["cmake"][..]),
        ] {
            let entry = BUILTIN_LANGUAGES
                .iter()
                .find(|l| l.name == name)
                .unwrap_or_else(|| panic!("`{name}` language entry must be present"));
            for ext in exts {
                assert!(entry.extensions.contains(ext), "`{name}` claims `.{ext}`");
            }
            assert!(
                entry.highlights_query.iter().any(|q| !q.is_empty()),
                "`{name}` ships a highlights query"
            );
        }
    }

    #[test]
    fn filename_grammars_load_and_parse() {
        // ABI acceptance: each 0.x/1.x grammar must be accepted by our
        // tree-sitter 0.26 core (set_language succeeds at runtime) and
        // parse a representative snippet without error, at its own root.
        let reg = SyntaxRegistry::new();
        let cases: &[(&str, &str, &[u8])] = &[
            (
                "dockerfile",
                "source_file",
                b"FROM alpine:3\nRUN apk add curl\nCMD [\"sh\"]\n",
            ),
            (
                "make",
                "makefile",
                b"all: build\n\tcc -o app main.c\n.PHONY: all\n",
            ),
            (
                "cmake",
                "source_file",
                b"cmake_minimum_required(VERSION 3.10)\nproject(demo)\n",
            ),
        ];
        for (lang, root_kind, src) in cases {
            let language = reg
                .language(lang)
                .unwrap_or_else(|| panic!("`{lang}` loads from BUILTIN_LANGUAGES"));
            let mut buf = fresh_buffer(&format!("probe.{lang}"));
            buf.apply_edit(EditOp::Insert { pos: 0, bytes: src })
                .unwrap();
            let view = ParseView::new(&buf, language, (*lang).to_owned());
            let handle = view.handle();
            let _vid = buf.attach_view(Box::new(view));
            let bundle = parse_synchronously(&handle);
            assert_eq!(
                bundle.tree.root_node().kind(),
                *root_kind,
                "`{lang}` roots at `{root_kind}`"
            );
            assert!(
                !bundle.tree.root_node().has_error(),
                "`{lang}` parses its snippet without error"
            );
        }
    }

    #[test]
    fn language_for_path_resolves_dockerfile_make_cmake_extensions() {
        let reg = SyntaxRegistry::new();
        for (path, lang) in [
            ("app.dockerfile", "dockerfile"),
            ("svc.containerfile", "dockerfile"),
            ("rules.mk", "make"),
            ("common.make", "make"),
            ("toolchain.cmake", "cmake"),
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some(lang),
                "{path} resolves to {lang}"
            );
        }
    }

    #[test]
    fn builtin_languages_include_gap_grammars() {
        for (name, exts) in [
            ("python", &["py", "pyi"][..]),
            ("go", &["go"][..]),
            ("javascript", &["js", "mjs", "cjs"][..]),
            ("javascriptreact", &["jsx"][..]),
            ("typescript", &["ts", "mts", "cts"][..]),
            ("typescriptreact", &["tsx"][..]),
            ("toml", &["toml"][..]),
            ("zig", &["zig", "zon"][..]),
        ] {
            let entry = BUILTIN_LANGUAGES
                .iter()
                .find(|l| l.name == name)
                .unwrap_or_else(|| panic!("`{name}` language entry must be present"));
            for ext in exts {
                assert!(entry.extensions.contains(ext), "`{name}` claims `.{ext}`");
            }
            assert!(
                entry.highlights_query.iter().any(|q| !q.is_empty()),
                "`{name}` ships a highlights query"
            );
        }
    }

    #[test]
    fn gap_grammars_load_and_parse() {
        // ABI acceptance for each new grammar (set_language succeeds at
        // runtime) + a snippet that parses without error at the expected
        // root. Covers both `tree-sitter-typescript` grammars.
        let reg = SyntaxRegistry::new();
        let cases: &[(&str, &str, &[u8])] = &[
            ("python", "module", b"def f(x):\n    return x + 1\n"),
            ("go", "source_file", b"package main\nfunc main() {}\n"),
            ("javascript", "program", b"const x = 1;\nlet y = [x];\n"),
            (
                "javascriptreact",
                "program",
                b"const e = <div id=\"a\"/>;\n",
            ),
            ("typescript", "program", b"const x: number = 1;\n"),
            ("typescriptreact", "program", b"const e = <div/>;\n"),
            ("toml", "document", b"[pkg]\nname = \"x\"\n"),
            ("zig", "source_file", b"const std = @import(\"std\");\n"),
        ];
        for (lang, root_kind, src) in cases {
            let language = reg
                .language(lang)
                .unwrap_or_else(|| panic!("`{lang}` loads from BUILTIN_LANGUAGES"));
            let mut buf = fresh_buffer(&format!("probe_{lang}"));
            buf.apply_edit(EditOp::Insert { pos: 0, bytes: src })
                .unwrap();
            let view = ParseView::new(&buf, language, (*lang).to_owned());
            let handle = view.handle();
            let _vid = buf.attach_view(Box::new(view));
            let bundle = parse_synchronously(&handle);
            assert_eq!(
                bundle.tree.root_node().kind(),
                *root_kind,
                "`{lang}` roots at `{root_kind}`"
            );
            assert!(
                !bundle.tree.root_node().has_error(),
                "`{lang}` parses its snippet without error"
            );
        }
    }

    #[test]
    fn typescript_highlights_compose_the_javascript_base() {
        // The bundled TypeScript highlights are a ~5-capture delta over
        // JavaScript; the entries prepend the JS query (and JSX for tsx).
        // Assert the COMPILED query resolves far more than the delta — the
        // JS base is really there, not just the ts-specific captures.
        let reg = SyntaxRegistry::new();
        for lang in ["typescript", "typescriptreact"] {
            let query = reg
                .highlights_query(lang)
                .unwrap_or_else(|| panic!("`{lang}` highlights compile"));
            assert!(
                query.capture_names().len() >= 15,
                "`{lang}` composes the JavaScript base (got {} captures, delta alone is ~5)",
                query.capture_names().len()
            );
        }
    }

    #[test]
    fn javascript_shadowed_builtin_is_not_mislabeled() {
        // `#is-not? local` (JS/TS use it for console/require/etc.) needs a
        // scope map from the LOCALS_QUERY we don't run, so
        // `compute_highlight_spans` drops captures guarded by it. Here
        // `console` is a LOCAL declaration — it must not surface as a
        // `*.builtin` capture (which is what a naive run of the shared JS
        // query would produce).
        let reg = SyntaxRegistry::new();
        let language = reg.language("javascript").expect("javascript loads");
        let query = reg
            .highlights_query("javascript")
            .expect("javascript highlights compile");
        let mut buf = fresh_buffer("shadow.js");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"const console = 5;\nconsole;\n",
        })
        .unwrap();
        let view = ParseView::new(&buf, language, "javascript".to_owned());
        let handle = view.handle();
        let _vid = buf.attach_view(Box::new(view));
        let bundle = parse_synchronously(&handle);
        let spans = compute_highlight_spans(&query, &bundle);
        assert!(!spans.is_empty(), "the JS query produced highlight spans");
        let names = query.capture_names();
        let builtin: Vec<&str> = spans
            .iter()
            .map(|s| names[s.capture_index as usize])
            .filter(|n| n.contains("builtin"))
            .collect();
        assert!(
            builtin.is_empty(),
            "a locally-shadowed `console` must not get a *.builtin capture; got {builtin:?}"
        );
        // ...and dropping the builtin pattern must not strip *all* styling:
        // each `console` occurrence still keeps its ordinary `@variable`
        // capture (the fallback), so the loss is only the `.builtin` refine.
        let src = "const console = 5;\nconsole;\n";
        for (pos, _) in src.match_indices("console") {
            let (start, end) = (pos as u32, (pos + "console".len()) as u32);
            let caps: Vec<&str> = spans
                .iter()
                .filter(|s| s.start_byte == start && s.end_byte == end)
                .map(|s| names[s.capture_index as usize])
                .collect();
            assert!(
                caps.iter().any(|n| n.starts_with("variable")),
                "`console` at byte {pos} keeps a variable capture; got {caps:?}"
            );
        }
    }

    #[test]
    fn gap_grammar_extensions_resolve() {
        let reg = SyntaxRegistry::new();
        for (path, lang) in [
            ("main.py", "python"),
            ("stub.pyi", "python"),
            ("server.go", "go"),
            ("app.js", "javascript"),
            ("mod.mjs", "javascript"),
            ("view.jsx", "javascriptreact"),
            ("index.ts", "typescript"),
            ("types.mts", "typescript"),
            ("App.tsx", "typescriptreact"),
            ("Cargo.toml", "toml"),
            ("build.zig", "zig"),
            ("config.zon", "zig"),
        ] {
            assert_eq!(
                reg.language_name_for_path(path).as_deref(),
                Some(lang),
                "{path} resolves to {lang}"
            );
        }
    }

    #[test]
    fn byte_to_point_handles_first_line() {
        let src = b"hello world";
        assert_eq!(byte_to_point(src, 0), tree_sitter::Point::new(0, 0));
        assert_eq!(byte_to_point(src, 5), tree_sitter::Point::new(0, 5));
        assert_eq!(byte_to_point(src, 11), tree_sitter::Point::new(0, 11));
    }

    #[test]
    fn byte_to_point_counts_newlines() {
        let src = b"a\nbb\nccc";
        assert_eq!(byte_to_point(src, 0), tree_sitter::Point::new(0, 0));
        assert_eq!(byte_to_point(src, 1), tree_sitter::Point::new(0, 1));
        assert_eq!(byte_to_point(src, 2), tree_sitter::Point::new(1, 0));
        assert_eq!(byte_to_point(src, 4), tree_sitter::Point::new(1, 2));
        assert_eq!(byte_to_point(src, 5), tree_sitter::Point::new(2, 0));
        assert_eq!(byte_to_point(src, 8), tree_sitter::Point::new(2, 3));
    }

    #[test]
    fn byte_to_point_clamps_past_end() {
        let src = b"abc\ndef";
        assert_eq!(byte_to_point(src, 999), byte_to_point(src, src.len()));
    }

    #[test]
    fn parse_view_records_insert_input_edit() {
        let mut buf = fresh_buffer("scratch.rs");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"fn main() {}\n",
        })
        .unwrap();
        let (view, handle) = rust_view(&buf);
        let _vid = buf.attach_view(Box::new(view));
        // Insert " let x = 1;" between `{` and `}`. Position 11 is
        // the byte after `{` in "fn main() {}\n".
        buf.apply_edit(EditOp::Insert {
            pos: 11,
            bytes: b" let x = 1;",
        })
        .unwrap();
        assert_eq!(handle.pending_edit_count(), 1);
        assert_eq!(handle.source_snapshot(), b"fn main() { let x = 1;}\n");
    }

    #[test]
    fn parse_view_round_trips_initial_then_incremental_parse() {
        let mut buf = fresh_buffer("scratch.rs");
        buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"fn main() {}\n",
        })
        .unwrap();
        let (view, handle) = rust_view(&buf);
        let _vid = buf.attach_view(Box::new(view));

        // Cold parse.
        let bundle = parse_synchronously(&handle);
        assert_eq!(bundle.tree.root_node().kind(), "source_file");
        assert_eq!(handle.pending_edit_count(), 0);

        // One incremental edit, then re-parse with the new source +
        // accumulated InputEdits.
        buf.apply_edit(EditOp::Insert {
            pos: 11,
            bytes: b" let _ = 1;",
        })
        .unwrap();
        assert_eq!(handle.pending_edit_count(), 1);
        let bundle = parse_synchronously(&handle);
        assert_eq!(bundle.tree.root_node().kind(), "source_file");
        assert_eq!(
            bundle.source.as_ref(),
            b"fn main() { let _ = 1;}\n",
            "bundle source must reflect post-edit bytes"
        );
        // Pending list cleared on make_request.
        assert_eq!(handle.pending_edit_count(), 0);
    }

    #[test]
    fn registry_tracks_inflight_parse_jobs_by_buffer() {
        let registry = SyntaxRegistry::new();
        let a = BufferId::next();
        let b = BufferId::next();

        registry.record_parse_job(11, a);
        registry.record_parse_job(12, b);

        assert!(registry.has_pending_parse_job_for(a));
        assert!(registry.has_pending_parse_job_for(b));

        assert_eq!(registry.take_parse_job(11), Some(a));
        assert!(!registry.has_pending_parse_job_for(a));
        assert!(registry.has_pending_parse_job_for(b));
    }
}
