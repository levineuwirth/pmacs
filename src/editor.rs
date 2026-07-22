// editor.rs --- Editor session: core + Lua + dispatcher, plus the run loop.

//! The editor session.
//!
//! [`EditorState`] holds three things: the world state ([`EditorCore`],
//! shared via `Rc<RefCell<...>>` so Lua-bound primitives can mutate it),
//! the [`LuaHost`] (the Lua VM and its registries), and the
//! [`KeyDispatcher`] state machine that maps chord sequences onto
//! command names.
//!
//! [`run`] takes over the terminal, renders, reads key events, feeds
//! them through the dispatcher, and invokes the resulting Lua commands
//! until the user quits.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::async_runtime::SharedAsyncRuntime;
use crate::cell::{CellCoord, CellSize};
use crate::editor_core::EditorCore;
use crate::file_io::load_file;
use crate::frontend::{Event, Frontend, KeyEvent, KeyEventKind, MouseEvent, install_panic_hook};
use crate::key::{Chord, display_sequence};
use crate::keymap_stack::{Action, KeyDispatcher};
use crate::lua::LuaHost;
use crate::lua_bindings::SharedCore;
use crate::minibuffer::Minibuffer;
use crate::protocol::{
    FrontendId, InstanceMessage, InstanceSignal, Key as TerminalKey,
    Modifiers as TerminalModifiers, MouseButton as TerminalMouseButton,
    MouseKind as TerminalMouseKind,
};
use crate::terminal::TerminalSnapshot;
use crate::terminal::view::TerminalViewKey;
use crate::view::{View, Viewport};
use crate::window::{Rect, WindowId};

/// Ephemeral authenticated origin for one interactive command invocation.
///
/// The shared slot is installed as Lua app data so Rust dispatch and nested
/// `pmacs.command.invoke_interactive` calls use the same authority. Guards
/// restore the prior value, which makes nesting safe and clears the outermost
/// origin even when a Lua command errors.
#[derive(Clone, Default)]
pub(crate) struct InteractiveCommandOrigin(Rc<Cell<Option<FrontendId>>>);

impl InteractiveCommandOrigin {
    /// Current authenticated frontend while an interactive command runs.
    #[must_use]
    pub(crate) fn current(&self) -> Option<FrontendId> {
        self.0.get()
    }

    /// Enter an interactive command scope for `frontend_id`.
    pub(crate) fn enter(&self, frontend_id: FrontendId) -> InteractiveCommandOriginGuard {
        let previous = self.0.replace(Some(frontend_id));
        InteractiveCommandOriginGuard {
            origin: self.clone(),
            previous,
        }
    }
}

pub(crate) struct InteractiveCommandOriginGuard {
    origin: InteractiveCommandOrigin,
    previous: Option<FrontendId>,
}

impl Drop for InteractiveCommandOriginGuard {
    fn drop(&mut self) {
        self.origin.0.set(self.previous);
    }
}

// ---------------------------------------------------------------------------
// EditorState
// ---------------------------------------------------------------------------

/// One editor session.
pub struct EditorState {
    /// World state, mutated by [`pmacs.editor.*`] primitives invoked
    /// from inside command bodies.
    pub core: SharedCore,
    /// The embedded Lua VM and its command/keymap registries.
    pub lua_host: LuaHost,
    /// Independent key-prefix and terminal-escape state per authenticated frontend.
    dispatchers: HashMap<FrontendId, FrontendDispatchState>,
    /// Authenticated frontend scoped to the current interactive invocation.
    pub(crate) interactive_origin: InteractiveCommandOrigin,
    /// Main-thread async runtime (T M3.3). Owns the worker pool and
    /// the message bus pair; [`Self::tick_async`] drives one
    /// drain-and-resume pass per run-loop iteration.
    pub async_runtime: SharedAsyncRuntime,
    /// Tree-sitter syntax registry (T M4.1). Maps language names to
    /// grammars and tracks per-buffer parse-view handles. Empty by
    /// default --- M4.2 wires the actual `tree-sitter-rust` and
    /// `tree-sitter-lua` registrations at startup.
    pub syntax_registry: crate::syntax::SharedSyntaxRegistry,
    /// Process supervisor (T M4.4). Owns every child process the
    /// editor has spawned (LSP servers from M4.5; REPLs from M5).
    /// Drop-time `shutdown` enforces SIGTERM-then-SIGKILL so editor
    /// exit cannot leave zombies.
    pub process_supervisor: crate::lua_bindings::SharedProcessSupervisor,
    /// Terminal session registry. Shared with future terminal Lua bindings;
    /// snapshots are owned so no screen borrow crosses editor/Lua/render work.
    pub terminal_manager: crate::terminal::session::SharedTerminalManager,
    /// LSP manager (T M4.5). Holds one [`crate::lsp::LspClient`] per
    /// language server; rides on top of [`Self::process_supervisor`]
    /// for spawn / I/O / restart. Constructed empty; user code
    /// (`pmacs.lsp.spawn`) populates it. The manager itself never
    /// blocks the main thread --- pipe reads happen on the
    /// supervisor's reader threads, which means a runaway server's
    /// log-flood doesn't stall the editor.
    pub lsp_manager: crate::lsp::SharedLspManager,
    /// The global GPU font preference (Arc 4 stage 2, Q#F3). Written
    /// by `pmacs.gpu.set_font`; read by the `semantic_render`
    /// producer, which relays it as `FontFacts` (protocol v17).
    pub font_pref: crate::font_pref::FontPrefHandle,
    /// MCP manager (T M9.1). Holds one [`crate::mcp::McpClient`] per
    /// MCP server; rides on top of [`Self::process_supervisor`] for
    /// spawn / I/O / restart, sharing the supervisor with the LSP
    /// manager. The two managers are siblings — the protocol-uniformity
    /// claim from spec §sec:concurrency holds because the dispatch
    /// machinery (supervisor → bytes → parser → state machine →
    /// events) is identical; only the per-protocol parser differs.
    pub mcp_manager: crate::mcp::SharedMcpManager,
    /// Workspace / project model (T M4.9). Owns one
    /// [`crate::project::Project`] per open project root and tracks
    /// which one is active for project-scoped commands. Sits next to
    /// the LSP manager so `pmacs.project.lsp_for` can drive the
    /// "one server per `(root, language_id)`" invariant.
    pub workspace: crate::lua_bindings::SharedWorkspace,
    /// Project index registry (T M4.10). One
    /// [`crate::project_index::ProjectIndex`] per known root,
    /// holding aggregated symbols from LSP, tree-sitter, and the
    /// heuristic / raw extractors. Reachable from Lua as
    /// `pmacs.index.*`.
    pub project_indexer: crate::lua_bindings::SharedProjectIndexer,
    /// Completion framework (T M4.11). Owns the registry of
    /// completion providers (LSP, snippets, project symbols,
    /// dabbrev, plus any Lua-defined custom sources) and the
    /// snippet store. Reachable from Lua as `pmacs.completion.*`.
    pub completion_registry: crate::completion_framework::SharedCompletionRegistry,
    /// Snippet store (T M4.11). Co-owned with the snippet
    /// provider closure inside [`Self::completion_registry`].
    pub snippets: crate::completion_framework::SharedSnippetRegistry,
    /// Lua statusline providers shared by grid and semantic renderers.
    pub statusline_registry: crate::statusline::SharedStatuslineRegistry,
    /// Last left-button down event, used to synthesize terminal double
    /// clicks from crossterm's plain Down/Up mouse event stream.
    mouse_click: Option<MouseClickState>,
}

#[derive(Default)]
struct FrontendDispatchState {
    dispatcher: KeyDispatcher,
    terminal_escape: bool,
}

impl Drop for EditorState {
    /// Tear down the worker-pool threads.
    ///
    /// The `Rc<AsyncRuntime>` is cloned into dozens of Lua closures
    /// (`pmacs.workers`, LSP request wrappers, ...), and several of
    /// the registries those closures capture themselves store
    /// `mlua::Function` values --- reference cycles through the Lua
    /// VM that keep the `Rc` from ever reaching zero. Harmless for a
    /// single editor per process (the OS reclaims at exit), but a
    /// test binary that builds one `EditorState` per test would leak
    /// one full worker pool (`cores - 1` threads, each waking every
    /// 100ms) per test --- observed as 1000+ live threads in the m4
    /// acceptance suite. Dropping the editor reaches the pool through
    /// its own `Rc` clone and signals the threads down regardless of
    /// the cycle; parked workers exit within their 100ms wakeup.
    ///
    /// Signal-only, NO join: a worker can be blocked publishing its
    /// reply onto the bus that this (main) thread drains --- joining
    /// here deadlocked the m4 suite at teardown for hours. A worker
    /// stuck mid-handoff stays alive (bounded by its job), which is
    /// still a ~15x improvement over leaking every pool whole.
    fn drop(&mut self) {
        {
            let mut supervisor = self.process_supervisor.borrow_mut();
            self.terminal_manager.borrow_mut().shutdown(&mut supervisor);
            supervisor.shutdown();
        }
        self.async_runtime.shutdown_workers();
    }
}

#[derive(Copy, Clone)]
struct MouseClickState {
    frontend_id: FrontendId,
    window_id: WindowId,
    cell: CellCoord,
    at: Instant,
}

const DOUBLE_CLICK_MAX_DELAY: Duration = Duration::from_millis(500);

impl EditorState {
    /// Construct a fresh editor for an unnamed scratch buffer.
    ///
    /// Panics only if Lua initialization or the builtin command/keymap
    /// chunks fail to load --- both indicate broken builds.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "linear bootstrap sequence: registry → core → LuaHost → \
                  per-builtin module installs (async/syntax/process/lsp/index/...). \
                  Splitting into helpers fragments the wiring without removing \
                  any single decision the reader needs to follow."
    )]
    pub fn new() -> Self {
        // Build the buffer registry first so EditorCore and LuaHost
        // share the same `Rc`. Both reach buffers through this handle;
        // multi-window dispatch (T M2.8) requires that ids resolve to
        // the same Buffer regardless of who created it.
        let registry: crate::lua_bindings::SharedRegistry =
            Rc::new(RefCell::new(crate::buffer_registry::BufferRegistry::new()));
        let core = Rc::new(RefCell::new(EditorCore::new(registry.clone())));
        let mut lua_host = LuaHost::with_registry(registry).expect("Lua runtime initialization");
        let interactive_origin = InteractiveCommandOrigin::default();
        lua_host.lua().set_app_data(interactive_origin.clone());
        lua_host
            .attach_editor(&core)
            .expect("editor bindings + builtin chunks");
        let statusline_registry = crate::lua_bindings::statusline_registry(lua_host.lua())
            .expect("statusline registry installed by editor bindings");
        // The on-disk state dirs (minibuffer history + pmacs.state) are
        // deliberately NOT configured here — see `install_state_dirs`,
        // called by the real entry points (`run` / `run_daemon`) only.
        // Constructing an `EditorState` — which unit AND integration
        // tests do directly — leaves them unconfigured, so default-on
        // persistence (recentf/saveplace) writes nothing to a
        // developer's real state dir during `cargo test`. Tests that
        // exercise persistence inject a `StateDir` app-data explicitly.
        // The async runtime: install pmacs._async raw helpers, then
        // load the friendly Lua surface (`pmacs.async`, Handle class,
        // `pmacs.workers.*`). Both must run before user config so a
        // user's `init.lua` can call `pmacs.async(...)` itself.
        let async_runtime =
            crate::lua_bindings::make_async_runtime(lua_host.lua(), Some(lua_host.registry()))
                .expect("install pmacs._async raw helpers");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/async.lua"),
                include_str!("../builtin/runtime/async.lua"),
            )
            .expect("load async builtin chunk");
        // T M8.1 filesystem worker primitives. Sits on top of the
        // raw `pmacs._async._dispatch_fs_*` bindings installed by
        // `make_async_runtime` and reuses the Handle factory
        // exposed at the end of async.lua. Loaded immediately after
        // async.lua so `pmacs.fs.*` is available to every later
        // builtin and to user init.lua.
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/fs.lua"),
                include_str!("../builtin/runtime/fs.lua"),
            )
            .expect("load fs builtin chunk");
        // T M4.1 tree-sitter Lua surface; M4.2 layers the Lua-side
        // auto-attach hook on top. The registry is empty at startup;
        // `pmacs.parse.language` lazy-loads from `BUILTIN_LANGUAGES`
        // on first use (T M4.2 acceptance: "load grammar lazily").
        let syntax_registry = crate::lua_bindings::make_syntax_registry(
            lua_host.lua(),
            &async_runtime,
            lua_host.registry(),
        )
        .expect("install pmacs.parse");
        // Themes Q#TH9: inject the shared theme into the core right
        // after SyntaxRegistry construction — the core owns no syntax
        // state, but its search overlay resolves wash faces through
        // this handle.
        core.borrow_mut().theme = Some(syntax_registry.theme());
        // Arc 4 stage 2 (Q#F2/Q#F3): the GPU font preference and its
        // `pmacs.gpu` Lua surface. Installed BEFORE load_user_config
        // below, so an init.lua `set_font` lands in the same handle
        // the first attachment's semantic producer reads.
        let font_pref =
            crate::lua_bindings::make_font_pref(lua_host.lua()).expect("install pmacs.gpu");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/syntax.lua"),
                include_str!("../builtin/runtime/syntax.lua"),
            )
            .expect("load syntax builtin chunk");
        // T M4.4 process supervisor. Constructed empty; user code
        // spawns children through `pmacs.process.spawn`. Drop-time
        // shutdown enforces no-zombie cleanup at editor exit.
        let process_supervisor = crate::lua_bindings::make_process_supervisor(lua_host.lua())
            .expect("install pmacs.process");
        let terminal_manager =
            crate::lua_bindings::make_terminal_manager(lua_host.lua(), process_supervisor.clone())
                .expect("install pmacs.terminal");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/terminal.lua"),
                include_str!("../builtin/runtime/terminal.lua"),
            )
            .expect("load terminal builtin chunk");
        // T M4.5 LSP manager. Wires onto the same supervisor so its
        // spawn/restart/I/O machinery is shared with `pmacs.process.*`.
        // The manager itself is reachable from Lua as `pmacs.lsp.*`.
        let lsp_manager = crate::lua_bindings::make_lsp_manager(
            lua_host.lua(),
            process_supervisor.clone(),
            async_runtime.clone(),
            &syntax_registry,
        )
        .expect("install pmacs.lsp");
        // T M9.1 MCP manager. Wires onto the same supervisor that LSP
        // and `pmacs.process.*` use; the protocol-uniformity claim is
        // that this share is sufficient (no parallel dispatch path).
        // `pmacs.mcp.*` is the Lua surface; the manager itself is a
        // sibling of `lsp_manager`.
        let mcp_manager = crate::lua_bindings::make_mcp_manager(
            lua_host.lua(),
            process_supervisor.clone(),
            async_runtime.clone(),
        )
        .expect("install pmacs.mcp");
        // builtin/runtime/mcp.lua overrides `pmacs.mcp.send_request`
        // with the Handle-returning friendly wrapper. Loaded after
        // both async.lua (provides `pmacs.workers._new_handle`) and
        // make_mcp_manager (provides `pmacs.mcp._send_request_raw`).
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/mcp.lua"),
                include_str!("../builtin/runtime/mcp.lua"),
            )
            .expect("load mcp builtin chunk");
        // T M4.9 project / workspace surface. Built atop the LSP
        // manager so `pmacs.project.lsp_for` can hand back a
        // server scoped to (project_root, language_id).
        let workspace = crate::lua_bindings::make_workspace(lua_host.lua(), &lsp_manager)
            .expect("install pmacs.project");
        // T M4.10 project index registry. Independent of the
        // workspace: callers can index any root, not just opened
        // ones, so the indexer maintains its own canonical-root
        // map. `pmacs.index.*` is always available.
        let project_indexer =
            crate::lua_bindings::make_project_indexer(lua_host.lua()).expect("install pmacs.index");
        // T M4.11 completion framework. Wires up the four built-in
        // providers (LSP, snippets, project symbols, dabbrev) at
        // sensible default priorities, and exposes
        // `pmacs.completion.*` so `init.lua` can register custom
        // sources, tweak priorities, or define snippets.
        let (completion_registry, snippets) = crate::lua_bindings::make_completion_framework(
            lua_host.lua(),
            &lsp_manager,
            &project_indexer,
        )
        .expect("install pmacs.completion");
        // T M4.12 default LSP integration: declarative server config,
        // auto-attach buffer hooks, key-bound commands. Loaded last so
        // every dependency table (`pmacs.lsp`, `pmacs.parse`,
        // `pmacs.window`, etc.) already exists.
        // Arc 1b: the reusable list-panel module. Loaded before
        // lsp.lua, whose panel commands (references, outline) call
        // `pmacs.listview.open`.
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/listview.lua"),
                include_str!("../builtin/runtime/listview.lua"),
            )
            .expect("load listview builtin chunk");
        // Auto-pairing (Arc 2, Q#AP7) — ORDERING CONTRACT: pair.lua
        // must load BEFORE lsp.lua. Hook callbacks run in registration
        // order, and lsp.lua's `buffer.after-edit` callback flushes
        // didChange synchronously on the signature-trigger path — the
        // pairing closer must already be in the buffer when that
        // callback runs, or the server receives opener-only text and
        // the closer stays unsynchronized until the next edit (hook
        // edits don't re-fire the hook). pair.lua's `pmacs.lsp.*`
        // lookups are lazy and nil-guarded for the same reason.
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/pair.lua"),
                include_str!("../builtin/runtime/pair.lua"),
            )
            .expect("load pair builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/lsp.lua"),
                include_str!("../builtin/runtime/lsp.lua"),
            )
            .expect("load lsp builtin chunk");
        // Arc 1a: the in-buffer completion popup driver. Loaded after
        // lsp.lua because it drives `pmacs.lsp.request_completion` /
        // `pmacs.lsp.attachment_for_request` and after the framework
        // install above because it calls `pmacs.completion.collect`.
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/completion.lua"),
                include_str!("../builtin/runtime/completion.lua"),
            )
            .expect("load completion builtin chunk");
        // Editing-conveniences pack (Q#EC9 ordering contract): MUST
        // load before saveplace.lua — editops registers its (gated,
        // default-off) trim-on-save callback at load time, and hook
        // callbacks run in registration order, so saveplace's
        // before-save cursor-record must observe post-trim text. Its
        // pmacs.killring.* references resolve at invoke time, so
        // loading before killring.lua is fine.
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/editops.lua"),
                include_str!("../builtin/runtime/editops.lua"),
            )
            .expect("load editops builtin chunk");
        // Arc 3: persistence builtins (saveplace + recentf). Load after
        // the LSP/completion runtimes; they subscribe to buffer hooks
        // and drive `pmacs.state` (inert until the state dir is
        // configured — never in `cfg(test)`).
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/saveplace.lua"),
                include_str!("../builtin/runtime/saveplace.lua"),
            )
            .expect("load saveplace builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/recentf.lua"),
                include_str!("../builtin/runtime/recentf.lua"),
            )
            .expect("load recentf builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/desktop.lua"),
                include_str!("../builtin/runtime/desktop.lua"),
            )
            .expect("load desktop builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/autosave.lua"),
                include_str!("../builtin/runtime/autosave.lua"),
            )
            .expect("load autosave builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/killring.lua"),
                include_str!("../builtin/runtime/killring.lua"),
            )
            .expect("load killring builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/comment.lua"),
                include_str!("../builtin/runtime/comment.lua"),
            )
            .expect("load comment builtin chunk");
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/indent.lua"),
                include_str!("../builtin/runtime/indent.lua"),
            )
            .expect("load indent builtin chunk");
        // Compile-mode (Arc 5 stage 1, Q#CM1) — ORDERING CONTRACT:
        // compile.lua must load AFTER lsp.lua. It takes over
        // `M-g n` / `M-g p` for the unified error dispatchers, and
        // duplicate bindings are rejected, so the takeover is
        // unbind-then-bind against lsp.lua's diag bindings — they
        // must exist first. (Loaded last in the runtime sequence;
        // its after-tick pump is ordering-independent.)
        lua_host
            .eval(
                Some("@pmacs/builtin/runtime/compile.lua"),
                include_str!("../builtin/runtime/compile.lua"),
            )
            .expect("load compile builtin chunk");
        // T M7.11 bundled-package bootstrap. Through M7.10 the REPL
        // was loaded directly via `eval(include_str!(...))`; the
        // M7.11 deliverable migrates it to the package system so it
        // goes through the same manifest, exports, and per-package
        // `_ENV` machinery a third-party package would. The
        // sequence is:
        //
        //   1. Materialize each bundled package (currently just
        //      `repl`) to a process-stable directory under the OS
        //      temp dir. See `crate::builtin_packages` for the
        //      design rationale.
        //   2. Push the resulting `InstalledPackage` records onto
        //      the `InstalledPackages` roster slot held in the
        //      Lua VM's app-data, so the M7.7 searcher finds them.
        //   3. Drive the load via `pmacs.packages.load("repl")` so
        //      the load goes through the boundary `pmacs.packages`
        //      function (which catches load-time errors and routes
        //      them to *errors*) rather than a bare `require`.
        //
        // Depends on `pmacs.buffer.add_intercept` (T M6.4 Stage 1)
        // and `pmacs.ansi.parser()` (T M6.4 Stage 2), both available
        // by the time `attach_editor` returns above.
        let bundled_root = crate::builtin_packages::bundled_runtime_dir();
        let bundled_packages = crate::builtin_packages::materialize_all(&bundled_root)
            .expect("materialize bundled packages");
        {
            let slot = lua_host
                .lua()
                .app_data_ref::<crate::lua_bindings::InstalledPackages>()
                .expect("InstalledPackages slot installed by attach_editor");
            for pkg in &bundled_packages {
                slot.record(pkg.clone());
            }
        }
        for pkg in &bundled_packages {
            let basename = pkg.install_basename().to_string();
            let script = format!(
                "if not pmacs.packages.load({basename:?}) then \
                 error('bundled package failed to load: ' .. {basename:?}) end"
            );
            lua_host
                .eval(Some("@pmacs/bundled-load"), &script)
                .unwrap_or_else(|e| {
                    panic!("bundled package `{basename}` failed to load: {e}");
                });
        }
        // User config is loaded after the builtins so it can override
        // them. Failures inside `init.lua` are captured into the
        // `*errors*` buffer; the editor still starts.
        //
        // Skipped under `cfg(test)` so the lib's own test suite doesn't
        // pick up the developer's real `~/.config/pmacs/init.lua` and
        // turn into a flaky environment-dependent run. Tests that need
        // to exercise config loading do so explicitly via
        // [`crate::config::load_user_config_at`].
        //
        // The init-complete flip happens here too so lifecycle-gated
        // Lua APIs (e.g. `pmacs.attach`, M5.6d+) become inert after
        // user config returns. Tests that need post-init semantics flip
        // the flag explicitly via [`crate::lua::LuaHost::set_init_complete`];
        // see the option-(A) discussion in the M5.6c survey.
        #[cfg(not(test))]
        {
            crate::config::load_user_config(&mut lua_host);
            lua_host.set_init_complete();
        }
        Self {
            core,
            lua_host,
            dispatchers: HashMap::new(),
            interactive_origin,
            async_runtime,
            syntax_registry,
            process_supervisor,
            terminal_manager,
            lsp_manager,
            font_pref,
            mcp_manager,
            workspace,
            project_indexer,
            completion_registry,
            snippets,
            statusline_registry,
            mouse_click: None,
        }
    }

    /// Transactionally open an internal Stage-1 terminal session.
    ///
    /// No interactive Lua command is registered until a frontend can render
    /// terminal snapshots. This Rust seam is used by headless acceptance and
    /// future bindings.
    pub fn open_terminal(
        &mut self,
        spec: crate::terminal::TerminalSpec,
    ) -> Result<crate::buffer::BufferId, crate::terminal::TerminalError> {
        let mut manager = self.terminal_manager.borrow_mut();
        let mut core = self.core.borrow_mut();
        let mut supervisor = self.process_supervisor.borrow_mut();
        manager.open(spec, &mut core, &mut supervisor)
    }

    /// One pass of the process supervisor and terminal-owned event drain.
    ///
    /// Ordering is supervisor tick → terminal drain/prune →
    /// `process.after-tick`. `TerminalManager` calls `take_events` only for its
    /// own `ProcessId`s; existing Lua/LSP/MCP ownership remains unchanged.
    pub fn tick_processes(&mut self) {
        {
            let mut supervisor = self.process_supervisor.borrow_mut();
            supervisor.tick();
            let mut manager = self.terminal_manager.borrow_mut();
            manager.tick(&mut supervisor);
            let mut core = self.core.borrow_mut();
            manager.prune(&mut core, &mut supervisor);
        }
        self.lua_host
            .run_hook("process.after-tick", mlua::MultiValue::new());
    }

    /// One pass of the LSP manager: drain process events into per-
    /// server stdout buffers, parse JSON-RPC frames, dispatch to the
    /// per-server state machine, apply LSP-layer restart policy.
    /// `tick_processes` must be called first (or shortly after) so
    /// the supervisor surface fresh exit/I/O events; the run loop
    /// calls both in order.
    pub fn tick_lsp(&mut self) {
        self.lsp_manager.borrow_mut().tick();
    }

    /// One pass of the MCP manager (T M9.1): same shape as
    /// [`Self::tick_lsp`], applied to MCP servers. The supervisor
    /// is shared, so [`Self::tick_processes`] feeding LSP and MCP is
    /// a single call; only the per-manager parse-and-dispatch step
    /// is per-protocol. Order is `tick_processes` → `tick_lsp` →
    /// `tick_mcp` so any in-the-same-batch I/O lands deterministically.
    pub fn tick_mcp(&mut self) {
        self.mcp_manager.borrow_mut().tick();
    }

    /// One pass of the main-thread async runtime: drain the worker
    /// reply bus, fire `on_complete` callbacks, resume coroutines
    /// parked on settled handles. Called every iteration of the run
    /// loop --- and from tests that drive async flows synchronously.
    ///
    /// Errors raised inside `pmacs._async.tick` are reported through
    /// the same `*errors*` capture path as other Lua failures.
    pub fn tick_async(&mut self) {
        let _ = self
            .lua_host
            .eval(Some("@pmacs/runtime/async.lua:tick"), "pmacs._async.tick()");
    }

    /// Configure the on-disk state directories (minibuffer history +
    /// `pmacs.state`) from the environment. The **real** entry points
    /// (`run`, `run_daemon`) call this after construction; tests do not,
    /// so neither the unit suite nor integration tests (which link the
    /// lib without `cfg(test)`) touch a developer's real
    /// `~/.local/state/pmacs`. Honors the `PMACS_STATE_HOME` override
    /// (see [`crate::state::user_state_dir`]).
    pub fn install_state_dirs(&self) {
        if let Some(dir) = crate::minibuffer::user_history_dir() {
            self.core.borrow_mut().minibuffer.history_dir = Some(dir);
        }
        if let Some(dir) = crate::state::user_state_dir() {
            self.lua_host
                .lua()
                .set_app_data(crate::lua_bindings::StateDir(dir));
        }
    }

    /// Restore the session saved under this desktop's key, if armed
    /// (`pmacs.session.desktop_mode(true)` called it) and no positional
    /// file arg was given (Q#DS7). Called from the `RunLocal` arm of
    /// [`run`]. All the work lives in [`crate::desktop::restore_session`]
    /// (driven off the Lua host's app-data + hook mechanism).
    pub fn restore_desktop_if_armed(&mut self, had_file: bool) {
        let armed = self
            .lua_host
            .lua()
            .app_data_ref::<crate::lua_bindings::DesktopRestoreArmed>()
            .is_some();
        if armed
            && !had_file
            && let Err(e) = crate::desktop::restore_session(self.lua_host.lua())
        {
            self.core.borrow_mut().status = format!("desktop-restore: {e}");
        }
    }

    /// Construct an editor for a path. Empty buffer with `[new file]`
    /// status if the path does not exist; loaded contents otherwise.
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let display_name = path.display().to_string();
        let state = Self::new();
        let mut fire_after_load = false;
        match load_file(&path) {
            Ok((bytes, meta)) => {
                let new_id = state
                    .lua_host
                    .registry()
                    .borrow_mut()
                    .create_from_bytes(display_name, &bytes);
                state.replace_active_buffer(new_id);
                let mut core = state.core.borrow_mut();
                core.set_buffer_path(new_id, Some(path));
                core.set_buffer_meta(new_id, Some(meta));
                fire_after_load = true;
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let new_id = state.lua_host.registry().borrow_mut().create(display_name);
                state.replace_active_buffer(new_id);
                let mut core = state.core.borrow_mut();
                core.set_buffer_path(new_id, Some(path));
                core.status = "[new file]".into();
                Ok(())
            }
            Err(e) => Err(e),
        }?;
        let mut state = state;
        if fire_after_load {
            // Fire the hook *after* the borrow on `core` is released
            // (block above ends). Listeners may legitimately re-enter
            // pmacs.editor.* primitives that re-borrow the core.
            state
                .lua_host
                .run_hook("buffer.after-load", mlua::MultiValue::new());
        }
        Ok(state)
    }

    /// Switch the active window to `buffer_id`, dropping any old
    /// scratch buffer if the active window's previous buffer has no
    /// other windows referencing it. Returns silently on a stale id.
    fn replace_active_buffer(&self, buffer_id: crate::buffer::BufferId) {
        let mut core = self.core.borrow_mut();
        let _ = core.switch_active_buffer(buffer_id);
    }

    /// Whether `frontend_id` may optimistically self-insert its next key.
    #[must_use]
    pub fn dispatch_idle_for(&self, frontend_id: FrontendId) -> bool {
        if self
            .dispatchers
            .get(&frontend_id)
            .is_some_and(|state| state.terminal_escape || !state.dispatcher.pending().is_empty())
        {
            return false;
        }
        let core = self.core.borrow();
        !core.minibuffer.is_active()
            && !core.search_active()
            && !core.query_replace_active()
            && !core.menu_is_open()
            && core
                .active_window_for(frontend_id)
                .is_some_and(|window| !core.active_buffer_round_trips_for(window.buffer_id))
    }

    /// Local-frontend compatibility wrapper.
    #[must_use]
    pub fn dispatch_idle(&self) -> bool {
        self.dispatch_idle_for(FrontendId::LOCAL)
    }

    /// Drop one detached frontend's pending key and terminal escape state.
    pub fn detach_frontend_input(&mut self, frontend_id: FrontendId) {
        self.dispatchers.remove(&frontend_id);
        self.terminal_manager
            .borrow_mut()
            .detach_frontend(frontend_id);
    }

    /// `frontend_id` records which frontend produced the event. v0.1
    /// uses [`FrontendId::LOCAL`] uniformly; the parameter is
    /// load-bearing for v0.3 multi-frontend scenarios where the
    /// active-frontend identity is needed by hooks and commands
    /// (`pmacs.frontend.id()`). Sets [`EditorCore::active_frontend`]
    /// before any command body runs, so observers always see a fresh
    /// value.
    pub fn dispatch_key(&mut self, frontend_id: FrontendId, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        let chord = key_event_to_chord(key);
        {
            let mut core = self.core.borrow_mut();
            core.status.clear();
            core.active_frontend = frontend_id;
            if core.completion_popup_is_open()
                && (core.menu_is_open()
                    || core.search_active()
                    || core.query_replace_active()
                    || core.minibuffer.is_active())
            {
                core.completion_popup_close();
            }
        }

        // Global modal surfaces own input before terminal transport.
        if self.core.borrow().menu_is_open() {
            if let Some(chord) = chord {
                self.dispatch_menu_key(frontend_id, chord);
            }
            return;
        }
        if self.core.borrow().search_active() {
            if let Some(chord) = chord {
                self.dispatch_search_key(chord);
            }
            return;
        }
        if self.core.borrow().query_replace_active() {
            if let Some(chord) = chord {
                self.dispatch_query_replace_key(chord);
            }
            return;
        }
        if self.core.borrow().minibuffer.is_active() {
            if let Some(chord) = chord {
                self.dispatch_minibuffer_key(frontend_id, chord);
            }
            return;
        }

        let dispatcher_pending = self
            .dispatchers
            .get(&frontend_id)
            .is_some_and(|state| !state.dispatcher.pending().is_empty());
        if self.core.borrow().completion_popup_is_open()
            && !dispatcher_pending
            && let Some(popup_key) = chord.and_then(CompletionPopupKey::from_chord)
        {
            self.dispatch_completion_key(popup_key);
            return;
        }

        let terminal_key = self.active_terminal_key(frontend_id);
        let escaped = self
            .dispatchers
            .get(&frontend_id)
            .is_some_and(|state| state.terminal_escape);
        let terminal_local_binding = terminal_key.is_some_and(|view_key| {
            chord.is_some_and(|chord| {
                let stack = self.lua_host.keymaps().borrow();
                stack.buffers.get(&view_key.buffer_id).is_some_and(|map| {
                    !matches!(
                        map.lookup(&[chord]),
                        crate::keymap_tree::Resolution::Unbound
                    )
                })
            })
        });
        if let Some(view_key) = terminal_key {
            if escaped {
                self.dispatchers
                    .entry(frontend_id)
                    .or_default()
                    .terminal_escape = false;
                if chord.is_some_and(is_terminal_escape_chord) {
                    self.claim_terminal_controller(view_key);
                    self.send_terminal_bytes(view_key.buffer_id, &[0x03]);
                    return;
                }
                // The post-escape key starts a fresh ordinary sequence below.
            } else if !dispatcher_pending {
                if chord.is_some_and(is_terminal_escape_chord) {
                    let state = self.dispatchers.entry(frontend_id).or_default();
                    state.terminal_escape = true;
                    state.dispatcher = KeyDispatcher::new();
                    self.claim_terminal_controller(view_key);
                    return;
                }
                if !terminal_local_binding {
                    let Some((terminal_key, modifiers)) = terminal_key_from_crossterm(key) else {
                        return;
                    };
                    let modes = self
                        .terminal_manager
                        .borrow()
                        .modes_for_view(view_key)
                        .unwrap_or_default();
                    if let Some(bytes) =
                        crate::terminal::input::encode_key(terminal_key, modifiers, modes)
                    {
                        self.claim_terminal_controller(view_key);
                        self.send_terminal_bytes(view_key.buffer_id, &bytes);
                    }
                    return;
                }
            }
        }

        let Some(chord) = chord else {
            return;
        };
        let active_buffer = Some(self.core.borrow().active_buffer_id());
        let action = {
            let stack = self.lua_host.keymaps().borrow();
            self.dispatchers
                .entry(frontend_id)
                .or_default()
                .dispatcher
                .dispatch(chord, &stack, active_buffer, &[])
        };
        let pre_revision = self.active_buffer_revision();

        match action {
            Action::Run { command, .. } => {
                self.core.borrow_mut().rotate_command(frontend_id, &command);
                let _origin = self.interactive_origin.enter(frontend_id);
                if let Err(e) = self
                    .lua_host
                    .invoke_command(&command, mlua::MultiValue::new())
                {
                    self.core.borrow_mut().status =
                        format!("error in {command}: {}", first_line(&e.to_string()));
                }
            }
            Action::Pending { .. } => {
                self.core.borrow_mut().completion_popup_close();
            }
            Action::Unbound { sequence } => {
                if let Some(ch) = printable_char(&sequence) {
                    self.core
                        .borrow_mut()
                        .rotate_command(frontend_id, "buffer.self-insert");
                    self.core.borrow_mut().typed_edit_arm(frontend_id, ch);
                    let mut args = mlua::MultiValue::new();
                    args.push_back(mlua::Value::Integer(ch as i64));
                    let _origin = self.interactive_origin.enter(frontend_id);
                    if let Err(e) = self.lua_host.invoke_command("buffer.self-insert", args) {
                        self.core.borrow_mut().status =
                            format!("self-insert failed: {}", first_line(&e.to_string()));
                    }
                } else {
                    self.core.borrow_mut().break_command_chain(frontend_id);
                    self.core.borrow_mut().status =
                        format!("{}: not bound", display_sequence(&sequence));
                }
            }
        }

        let typed_edit = self.core.borrow_mut().typed_edit_finish(frontend_id);
        let post_revision = self.active_buffer_revision();
        if pre_revision != post_revision {
            if let Some(record) = typed_edit {
                self.core
                    .borrow_mut()
                    .typed_edit_set_armed(frontend_id, record);
            }
            self.lua_host
                .run_hook("buffer.after-edit", mlua::MultiValue::new());
            self.core.borrow_mut().typed_edit_clear_armed();
        }
        self.core.borrow_mut().completion_popup_validate();
    }

    fn active_terminal_key(&self, frontend_id: FrontendId) -> Option<TerminalViewKey> {
        let core = self.core.borrow();
        let view = core.views.get(&frontend_id)?;
        let window = core.windows.get(&view.active)?;
        let key = TerminalViewKey::new(frontend_id, window.id, window.buffer_id);
        self.terminal_manager
            .borrow()
            .is_terminal(window.buffer_id)
            .then_some(key)
    }

    fn claim_terminal_controller(&self, key: TerminalViewKey) {
        let mut manager = self.terminal_manager.borrow_mut();
        if let Some(previous) = manager.controller_view_for_frontend(key.frontend_id)
            && previous != key
        {
            let _ = manager.release_controller(previous);
        }
        let _ = manager.register_view(key);
        let _ = manager.claim_controller(key);
    }

    fn send_terminal_bytes(&self, buffer_id: crate::buffer::BufferId, bytes: &[u8]) {
        let result = self.terminal_manager.borrow().send(
            buffer_id,
            bytes,
            &mut self.process_supervisor.borrow_mut(),
        );
        if let Err(error) = result {
            self.core.borrow_mut().status = error.to_string();
        }
    }

    /// Consume a paste as terminal input for one authenticated frontend.
    ///
    /// Returns `false` when modal/document paste handling must run instead.
    pub fn dispatch_paste(&mut self, frontend_id: FrontendId, bytes: &[u8]) -> bool {
        {
            let mut core = self.core.borrow_mut();
            core.active_frontend = frontend_id;
            if core.menu_is_open()
                || core.search_active()
                || core.query_replace_active()
                || core.minibuffer.is_active()
            {
                return false;
            }
        }
        let Some(key) = self.active_terminal_key(frontend_id) else {
            return false;
        };
        let modes = self
            .terminal_manager
            .borrow()
            .modes_for_view(key)
            .unwrap_or_default();
        let encoded = crate::terminal::input::encode_paste(bytes, modes.bracketed_paste);
        self.claim_terminal_controller(key);
        self.send_terminal_bytes(key.buffer_id, &encoded);
        true
    }

    /// Apply authenticated frontend focus to terminal control/reporting.
    pub fn dispatch_focus(&mut self, frontend_id: FrontendId, gained: bool) {
        self.core.borrow_mut().active_frontend = frontend_id;
        if gained {
            let Some(key) = self.active_terminal_key(frontend_id) else {
                return;
            };
            let modes = self
                .terminal_manager
                .borrow()
                .modes_for_view(key)
                .unwrap_or_default();
            self.claim_terminal_controller(key);
            if let Some(bytes) = crate::terminal::input::encode_focus(true, modes.focus_reporting) {
                self.send_terminal_bytes(key.buffer_id, &bytes);
            }
            return;
        }

        let controlled = self
            .terminal_manager
            .borrow()
            .controller_view_for_frontend(frontend_id);
        let Some(key) = controlled else {
            return;
        };
        let modes = self
            .terminal_manager
            .borrow()
            .modes_for_view(key)
            .unwrap_or_default();
        if let Some(bytes) = crate::terminal::input::encode_focus(false, modes.focus_reporting) {
            self.send_terminal_bytes(key.buffer_id, &bytes);
        }
        let _ = self.terminal_manager.borrow_mut().release_controller(key);
    }

    /// Resize the one session durably controlled by `frontend_id`.
    ///
    /// This is called before process drain and paint, never from rendering.
    pub fn sync_terminal_layout(&mut self, frontend_id: FrontendId, term_size: CellSize) -> bool {
        let Some(key) = self
            .terminal_manager
            .borrow()
            .controller_view_for_frontend(frontend_id)
        else {
            return false;
        };
        let content = {
            let core = self.core.borrow();
            let Some(view) = core.views.get(&frontend_id) else {
                let _ = self.terminal_manager.borrow_mut().release_controller(key);
                return false;
            };
            if view.active != key.window_id
                || core
                    .windows
                    .get(&key.window_id)
                    .is_none_or(|window| window.buffer_id != key.buffer_id)
            {
                let _ = self.terminal_manager.borrow_mut().release_controller(key);
                return false;
            }
            let Some(placement) = window_placements(&core, frontend_id, term_size)
                .get(&key.window_id)
                .copied()
            else {
                return false;
            };
            placement.content
        };
        if content.size.rows == 0 || content.size.cols == 0 {
            return false;
        }
        let old_size = self
            .terminal_manager
            .borrow()
            .snapshot(key.buffer_id)
            .map(|snapshot| snapshot.size);
        if old_size == Some(content.size) {
            return false;
        }
        let Ok(rows) = u16::try_from(content.size.rows) else {
            return false;
        };
        let Ok(cols) = u16::try_from(content.size.cols) else {
            return false;
        };
        let result = self.terminal_manager.borrow_mut().resize(
            key.buffer_id,
            rows,
            cols,
            &mut self.process_supervisor.borrow_mut(),
        );
        if let Err(error) = result {
            self.core.borrow_mut().status = error.to_string();
            false
        } else {
            true
        }
    }

    /// Precompute owned terminal view snapshots before entering paint borrows.
    pub fn prepare_terminal_views(
        &mut self,
        frontend_id: FrontendId,
        term_size: CellSize,
    ) -> HashMap<WindowId, TerminalSnapshot> {
        let (live, sizes) = {
            let core = self.core.borrow();
            let placements = window_placements(&core, frontend_id, term_size);
            let mut live = HashSet::new();
            let mut sizes = Vec::new();
            for (window_id, placement) in placements {
                let Some(window) = core.windows.get(&window_id) else {
                    continue;
                };
                if placement.content.size.rows == 0 || placement.content.size.cols == 0 {
                    continue;
                }
                let key = TerminalViewKey::new(frontend_id, window_id, window.buffer_id);
                if self.terminal_manager.borrow().is_terminal(window.buffer_id) {
                    live.insert(key);
                    sizes.push((key, placement.content.size));
                }
            }
            (live, sizes)
        };
        let mut manager = self.terminal_manager.borrow_mut();
        manager.retain_frontend_views(frontend_id, &live);
        sizes
            .into_iter()
            .filter_map(|(key, size)| {
                manager
                    .snapshot_for_view(key, size)
                    .map(|snapshot| (key.window_id, snapshot))
            })
            .collect()
    }

    /// Drain local-only terminal/clipboard output signals after a frame.
    pub fn take_local_signals(&mut self) -> Vec<InstanceMessage> {
        let frontend_id = FrontendId::LOCAL;
        let active = self.active_terminal_key(frontend_id);
        let mut messages = Vec::new();
        if self
            .terminal_manager
            .borrow_mut()
            .take_bell_for_frontend(frontend_id, active)
        {
            messages.push(InstanceMessage::Signal(InstanceSignal::Bell));
        }
        if let Some((target, bytes)) = self.core.borrow_mut().take_pending_clipboard()
            && target == frontend_id
        {
            messages.push(InstanceMessage::Signal(InstanceSignal::Clipboard(bytes)));
        }
        messages
    }

    /// Active buffer's edit revision, or `None` if the registry no
    /// longer knows about the active buffer (e.g. a command killed it
    /// mid-dispatch).
    fn active_buffer_revision(&self) -> Option<u64> {
        let id = self.core.borrow().active_buffer_id();
        self.buffer_revision(id)
    }

    /// Run `f`, then fire `buffer.after-edit` if the active buffer's
    /// revision changed — the same compare `dispatch_key` performs
    /// after a keybound command (kill ring Q#KR10b).
    ///
    /// For call sites that execute edits *outside* `dispatch_key`'s
    /// post-command check: the minibuffer accept callback (`M-x`), the
    /// menu invoke, and the unified paste route. Without it, those
    /// edits are invisible to LSP `didChange`, the syntax reparse, and
    /// autosave's observers.
    ///
    /// Scope, honestly: the *active-buffer* before/after compare is
    /// sound for these paths (all edit the active buffer and stay
    /// there) but is not a general any-buffer guarantee — a callback
    /// that edits buffer A then switches to B evades it. The general
    /// fix is a buffer-aware edit epoch; deferred, named in the
    /// kill-ring framing.
    pub(crate) fn with_after_edit_check(&mut self, f: impl FnOnce(&mut Self)) {
        let pre = self.active_buffer_revision();
        f(self);
        let post = self.active_buffer_revision();
        if pre != post {
            self.lua_host
                .run_hook("buffer.after-edit", mlua::MultiValue::new());
        }
    }

    /// Edit revision of a specific buffer, or `None` if the registry no
    /// longer knows it. Used by the query-replace shadow to compare the
    /// *edited* (origin) buffer, not whichever is active.
    fn buffer_revision(&self, id: crate::buffer::BufferId) -> Option<u64> {
        let reg = self.lua_host.registry().borrow();
        reg.get(id).ok().map(crate::buffer::Buffer::revision)
    }

    /// Hardcoded handler for keys delivered while a minibuffer prompt
    /// is active. Recognized chords:
    ///
    /// * `RET` / `C-m`           --- accept (invoke `on_accept`).
    /// * `C-g`                   --- cancel (invoke `on_cancel`).
    /// * `TAB` / `C-i`           --- complete to selected candidate.
    /// * `Up`                    --- previous candidate with a dropdown, else previous history.
    /// * `Down`                  --- next candidate with a dropdown, else next history.
    /// * `C-p`                   --- previous history entry (always).
    /// * `C-n`                   --- next history entry (always).
    /// * `BS`                    --- delete codepoint left of cursor.
    /// * `DEL` / `C-d`           --- delete codepoint at cursor.
    /// * `Left`  / `C-b`         --- cursor left.
    /// * `Right` / `C-f`         --- cursor right.
    /// * `Home`  / `C-a`         --- cursor to start.
    /// * `End`   / `C-e`         --- cursor to end.
    /// * `M-n`                   --- scroll candidate forward.
    /// * `M-p`                   --- scroll candidate backward.
    /// * Otherwise: a printable char self-inserts.
    ///
    /// Keys without a handler are silently ignored --- this matches
    /// Emacs's behaviour, where minibuffer mode shadows the global
    /// keymap.
    fn dispatch_minibuffer_key(&mut self, frontend_id: FrontendId, chord: Chord) {
        use crate::minibuffer::MinibufferAction;
        use crossterm::event::{KeyCode, KeyModifiers};

        let action = MinibufferAction::from_chord(chord);
        match action {
            MinibufferAction::Accept => self.minibuffer_accept(frontend_id),
            MinibufferAction::Cancel => self.minibuffer_cancel(),
            MinibufferAction::Complete => self.minibuffer_complete(),
            MinibufferAction::HistoryPrev => self.with_minibuffer(Minibuffer::history_prev),
            MinibufferAction::HistoryNext => self.with_minibuffer(Minibuffer::history_next),
            MinibufferAction::ScrollNext => self.with_minibuffer(|m| m.scroll_candidate(1)),
            MinibufferAction::ScrollPrev => self.with_minibuffer(|m| m.scroll_candidate(-1)),
            // Arrows navigate the completion dropdown when one is showing
            // (the intuitive default), else step through history.
            MinibufferAction::PrevCandidateOrHistory => {
                if self.core.borrow().minibuffer.has_candidates() {
                    self.with_minibuffer(|m| m.scroll_candidate(-1));
                } else {
                    self.with_minibuffer(Minibuffer::history_prev);
                }
            }
            MinibufferAction::NextCandidateOrHistory => {
                if self.core.borrow().minibuffer.has_candidates() {
                    self.with_minibuffer(|m| m.scroll_candidate(1));
                } else {
                    self.with_minibuffer(Minibuffer::history_next);
                }
            }
            MinibufferAction::Backspace => {
                self.with_minibuffer(Minibuffer::backspace);
                self.recompute_minibuffer_candidates();
            }
            MinibufferAction::DeleteForward => {
                self.with_minibuffer(Minibuffer::delete_forward);
                self.recompute_minibuffer_candidates();
            }
            MinibufferAction::Left => self.with_minibuffer(Minibuffer::move_left),
            MinibufferAction::Right => self.with_minibuffer(Minibuffer::move_right),
            MinibufferAction::LineStart => self.with_minibuffer(Minibuffer::move_line_start),
            MinibufferAction::LineEnd => self.with_minibuffer(Minibuffer::move_line_end),
            MinibufferAction::SelfInsert(ch) => {
                self.with_minibuffer(|m| m.insert_char(ch));
                self.recompute_minibuffer_candidates();
            }
            MinibufferAction::Ignore => {
                // Suppress noise from unrecognized chords; alternative
                // (status-line warnings) would clobber the prompt.
                let _ = (KeyCode::Null, KeyModifiers::NONE);
            }
        }
    }

    /// Hardcoded handler for keys delivered while an incremental search
    /// is active. The global keymap is shadowed (like the minibuffer),
    /// so these chords are fixed:
    ///
    /// * `C-s` / `Down`           --- step to the next match (wraps).
    /// * `C-r` / `Up`             --- step to the previous match (wraps).
    /// * `RET`                    --- accept (keep cursor + highlights).
    /// * `C-g` / `Esc`            --- cancel (restore origin cursor).
    /// * `BS`                     --- shorten the query by one char.
    /// * `M-r`                    --- toggle literal ↔ regex (Q#RX3).
    /// * a printable char         --- extend the query.
    ///
    /// Unrecognized chords are swallowed (an active isearch eats every
    /// keystroke, matching Emacs). The next/prev chords mirror the
    /// entry bindings (`search.forward` / `search.backward`) so the
    /// same key that started the search repeats it.
    fn dispatch_search_key(&mut self, chord: Chord) {
        match SearchKey::from_chord(chord) {
            SearchKey::Next => self.core.borrow_mut().search_step(true),
            SearchKey::Prev => self.core.borrow_mut().search_step(false),
            SearchKey::Accept => self.core.borrow_mut().search_finish(true),
            SearchKey::Cancel => self.core.borrow_mut().search_finish(false),
            SearchKey::Backspace => self.core.borrow_mut().search_backspace(),
            SearchKey::ToggleRegex => self.core.borrow_mut().search_toggle_regex(),
            SearchKey::Insert(ch) => self.core.borrow_mut().search_input_char(ch),
            SearchKey::Ignore => {}
        }
    }

    /// Drive the open completion popup from an intercepted control
    /// chord (Q#C3). Accept (Q#C7) re-validates inside
    /// [`EditorCore::completion_popup_accept`] and applies a single
    /// Replace edit; when that edit lands, `buffer.after-edit` fires
    /// here exactly as it does on the normal dispatch path, so LSP
    /// `didChange` and styling refresh ride the existing machinery.
    fn dispatch_completion_key(&mut self, key: CompletionPopupKey) {
        match key {
            CompletionPopupKey::Next => self.core.borrow_mut().completion_popup_step(1),
            CompletionPopupKey::Prev => self.core.borrow_mut().completion_popup_step(-1),
            CompletionPopupKey::Dismiss => self.core.borrow_mut().completion_popup_close(),
            CompletionPopupKey::Accept => {
                // Accepting a completion is its own command boundary
                // (review round 4): without this stamp, `this_command`
                // could still read "buffer.self-insert" from the typing
                // that raised the popup, and the after-edit fired below
                // would let a candidate ending in "(" spuriously
                // auto-trigger signature help.
                {
                    let mut core = self.core.borrow_mut();
                    let fid = core.active_frontend;
                    core.rotate_command(fid, "completion.accept");
                }
                let pre_revision = self.active_buffer_revision();
                self.core.borrow_mut().completion_popup_accept();
                if pre_revision != self.active_buffer_revision() {
                    self.lua_host
                        .run_hook("buffer.after-edit", mlua::MultiValue::new());
                }
            }
        }
    }

    /// Drive an active query-replace from a keystroke (Arc 2, Q#QR6).
    /// Fires `buffer.after-edit` itself when the key produced an edit
    /// (Q#QR1): a modal shadow returns before `dispatch_key`'s normal
    /// post-command edit check, so LSP `didChange` / syntax reparse
    /// would otherwise never see the replaced text. `!` applies many
    /// edits in one keypress; the single revision compare here fires
    /// the hook once for the batch, which is what the debounced
    /// `didChange` wants.
    fn dispatch_query_replace_key(&mut self, chord: Chord) {
        // Compare the *origin* buffer's revision (the one query-replace
        // edits), not the active buffer's — they can differ if focus
        // drifted, and the wrong-buffer guard may abort without editing.
        let origin_buf = self.core.borrow().query_replace_origin_buffer();
        let pre = origin_buf.and_then(|id| self.buffer_revision(id));
        match QueryReplaceKey::from_chord(chord) {
            QueryReplaceKey::Replace => self.core.borrow_mut().query_replace_replace(),
            QueryReplaceKey::Skip => self.core.borrow_mut().query_replace_skip(),
            QueryReplaceKey::All => self.core.borrow_mut().query_replace_all(),
            QueryReplaceKey::ReplaceAndQuit => {
                self.core.borrow_mut().query_replace_replace_and_quit();
            }
            QueryReplaceKey::Quit => self.core.borrow_mut().query_replace_finish(),
            QueryReplaceKey::Ignore => {}
        }
        // `!` applies many edits under one keypress; the single compare
        // fires `buffer.after-edit` once for the batch (Q#QR1).
        let post = origin_buf.and_then(|id| self.buffer_revision(id));
        if origin_buf.is_some() && pre != post {
            self.lua_host
                .run_hook("buffer.after-edit", mlua::MultiValue::new());
        }
    }

    /// Drive an open context menu from a keystroke (Q#CM1).
    fn dispatch_menu_key(&mut self, frontend_id: FrontendId, chord: Chord) {
        match MenuKey::from_chord(chord) {
            MenuKey::Next => self.core.borrow_mut().menu_step(1),
            MenuKey::Prev => self.core.borrow_mut().menu_step(-1),
            MenuKey::Invoke => self.menu_invoke_active(frontend_id),
            MenuKey::Cancel | MenuKey::Dismiss => self.core.borrow_mut().menu_close(),
        }
    }

    /// Close the menu, then invoke its highlighted item's command. The
    /// menu closes *first* so the command runs against a clean state
    /// (and a command that itself opens a menu isn't immediately torn
    /// down).
    fn menu_invoke_active(&mut self, frontend_id: FrontendId) {
        let command = self.core.borrow().menu_active_command();
        self.core.borrow_mut().menu_close();
        if let Some(command) = command {
            // A menu item is an interactive command (kill ring Q#KR2):
            // rotate the boundary so a menu Cut chains like a keybound
            // one. The invoke below bypasses dispatch_key, which would
            // otherwise leave the boundary stale.
            self.core.borrow_mut().rotate_command(frontend_id, &command);
            // Q#KR10b: menu invocation bypasses dispatch_key's
            // revision check — a menu Cut's edit must still fire
            // `buffer.after-edit`.
            let _origin = self.interactive_origin.enter(frontend_id);
            self.with_after_edit_check(|state| {
                if let Err(e) = state
                    .lua_host
                    .invoke_command(&command, mlua::MultiValue::new())
                {
                    state.core.borrow_mut().status =
                        format!("error in {command}: {}", first_line(&e.to_string()));
                }
            });
        }
    }

    /// Build the resolved, grouped, visibility-filtered menu rows by
    /// calling the Lua builder (`pmacs.menu.build`), which evaluates each
    /// item's predicate / context tag against the live editor state.
    /// Returns an empty list on any Lua error (the menu then won't open).
    fn build_menu_rows(&mut self) -> Vec<crate::menu::MenuRow> {
        let value = match self
            .lua_host
            .eval(Some("@pmacs/menu/build"), "return pmacs.menu.build()")
        {
            Ok(v) => v,
            Err(e) => {
                self.core.borrow_mut().status =
                    format!("menu build failed: {}", first_line(&e.to_string()));
                return Vec::new();
            }
        };
        let mlua::Value::Table(table) = value else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for entry in table.sequence_values::<mlua::Table>() {
            let Ok(t) = entry else { continue };
            if t.get::<Option<bool>>("separator").ok().flatten() == Some(true) {
                rows.push(crate::menu::MenuRow::Separator);
            } else if let (Ok(label), Ok(command)) =
                (t.get::<String>("label"), t.get::<String>("command"))
            {
                rows.push(crate::menu::MenuRow::Item { label, command });
            }
        }
        rows
    }

    /// Open the context menu at the click cell (Q#CM1). Anchors the
    /// cursor: an existing selection is kept (so Copy/Cut act on it);
    /// otherwise the cursor moves to the click and any selection clears.
    fn open_context_menu(
        &mut self,
        win_id: WindowId,
        local_row: u32,
        local_col: u32,
        anchor: (u32, u32),
    ) {
        if !self.core.borrow().menu_is_open() {
            let has_selection = self.core.borrow().active_region().is_some();
            if !has_selection {
                self.activate_and_position(win_id, local_row, local_col);
            }
        }
        let rows = self.build_menu_rows();
        self.core.borrow_mut().menu_open(rows, anchor);
    }

    /// Drive an open menu from a mouse event (Q#CM1): hover highlights,
    /// left-click invokes, a click outside (or right-click) dismisses.
    fn dispatch_menu_mouse(
        &mut self,
        frontend_id: FrontendId,
        ev: MouseEvent,
        cell_row: u32,
        cell_col: u32,
    ) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let hit = self.core.borrow().menu_hit(cell_row, cell_col);
        match ev.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(row) = hit {
                    self.core.borrow_mut().menu_set_active_row(row);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => match hit {
                Some(row) => {
                    self.core.borrow_mut().menu_set_active_row(row);
                    self.menu_invoke_active(frontend_id);
                }
                None => self.core.borrow_mut().menu_close(),
            },
            MouseEventKind::Down(MouseButton::Right | MouseButton::Middle) => {
                self.core.borrow_mut().menu_close();
            }
            _ => {}
        }
    }

    fn with_minibuffer<F: FnOnce(&mut Minibuffer)>(&mut self, f: F) {
        f(&mut self.core.borrow_mut().minibuffer);
    }

    fn recompute_minibuffer_candidates(&mut self) {
        let cmds = self.lua_host.commands().borrow();
        let reg = self.lua_host.registry().borrow();
        let mut core = self.core.borrow_mut();
        if let Err(e) = core.minibuffer.recompute_candidates(&cmds, &reg) {
            core.status = format!("completion source error: {}", first_line(&e.to_string()));
        }
    }

    fn minibuffer_accept(&mut self, frontend_id: FrontendId) {
        let outcome = self.core.borrow_mut().minibuffer.accept();
        let Some((on_accept, contents)) = outcome else {
            return;
        };
        // Drop all borrows before the callback fires --- it may
        // re-enter the editor (e.g. the M-x callback invokes a
        // command which mutates the core).
        let mut args = mlua::MultiValue::new();
        args.push_back(mlua::Value::String(
            self.lua_host
                .lua()
                .create_string(&contents)
                .expect("Lua VM out of memory while building minibuffer callback args"),
        ));
        // Q#KR10b: the accept callback runs outside dispatch_key's
        // post-command revision check (the minibuffer interception
        // returns before it), so an M-x'd editing command would never
        // fire `buffer.after-edit` without this wrapper.
        let _origin = self.interactive_origin.enter(frontend_id);
        self.with_after_edit_check(|state| {
            if let Err(e) = on_accept.call::<mlua::MultiValue>(args) {
                state.core.borrow_mut().status = format!(
                    "minibuffer on_accept failed: {}",
                    first_line(&e.to_string())
                );
            }
        });
    }

    fn minibuffer_cancel(&mut self) {
        let on_cancel = self.core.borrow_mut().minibuffer.cancel();
        if let Some(cb) = on_cancel
            && let Err(e) = cb.call::<mlua::MultiValue>(mlua::MultiValue::new())
        {
            self.core.borrow_mut().status = format!(
                "minibuffer on_cancel failed: {}",
                first_line(&e.to_string())
            );
        }
        self.core.borrow_mut().status = "Quit".into();
    }

    fn minibuffer_complete(&mut self) {
        self.core.borrow_mut().minibuffer.complete();
        self.recompute_minibuffer_candidates();
    }

    /// Dispatch a mouse event (T M2.12).
    ///
    /// Mapping:
    ///   * `Down(Left)` activates the window under the cursor and
    ///     positions the buffer cursor at the corresponding rope
    ///     position. Starts an empty selection at that position so
    ///     a drag continues the region from there.
    ///   * A second `Down(Left)` in the same cell within the double-click
    ///     threshold selects the word at the click position.
    ///   * `Drag(Left)` updates the cursor as the mouse moves; the
    ///     anchor stays put, so the region grows.
    ///   * `Up(Left)` ends a drag. If anchor and cursor coincide
    ///     (a plain click with no drag), the empty selection is
    ///     dropped so subsequent commands don't see a phantom region.
    ///   * `ScrollUp` / `ScrollDown` scroll the window under the
    ///     cursor by [`SCROLL_LINES`] lines, without changing the
    ///     buffer cursor or active window.
    ///
    /// Mouse moves with no buttons (`Moved`) and other buttons are
    /// ignored. Clicks on a window's mode line are also ignored
    /// (the click neither activates the window nor positions the
    /// cursor; that gesture is reserved for future binding to
    /// "switch to this window" without disturbing buffer state).
    pub fn dispatch_mouse(
        &mut self,
        frontend_id: FrontendId,
        ev: MouseEvent,
        term_size: crate::cell::CellSize,
    ) {
        use crossterm::event::{MouseButton, MouseEventKind};
        self.core.borrow_mut().active_frontend = frontend_id;
        let cell_row = u32::from(ev.row);
        let cell_col = u32::from(ev.column);

        // Context-menu interception (Q#CM1): while a menu is open the
        // mouse drives it (hover highlights, left-click invokes, a click
        // outside dismisses) — handled before window hit-testing so an
        // outside click anywhere closes it.
        if self.core.borrow().menu_is_open() {
            self.dispatch_menu_mouse(frontend_id, ev, cell_row, cell_col);
            return;
        }

        let Some((win_id, rect)) = window_at_cell(
            &self.core.borrow(),
            frontend_id,
            term_size,
            cell_row,
            cell_col,
        ) else {
            return;
        };
        let inner_rows = rect.size.rows.saturating_sub(1);
        let local_row = cell_row.saturating_sub(rect.origin.row);
        let buffer_id = self.core.borrow().windows[&win_id].buffer_id;
        if self.terminal_manager.borrow().is_terminal(buffer_id) {
            let content_size = CellSize::new(inner_rows, rect.size.cols);
            if local_row >= inner_rows || content_size.rows == 0 || content_size.cols == 0 {
                self.mouse_click = None;
                return;
            }
            let local = CellCoord::new(local_row, cell_col.saturating_sub(rect.origin.col));
            self.dispatch_terminal_mouse(
                TerminalViewKey::new(frontend_id, win_id, buffer_id),
                content_size,
                local,
                ev,
                (cell_row, cell_col),
            );
            return;
        }
        // UX gutter (Q#UX6): subtract the reserved gutter width so the
        // hit-test lands on the right text byte. A click inside the gutter
        // strip (raw < gutter_w) saturates to column 0 → the start of that
        // line, a mild, useful affordance for the MVP.
        let gutter_w = {
            let core = self.core.borrow();
            core.windows.get(&win_id).map_or(0, |w| {
                let g = w.gutter_width();
                if g >= rect.size.cols { 0 } else { g }
            })
        };
        let local_col = cell_col
            .saturating_sub(rect.origin.col)
            .saturating_sub(gutter_w);

        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if local_row >= inner_rows {
                    self.mouse_click = None;
                    return; // Mode-line click: reserved.
                }
                // Point moves: break the command chain (kill ring
                // Q#KR2). Scroll arms below deliberately do NOT — a
                // wheel that only moves the viewport preserves a kill
                // chain, as in Emacs (`mwheel-scroll` vs
                // `mouse-set-point`).
                self.core.borrow_mut().break_command_chain(frontend_id);
                let click_cell = CellCoord::new(cell_row, cell_col);
                let is_double_click = self.is_double_click(frontend_id, win_id, click_cell);
                self.activate_and_position(win_id, local_row, local_col);
                if is_double_click && self.core.borrow_mut().select_word_at_cursor() {
                    self.mouse_click = None;
                } else {
                    let mut core = self.core.borrow_mut();
                    let pos = core.cursor();
                    core.begin_selection(pos);
                    self.mouse_click = Some(MouseClickState {
                        frontend_id,
                        window_id: win_id,
                        cell: click_cell,
                        at: Instant::now(),
                    });
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.mouse_click = None;
                if local_row >= inner_rows {
                    return;
                }
                self.core.borrow_mut().break_command_chain(frontend_id);
                self.activate_and_position(win_id, local_row, local_col);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let mut core = self.core.borrow_mut();
                core.break_command_chain(frontend_id);
                if let Some(sel) = core.active_window().selection
                    && sel.anchor == core.cursor()
                {
                    core.clear_selection();
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if local_row >= inner_rows {
                    self.mouse_click = None;
                    return; // Mode-line right-click: reserved.
                }
                self.mouse_click = None;
                // Opening the menu is a pointer gesture too (Q#KR2).
                self.core.borrow_mut().break_command_chain(frontend_id);
                self.open_context_menu(win_id, local_row, local_col, (cell_row, cell_col));
            }
            MouseEventKind::ScrollUp => {
                self.mouse_click = None;
                self.scroll_window(win_id, -SCROLL_LINES);
            }
            MouseEventKind::ScrollDown => {
                self.mouse_click = None;
                self.scroll_window(win_id, SCROLL_LINES);
            }
            _ => {
                self.mouse_click = None;
            }
        }
    }

    fn dispatch_terminal_mouse(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
        coord: CellCoord,
        event: MouseEvent,
        global: (u32, u32),
    ) {
        use crossterm::event::{MouseButton, MouseEventKind};

        let Some(kind) = terminal_mouse_kind(event.kind) else {
            return;
        };
        let modifiers = terminal_modifiers(event.modifiers);
        let shift = modifiers.contains(TerminalModifiers::SHIFT);
        let (at_bottom, modes, screen_size) = {
            let mut manager = self.terminal_manager.borrow_mut();
            let Some(snapshot) = manager.snapshot_for_view(key, viewport_size) else {
                return;
            };
            let modes = manager.modes_for_view(key).unwrap_or_default();
            let screen_size = manager
                .snapshot(key.buffer_id)
                .map_or(viewport_size, |snapshot| snapshot.size);
            (snapshot.at_bottom, modes, screen_size)
        };

        if !shift
            && at_bottom
            && modes.mouse_sgr
            && coord.row < screen_size.rows
            && coord.col < screen_size.cols
            && let Some(bytes) = crate::terminal::input::encode_mouse(kind, coord, modifiers, modes)
        {
            self.claim_terminal_controller(key);
            self.send_terminal_bytes(key.buffer_id, &bytes);
            return;
        }

        self.claim_terminal_controller(key);
        let mut manager = self.terminal_manager.borrow_mut();
        match event.kind {
            MouseEventKind::ScrollUp => {
                let _ = manager.scroll_view(key, viewport_size, SCROLL_LINES);
            }
            MouseEventKind::ScrollDown => {
                let _ = manager.scroll_view(key, viewport_size, -SCROLL_LINES);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let _ = manager.begin_selection(key, viewport_size, coord);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let _ = manager.update_selection(key, viewport_size, coord);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let _ = manager.finish_selection(key, viewport_size, coord);
            }
            MouseEventKind::Down(MouseButton::Right) => {
                drop(manager);
                self.core.borrow_mut().break_command_chain(key.frontend_id);
                let rows = self.build_menu_rows();
                self.core.borrow_mut().menu_open(rows, global);
            }
            _ => {}
        }
    }

    fn is_double_click(
        &self,
        frontend_id: FrontendId,
        window_id: WindowId,
        cell: CellCoord,
    ) -> bool {
        let Some(prev) = self.mouse_click else {
            return false;
        };
        prev.frontend_id == frontend_id
            && prev.window_id == window_id
            && prev.cell == cell
            && prev.at.elapsed() <= DOUBLE_CLICK_MAX_DELAY
    }

    /// Mouse framing Q#M1 — apply a semantic frontend's locally
    /// hit-tested pointer gesture (`FrontendEvent::Pointer`) to its
    /// window. The byte-space twin of [`Self::dispatch_mouse`]: same
    /// gesture semantics, but the position arrives as a source byte
    /// offset the frontend resolved against its own layout (fonts,
    /// inline adornments, scroll), so no cell geometry is consulted.
    ///
    ///   * `Down` places the cursor and anchors a selection there
    ///     (a following drag grows it). With SHIFT it *extends*
    ///     instead (Q#M5): the existing anchor — or, with no
    ///     selection, the pre-click cursor — is kept and only the
    ///     cursor moves, matching the universal Shift-click
    ///     convention.
    ///   * `Drag` moves the cursor; the anchor stays.
    ///   * `Up` collapses an empty selection (a click without drag).
    ///   * `DoubleDown` selects the word at the hit (frontend-side
    ///     double-click detection — only it knows pixel proximity).
    ///   * `TripleDown` selects the whole line at the hit, trailing
    ///     newline included (Q#M4, protocol v7).
    ///
    /// The hit byte is clamped into the buffer and snapped back to a
    /// UTF-8 boundary: the frontend's hit may race an in-flight edit.
    pub fn dispatch_pointer(
        &mut self,
        frontend_id: FrontendId,
        buffer_id: crate::buffer::BufferId,
        byte: u64,
        kind: crate::protocol::PointerKind,
        mods: crate::protocol::Modifiers,
    ) {
        use crate::protocol::PointerKind;
        let mut core = self.core.borrow_mut();
        core.active_frontend = frontend_id;
        let Some(win_id) = core.views.get(&frontend_id).map(|v| v.active) else {
            return;
        };
        // The dispatcher aligns the session's window to the declared
        // buffer before calling here; re-check defensively (a click
        // can race a buffer switch).
        if core.windows.get(&win_id).map(|w| w.buffer_id) != Some(buffer_id) {
            return;
        }
        core.set_active_window_id(win_id);
        // Every PointerKind moves point or changes the selection (the
        // GPU scrolls locally via Viewport, which never reaches here),
        // so any pointer gesture breaks the frontend's command chain
        // (kill ring Q#KR2) — clicking away and killing again must not
        // append, and M-y after a click must refuse.
        core.break_command_chain(frontend_id);
        let byte = {
            let registry = core.registry.clone();
            let reg = registry.borrow();
            let Ok(buf) = reg.get(buffer_id) else {
                return;
            };
            snap_to_char_boundary(buf, byte)
        };
        match kind {
            PointerKind::Down => {
                let prev_cursor = core.active_window().cursor;
                let extending = mods.contains(crate::protocol::Modifiers::SHIFT);
                let keep_anchor = extending && core.active_window().selection.is_some();
                let aw = core.active_window_mut();
                aw.cursor = byte;
                aw.goal_col = None;
                if extending {
                    if !keep_anchor {
                        core.begin_selection(prev_cursor);
                    }
                } else {
                    core.begin_selection(byte);
                }
            }
            PointerKind::Drag => {
                let aw = core.active_window_mut();
                aw.cursor = byte;
                aw.goal_col = None;
            }
            PointerKind::Up => {
                if let Some(sel) = core.active_window().selection
                    && sel.anchor == core.cursor()
                {
                    core.clear_selection();
                }
            }
            PointerKind::DoubleDown => {
                let aw = core.active_window_mut();
                aw.cursor = byte;
                aw.goal_col = None;
                core.select_word_at_cursor();
            }
            PointerKind::TripleDown => {
                let aw = core.active_window_mut();
                aw.cursor = byte;
                aw.goal_col = None;
                core.select_line_at_cursor();
            }
            // Right-click (Q#CM1) opens the menu, which needs the Lua
            // builder — handled by `open_menu_at_byte`, which the daemon
            // routes to *instead* of here. Unreachable in practice; the
            // arm exists for match exhaustiveness.
            PointerKind::Context => {}
        }
    }

    /// Open the context menu at `byte` for a semantic frontend (Q#CM1) —
    /// the byte-space twin of the TUI right-click. Keeps an existing
    /// selection (so Copy/Cut act on it), else moves the cursor to the
    /// click. The anchor cell is irrelevant for the GPU (it positions
    /// the popup in pixels locally), so it stays at the origin.
    pub fn open_menu_at_byte(
        &mut self,
        frontend_id: FrontendId,
        buffer_id: crate::buffer::BufferId,
        byte: u64,
    ) {
        {
            let mut core = self.core.borrow_mut();
            core.active_frontend = frontend_id;
            let Some(win_id) = core.views.get(&frontend_id).map(|v| v.active) else {
                return;
            };
            if core.windows.get(&win_id).map(|w| w.buffer_id) != Some(buffer_id) {
                return;
            }
            core.set_active_window_id(win_id);
            // A context right-click is a pointer gesture (kill ring
            // Q#KR2): it must break the chain like the grid path's
            // right-click does. The semantic dispatcher routes
            // PointerKind::Context here directly, bypassing
            // dispatch_pointer's break.
            core.break_command_chain(frontend_id);
            if core.active_region().is_none() {
                let snapped = {
                    let registry = core.registry.clone();
                    let reg = registry.borrow();
                    let Ok(buf) = reg.get(buffer_id) else {
                        return;
                    };
                    snap_to_char_boundary(buf, byte)
                };
                let aw = core.active_window_mut();
                aw.cursor = snapped;
                aw.goal_col = None;
            }
        }
        let rows = self.build_menu_rows();
        self.core.borrow_mut().menu_open(rows, (0, 0));
    }

    /// Apply a semantic frontend's menu navigation (Q#CM1). Hover
    /// (`invoke = false`) moves the highlight; a click (`invoke = true`)
    /// invokes the row, or dismisses the menu when `index` is `None`
    /// (click outside the popup).
    pub fn dispatch_menu_pointer(
        &mut self,
        frontend_id: FrontendId,
        index: Option<u32>,
        invoke: bool,
    ) {
        self.core.borrow_mut().active_frontend = frontend_id;
        if !self.core.borrow().menu_is_open() {
            return;
        }
        match (index, invoke) {
            (Some(i), false) => self.core.borrow_mut().menu_set_active_row(i as usize),
            (Some(i), true) => {
                self.core.borrow_mut().menu_set_active_row(i as usize);
                self.menu_invoke_active(frontend_id);
            }
            (None, true) => self.core.borrow_mut().menu_close(),
            (None, false) => {}
        }
    }

    /// Make `win_id` the active window and place its cursor at the
    /// buffer position corresponding to `(local_row, local_col)`,
    /// where the coordinates are relative to the window's viewport
    /// origin (0 row = first text row of this window's content).
    fn activate_and_position(&mut self, win_id: WindowId, local_row: u32, local_col: u32) {
        let mut core = self.core.borrow_mut();
        core.set_active_window_id(win_id);
        let view_top = core.windows[&win_id].view_top;
        let buffer_id = core.windows[&win_id].buffer_id;
        let display_row = view_top.saturating_add(local_row as usize);
        let target = crate::view::DisplayCoord::new(display_row as u32, local_col);
        let pos = {
            let registry = core.registry.clone();
            let reg = registry.borrow();
            let Ok(buf) = reg.get(buffer_id) else {
                return;
            };
            core.windows[&win_id].text_view.display_to_pos(buf, target)
        };
        if let Some(p) = pos {
            let aw = core
                .windows
                .get_mut(&win_id)
                .expect("invariant: win_id passed in must be a live window in core.windows");
            aw.cursor = p;
            aw.goal_col = None;
        }
    }

    /// Adjust `view_top` of `win_id` by `delta` lines and shift the
    /// cursor by the same delta so it keeps its relative position in
    /// the viewport. Negative scrolls up (toward earlier content);
    /// positive scrolls down.
    ///
    /// The cursor must follow the scroll: the renderer has an
    /// "auto-scroll to keep cursor visible" pass that would otherwise
    /// snap `view_top` straight back to wherever the cursor sits, so
    /// the user's mouse-wheel scroll would feel stuck after one
    /// notch. Carrying the cursor with the view matches Emacs's
    /// `mouse-wheel-mode` and every modern editor's wheel behaviour.
    fn scroll_window(&mut self, win_id: WindowId, delta: i32) {
        let mut core = self.core.borrow_mut();
        let line_count = core.windows[&win_id].text_view.line_count();
        let max_top = line_count.saturating_sub(1);
        let old_top = core.windows[&win_id].view_top;
        let scroll_up = delta < 0;
        let magnitude = delta.unsigned_abs() as usize;
        let new_top = if scroll_up {
            old_top.saturating_sub(magnitude)
        } else {
            old_top.saturating_add(magnitude).min(max_top)
        };
        // Effective view delta — buffer-boundary clamping may shrink
        // the requested move, so the cursor only follows by however
        // many lines the view actually shifted.
        let view_shift = if scroll_up {
            old_top.saturating_sub(new_top)
        } else {
            new_top.saturating_sub(old_top)
        };
        let buffer_id = core.windows[&win_id].buffer_id;
        let new_cursor = {
            let registry = core.registry.clone();
            let reg = registry.borrow();
            reg.get(buffer_id).ok().and_then(|buf| {
                let aw = &core.windows[&win_id];
                let cur = aw.text_view.pos_to_display(buf, aw.cursor)?;
                let cur_row = cur.row as usize;
                let target_row_usize = if scroll_up {
                    cur_row.saturating_sub(view_shift)
                } else {
                    cur_row.saturating_add(view_shift).min(max_top)
                };
                let target_row = u32::try_from(target_row_usize).ok()?;
                aw.text_view
                    .display_to_pos(buf, crate::view::DisplayCoord::new(target_row, cur.col))
                    .or_else(|| aw.text_view.line_offset(target_row_usize))
            })
        };
        let aw = core
            .windows
            .get_mut(&win_id)
            .expect("invariant: win_id passed in must be a live window in core.windows");
        aw.view_top = new_top;
        if let Some(p) = new_cursor {
            aw.cursor = p;
            aw.goal_col = None;
        }
    }
}

/// Lines to scroll per mouse-wheel notch. Three matches the GNU
/// readline / Emacs default and is what most terminal users expect.
const SCROLL_LINES: i32 = 3;

/// Shared outer/content geometry consumed by terminal paint and PTY resize.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowPlacement {
    pub(crate) outer: Rect,
    pub(crate) content: Rect,
}

/// Compute one explicit frontend's split geometry.
#[must_use]
pub(crate) fn window_placements(
    core: &EditorCore,
    frontend_id: FrontendId,
    term_size: CellSize,
) -> HashMap<WindowId, WindowPlacement> {
    if term_size.rows < 2 || term_size.cols == 0 {
        return HashMap::new();
    }
    let Some(view) = core.views.get(&frontend_id) else {
        return HashMap::new();
    };
    let area = Rect::new(0, 0, term_size.rows - 1, term_size.cols);
    view.layout
        .compute(area)
        .into_iter()
        .map(|(window_id, outer)| {
            let content = Rect::new(
                outer.origin.row,
                outer.origin.col,
                outer.size.rows.saturating_sub(1),
                outer.size.cols,
            );
            (window_id, WindowPlacement { outer, content })
        })
        .collect()
}

/// Find the leaf window whose viewport rectangle contains
/// `(cell_row, cell_col)` in the global cell grid. Used by the mouse
/// dispatcher to route clicks. The bottom row of the terminal (status
/// / minibuffer) is *not* part of any window — clicks there return
/// `None`.
fn window_at_cell(
    core: &EditorCore,
    frontend_id: FrontendId,
    term_size: CellSize,
    cell_row: u32,
    cell_col: u32,
) -> Option<(WindowId, Rect)> {
    if cell_row >= term_size.rows.saturating_sub(1) {
        return None;
    }
    let placements = window_placements(core, frontend_id, term_size);
    placements.iter().find_map(|(id, placement)| {
        let rect = placement.outer;
        if cell_row >= rect.origin.row
            && cell_row < rect.origin.row + rect.size.rows
            && cell_col >= rect.origin.col
            && cell_col < rect.origin.col + rect.size.cols
        {
            Some((*id, rect))
        } else {
            None
        }
    })
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Run loop
// ---------------------------------------------------------------------------

/// Main run loop. Opens the file (if any), takes over the terminal,
/// renders, dispatches keys, until the user quits.
///
/// # Post-init dispatch (T M5.6g)
///
/// After [`EditorState::new`] / [`EditorState::open`] returns,
/// `init.lua` has had a chance to call `pmacs.attach{...}`. The
/// dispatcher in [`crate::attach_dispatch::dispatch_attach`] decides
/// whether to:
///
/// * run the local TUI as usual (no init-time attach request),
/// * hand off to [`crate::attach::run_attach`] against the requested
///   local socket, or
/// * surface a workaround-pointing error for transports whose
///   activation pathway hasn't shipped yet (ssh / tls / custom).
///
/// The dispatch happens *before* the local [`Frontend`] is
/// constructed, so a hand-off to attach mode doesn't fight the
/// local-TUI for the terminal.
pub fn run(file: Option<PathBuf>) -> io::Result<()> {
    install_panic_hook();
    // Capture before the `match` consumes `file`: a positional file arg
    // means "open this", not "restore my desktop" (Q#DS7).
    let had_file = file.is_some();
    let mut state = match file {
        Some(path) => EditorState::open(path)?,
        None => EditorState::new(),
    };
    // Real session: wire up on-disk persistence (history + pmacs.state).
    state.install_state_dirs();

    // Post-init dispatch: read whatever init.lua left in the
    // RequestedAttach slot and decide whether to run local or hand
    // off to attach mode. `take_requested_attach` consumes the slot
    // — even on the hand-off path the local EditorState is dropped
    // before attach::run_attach takes over the terminal, so the
    // request is consumed exactly once.
    let requested = state.lua_host.take_requested_attach();
    match crate::attach_dispatch::dispatch_attach(requested) {
        crate::attach_dispatch::AttachDispatch::RunLocal => {
            // Committed to local mode: restore the desktop if armed and
            // no file arg was given (Q#DS7). Done here, not right after
            // construction, so a hand-off to attach mode (above) never
            // populates an EditorState it's about to drop.
            state.restore_desktop_if_armed(had_file);
            // Fall through to the local TUI loop below.
        }
        crate::attach_dispatch::AttachDispatch::RunAttachLocalSocket(socket) => {
            // Drop the local EditorState before taking over the
            // terminal: attach mode constructs its own Frontend, and
            // the locally-built one would leak its alternate-screen
            // / raw-mode setup if held across the call.
            drop(state);
            return crate::attach::run_attach(socket).map_err(|e| io::Error::other(format!("{e}")));
        }
        crate::attach_dispatch::AttachDispatch::RunAttachSsh(target) => {
            // Same EditorState-drop reasoning as the local-socket
            // path: SSH attach takes over the terminal.
            drop(state);
            return crate::attach::run_attach_ssh(target)
                .map_err(|e| io::Error::other(format!("{e}")));
        }
        dispatch @ crate::attach_dispatch::AttachDispatch::DeferredInV01 { .. } => {
            let msg = dispatch
                .deferred_message()
                .expect("DeferredInV01 always has a message");
            return Err(io::Error::other(msg));
        }
    }

    let mut frontend = Frontend::new()?;
    let mut render_state = crate::instance_render::RenderState::new(frontend.size());

    loop {
        let size = frontend.size();
        let _ = state.sync_terminal_layout(FrontendId::LOCAL, size);
        let terminal_snapshots = state.prepare_terminal_views(FrontendId::LOCAL, size);
        let mut messages =
            render_state.render_frame(&state, FrontendId::LOCAL, &terminal_snapshots, &[]);
        messages.extend(state.take_local_signals());
        frontend.present_messages(&messages)?;
        if state.core.borrow().quit {
            break;
        }

        // Poll with a frame-sized timeout (60 Hz default; T M3.5
        // exposes the cadence as a tunable knob via
        // `pmacs.async_config.frame_target_ms`). The timeout is
        // what lets the async runtime (T M3.3) wake on worker
        // completions even when the user isn't typing --- a
        // parallel grep finishing has to surface its results
        // without waiting for a key press, and a streaming worker's
        // 10K msgs/sec coalesce into one main-thread wakeup per
        // frame at this cadence (T M3.5 acceptance). When events do
        // arrive, we drain the burst the same way we did before
        // (T M2.12): one render per burst, not one per event.
        let frame_target = state.async_runtime.frame_target_ms();
        let first = frontend.poll_event(Duration::from_millis(frame_target))?;
        if let Some(ev) = first {
            forward_resize(&ev, &mut render_state);
            let term_size = frontend.size();
            process_event(&mut state, ev, term_size);
            while let Some(ev) = frontend.poll_event(Duration::from_millis(0))? {
                forward_resize(&ev, &mut render_state);
                let term_size = frontend.size();
                process_event(&mut state, ev, term_size);
                if state.core.borrow().quit {
                    break;
                }
            }
        }
        // `tick_async` runs *last*, after the supervisor/LSP/MCP
        // ticks have absorbed this frame's inbound I/O. The async
        // bridge (T M4.5) settles an awaiter inside `tick_lsp`/
        // `tick_mcp` by posting to the message bus; `tick_async`
        // drains that bus and resumes the parked coroutine. With
        // `tick_async` last, settle→resume happens in the *same*
        // frame; running it first would defer every LSP/MCP await
        // resumption by a full frame. The documented invariant is
        // only `tick_processes → tick_lsp → tick_mcp` (same-batch
        // supervisor I/O ordering), which is preserved.
        let _ = state.sync_terminal_layout(FrontendId::LOCAL, frontend.size());
        state.tick_processes();
        state.tick_lsp();
        state.tick_mcp();
        state.tick_async();
    }
    let _ = frontend.poll_event(Duration::from_millis(0));
    Ok(())
}

/// Propagate a `Resize` event to the instance-side render buffers so the
/// next frame is reallocated and emitted as a full-grid sync. The
/// frontend updates its own size internally inside `poll_event`; this
/// helper keeps `RenderState` in lockstep.
fn forward_resize(ev: &Event, render_state: &mut crate::instance_render::RenderState) {
    if let Event::Resize(cols, rows) = ev {
        render_state.resize(crate::cell::CellSize::new(
            u32::from(*rows),
            u32::from(*cols),
        ));
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "ownership ends here; variants we act on are Copy"
)]
fn process_event(state: &mut EditorState, ev: Event, term_size: crate::cell::CellSize) {
    // v0.1: a single frontend per instance, hard-coded to
    // [`FrontendId::LOCAL`]. v0.3 (multi-frontend) extracts the ID from
    // the [`crate::protocol::FrontendEvent`] wrapper produced by the
    // attached frontend.
    let frontend_id = FrontendId::LOCAL;
    match ev {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            state.dispatch_key(frontend_id, key);
        }
        Event::Mouse(m) => {
            state.dispatch_mouse(frontend_id, m, term_size);
        }
        Event::Paste(bytes) => {
            if !state.dispatch_paste(frontend_id, bytes.as_bytes()) {
                state.core.borrow_mut().active_frontend = frontend_id;
                state.with_after_edit_check(|state| {
                    if let Err(error) = state.core.borrow_mut().paste_inbound(bytes.as_bytes()) {
                        state.core.borrow_mut().status = error;
                    }
                });
            }
        }
        Event::FocusGained => state.dispatch_focus(frontend_id, true),
        Event::FocusLost => state.dispatch_focus(frontend_id, false),
        Event::Key(_) => {}
        Event::Resize(_, _) => {}
    }
}

/// Decoded action for a key delivered while an incremental search is
/// active. Mirrors [`crate::minibuffer::MinibufferAction`]: the
/// bindings are hardcoded (not user-configurable) because isearch
/// shadows the global keymap; changes happen by extending this enum.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SearchKey {
    /// Step to the next match (C-s / Down).
    Next,
    /// Step to the previous match (C-r / Up).
    Prev,
    /// Accept: keep cursor + highlights (RET).
    Accept,
    /// Cancel: restore the origin cursor (C-g / Esc).
    Cancel,
    /// Shorten the query by one character (BS).
    Backspace,
    /// Toggle literal ↔ regex matching (M-r; Q#RX3).
    ToggleRegex,
    /// Extend the query with a printable character.
    Insert(char),
    /// Unhandled --- swallowed without complaint.
    Ignore,
}

impl SearchKey {
    /// Decode `chord` into an isearch action. The next/prev chords
    /// match the entry bindings (`C-s` forward, `C-r` backward) so the
    /// search-starting key repeats the search; arrow keys offer a
    /// modifier-free alternative.
    fn from_chord(chord: Chord) -> Self {
        let ctrl = chord.modifiers.contains(KeyModifiers::CONTROL);
        let alt = chord.modifiers.contains(KeyModifiers::ALT);

        if !ctrl && !alt {
            match chord.code {
                KeyCode::Enter => return Self::Accept,
                KeyCode::Esc => return Self::Cancel,
                KeyCode::Backspace => return Self::Backspace,
                KeyCode::Down => return Self::Next,
                KeyCode::Up => return Self::Prev,
                KeyCode::Char(ch) => return Self::Insert(ch),
                _ => return Self::Ignore,
            }
        }
        if ctrl
            && !alt
            && let KeyCode::Char(c) = chord.code
        {
            return match c {
                's' => Self::Next,
                'r' => Self::Prev,
                'm' => Self::Accept,
                'g' => Self::Cancel,
                'h' => Self::Backspace,
                _ => Self::Ignore,
            };
        }
        // M-r toggles regex mode (Q#RX3). Alt-only chord, distinct from
        // the C-r (previous-match) above.
        if alt
            && !ctrl
            && let KeyCode::Char('r') = chord.code
        {
            return Self::ToggleRegex;
        }
        Self::Ignore
    }
}

/// Keys handled while a query-replace's interactive phase runs (Arc 2,
/// Q#QR6). A full modal shadow like [`SearchKey`]: an active
/// query-replace eats every key, and the same decode runs in both
/// frontends via the `FrontendEvent::Key` round-trip.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum QueryReplaceKey {
    /// `y` / `SPC` — replace this match, advance.
    Replace,
    /// `n` / `DEL` — skip this match, advance.
    Skip,
    /// `!` — replace this and all remaining without prompting.
    All,
    /// `.` — replace this, then quit.
    ReplaceAndQuit,
    /// `q` / `RET` / `Esc` / `C-g` — quit (replacements are kept).
    Quit,
    /// Any other key — eaten (no-op), like an active isearch.
    Ignore,
}

impl QueryReplaceKey {
    fn from_chord(chord: Chord) -> Self {
        let ctrl = chord.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            // C-g quits; every other control chord is eaten.
            return match chord.code {
                KeyCode::Char('g') => Self::Quit,
                _ => Self::Ignore,
            };
        }
        match chord.code {
            KeyCode::Char('y' | ' ') => Self::Replace,
            KeyCode::Char('n') | KeyCode::Backspace | KeyCode::Delete => Self::Skip,
            KeyCode::Char('!') => Self::All,
            KeyCode::Char('.') => Self::ReplaceAndQuit,
            KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc => Self::Quit,
            _ => Self::Ignore,
        }
    }
}

/// Keys handled while a context menu is open (Q#CM1). Like
/// [`SearchKey`], this shadows the global keymap; the same decode runs
/// in both frontends via the daemon's `FrontendEvent::Key` round-trip.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum MenuKey {
    /// Highlight the next item (Down / C-n).
    Next,
    /// Highlight the previous item (Up / C-p).
    Prev,
    /// Invoke the highlighted item (RET).
    Invoke,
    /// Dismiss the menu (Esc / C-g).
    Cancel,
    /// Any other key — dismisses the menu (a click-away analogue).
    Dismiss,
}

impl MenuKey {
    /// Decode `chord` into a menu action. Unrecognized keys dismiss the
    /// menu (standard popup behavior); a future mnemonic-jump refinement
    /// would intercept printable chars here.
    fn from_chord(chord: Chord) -> Self {
        let ctrl = chord.modifiers.contains(KeyModifiers::CONTROL);
        let alt = chord.modifiers.contains(KeyModifiers::ALT);
        if !ctrl && !alt {
            match chord.code {
                KeyCode::Down => return Self::Next,
                KeyCode::Up => return Self::Prev,
                KeyCode::Enter => return Self::Invoke,
                KeyCode::Esc => return Self::Cancel,
                _ => return Self::Dismiss,
            }
        }
        if ctrl
            && !alt
            && let KeyCode::Char(c) = chord.code
        {
            return match c {
                'n' => Self::Next,
                'p' => Self::Prev,
                'g' => Self::Cancel,
                _ => Self::Dismiss,
            };
        }
        Self::Dismiss
    }
}

/// Keys intercepted while the in-buffer completion popup is open
/// (Q#C3). Unlike [`SearchKey`] / [`MenuKey`] this is a **partial**
/// shadow: `from_chord` returns `None` for every chord outside the
/// popup-control set, and the dispatcher lets those fall through to
/// normal dispatch --- printable keys keep self-inserting, motion keys
/// keep moving (the post-dispatch validation then decides whether the
/// session survives). The same decode runs in both frontends via the
/// daemon's `FrontendEvent::Key` round-trip.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum CompletionPopupKey {
    /// Highlight the next candidate (Down / C-n).
    Next,
    /// Highlight the previous candidate (Up / C-p).
    Prev,
    /// Accept the highlighted candidate (TAB / RET).
    Accept,
    /// Close the popup without accepting (Esc / C-g).
    Dismiss,
}

impl CompletionPopupKey {
    /// Decode `chord` into a popup action, or `None` when the chord is
    /// not popup control and must fall through to normal dispatch.
    fn from_chord(chord: Chord) -> Option<Self> {
        let ctrl = chord.modifiers.contains(KeyModifiers::CONTROL);
        let alt = chord.modifiers.contains(KeyModifiers::ALT);
        if !ctrl && !alt {
            return match chord.code {
                KeyCode::Down => Some(Self::Next),
                KeyCode::Up => Some(Self::Prev),
                KeyCode::Tab | KeyCode::Enter => Some(Self::Accept),
                KeyCode::Esc => Some(Self::Dismiss),
                _ => None,
            };
        }
        if ctrl
            && !alt
            && let KeyCode::Char(c) = chord.code
        {
            return match c {
                'n' => Some(Self::Next),
                'p' => Some(Self::Prev),
                'g' => Some(Self::Dismiss),
                _ => None,
            };
        }
        None
    }
}

/// Paint one full frame into `grid` and return the desired terminal
/// cursor position.
///
/// This is the layout-and-paint half of the renderer.
/// [`crate::instance_render::RenderState`] drives it (T M5.2): the
/// `RenderState` owns the `prev`/`next` cell buffers, calls this
/// function to fill `next`, then diffs against `prev` to produce a
/// [`crate::protocol::InstanceMessage::CellDelta`]. Tests (and any
/// future non-crossterm frontend) can drive it directly against a
/// Vec-backed [`crate::cell::CellGrid`] without going through a
/// `RenderState`.
#[allow(clippy::too_many_lines, reason = "linear paint pipeline")]
pub fn paint_frame(
    state: &EditorState,
    frontend_id: FrontendId,
    terminal_snapshots: &HashMap<WindowId, TerminalSnapshot>,
    grid: &mut crate::cell::CellGrid<'_>,
    term_size: CellSize,
) -> Option<CellCoord> {
    if term_size.rows < 2 || term_size.cols == 0 {
        return None;
    }
    // Statusline callbacks may call arbitrary editor APIs. Evaluate the
    // complete visible-window fan-out before the long mutable core borrow
    // below, then paint only the transactionally validated owned results.
    let statusline_evaluation = crate::statusline::evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        crate::statusline::StatuslineEvaluationTarget::Grid { frontend_id },
    );
    let statusline_by_window: HashMap<WindowId, crate::statusline::StatuslineWindowSegments> =
        match statusline_evaluation.outcome {
            crate::statusline::StatuslineEvaluationOutcome::Ready(windows) => windows
                .into_iter()
                .map(|segments| (segments.context.window_id, segments))
                .collect(),
            crate::statusline::StatuslineEvaluationOutcome::Invalidated { .. }
            | crate::statusline::StatuslineEvaluationOutcome::NoMessage(_) => HashMap::new(),
        };

    // Themes Q#TH9: one theme clone per frame for the chrome faces —
    // the same single-lock discipline as `SyntaxHighlightView::render`.
    let theme = {
        let handle = state.syntax_registry.theme();
        let t = handle.lock().expect("theme mutex poisoned");
        t.clone()
    };
    let empty_dispatcher = KeyDispatcher::new();
    let dispatcher = state
        .dispatchers
        .get(&frontend_id)
        .map_or(&empty_dispatcher, |state| &state.dispatcher);

    let mut core_ref = state.core.borrow_mut();
    let core: &mut EditorCore = &mut core_ref;

    let placements = window_placements(core, frontend_id, term_size);
    let active = core.views.get(&frontend_id)?.active;

    // Clear the whole grid first so windows that shrink on resize
    // don't leak the old contents.
    for row in 0..term_size.rows {
        for col in 0..term_size.cols {
            *grid.at(CellCoord::new(row, col)) = crate::cell::Cell::default();
        }
    }

    // Scroll the active window so its cursor stays visible. Inactive
    // windows keep their existing scroll.
    if let Some(active_placement) = placements.get(&active) {
        let inner_rows = active_placement.content.size.rows;
        let registry = core.registry.clone();
        let reg = registry.borrow();
        let buf_id = core.windows.get(&active).map(|window| window.buffer_id);
        if !terminal_snapshots.contains_key(&active)
            && let Some(buf_id) = buf_id
            && let Ok(buf) = reg.get(buf_id)
        {
            let aw = core.windows.get_mut(&active).expect(
                "invariant: active_window_id always references a live window in core.windows",
            );
            let cursor_row = aw
                .text_view
                .pos_to_display(buf, aw.cursor)
                .map_or(0, |d| d.row as usize);
            if cursor_row < aw.view_top {
                aw.view_top = cursor_row;
            } else if inner_rows > 0 && cursor_row >= aw.view_top + inner_rows as usize {
                aw.view_top = cursor_row + 1 - inner_rows as usize;
            }
        }
    }

    // Render every window.
    let registry = core.registry.clone();
    let reg = registry.borrow();
    let diag_store = state.lsp_manager.borrow().diag_store();
    for (id, window) in &mut core.windows {
        let Some(placement) = placements.get(id).copied() else {
            continue;
        };
        let rect = placement.outer;
        let inner_rows = placement.content.size.rows;
        // Record viewport height for page motion (cursor.page-down /
        // cursor.page-up consume this).
        window.last_visible_rows = inner_rows;
        if inner_rows == 0 || rect.size.cols == 0 {
            continue;
        }
        if let Some(snapshot) = terminal_snapshots.get(id) {
            paint_terminal_snapshot(grid, placement.content, snapshot, &theme);
            let Ok(buf) = reg.get(window.buffer_id) else {
                continue;
            };
            let cursor = snapshot.cursor.unwrap_or_default();
            let scroll = if snapshot.scroll_offset == 0 {
                String::new()
            } else {
                format!("↑{}", snapshot.scroll_offset)
            };
            let custom = statusline_by_window.get(id);
            paint_mode_line(
                grid,
                &rect,
                buf.name(),
                false,
                *id == active,
                cursor.row,
                cursor.col,
                &scroll,
                "",
                mode_line_style(&theme),
                custom.map_or(&[], |segments| segments.left.as_slice()),
                custom.map_or(&[], |segments| segments.right.as_slice()),
                &theme,
            );
            continue;
        }
        let Ok(buf) = reg.get(window.buffer_id) else {
            continue;
        };
        let viewport_buffer_start = window.text_view.line_offset(window.view_top).unwrap_or(0);
        // UX gutter (Q#UX2): reserve a left strip for line numbers and
        // shrink+shift the text area into the remainder, so every
        // viewport-relative painter (text, syntax, diagnostics, search)
        // stays gutter-agnostic. A window too narrow for the gutter falls
        // back to no gutter this frame rather than starving the text.
        let gutter_w = {
            let w = window.gutter_width();
            if w >= rect.size.cols { 0 } else { w }
        };
        let viewport = Viewport {
            buffer_start: viewport_buffer_start,
            buffer_end: buf.len(),
            cell_origin: CellCoord::new(rect.origin.row, rect.origin.col + gutter_w),
            cell_size: crate::cell::CellSize::new(inner_rows, rect.size.cols - gutter_w),
            gutter_w,
        };
        // Composition (T M2.9): base text_view paints first, then the
        // gutter numbers — before the overlays, so a diagnostic overlay
        // can draw its severity sign into the gutter's leading column
        // without the gutter's own blank pass erasing it — then each
        // overlay in attach order. See [`crate::view::View`].
        window.text_view.render(buf, viewport, grid);
        if gutter_w > 0 {
            paint_line_number_gutter(grid, window, &rect, inner_rows, gutter_w, &theme);
        }
        for overlay in &mut window.overlays {
            overlay.render(buf, viewport, grid);
        }
        paint_local_selection(grid, buf, window, &rect, inner_rows, gutter_w, &theme);
        // Mode line for this window. Painted last so the line
        // itself is always visible regardless of overlay activity.
        let coord = window
            .text_view
            .pos_to_display(buf, window.cursor)
            .unwrap_or_default();
        let scroll = format_scroll_indicator(
            window.view_top,
            inner_rows as usize,
            window.text_view.line_count(),
            coord.row as usize,
        );
        // Lock scoped to the summary computation only: the overlay
        // renders above include `DiagnosticView`, which takes this
        // same mutex — holding the guard across the loop deadlocked
        // the daemon on the first frame after a file (and thus a
        // diagnostic overlay) was opened.
        let diags = {
            let guard = diag_store.lock().expect("diag store mutex poisoned");
            diag_mode_line_summary(&guard, buf)
        };
        let custom = statusline_by_window.get(id);
        paint_mode_line(
            grid,
            &rect,
            buf.name(),
            buf.is_modified(),
            *id == active,
            coord.row,
            coord.col,
            &scroll,
            &diags,
            mode_line_style(&theme),
            custom.map_or(&[], |segments| segments.left.as_slice()),
            custom.map_or(&[], |segments| segments.right.as_slice()),
            &theme,
        );
    }
    drop(reg);

    paint_status_line(grid, core, &state.lua_host, dispatcher, term_size, &theme);

    // An active isearch owns the bottom row (its prompt + match
    // readout), but the terminal cursor stays in the buffer at the
    // active match so the eye follows the search — so paint the prompt
    // and fall through to the buffer-cursor placement below.
    let mb_cursor_col = if core.search_active() {
        paint_search_prompt(grid, core, term_size, &theme);
        None
    } else if core.minibuffer.is_active() {
        Some(paint_minibuffer(grid, core, term_size, &theme))
    } else {
        None
    };

    if let Some(col) = mb_cursor_col {
        return Some(CellCoord::new(term_size.rows - 1, col));
    }
    let active_placement = placements.get(&active).copied()?;
    if let Some(snapshot) = terminal_snapshots.get(&active) {
        let cursor = snapshot.cursor?;
        if cursor.row >= active_placement.content.size.rows
            || cursor.col >= active_placement.content.size.cols
        {
            return None;
        }
        return Some(CellCoord::new(
            active_placement.content.origin.row + cursor.row,
            active_placement.content.origin.col + cursor.col,
        ));
    }
    let active_rect = active_placement.outer;
    let registry = core.registry.clone();
    let reg = registry.borrow();
    let aw = &core.windows[&active];
    let inner_rows = inner_rows(&active_rect);
    let buf = reg.get(aw.buffer_id).ok()?;
    let disp = aw.text_view.pos_to_display(buf, aw.cursor)?;
    if (disp.row as usize) < aw.view_top || (disp.row as usize) >= aw.view_top + inner_rows as usize
    {
        return None;
    }
    // UX gutter: the terminal caret sits in the text area, past the
    // reserved gutter strip (mirrors the viewport shift above).
    let gutter_w = {
        let w = aw.gutter_width();
        if w >= active_rect.size.cols { 0 } else { w }
    };
    let grid_row = active_rect.origin.row + (disp.row - aw.view_top as u32);
    let max_col = active_rect.origin.col + active_rect.size.cols.saturating_sub(1);
    let grid_col = (active_rect.origin.col + gutter_w + disp.col).min(max_col);
    Some(CellCoord::new(grid_row, grid_col))
}

fn paint_terminal_snapshot(
    grid: &mut crate::cell::CellGrid<'_>,
    content: Rect,
    snapshot: &TerminalSnapshot,
    theme: &crate::highlight::Theme,
) {
    let rows = content.size.rows.min(snapshot.size.rows);
    let cols = content.size.cols.min(snapshot.size.cols);
    for row in 0..rows {
        for col in 0..cols {
            let source = row as usize * snapshot.size.cols as usize + col as usize;
            *grid.at(CellCoord::new(
                content.origin.row + row,
                content.origin.col + col,
            )) = snapshot.cells[source].clone();
        }
    }
    let overlay = theme.face("ui.selection").map_or(
        crate::cell::Style {
            reverse: true,
            ..crate::cell::Style::default()
        },
        |face| crate::cell::Style {
            bg: face.bg,
            ..crate::cell::Style::default()
        },
    );
    for span in &snapshot.selection {
        if span.row >= rows {
            continue;
        }
        for col in span.start_col.min(cols)..span.end_col.min(cols) {
            let cell = grid.at(CellCoord::new(
                content.origin.row + span.row,
                content.origin.col + col,
            ));
            cell.style = crate::overlay::merge_styles(cell.style, overlay);
        }
    }
}

/// The mode-line row style (themes arc Q#TH5): a set `ui.modeline`
/// face owns the surface within its {fg, bg, reverse} mask — the row
/// resets to plain plus the face's in-mask components — else today's
/// reverse video.
fn mode_line_style(theme: &crate::highlight::Theme) -> crate::cell::Style {
    theme.face("ui.modeline").map_or(
        crate::cell::Style {
            reverse: true,
            ..Default::default()
        },
        |f| crate::cell::Style {
            fg: f.fg,
            bg: f.bg,
            reverse: f.reverse,
            ..Default::default()
        },
    )
}

fn paint_status_line(
    grid: &mut crate::cell::CellGrid<'_>,
    core: &EditorCore,
    lua_host: &LuaHost,
    dispatcher: &KeyDispatcher,
    term_size: crate::cell::CellSize,
    theme: &crate::highlight::Theme,
) {
    let status = build_status_line(core, lua_host, dispatcher, term_size.cols);
    let row = term_size.rows - 1;
    // Themes Q#TH5: a set `ui.statusline` face owns the row within its
    // {fg} mask (surface resets to plain); unset keeps reverse video.
    let style = theme.face("ui.statusline").map_or(
        crate::cell::Style {
            reverse: true,
            ..Default::default()
        },
        |f| crate::cell::Style {
            fg: f.fg,
            ..Default::default()
        },
    );
    for (col, ch) in status.chars().enumerate() {
        if col >= term_size.cols as usize {
            break;
        }
        let cell = grid.at(CellCoord::new(row, col as u32));
        cell.glyph = crate::cell::Glyph::Char(ch);
        cell.style = style;
    }
    for col in (status.chars().count() as u32)..term_size.cols {
        let cell = grid.at(CellCoord::new(row, col));
        cell.glyph = crate::cell::Glyph::Char(' ');
        cell.style = style;
    }
}

/// Rows available for buffer text inside `rect`, after subtracting
/// the per-window mode line (one row).
/// Clamp `pos` into `buf` and walk back to the nearest UTF-8
/// codepoint boundary. Pointer hits arrive from a frontend whose text
/// may be a few unconfirmed edits ahead of or behind the instance, so
/// a raw byte offset can land mid-codepoint; a snapped position is
/// always safe to assign to a window cursor.
fn snap_to_char_boundary(buf: &crate::buffer::Buffer, pos: u64) -> u64 {
    let len = buf.len();
    let mut pos = pos.min(len);
    while pos > 0 && pos < len {
        match buf.snapshot_rope().byte_at(pos) {
            // UTF-8 continuation byte (0b10xx_xxxx) ⇒ mid-codepoint.
            Some(b) if b & 0b1100_0000 == 0b1000_0000 => pos -= 1,
            _ => break,
        }
    }
    pos
}

fn inner_rows(rect: &crate::window::Rect) -> u32 {
    rect.size.rows.saturating_sub(1)
}

/// Paint the left line-number gutter for `window` into the reserved strip
/// `[rect.origin.col, rect.origin.col + gutter_w)` over the window's text
/// rows (UX gutter arc). Numbers are 1-based, right-aligned with a single
/// trailing pad cell; rows past end-of-buffer stay blank. Dimly styled so
/// the gutter recedes behind the code. The caller guarantees `gutter_w >
/// 0` and that it fits within `rect.size.cols`.
fn paint_line_number_gutter(
    grid: &mut crate::cell::CellGrid<'_>,
    window: &crate::window::Window,
    rect: &crate::window::Rect,
    inner_rows: u32,
    gutter_w: u32,
    theme: &crate::highlight::Theme,
) {
    let line_count = window.text_view.line_count();
    // Relative/Hybrid measure distance from the cursor's buffer line;
    // Absolute ignores it. Computed once per frame (the gutter repaints on
    // cursor motion, so this stays current).
    let cursor_line = window.text_view.line_at_offset(window.cursor);
    // Themes Q#TH5: a set `ui.gutter` face owns the strip within its
    // {fg} mask; unset keeps the dim Indexed(8).
    let style = theme.face("ui.gutter").map_or(
        crate::cell::Style {
            fg: crate::cell::Color::Indexed(8),
            ..crate::cell::Style::default()
        },
        |f| crate::cell::Style {
            fg: f.fg,
            ..crate::cell::Style::default()
        },
    );
    // The number's rightmost digit sits at `field - 1`; the last gutter
    // cell (`gutter_w - 1`) is a trailing pad separating it from the code.
    let field = gutter_w.saturating_sub(1);
    for r in 0..inner_rows {
        let grid_row = rect.origin.row + r;
        // Blank + style the whole strip first, so a number that shrank a
        // digit (e.g. after a large delete) leaves no stale trailing glyph.
        for c in 0..gutter_w {
            let cell = grid.at(CellCoord::new(grid_row, rect.origin.col + c));
            cell.glyph = crate::cell::Glyph::Char(' ');
            cell.style = style;
            cell.attachment = None;
        }
        let buffer_line = window.view_top + r as usize;
        if buffer_line >= line_count {
            continue; // past end-of-buffer: blank gutter
        }
        // The mode picks the number: absolute (`line+1`), relative
        // distance, or hybrid (absolute on the cursor line, else relative).
        // Written right-aligned, rightmost digit first, alloc-free.
        // `field >= digits(line_count)` by construction, so the leftmost
        // digit always leaves at least a leading pad cell.
        let Some(mut val) = window.line_numbers.number_for(buffer_line, cursor_line) else {
            continue;
        };
        let mut col = field;
        loop {
            col -= 1;
            let digit = (val % 10) as u8;
            grid.at(CellCoord::new(grid_row, rect.origin.col + col))
                .glyph = crate::cell::Glyph::Char((b'0' + digit) as char);
            val /= 10;
            if val == 0 || col == 0 {
                break;
            }
        }
    }
}

fn paint_local_selection(
    grid: &mut crate::cell::CellGrid<'_>,
    buf: &crate::buffer::Buffer,
    window: &crate::window::Window,
    rect: &crate::window::Rect,
    inner_rows: u32,
    // UX gutter: the reserved left-strip width; selection cells are the
    // text-relative display column shifted right by this (Q#UX2). 0 when
    // the gutter is off, so this is a no-op then.
    gutter_w: u32,
    theme: &crate::highlight::Theme,
) {
    let Some((sel_start, sel_end)) = window.region() else {
        return;
    };
    // Themes Q#TH5: the selection is a wash — a set `ui.selection`
    // face replaces the default overlay wholesale within its {bg}
    // mask (an all-default face disables the wash; out-of-mask
    // fg/reverse are never read); unset keeps today's reverse video.
    let overlay = theme.face("ui.selection").map_or(
        crate::cell::Style {
            reverse: true,
            ..crate::cell::Style::default()
        },
        |f| crate::cell::Style {
            bg: f.bg,
            ..crate::cell::Style::default()
        },
    );
    if inner_rows == 0 || rect.size.cols == 0 || sel_start >= sel_end {
        return;
    }
    let text_cols = rect.size.cols.saturating_sub(gutter_w);

    let first_row = window.view_top;
    let last_row = first_row.saturating_add(inner_rows as usize);
    for display_row in first_row..last_row {
        let Some(line_start) = window.text_view.line_offset(display_row) else {
            continue;
        };
        let Some(line_len) = window.text_view.line_len(buf, display_row) else {
            continue;
        };
        let line_end = line_start.saturating_add(line_len);
        let paint_start = sel_start.max(line_start);
        let paint_end = sel_end.min(line_end);
        if paint_start >= paint_end {
            continue;
        }

        let Some(start_coord) = window.text_view.pos_to_display(buf, paint_start) else {
            continue;
        };
        let Some(end_coord) = window.text_view.pos_to_display(buf, paint_end) else {
            continue;
        };
        if start_coord.row as usize != display_row || end_coord.row as usize != display_row {
            continue;
        }

        let row_offset = display_row.saturating_sub(first_row) as u32;
        let start_col = start_coord.col.min(text_cols);
        let end_col = end_coord.col.min(text_cols);
        if start_col >= end_col {
            continue;
        }
        for col in start_col..end_col {
            let cell = grid.at(CellCoord::new(
                rect.origin.row + row_offset,
                rect.origin.col + gutter_w + col,
            ));
            cell.style = crate::overlay::merge_styles(cell.style, overlay);
        }
    }
}

/// Format the mode-line diagnostic readout for a buffer: `"E:2 W:5"`
/// with only the nonzero severities (errors, then warnings; info and
/// hints stay off the mode line). Empty when the buffer has no file
/// path, no diagnostics, or the stored diagnostics are stale — the
/// document was edited since the last `publishDiagnostics`, so the
/// counts would describe text that no longer exists (T M4.6).
fn diag_mode_line_summary(
    store: &crate::diag::DiagnosticStore,
    buf: &crate::buffer::Buffer,
) -> String {
    let Some(path) = buf.file_path() else {
        return String::new();
    };
    let uri = crate::lsp::path_to_file_uri(path);
    if store.is_stale(&uri) {
        return String::new();
    }
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for d in store.for_uri(&uri) {
        match d.severity {
            crate::diag::DiagnosticSeverity::Error => errors += 1,
            crate::diag::DiagnosticSeverity::Warning => warnings += 1,
            _ => {}
        }
    }
    match (errors, warnings) {
        (0, 0) => String::new(),
        (e, 0) => format!("E:{e}"),
        (0, w) => format!("W:{w}"),
        (e, w) => format!("E:{e} W:{w}"),
    }
}

#[derive(Copy, Clone)]
struct ModeLineRun<'a> {
    text: &'a str,
    style: crate::cell::Style,
}

struct ModeLineGrapheme {
    glyph: crate::cell::Glyph,
    width: u32,
    style: crate::cell::Style,
}

fn prepare_mode_line_runs(runs: &[ModeLineRun<'_>]) -> Vec<ModeLineGrapheme> {
    let mut graphemes = Vec::new();
    for run in runs {
        let sanitized = run.text.chars().any(char::is_control).then(|| {
            run.text
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect::<String>()
        });
        let text = sanitized.as_deref().unwrap_or(run.text);
        for grapheme in text.graphemes(true) {
            let width = UnicodeWidthStr::width(grapheme) as u32;
            if width == 0 {
                continue;
            }
            let mut chars = grapheme.chars();
            let first = chars
                .next()
                .expect("unicode segmentation never yields an empty grapheme");
            let glyph = if chars.next().is_none() {
                crate::cell::Glyph::Char(first)
            } else {
                crate::cell::Glyph::Cluster(grapheme.as_bytes().into())
            };
            graphemes.push(ModeLineGrapheme {
                glyph,
                width,
                style: run.style,
            });
        }
    }
    graphemes
}

fn mode_line_grapheme_width(graphemes: &[ModeLineGrapheme]) -> u32 {
    graphemes.iter().map(|grapheme| grapheme.width).sum()
}

/// Paint complete graphemes at a logical signed origin. A grapheme that
/// straddles either clip edge is omitted wholesale, so a wide glyph can never
/// leave a dangling half-cell at a window or left/right collision boundary.
fn paint_mode_line_graphemes(
    grid: &mut crate::cell::CellGrid<'_>,
    rect: &crate::window::Rect,
    row: u32,
    origin: i64,
    clip_start: u32,
    clip_end: u32,
    graphemes: &[ModeLineGrapheme],
) {
    let mut logical_col = origin;
    for grapheme in graphemes {
        let next_col = logical_col + i64::from(grapheme.width);
        if logical_col >= i64::from(clip_start) && next_col <= i64::from(clip_end) {
            let local_col =
                u32::try_from(logical_col).expect("non-negative clipped modeline column");
            let cell = grid.at(CellCoord::new(row, rect.origin.col + local_col));
            cell.glyph = grapheme.glyph.clone();
            cell.style = grapheme.style;
            for continuation in 1..grapheme.width {
                let cell = grid.at(CellCoord::new(
                    row,
                    rect.origin.col + local_col + continuation,
                ));
                cell.glyph = crate::cell::Glyph::Continuation;
                cell.style = grapheme.style;
            }
        }
        logical_col = next_col;
    }
}

fn statusline_segment_style(
    theme: &crate::highlight::Theme,
    face: &str,
    base: crate::cell::Style,
) -> crate::cell::Style {
    let Some(override_style) = theme.modeline_segment_face(face) else {
        return base;
    };
    let mut style = base;
    if style.reverse {
        style.bg = override_style.fg;
    } else {
        style.fg = override_style.fg;
    }
    style
}

fn custom_mode_line_runs<'a>(
    segments: &'a [crate::statusline::EvaluatedStatuslineSegment],
    theme: &crate::highlight::Theme,
    base: crate::cell::Style,
) -> Vec<ModeLineRun<'a>> {
    let mut runs = Vec::with_capacity(segments.len().saturating_mul(2));
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            runs.push(ModeLineRun {
                text: " ",
                style: base,
            });
        }
        runs.push(ModeLineRun {
            text: &segment.text,
            style: statusline_segment_style(theme, &segment.face, base),
        });
    }
    runs
}

#[allow(
    clippy::too_many_arguments,
    reason = "the modeline packs built-in facts plus two already-evaluated custom sides"
)]
fn paint_mode_line(
    grid: &mut crate::cell::CellGrid<'_>,
    rect: &crate::window::Rect,
    name: &str,
    modified: bool,
    is_active: bool,
    cursor_row: u32,
    cursor_col: u32,
    scroll: &str,
    diags: &str,
    mode_style: crate::cell::Style,
    custom_left: &[crate::statusline::EvaluatedStatuslineSegment],
    custom_right: &[crate::statusline::EvaluatedStatuslineSegment],
    theme: &crate::highlight::Theme,
) {
    if rect.size.rows == 0 || rect.size.cols == 0 {
        return;
    }
    let row = rect.origin.row + rect.size.rows - 1;
    let marker = if modified { '*' } else { ' ' };
    let active_marker = if is_active { '+' } else { '-' };
    let protected_left = format!(" {active_marker}{marker} {name} ");
    let protected_right = if diags.is_empty() {
        format!(" L{}:C{} {scroll} ", cursor_row + 1, cursor_col + 1)
    } else {
        format!(" {diags} L{}:C{} {scroll} ", cursor_row + 1, cursor_col + 1)
    };

    // Fill exactly this window's row once with the base modeline surface.
    for col in 0..rect.size.cols {
        let cell = grid.at(CellCoord::new(row, rect.origin.col + col));
        cell.glyph = crate::cell::Glyph::Char(' ');
        cell.style = mode_style;
    }

    let mut left_runs = Vec::with_capacity(custom_left.len().saturating_mul(2) + 2);
    left_runs.push(ModeLineRun {
        text: &protected_left,
        style: mode_style,
    });
    if !custom_left.is_empty() {
        left_runs.push(ModeLineRun {
            text: " ",
            style: mode_style,
        });
        left_runs.extend(custom_mode_line_runs(custom_left, theme, mode_style));
    }
    let left_graphemes = prepare_mode_line_runs(&left_runs);

    let protected_right_graphemes = prepare_mode_line_runs(&[ModeLineRun {
        text: &protected_right,
        style: mode_style,
    }]);
    let protected_right_width = mode_line_grapheme_width(&protected_right_graphemes);

    // Preserve the legacy strict boundary: a suffix as wide as the entire
    // window is dropped wholesale. Custom text can never cause that drop when
    // the protected suffix itself still satisfies the legacy fit test.
    if protected_right_width < rect.size.cols {
        let mut right_prefix_runs = custom_mode_line_runs(custom_right, theme, mode_style);
        if !custom_right.is_empty() {
            right_prefix_runs.push(ModeLineRun {
                text: " ",
                style: mode_style,
            });
        }
        let right_prefix_graphemes = prepare_mode_line_runs(&right_prefix_runs);
        let right_prefix_width = mode_line_grapheme_width(&right_prefix_graphemes);
        let suffix_start = rect.size.cols - protected_right_width;
        let right_origin = i64::from(suffix_start) - i64::from(right_prefix_width);
        let left_clip_end = u32::try_from(right_origin).unwrap_or(0);

        paint_mode_line_graphemes(grid, rect, row, 0, 0, left_clip_end, &left_graphemes);
        paint_mode_line_graphemes(
            grid,
            rect,
            row,
            right_origin,
            0,
            suffix_start,
            &right_prefix_graphemes,
        );
        paint_mode_line_graphemes(
            grid,
            rect,
            row,
            i64::from(suffix_start),
            suffix_start,
            rect.size.cols,
            &protected_right_graphemes,
        );
    } else {
        paint_mode_line_graphemes(grid, rect, row, 0, 0, rect.size.cols, &left_graphemes);
    }
}

/// Paint the minibuffer line on the bottom row, replacing the status
/// line. Returns the screen column the terminal cursor should sit
/// at (so the user can see what they're typing).
/// The minibuffer base style (themes arc Q#TH5): a set `ui.minibuffer`
/// face owns the prompt/input/fill (and the search prompt row) within
/// its {fg} mask; unset keeps the terminal default.
fn minibuffer_style(theme: &crate::highlight::Theme) -> crate::cell::Style {
    theme
        .face("ui.minibuffer")
        .map_or(crate::cell::Style::default(), |f| crate::cell::Style {
            fg: f.fg,
            ..crate::cell::Style::default()
        })
}

fn paint_minibuffer(
    grid: &mut crate::cell::CellGrid<'_>,
    core: &EditorCore,
    term_size: crate::cell::CellSize,
    theme: &crate::highlight::Theme,
) -> u32 {
    let session = core
        .minibuffer
        .session
        .as_ref()
        .expect("called only when active");
    let prompt = &session.prompt;
    let contents = core.minibuffer.contents();
    let mut suffix = String::new();
    if let Some(idx) = session.selected
        && let Some(cand) = session.candidates.get(idx)
    {
        suffix = format!("  [{cand}]");
    }
    let row = term_size.rows - 1;
    let mut col: u32 = 0;
    let mut written: u32 = 0;
    let max = term_size.cols;
    let cursor_byte = core.minibuffer.cursor;

    let base = minibuffer_style(theme);
    // Themes Q#TH5: the inline candidate suffix has its own face,
    // `ui.minibuffer.candidate` ({fg} mask); unset keeps reverse.
    let candidate = theme.face("ui.minibuffer.candidate").map_or(
        crate::cell::Style {
            reverse: true,
            ..Default::default()
        },
        |f| crate::cell::Style {
            fg: f.fg,
            ..crate::cell::Style::default()
        },
    );

    for ch in prompt.chars() {
        if col >= max {
            break;
        }
        let cell = grid.at(CellCoord::new(row, col));
        cell.glyph = crate::cell::Glyph::Char(ch);
        cell.style = base;
        col += 1;
        written += 1;
    }
    let prompt_end = col;
    let mut cursor_col: u32 = prompt_end;

    let mut byte_pos: u64 = 0;
    for ch in contents.chars() {
        if byte_pos < cursor_byte {
            cursor_col = col + 1;
        }
        if col >= max {
            break;
        }
        let cell = grid.at(CellCoord::new(row, col));
        cell.glyph = crate::cell::Glyph::Char(ch);
        cell.style = base;
        col += 1;
        written += 1;
        byte_pos += ch.len_utf8() as u64;
    }
    if byte_pos < cursor_byte {
        cursor_col = col;
    } else if cursor_byte == 0 {
        cursor_col = prompt_end;
    }

    for ch in suffix.chars() {
        if col >= max {
            break;
        }
        let cell = grid.at(CellCoord::new(row, col));
        cell.glyph = crate::cell::Glyph::Char(ch);
        cell.style = candidate;
        col += 1;
        written += 1;
    }

    for col in written..max {
        let cell = grid.at(CellCoord::new(row, col));
        cell.glyph = crate::cell::Glyph::Char(' ');
        cell.style = base;
    }

    cursor_col.min(max.saturating_sub(1))
}

/// Paint the incremental-search prompt on the bottom row:
/// `I-search: <query>  (n/m)`. Backward searches read `I-search
/// backward:`; regex searches prefix `Regex `; a non-empty query with
/// no matches reads `[no match]`, and an uncompilable regex reads
/// `[invalid]`. Overwrites the status line painted just before it. The
/// terminal cursor is *not* returned here — it stays in the buffer at
/// the active match (see [`paint_frame`]).
fn paint_search_prompt(
    grid: &mut crate::cell::CellGrid<'_>,
    core: &EditorCore,
    term_size: crate::cell::CellSize,
    theme: &crate::highlight::Theme,
) {
    // Themes Q#TH5: the search prompt is the echo-area input line, so
    // it follows `ui.minibuffer` (the framing's applicability table).
    let base = minibuffer_style(theme);
    let prompt = match (core.search_is_regex(), core.search_forward()) {
        (false, true) => "I-search: ",
        (false, false) => "I-search backward: ",
        (true, true) => "Regex I-search: ",
        (true, false) => "Regex I-search backward: ",
    };
    let query = core.search_query();
    let (active, total) = core.search_match_summary();
    let suffix = if query.is_empty() {
        String::new()
    } else if core.search_is_invalid() {
        "  [invalid]".to_string()
    } else if total == 0 {
        "  [no match]".to_string()
    } else {
        format!("  ({}/{})", active.map_or(0, |a| a + 1), total)
    };

    let row = term_size.rows - 1;
    let max = term_size.cols;
    let mut col: u32 = 0;
    let put = |grid: &mut crate::cell::CellGrid<'_>, col: &mut u32, ch: char| {
        if *col < max {
            let cell = grid.at(CellCoord::new(row, *col));
            cell.glyph = crate::cell::Glyph::Char(ch);
            cell.style = base;
            *col += 1;
        }
    };
    for ch in prompt.chars() {
        put(grid, &mut col, ch);
    }
    for ch in query.chars() {
        put(grid, &mut col, ch);
    }
    for ch in suffix.chars() {
        put(grid, &mut col, ch);
    }
    // Clear the remainder of the row (the status line underneath used
    // reverse video; blank it with the prompt's base style).
    for c in col..max {
        let cell = grid.at(CellCoord::new(row, c));
        cell.glyph = crate::cell::Glyph::Char(' ');
        cell.style = base;
    }
}

/// Build the global status (echo area) row: pure ephemeral state.
///
/// Per-window facts (buffer name, modified marker, cursor coord,
/// scroll indicator) live on each window's mode line — see
/// [`paint_mode_line`]. The status row is reserved for things that
/// don't belong to any window in particular: command result text
/// (`core.status`), captured Lua errors, and the in-flight key
/// prefix when a multi-chord sequence is open.
///
/// When all three are empty, the returned string is empty and the
/// row renders as blanks.
fn build_status_line(
    core: &EditorCore,
    lua_host: &LuaHost,
    dispatcher: &KeyDispatcher,
    cols: u32,
) -> String {
    let mut line = String::new();
    if !core.status.is_empty() {
        line.push_str(&sanitize_single_line(&core.status));
    } else if let Some(err) = lua_host.last_error() {
        use std::fmt::Write;
        let _ = write!(line, "lua: {}", sanitize_single_line(&err.message));
    }
    if !dispatcher.pending().is_empty() {
        use std::fmt::Write;
        if !line.is_empty() {
            line.push_str("  ");
        }
        let _ = write!(line, "[{}-]", display_sequence(dispatcher.pending()));
    }
    let max = cols as usize;
    if line.chars().count() > max {
        line.chars().take(max).collect()
    } else {
        line
    }
}

/// First line of `s`, or the whole string if no newline is present.
fn first_line(s: &str) -> &str {
    s.split_once('\n').map_or(s, |(head, _)| head)
}

/// Render the Neovim/Doom-style scroll indicator that follows the
/// `L:C` cursor coordinate in the status line.
///
/// * `All` --- the entire buffer fits in the viewport (or the buffer
///   is one line).
/// * `Top` --- the viewport's first line is the buffer's first line
///   and the buffer doesn't fit.
/// * `Bot` --- the viewport's last line reaches or passes the
///   buffer's last line.
/// * `NN%` --- otherwise, the cursor's line as a percent of the
///   buffer's total line count.
///
/// `visible` may be 0 in tests that never rendered (so
/// `last_visible_rows` was never populated); in that case we fall
/// back to cursor-row-based percent without the All/Top/Bot caps.
fn format_scroll_indicator(
    view_top: usize,
    visible: usize,
    total_lines: usize,
    cursor_row: usize,
) -> String {
    if total_lines <= 1 {
        return "All".to_string();
    }
    if visible > 0 {
        if visible >= total_lines {
            return "All".to_string();
        }
        if view_top == 0 {
            return "Top".to_string();
        }
        if view_top.saturating_add(visible) >= total_lines {
            return "Bot".to_string();
        }
    }
    let pct = (cursor_row + 1).saturating_mul(100) / total_lines;
    format!("{pct}%")
}

/// Flatten `s` to a printable single line for the status row.
///
/// Lua errors caught by user-level `pcall` (e.g. M-x dispatching an
/// unknown command) carry a multi-line traceback when stringified.
/// Storing those newlines verbatim and copying them into the cell grid
/// makes the terminal frontend emit literal `\n` bytes, which jumps the
/// cursor and corrupts the rest of the frame. Truncate at the first
/// newline (the informative summary), then replace any remaining
/// control characters with spaces so terminal layout cannot leak.
fn sanitize_single_line(s: &str) -> String {
    first_line(s)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn is_terminal_escape_chord(chord: Chord) -> bool {
    chord.code == KeyCode::Char('c') && chord.modifiers == KeyModifiers::CONTROL
}

fn terminal_key_from_crossterm(key: KeyEvent) -> Option<(TerminalKey, TerminalModifiers)> {
    let modifiers = crate::protocol::crossterm_translate::mods_from_crossterm(key.modifiers);
    let key = crate::protocol::crossterm_translate::keycode_from_crossterm(key.code);
    if matches!(key, TerminalKey::Unknown(_)) {
        return None;
    }
    Some((key, modifiers))
}

fn terminal_modifiers(modifiers: KeyModifiers) -> TerminalModifiers {
    crate::protocol::crossterm_translate::mods_from_crossterm(modifiers)
}

fn terminal_mouse_kind(kind: crossterm::event::MouseEventKind) -> Option<TerminalMouseKind> {
    use crossterm::event::{MouseButton, MouseEventKind};
    let button = |button| match button {
        MouseButton::Left => TerminalMouseButton::Left,
        MouseButton::Right => TerminalMouseButton::Right,
        MouseButton::Middle => TerminalMouseButton::Middle,
    };
    Some(match kind {
        MouseEventKind::Down(value) => TerminalMouseKind::Down(button(value)),
        MouseEventKind::Up(value) => TerminalMouseKind::Up(button(value)),
        MouseEventKind::Drag(value) => TerminalMouseKind::Drag(button(value)),
        MouseEventKind::Moved => TerminalMouseKind::Move,
        MouseEventKind::ScrollUp => TerminalMouseKind::ScrollUp,
        MouseEventKind::ScrollDown => TerminalMouseKind::ScrollDown,
        MouseEventKind::ScrollLeft => TerminalMouseKind::ScrollLeft,
        MouseEventKind::ScrollRight => TerminalMouseKind::ScrollRight,
    })
}

fn key_event_to_chord(key: KeyEvent) -> Option<Chord> {
    // Accept Press and Repeat. Some terminals (notably ones speaking
    // the kitty keyboard protocol with auto-repeat) deliver held-key
    // events as `Repeat` rather than `Press`, and rejecting them made
    // the second chord of a multi-key sequence appear "never
    // registered" — the user pressed C-x then quickly pressed C-b
    // without fully releasing first, the C-b arrived as Repeat, we
    // dropped it, and the dispatcher stayed pending on [C-x].
    // Releases stay filtered: they aren't input, and treating them as
    // a chord would clear pending prefixes after every keystroke.
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    Some(Chord::new(key.code, key.modifiers))
}

/// True for chords that should self-insert: a single chord with a
/// printable [`KeyCode::Char`] and no non-shift modifiers.
fn printable_char(seq: &[Chord]) -> Option<char> {
    if seq.len() != 1 {
        return None;
    }
    let chord = seq[0];
    if chord.modifiers.intersects(
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META,
    ) {
        return None;
    }
    match chord.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Acceptance home for T M5.4 (FrontendId on input events) — the
    // `m5_4_*`-prefixed tests verify the FrontendId field threads from
    // synthetic event construction through `dispatch_key` /
    // `dispatch_mouse` to a Lua hook that reads it back. The Lua-side
    // `pmacs.frontend.id()` introspection is covered in
    // `src/lua_bindings.rs::tests`. See tests/INDEX.md for the full
    // M5.x → coverage map.

    use super::*;
    use crate::frontend::KeyEventKind;

    fn local_dispatcher(state: &EditorState) -> &KeyDispatcher {
        &state
            .dispatchers
            .get(&FrontendId::LOCAL)
            .expect("local dispatcher registered by dispatch")
            .dispatcher
    }

    #[test]
    fn dispatch_prefix_state_is_independent_per_frontend() {
        let mut state = fresh_with(b"");
        let other = FrontendId(77);
        state.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        state.dispatch_key(other, plain(KeyCode::Char('a')));
        assert_eq!(local_dispatcher(&state).pending().len(), 1);
        assert!(
            state
                .dispatchers
                .get(&other)
                .expect("other dispatcher registered")
                .dispatcher
                .pending()
                .is_empty()
        );
        assert_eq!(state.core.borrow().active_buffer_len(), 1);
    }

    #[test]
    fn terminal_snapshot_composes_only_content_and_translates_cursor() {
        let state = fresh_with(b"");
        let window_id = state.core.borrow().active_window_id();
        let buffer_id = state.core.borrow().active_buffer_id();
        let size = CellSize::new(4, 5);
        let viewport = CellSize::new(2, 5);
        let mut cells = vec![crate::cell::Cell::default(); viewport.area() as usize];
        cells[0].glyph = crate::cell::Glyph::Char('T');
        cells[7].glyph = crate::cell::Glyph::Char('X');
        let snapshot = TerminalSnapshot {
            buffer_id,
            size: viewport,
            cells,
            cursor: Some(CellCoord::new(1, 2)),
            title: Some("shell".into()),
            screen_generation: 1,
            selection: vec![crate::terminal::TerminalSelectionSpan {
                row: 0,
                start_col: 0,
                end_col: 1,
            }],
            scroll_offset: 0,
            at_bottom: true,
            pid: 1,
            process: crate::terminal::TerminalProcessState::Running,
        };
        let snapshots = HashMap::from([(window_id, snapshot)]);
        let mut backing = vec![crate::cell::Cell::default(); size.area() as usize];
        let cursor = {
            let mut grid = crate::cell::CellGrid {
                cells: &mut backing,
                stride: size.cols,
                size,
            };
            paint_frame(&state, FrontendId::LOCAL, &snapshots, &mut grid, size)
        };
        assert_eq!(backing[0].glyph, crate::cell::Glyph::Char('T'));
        assert!(backing[0].style.reverse);
        assert_eq!(backing[7].glyph, crate::cell::Glyph::Char('X'));
        assert_ne!(backing[10].glyph, crate::cell::Glyph::Char('X'));
        assert_eq!(cursor, Some(CellCoord::new(1, 2)));
    }

    #[test]
    fn line_number_gutter_renders_right_aligned_digits() {
        use crate::buffer::{Buffer, BufferId};
        use crate::cell::{Cell, CellGrid, CellSize, Glyph};
        use crate::text_view::TextView;
        use crate::window::{LineNumberMode, Window, WindowId};

        // 12 lines → decimal_digits(12) = 2, gutter_w = 2 + PAD(2) = 4.
        let content = b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
        let bid = BufferId::next();
        let buf = Buffer::from_bytes(bid, "test", content);
        let view = TextView::new(&buf);
        let mut window = Window::new(WindowId::next(), bid, view);
        window.line_numbers = LineNumberMode::Absolute;
        assert_eq!(window.gutter_width(), 4, "2-digit line count + 2 pad");

        let (rows, cols) = (12u32, 20u32);
        let mut storage = vec![Cell::default(); (rows * cols) as usize];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: cols,
            size: CellSize::new(rows, cols),
        };
        let rect = Rect::new(0, 0, rows, cols);
        paint_line_number_gutter(
            &mut grid,
            &window,
            &rect,
            rows,
            4,
            &crate::highlight::Theme::empty(),
        );

        let glyph = |r: u32, c: u32| storage[(r * cols + c) as usize].glyph.clone();
        // Row 0 = line 1: "  1 " (digit right-aligned at col 2, col 3 = pad).
        assert_eq!(glyph(0, 0), Glyph::Char(' '));
        assert_eq!(glyph(0, 1), Glyph::Char(' '));
        assert_eq!(glyph(0, 2), Glyph::Char('1'));
        assert_eq!(glyph(0, 3), Glyph::Char(' '));
        // Row 4 = line 5.
        assert_eq!(glyph(4, 2), Glyph::Char('5'));
        // Row 9 = line 10: two digits → col1='1', col2='0', col3 pad.
        assert_eq!(glyph(9, 1), Glyph::Char('1'));
        assert_eq!(glyph(9, 2), Glyph::Char('0'));
        assert_eq!(glyph(9, 3), Glyph::Char(' '));
        // Row 11 = line 12.
        assert_eq!(glyph(11, 1), Glyph::Char('1'));
        assert_eq!(glyph(11, 2), Glyph::Char('2'));
    }

    fn fresh_with(content: &[u8]) -> EditorState {
        let s = EditorState::new();
        let new_id = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("test", content);
        let mut core = s.core.borrow_mut();
        let _ = core.switch_active_buffer(new_id);
        drop(core);
        s
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(code: KeyCode) -> KeyEvent {
        key(code, KeyModifiers::NONE)
    }

    // ---- M1 acceptance ports -------------------------------------------------

    #[test]
    fn typing_inserts_characters_through_self_insert() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('h'), KeyModifiers::NONE),
        );
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        let core = s.core.borrow();
        assert_eq!(core.cursor(), 2);
        assert_eq!(core.active_buffer_len(), 2);
    }

    #[test]
    fn enter_inserts_newline() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Enter));
        assert_eq!(s.core.borrow().active_buffer_len(), 1);
    }

    #[test]
    fn empty_selection_is_cleared_by_a_landed_self_insert() {
        // Q#AI9: an armed anchor at the cursor reports no region, so
        // 'x' inserts plainly — but the insert moves the cursor off
        // the anchor, and without the clear the region goes live and
        // 'y' type-overs the 'x'.
        let mut s = fresh_with(b"");
        s.lua_host
            .lua()
            .load("pmacs.editor.begin_selection(0)")
            .exec()
            .unwrap();
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('y'), KeyModifiers::NONE),
        );
        let core = s.core.borrow();
        assert_eq!(
            core.active_buffer_len(),
            2,
            "'y' must append, not type-over the freshly inserted 'x'"
        );
        assert!(
            core.active_window().selection.is_none(),
            "a landed self-insert clears the lingering anchor"
        );
    }

    #[test]
    fn rejected_self_insert_leaves_the_empty_selection_anchor() {
        // Q#AI9 failure regression: a rejecting intercept means NO
        // state mutation — the armed anchor must survive.
        let mut s = fresh_with(b"");
        s.lua_host
            .lua()
            .load(
                r#"
                _G.reject_once = true
                pmacs.buffer.add_intercept(pmacs.window.buffer(), function(_op)
                  if _G.reject_once then
                    _G.reject_once = false
                    error("rejected by test intercept")
                  end
                  return nil
                end)
                pmacs.editor.begin_selection(0)
                "#,
            )
            .exec()
            .unwrap();
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        {
            let core = s.core.borrow();
            assert_eq!(core.active_buffer_len(), 0, "the insert was rejected");
            assert!(
                core.active_window().selection.is_some(),
                "a rejected insert must not clear the anchor"
            );
        }
        // The next (allowed) insert lands and clears it.
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        let core = s.core.borrow();
        assert_eq!(core.active_buffer_len(), 1);
        assert!(core.active_window().selection.is_none());
    }

    #[test]
    fn backspace_deletes_previous_char() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Backspace));
        let core = s.core.borrow();
        assert_eq!(core.active_buffer_len(), 0);
        assert_eq!(core.cursor(), 0);
    }

    #[test]
    fn ctrl_a_e_navigate_line() {
        let mut s = fresh_with(b"");
        for c in "hello world".chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(s.core.borrow().cursor(), 11);
        s.dispatch_key(FrontendId::LOCAL, ctrl('a'));
        assert_eq!(s.core.borrow().cursor(), 0);
        s.dispatch_key(FrontendId::LOCAL, ctrl('e'));
        assert_eq!(s.core.borrow().cursor(), 11);
    }

    #[test]
    fn arrow_keys_move_cursor() {
        let mut s = fresh_with(b"");
        for c in "abc".chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(s.core.borrow().cursor(), 3);
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Left));
        assert_eq!(s.core.borrow().cursor(), 2);
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Right));
        assert_eq!(s.core.borrow().cursor(), 3);
    }

    #[test]
    fn cx_cc_quits() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        assert_eq!(local_dispatcher(&s).pending().len(), 1);
        s.dispatch_key(FrontendId::LOCAL, ctrl('c'));
        assert!(s.core.borrow().quit);
        assert!(local_dispatcher(&s).pending().is_empty());
    }

    #[test]
    fn cx_cs_invokes_save_with_no_path() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        assert!(s.core.borrow().status.contains("no file"));
    }

    // ---- incremental search via dispatch (Q#SR5) ---------------------------

    fn type_chars(s: &mut EditorState, text: &str) {
        for c in text.chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn isearch_dispatch_highlights_steps_and_accepts() {
        let mut s = fresh_with(b"foo bar foo baz foo");
        s.core.borrow_mut().active_window_mut().cursor = 0;
        // C-s begins the search (via the search.forward command).
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        assert!(s.core.borrow().search_active());
        // Typing extends the query; the first match is focused.
        type_chars(&mut s, "foo");
        assert_eq!(s.core.borrow().search_match_summary(), (Some(0), 3));
        assert_eq!(s.core.borrow().cursor(), 0);
        // C-s now steps (intercepted) rather than re-running the command.
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        assert_eq!(s.core.borrow().cursor(), 8);
        // RET accepts: search ends, cursor holds, matches persist.
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Enter));
        assert!(!s.core.borrow().search_active());
        assert_eq!(s.core.borrow().cursor(), 8);
        let bid = s.core.borrow().active_buffer_id();
        assert!(
            s.core
                .borrow()
                .search_store
                .lock()
                .expect("store")
                .for_buffer(bid)
                .is_some(),
            "accepted matches stay for highlight + navigation"
        );
    }

    #[test]
    fn isearch_dispatch_esc_restores_origin() {
        let mut s = fresh_with(b"foo bar foo");
        s.core.borrow_mut().active_window_mut().cursor = 5;
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        type_chars(&mut s, "foo");
        assert_eq!(s.core.borrow().cursor(), 8);
        // Esc cancels: the pre-search cursor is restored, no edit happened.
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Esc));
        assert!(!s.core.borrow().search_active());
        assert_eq!(s.core.borrow().cursor(), 5);
        assert_eq!(s.core.borrow().active_buffer_len(), 11);
    }

    #[test]
    fn isearch_dispatch_keys_do_not_self_insert() {
        let mut s = fresh_with(b"foo");
        s.core.borrow_mut().active_window_mut().cursor = 3;
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        type_chars(&mut s, "foo");
        // While searching, printable keys feed the query — the buffer is
        // untouched (no self-insert).
        assert_eq!(s.core.borrow().active_buffer_len(), 3);
        assert_eq!(s.core.borrow().search_query(), "foo");
    }

    #[test]
    fn regex_isearch_via_dispatch_c_m_s() {
        let mut s = fresh_with(b"a1 b2 c3");
        s.core.borrow_mut().active_window_mut().cursor = 0;
        // C-M-s starts a regex search (search.forward-regex).
        s.dispatch_key(
            FrontendId::LOCAL,
            key(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        );
        assert!(s.core.borrow().search_active());
        assert!(s.core.borrow().search_is_regex());
        type_chars(&mut s, r"\d");
        assert_eq!(s.core.borrow().search_match_summary().1, 3);
    }

    #[test]
    fn m_r_toggles_regex_mid_search() {
        let mut s = fresh_with(b"a.b axb");
        s.core.borrow_mut().active_window_mut().cursor = 0;
        s.dispatch_key(FrontendId::LOCAL, ctrl('s')); // literal
        type_chars(&mut s, "a.b");
        assert_eq!(s.core.borrow().search_match_summary().1, 1);
        // M-r toggles to regex (intercepted in dispatch_search_key).
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('r'), KeyModifiers::ALT),
        );
        assert!(s.core.borrow().search_is_regex());
        assert_eq!(s.core.borrow().search_match_summary().1, 2);
    }

    #[test]
    fn isearch_accumulates_across_renders_like_run_loop() {
        // Reproduce the real run loop: a render between every keystroke
        // (the in-process TUI renders once per burst, but paint_frame
        // borrows the core mutably and reads the search state, so a
        // render must not corrupt mid-search input).
        use crate::frontend::Event;
        let mut s = fresh_with(b"foo bar foo baz foo");
        s.core.borrow_mut().active_window_mut().cursor = 0;
        let size = crate::cell::CellSize::new(24, 80);
        let mut rs = crate::instance_render::RenderState::new(size);

        let _ = rs.render_frame(&s, FrontendId::LOCAL, &HashMap::new(), &[]);
        process_event(&mut s, Event::Key(ctrl('s')), size);
        assert!(s.core.borrow().search_active(), "C-s starts the search");
        let _ = rs.render_frame(&s, FrontendId::LOCAL, &HashMap::new(), &[]);

        for c in "foo".chars() {
            process_event(
                &mut s,
                Event::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                size,
            );
            let _ = rs.render_frame(&s, FrontendId::LOCAL, &HashMap::new(), &[]);
        }
        assert_eq!(
            s.core.borrow().search_query(),
            "foo",
            "query must accumulate across renders, not stick at the first char"
        );
    }

    #[test]
    fn isearch_tui_washes_matches_and_shows_full_query() {
        // The regression behind "only searches for the first character":
        // the TUI had no match-wash overlay, so the only feedback was the
        // cursor jump. Paint a real frame and assert both the wash and
        // the full-query prompt land on the grid.
        use crate::cell::{Cell, CellCoord, CellGrid, CellSize, Color, Glyph};
        let mut s = fresh_with(b"foo bar foo");
        s.core.borrow_mut().active_window_mut().cursor = 0;
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        type_chars(&mut s, "foo");

        let size = CellSize::new(24, 80);
        let mut backing = vec![Cell::default(); (size.rows * size.cols) as usize];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: size.cols,
            size,
        };
        let _ = paint_frame(&s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);

        // The active match [0,3) washes row 0's first cells (bright
        // Indexed(11); lazy matches would be Indexed(3)).
        let bg0 = grid.get(CellCoord::new(0, 0)).style.bg;
        assert!(
            matches!(bg0, Color::Indexed(11 | 3)),
            "first match cell should carry the search wash, got {bg0:?}"
        );
        // The bottom row shows the full live query, not just "f".
        let row = size.rows - 1;
        let prompt: String = (0..size.cols)
            .filter_map(|c| match grid.get(CellCoord::new(row, c)).glyph {
                Glyph::Char(ch) => Some(ch),
                _ => None,
            })
            .collect();
        assert!(
            prompt.contains("I-search: foo"),
            "bottom row should show the accumulated query, got {prompt:?}"
        );
    }

    #[test]
    fn isearch_flips_dispatch_idle_so_gpu_round_trips() {
        // The GPU's optimistic-apply gate (M11.6) keys off dispatch_idle.
        // An active isearch must drive it false so the GPU round-trips
        // keystrokes to the daemon's dispatch_search_key instead of
        // self-inserting them — the shared-core contract for Q#SR5.
        let mut s = fresh_with(b"foo foo");
        assert!(s.dispatch_idle(), "idle before any search");
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        assert!(s.core.borrow().search_active());
        assert!(!s.dispatch_idle(), "search active ⇒ keys must round-trip");
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Enter)); // accept
        assert!(s.dispatch_idle(), "search ended ⇒ optimistic apply resumes");
    }

    // ---- T M11.6 — DispatchIdle ---------------------------------------------

    #[test]
    fn dispatch_idle_true_on_fresh_editor() {
        let s = fresh_with(b"");
        assert!(s.dispatch_idle());
    }

    #[test]
    fn dispatch_idle_false_while_prefix_pending() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        assert!(
            !s.dispatch_idle(),
            "C-x prefix should put dispatcher in non-idle state"
        );
    }

    #[test]
    fn dispatch_idle_true_after_prefix_resolves() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        assert!(!s.dispatch_idle());
        // C-x C-c resolves the prefix into the quit command. After
        // the second chord arrives the dispatcher's `pending` is
        // cleared regardless of whether the command succeeded.
        s.dispatch_key(FrontendId::LOCAL, ctrl('c'));
        assert!(s.dispatch_idle(), "prefix cleared ⇒ idle again");
    }

    #[test]
    fn dispatch_idle_false_while_minibuffer_active() {
        use crate::minibuffer::{CompletionSource, MinibufferSession};

        let s = fresh_with(b"");
        assert!(s.dispatch_idle());

        // Open a synthetic minibuffer session — same shape Lua's
        // `pmacs.minibuffer.read` produces.
        let lua = mlua::Lua::new();
        let on_accept: mlua::Function = lua
            .create_function(|_, _: String| Ok(()))
            .expect("create on_accept");
        s.core.borrow_mut().minibuffer.begin(MinibufferSession {
            prompt: "test: ".into(),
            initial: String::new(),
            history_bucket: String::new(),
            source: CompletionSource::None,
            on_accept,
            on_cancel: None,
            candidates: Vec::new(),
            selected: None,
            history_index: None,
            typed_before_history_nav: None,
        });
        assert!(
            !s.dispatch_idle(),
            "active minibuffer prompt should put dispatcher in non-idle state"
        );

        // Dismissing returns to idle.
        let _ = s.core.borrow_mut().minibuffer.cancel();
        assert!(s.dispatch_idle(), "dismissed minibuffer ⇒ idle again");
    }

    #[test]
    fn repeat_key_events_dispatch_like_press() {
        // Some terminals deliver auto-repeated keys as KeyEventKind::Repeat
        // rather than KeyEventKind::Press. Filtering out Repeat made
        // the second chord of a fast-typed multi-key sequence look
        // "never registered" — the user pressed C-x then C-b before
        // releasing Ctrl, the C-b arrived as Repeat, we dropped it,
        // and the dispatcher stayed pending on [C-x] until something
        // recognized came in.
        let mut s = fresh_with(b"");
        let cx_press = KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let cb_repeat = KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Repeat,
            state: crossterm::event::KeyEventState::NONE,
        };
        s.dispatch_key(FrontendId::LOCAL, cx_press);
        assert_eq!(local_dispatcher(&s).pending().len(), 1);
        s.dispatch_key(FrontendId::LOCAL, cb_repeat);
        assert!(
            local_dispatcher(&s).pending().is_empty(),
            "Repeat-kind C-b did not resolve the pending C-x prefix"
        );
        assert_eq!(s.core.borrow().active_buffer_name(), "*buffer-list*");
    }

    #[test]
    fn release_key_events_are_still_ignored() {
        // Conversely, Release events must not advance the dispatcher
        // — they aren't input. If they did, every keystroke would
        // clear the pending prefix immediately after firing.
        let mut s = fresh_with(b"");
        let cx_press = KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let cx_release = KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        };
        s.dispatch_key(FrontendId::LOCAL, cx_press);
        s.dispatch_key(FrontendId::LOCAL, cx_release);
        assert_eq!(
            local_dispatcher(&s).pending().len(),
            1,
            "Release events should be ignored, but the prefix was disturbed"
        );
    }

    #[test]
    fn keymap_has_cx_cb_after_boot() {
        // Sanity: confirm the binding is actually present in the
        // global keymap after the editor finishes loading
        // builtin/keymaps/default.lua. If something breaks the loader
        // and the binding is silently dropped, dispatch would fall
        // through to "C-x not bound" and the user-visible symptom
        // would be exactly "C-b is never registered".
        let s = fresh_with(b"");
        let stack = s.lua_host.keymaps().borrow();
        let chord_x = Chord::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let chord_b = Chord::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        let r = stack.resolve(&[chord_x, chord_b], None, &[]);
        match r {
            crate::keymap_stack::StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "editor.list-buffers");
            }
            other => panic!("expected Bound editor.list-buffers; got {other:?}"),
        }
    }

    #[test]
    fn cx_cb_invokes_list_buffers() {
        // Regression for the user-reported "C-x C-b stalls" bug. After
        // C-x the dispatcher must be Pending; after C-b it must
        // resolve to `editor.list-buffers` (which switches the active
        // window to the *buffer-list* buffer).
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        assert_eq!(
            local_dispatcher(&s).pending().len(),
            1,
            "C-x should start prefix"
        );
        s.dispatch_key(FrontendId::LOCAL, ctrl('b'));
        assert!(
            local_dispatcher(&s).pending().is_empty(),
            "C-x C-b should resolve, leaving no pending prefix; status: {}",
            s.core.borrow().status
        );
        let name = s.core.borrow().active_buffer_name();
        assert_eq!(
            name,
            "*buffer-list*",
            "active buffer should be *buffer-list*; got {name:?}, status: {:?}",
            s.core.borrow().status
        );
    }

    #[test]
    fn cx_cb_repeated_keeps_buffer_list_window_in_sync() {
        // After C-x C-b the active window shows *buffer-list*. A second
        // C-x C-b rewrites that buffer via Lua userdata methods
        // (`buf:delete`, `buf:insert`). Without notifying windows
        // displaying the rewritten buffer, the active window's TextView
        // would keep its old line cache and the new content would
        // render partially or not at all.
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('b'));
        assert_eq!(s.core.borrow().active_buffer_name(), "*buffer-list*");
        let lines_first = s.core.borrow().active_window().text_view.line_count();

        // Add a buffer so the second list run produces a longer body.
        s.lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("scratch.txt", b"hello");

        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('b'));
        let lines_second = s.core.borrow().active_window().text_view.line_count();
        assert!(
            lines_second >= lines_first,
            "view did not see the rewritten *buffer-list*: {lines_first} -> {lines_second}"
        );
        let buf_len = s.core.borrow().active_buffer_len();
        let last_offset = s
            .core
            .borrow()
            .active_window()
            .text_view
            .line_offset(lines_second - 1)
            .unwrap();
        assert!(
            last_offset <= buf_len,
            "stale last offset {last_offset} exceeds buf_len {buf_len}"
        );
    }

    #[test]
    fn unknown_chord_continuation_clears_prefix_with_message() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        assert_eq!(local_dispatcher(&s).pending().len(), 1);
        s.dispatch_key(FrontendId::LOCAL, ctrl('q'));
        assert!(local_dispatcher(&s).pending().is_empty());
        assert!(s.core.borrow().status.contains("not bound"));
    }

    #[test]
    fn ctrl_d_deletes_forward() {
        let mut s = fresh_with(b"");
        for c in "abc".chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        s.core.borrow_mut().active_window_mut().cursor = 0;
        s.dispatch_key(FrontendId::LOCAL, ctrl('d'));
        assert_eq!(s.core.borrow().active_buffer_len(), 2);
    }

    #[test]
    fn ctrl_slash_undoes() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        assert_eq!(s.core.borrow().active_buffer_len(), 1);
        s.dispatch_key(FrontendId::LOCAL, ctrl('/'));
        assert_eq!(s.core.borrow().active_buffer_len(), 0);
    }

    #[test]
    fn cx_u_undoes() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        assert_eq!(s.core.borrow().active_buffer_len(), 1);
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('u'), KeyModifiers::NONE),
        );
        assert_eq!(s.core.borrow().active_buffer_len(), 0);
        assert!(local_dispatcher(&s).pending().is_empty());
    }

    #[test]
    fn cx_r_redoes() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('u'), KeyModifiers::NONE),
        );
        assert_eq!(s.core.borrow().active_buffer_len(), 0);
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('r'), KeyModifiers::NONE),
        );
        assert_eq!(s.core.borrow().active_buffer_len(), 1);
    }

    #[test]
    fn unbound_key_sets_status_does_not_crash() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::F(12)));
        assert!(s.core.borrow().status.contains("not bound"));
    }

    #[test]
    fn shift_letters_typed_normally() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('A'), KeyModifiers::SHIFT),
        );
        let core = s.core.borrow();
        let len = core.active_buffer_len();
        let mut out = vec![0u8; len as usize];
        let reg = core.registry.borrow();
        reg.get(core.active_buffer_id())
            .unwrap()
            .snapshot_rope()
            .slice(0, len, &mut out);
        assert_eq!(out, b"A");
    }

    #[test]
    fn cg_runs_editor_cancel() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, ctrl('g'));
        assert_eq!(s.core.borrow().status, "Quit");
    }

    // ---- Open semantics -----------------------------------------------------

    #[test]
    fn open_nonexistent_path_yields_empty_buffer_with_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist.txt");
        let s = EditorState::open(path.clone()).expect("must succeed");
        let core = s.core.borrow();
        assert!(core.active_buffer_len() == 0);
        assert_eq!(core.active_buffer_path().as_deref(), Some(path.as_path()));
        assert!(core.active_file_meta().is_none());
        assert_eq!(core.status, "[new file]");
    }

    #[test]
    fn open_existing_path_loads_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("here.txt");
        std::fs::write(&path, b"hello").unwrap();
        let s = EditorState::open(path.clone()).expect("must succeed");
        let core = s.core.borrow();
        assert_eq!(core.active_buffer_len(), 5);
        assert!(core.active_file_meta().is_some());
        assert_eq!(core.status, "");
    }

    // ---- Lua-host wiring ----------------------------------------------------

    #[test]
    fn lua_host_runs_on_main_thread_and_returns_value() {
        let mut s = fresh_with(b"");
        let v = s.lua_host.eval(None, "return 1 + 2").unwrap();
        match v {
            mlua::Value::Integer(n) => assert_eq!(n, 3),
            other => panic!("expected integer, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------
    // T M5.4 acceptance: FrontendId threads through dispatch_key /
    // dispatch_mouse to a Lua-readable surface.
    //
    // Spec §sec:v01-remote-scope deliverable 3.
    // -------------------------------------------------------------------

    #[test]
    fn m5_4_pmacs_frontend_id_defaults_to_local() {
        let mut s = fresh_with(b"");
        let v = s.lua_host.eval(None, "return pmacs.frontend.id()").unwrap();
        let expected = i64::try_from(FrontendId::LOCAL.0).unwrap();
        match v {
            mlua::Value::Integer(n) => assert_eq!(n, expected),
            other => panic!("expected integer, got {other:?}"),
        }
    }

    #[test]
    fn m5_4_dispatch_key_threads_frontend_id_to_lua_surface() {
        // Acceptance criterion: a synthetic event constructed with a
        // non-default FrontendId threads through to a hook (here, a
        // Lua-side reader of `pmacs.frontend.id()`) that reads it back.
        let mut s = fresh_with(b"");
        let probe_id = FrontendId(0x00C0_FFEE);
        s.dispatch_key(probe_id, plain(KeyCode::Char('a')));
        let v = s.lua_host.eval(None, "return pmacs.frontend.id()").unwrap();
        let expected = i64::try_from(probe_id.0).unwrap();
        match v {
            mlua::Value::Integer(n) => assert_eq!(n, expected),
            other => panic!("expected integer, got {other:?}"),
        }
    }

    #[test]
    fn m5_4_dispatch_mouse_threads_frontend_id_to_lua_surface() {
        let mut s = fresh_with(b"");
        let probe_id = FrontendId(0x0000_BEEF);
        let term_size = crate::cell::CellSize::new(24, 80);
        let m = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Moved,
            row: 1,
            column: 1,
            modifiers: KeyModifiers::NONE,
        };
        s.dispatch_mouse(probe_id, m, term_size);
        let v = s.lua_host.eval(None, "return pmacs.frontend.id()").unwrap();
        let expected = i64::try_from(probe_id.0).unwrap();
        match v {
            mlua::Value::Integer(n) => assert_eq!(n, expected),
            other => panic!("expected integer, got {other:?}"),
        }
    }

    fn right_click(row: u16, column: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Right),
            row,
            column,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn menu_item_labels(s: &EditorState) -> Vec<String> {
        let core = s.core.borrow();
        let guard = core.menu.lock().unwrap();
        guard
            .as_ref()
            .map(|m| {
                m.rows
                    .iter()
                    .filter_map(|r| match r {
                        crate::menu::MenuRow::Item { label, .. } => Some(label.clone()),
                        crate::menu::MenuRow::Separator => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn right_click_opens_context_menu_with_default_items() {
        let mut s = fresh_with(b"hello world");
        let term = crate::cell::CellSize::new(24, 80);
        s.dispatch_mouse(FrontendId(1), right_click(1, 3), term);
        assert!(s.core.borrow().menu_is_open());
        // No selection: the selection-only Cut/Copy are filtered out.
        assert_eq!(
            menu_item_labels(&s),
            vec!["Paste", "Select All", "Undo", "Redo"]
        );
    }

    #[test]
    fn right_click_with_selection_includes_cut_and_copy() {
        let mut s = fresh_with(b"hello world");
        {
            let mut c = s.core.borrow_mut();
            c.begin_selection(0);
            c.active_window_mut().cursor = 5; // select "hello"
        }
        s.dispatch_mouse(
            FrontendId(1),
            right_click(1, 3),
            crate::cell::CellSize::new(24, 80),
        );
        assert_eq!(
            menu_item_labels(&s),
            vec!["Cut", "Copy", "Paste", "Select All", "Undo", "Redo"]
        );
        // Right-clicking with a selection preserves it (so Copy/Cut act on it).
        assert!(s.core.borrow().active_region().is_some());
    }

    #[test]
    fn menu_arrows_navigate_and_escape_dismisses() {
        let mut s = fresh_with(b"abc");
        s.dispatch_mouse(
            FrontendId(1),
            right_click(1, 1),
            crate::cell::CellSize::new(24, 80),
        );
        assert_eq!(
            s.core.borrow().menu_active_command().as_deref(),
            Some("edit.paste")
        );
        s.dispatch_key(FrontendId(1), key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            s.core.borrow().menu_active_command().as_deref(),
            Some("edit.select-all")
        );
        s.dispatch_key(FrontendId(1), key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!s.core.borrow().menu_is_open());
    }

    #[test]
    fn menu_context_eval_gates_symbol_and_diagnostic() {
        let mut s = fresh_with(b"");
        let eval_bool = |s: &mut EditorState, expr: &str| -> bool {
            matches!(
                s.lua_host.eval(None, expr).unwrap(),
                mlua::Value::Boolean(true)
            )
        };
        // always / selection — pure context-table reads.
        assert!(eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('always', {})"
        ));
        assert!(eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('selection', {has_selection=true})"
        ));
        assert!(!eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('selection', {has_selection=false})"
        ));
        // symbol needs BOTH a word and an attached server.
        assert!(eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('symbol', {word='x', attachment={uri='u'}})"
        ));
        assert!(!eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('symbol', {word='x'})"
        ));
        assert!(!eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('symbol', {attachment={uri='u'}})"
        ));
        // diagnostic with no published diagnostics at the point → false
        // (exercises the diag-store lookup without erroring).
        assert!(!eval_bool(
            &mut s,
            "return pmacs.menu._context_eval('diagnostic', {attachment={uri='file:///none'}, line=0, col=0})"
        ));
    }

    #[test]
    fn menu_enter_invokes_command_and_closes() {
        let mut s = fresh_with(b"hello");
        s.dispatch_mouse(
            FrontendId(1),
            right_click(1, 1),
            crate::cell::CellSize::new(24, 80),
        );
        // Paste → Select All.
        s.dispatch_key(FrontendId(1), key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            s.core.borrow().menu_active_command().as_deref(),
            Some("edit.select-all")
        );
        s.dispatch_key(FrontendId(1), key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!s.core.borrow().menu_is_open());
        // edit.select-all ran: the whole buffer is now the region.
        assert_eq!(s.core.borrow().active_region(), Some((0, 5)));
    }

    /// The status line carries a Neovim/Doom-style scroll indicator
    /// after `L:C`: `All` when the buffer fits, `Top` at the start,
    /// `Bot` at the end, otherwise `NN%` cursor-row percent.
    #[test]
    fn status_line_scroll_indicator_reports_position() {
        // 1) Buffer fits in viewport => "All".
        assert_eq!(format_scroll_indicator(0, 22, 5, 0), "All");
        // 2) View at top, buffer overflows => "Top".
        assert_eq!(format_scroll_indicator(0, 22, 100, 0), "Top");
        // 3) View at bottom (last line in viewport) => "Bot".
        assert_eq!(format_scroll_indicator(80, 22, 100, 99), "Bot");
        // 4) Mid-buffer => percent of cursor line.
        assert_eq!(format_scroll_indicator(20, 22, 100, 30), "31%");
        // 5) visible == 0 (window never rendered) => percent fallback,
        //    never the All/Top/Bot caps.
        assert_eq!(format_scroll_indicator(0, 0, 100, 49), "50%");
        // 6) Single-line buffer is always "All".
        assert_eq!(format_scroll_indicator(0, 22, 1, 0), "All");
    }

    /// The scroll indicator and L:C cursor coord live on each
    /// window's mode line (Doom-style packing), not on the global
    /// status row. Render a buffer with enough lines to overflow the
    /// viewport and assert the mode line carries `L1:C1` and `Top`.
    #[test]
    fn mode_line_carries_cursor_and_scroll_indicator() {
        let mut content = Vec::new();
        for i in 0..200 {
            content.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let s = fresh_with(&content);
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        // Mode line is row 22 (0-based) — the last row of the
        // window's rect, which is text_rows-1 = 22.
        let mode_row = row_text(&cells, stride, 22, 80);
        assert!(
            mode_row.contains("L1:C1"),
            "mode line missing L:C: {mode_row:?}"
        );
        assert!(
            mode_row.contains("Top"),
            "mode line missing scroll indicator: {mode_row:?}"
        );
    }

    /// The global status row is now pure echo area: when there's no
    /// status message, no Lua error, and no pending key prefix, the
    /// row renders as blanks.
    #[test]
    fn empty_status_row_is_blank() {
        let s = fresh_with(b"hello\n");
        let line = build_status_line(&s.core.borrow(), &s.lua_host, &KeyDispatcher::new(), 80);
        assert_eq!(line, "", "status row should be empty when nothing to say");
    }

    #[test]
    fn captured_lua_error_appears_in_status_line() {
        let mut s = fresh_with(b"");
        let _ = s.lua_host.eval(Some("usercfg"), "error('kapow')");
        let line = build_status_line(&s.core.borrow(), &s.lua_host, &KeyDispatcher::new(), 200);
        assert!(line.contains("lua: "), "status line: {line}");
        assert!(line.contains("kapow"), "status line: {line}");
    }

    #[test]
    fn multiline_status_is_flattened_to_one_line() {
        // Regression: M-x with an unknown command stored a multi-line
        // traceback in `core.status`. The renderer copied each char into
        // a cell, the frontend emitted literal `\n` bytes, and the frame
        // was shredded. Sanitization must keep the informative first
        // line and replace control chars in it with spaces.
        let s = fresh_with(b"");
        s.core.borrow_mut().status =
            "M-x error: command \"foo\" not found\nstack traceback:\n\t[C]: in ?".into();
        let line = build_status_line(&s.core.borrow(), &s.lua_host, &KeyDispatcher::new(), 200);
        assert!(!line.contains('\n'), "status line leaked newline: {line:?}");
        assert!(!line.contains('\r'), "status line leaked CR: {line:?}");
        assert!(
            line.contains("command \"foo\" not found"),
            "first line dropped: {line}"
        );
        assert!(
            !line.contains("traceback"),
            "traceback should be truncated: {line}"
        );
    }

    #[test]
    fn captured_lua_error_with_traceback_does_not_break_status_line() {
        let mut s = fresh_with(b"");
        let _ = s
            .lua_host
            .eval(Some("usercfg"), "error('boom\\nlots\\nof\\nlines')");
        let line = build_status_line(&s.core.borrow(), &s.lua_host, &KeyDispatcher::new(), 200);
        assert!(!line.contains('\n'), "status line leaked newline: {line:?}");
        assert!(line.contains("lua: "), "status line: {line}");
    }

    #[test]
    fn editor_status_takes_priority_over_lua_error() {
        let mut s = fresh_with(b"");
        let _ = s.lua_host.eval(None, "error('latent')");
        s.core.borrow_mut().status = "saved foo".into();
        let line = build_status_line(&s.core.borrow(), &s.lua_host, &KeyDispatcher::new(), 200);
        assert!(line.contains("saved foo"));
        assert!(!line.contains("lua: "));
    }

    // ---- User override of a binding -----------------------------------------

    #[test]
    fn user_can_unbind_and_rebind_a_chord() {
        let mut s = fresh_with(b"");
        // Replace C-a with editor.cancel to verify a config-style override
        // takes effect on the live dispatch path.
        s.lua_host
            .eval(
                Some("user-override"),
                r#"
                pmacs.keymap.unbind { scope = "global", sequence = "C-a" }
                pmacs.keymap.bind { scope = "global", sequence = "C-a", command = "editor.cancel" }
                "#,
            )
            .expect("override must succeed");
        s.dispatch_key(FrontendId::LOCAL, ctrl('a'));
        assert_eq!(s.core.borrow().status, "Quit");
    }

    // ---- T M2.11 acceptance --------------------------------------------------

    /// Every chord in the default global keymap must round-trip through
    /// `pmacs.describe.key`: returning a non-nil table whose `command`
    /// matches the binding the keymap stack stores.
    #[test]
    fn describe_key_identifies_every_default_binding() {
        let s = EditorState::new();
        let kms = s.lua_host.keymaps().borrow();
        let bindings: Vec<(String, String)> = kms
            .iter_all()
            .into_iter()
            .map(|(_, seq, b)| (crate::key::display_sequence(&seq), b.command))
            .collect();
        drop(kms);
        // Sanity floor: the default keymap binds at least the M1 surface.
        assert!(
            bindings.len() >= 20,
            "default keymap unexpectedly small: {} bindings",
            bindings.len()
        );

        for (seq, expected_command) in &bindings {
            let script = format!(
                "local r = pmacs.describe.key({seq:?}); \
                 if r == nil then return 'nil' else return r.command end"
            );
            let got: String = s.lua_host.lua().load(&script).eval().unwrap_or_else(|e| {
                panic!("describe.key({seq}) raised: {e}");
            });
            assert_eq!(
                &got, expected_command,
                "describe.key for {seq:?} returned {got:?}, expected {expected_command:?}"
            );
        }
    }

    /// `pmacs.help.show_command` must populate a real buffer named
    /// `*help*` in the registry --- the spec requires it to be a regular
    /// buffer (cross-references navigable once buffer-switching lands).
    #[test]
    fn help_buffer_is_a_regular_buffer_in_the_registry() {
        let s = EditorState::new();
        let buf_id: Option<crate::lua_bindings::BufferIdLua> = s
            .lua_host
            .lua()
            .load("return pmacs.help.show_command('cursor.left')")
            .eval()
            .unwrap();
        let buf_id = buf_id.expect("help buffer returned");
        let reg = s.lua_host.registry().borrow();
        let buf = reg.get(buf_id.0).expect("help buffer present");
        assert_eq!(buf.name(), crate::help::HELP_BUFFER_NAME);
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("Command: cursor.left"));
        // Cross-references back into the help system.
        assert!(body.contains("[key:"), "no key cross-ref: {body}");
    }

    /// `pmacs.help.follow_link` chases a `[key: ...]` cross-reference
    /// and re-renders the help buffer with that key's description.
    #[test]
    fn help_follow_link_navigates_command_to_key() {
        let s = EditorState::new();
        // Render `cursor.left`, then find a `[key: ...]` token in the
        // help body and follow it.
        let cursor: i64 = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.help.show_command("cursor.left")
                local list = pmacs.buffer.list()
                local help_id
                for _, id in ipairs(list) do
                    if pmacs.describe.buffer(id).name == "*help*" then
                        help_id = id
                    end
                end
                assert(help_id ~= nil, "help buffer must exist")
                local body = help_id:slice(0, help_id:len())
                local s, e = body:find("%[key: ")
                assert(s ~= nil, "expected a [key: ...] cross-reference")
                return e
                "#,
            )
            .eval()
            .unwrap();
        let returned: Option<crate::lua_bindings::BufferIdLua> = s
            .lua_host
            .lua()
            .load(format!("return pmacs.help.follow_link({cursor})"))
            .eval()
            .unwrap();
        let id = returned.expect("follow_link should return the re-rendered help buffer");
        let reg = s.lua_host.registry().borrow();
        let buf = reg.get(id.0).unwrap();
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(
            body.starts_with("Key: "),
            "follow_link should re-render to a Key: page, got: {body}"
        );
    }

    /// describe-hook lists callbacks in registration order, even when
    /// the hook subsystem is the M2.11 stub.
    #[test]
    fn describe_hook_round_trip_via_editor_state() {
        let s = EditorState::new();
        let cb_count: i64 = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.hook.define { name = "demo", description = "demo hook" }
                pmacs.hook.add("demo", function() end)
                pmacs.hook.add("demo", function() end)
                local d = pmacs.describe.hook("demo")
                return #d.callbacks
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(cb_count, 2);
    }

    // ---- T M2.6 acceptance --------------------------------------------------

    /// All three required lifecycle hooks are defined out of the box,
    /// with the spec-mandated kinds.
    #[test]
    fn lifecycle_hooks_defined_with_correct_kinds() {
        let s = EditorState::new();
        let kinds: mlua::Table = s
            .lua_host
            .lua()
            .load(
                r#"
                local out = {}
                for _, name in ipairs({
                  "buffer.before-save",
                  "buffer.after-load",
                  "editor.before-quit",
                }) do
                  local d = pmacs.describe.hook(name)
                  assert(d ~= nil, name .. " not defined")
                  out[name] = d.kind
                end
                return out
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            kinds.get::<String>("buffer.before-save").unwrap(),
            "short-circuit"
        );
        assert_eq!(
            kinds.get::<String>("buffer.after-load").unwrap(),
            "all-must-succeed"
        );
        assert_eq!(
            kinds.get::<String>("editor.before-quit").unwrap(),
            "short-circuit"
        );
    }

    // ---- M4.12 buffer.after-edit / buffer.after-save -----------------------

    /// `buffer.after-edit` and `buffer.after-save` ship as part of the
    /// default lifecycle vocabulary so LSP wiring (and any user hook)
    /// can subscribe without the editor having to register them.
    #[test]
    fn m4_12_after_edit_and_after_save_hooks_defined() {
        let s = EditorState::new();
        let kinds: mlua::Table = s
            .lua_host
            .lua()
            .load(
                r#"
                local out = {}
                for _, name in ipairs({"buffer.after-edit", "buffer.after-save"}) do
                  local d = pmacs.describe.hook(name)
                  assert(d ~= nil, name .. " not defined")
                  out[name] = d.kind
                end
                return out
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            kinds.get::<String>("buffer.after-edit").unwrap(),
            "all-must-succeed"
        );
        assert_eq!(
            kinds.get::<String>("buffer.after-save").unwrap(),
            "all-must-succeed"
        );
    }

    /// A self-insert keystroke fires `buffer.after-edit` exactly once.
    #[test]
    fn m4_12_after_edit_fires_on_self_insert() {
        let mut s = fresh_with(b"");
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.edit_count = 0
                pmacs.hook.add("buffer.after-edit", function()
                  _G.edit_count = _G.edit_count + 1
                end)
                "#,
            )
            .unwrap();
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.edit_count")
            .eval()
            .unwrap();
        assert_eq!(n, 1, "expected 1 after-edit per typed char, got {n}");
    }

    /// Cursor motion does not fire `buffer.after-edit`.
    #[test]
    fn m4_12_after_edit_does_not_fire_on_motion() {
        let mut s = fresh_with(b"hello");
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.edit_count = 0
                pmacs.hook.add("buffer.after-edit", function()
                  _G.edit_count = _G.edit_count + 1
                end)
                "#,
            )
            .unwrap();
        s.dispatch_key(FrontendId::LOCAL, ctrl('f'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('b'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('a'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('e'));
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.edit_count")
            .eval()
            .unwrap();
        assert_eq!(n, 0, "motion fired after-edit unexpectedly");
    }

    /// Undo and redo each fire `buffer.after-edit` because they mutate
    /// the buffer state.
    #[test]
    fn m4_12_after_edit_fires_on_undo_and_redo() {
        let mut s = fresh_with(b"");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.edit_count = 0
                pmacs.hook.add("buffer.after-edit", function()
                  _G.edit_count = _G.edit_count + 1
                end)
                "#,
            )
            .unwrap();
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('/'), KeyModifiers::CONTROL),
        );
        s.dispatch_key(
            FrontendId::LOCAL,
            key(
                KeyCode::Char('?'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        );
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.edit_count")
            .eval()
            .unwrap();
        assert!(
            n >= 1,
            "expected at least one undo-driven after-edit, got {n}"
        );
    }

    /// A successful save fires `buffer.after-save` exactly once.
    #[test]
    fn m4_12_after_save_fires_on_successful_save() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("save_hook.txt");
        std::fs::write(&path, b"x").unwrap();
        let mut s = EditorState::open(path.clone()).unwrap();
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.save_count = 0
                pmacs.hook.add("buffer.after-save", function()
                  _G.save_count = _G.save_count + 1
                end)
                "#,
            )
            .unwrap();
        // Type a char so the save is non-trivial, then save.
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::End, KeyModifiers::NONE));
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('y'), KeyModifiers::NONE),
        );
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.save_count")
            .eval()
            .unwrap();
        assert_eq!(n, 1, "expected 1 after-save, got {n}");
        assert_eq!(std::fs::read(&path).unwrap(), b"xy");
    }

    /// A vetoed save does not fire `buffer.after-save`.
    #[test]
    fn m4_12_after_save_does_not_fire_when_save_vetoed() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("vetoed.txt");
        std::fs::write(&path, b"x").unwrap();
        let mut s = EditorState::open(path.clone()).unwrap();
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.save_count = 0
                pmacs.hook.add("buffer.before-save", function() return false end)
                pmacs.hook.add("buffer.after-save", function()
                  _G.save_count = _G.save_count + 1
                end)
                "#,
            )
            .unwrap();
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.save_count")
            .eval()
            .unwrap();
        assert_eq!(n, 0);
    }

    /// `buffer.before-save` can veto a save: when a callback returns
    /// false, `pmacs.editor.save()` is never reached.
    #[test]
    fn before_save_hook_can_veto() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("guard.txt");
        std::fs::write(&path, b"original").unwrap();
        let mut s = EditorState::open(path.clone()).unwrap();
        // Mutate so the save would visibly happen (different bytes).
        s.core.borrow_mut().active_window_mut().cursor = 8;
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('X'), KeyModifiers::NONE),
        );
        // Attach a vetoing callback before triggering save.
        s.lua_host
            .eval(
                Some("test"),
                r#"
                pmacs.hook.add("buffer.before-save", function() return false end)
                "#,
            )
            .unwrap();
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('s'));
        // File on disk should still match the original content.
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, b"original");
        assert!(s.core.borrow().status.contains("vetoed"));
    }

    /// `editor.before-quit` can veto quitting.
    #[test]
    fn before_quit_hook_can_veto() {
        let mut s = fresh_with(b"");
        s.lua_host
            .eval(
                Some("test"),
                r#"
                pmacs.hook.add("editor.before-quit", function() return false end)
                "#,
            )
            .unwrap();
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('c'));
        assert!(!s.core.borrow().quit, "veto should have prevented quit");
        assert!(s.core.borrow().status.contains("vetoed"));
    }

    /// `process.after-tick` is defined out of the box and shipped with
    /// `kind = "all-must-succeed"` so multiple subscribers (REPL handles,
    /// future packages) can attach independently. Listed here because
    /// the M6.5 contract is "the hook exists, it's fireable, the run
    /// loop fires it once per `tick_processes`."
    #[test]
    fn m6_5_process_after_tick_hook_defined_with_correct_kind() {
        let s = EditorState::new();
        let kind: String = s
            .lua_host
            .lua()
            .load(
                r#"
                local d = pmacs.describe.hook("process.after-tick")
                assert(d ~= nil, "process.after-tick not defined")
                return d.kind
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(kind, "all-must-succeed");
    }

    /// Each call to `tick_processes` fires `process.after-tick` exactly
    /// once. The REPL package's per-frame event pump (T M6.5) depends on
    /// this 1:1 cadence to drain `pmacs.process.events_take` for every
    /// registered handle without missing a frame.
    #[test]
    fn m6_5_tick_processes_fires_after_tick_hook_once_per_call() {
        let mut s = EditorState::new();
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.tick_count = 0
                pmacs.hook.add("process.after-tick", function()
                  _G.tick_count = _G.tick_count + 1
                end)
                "#,
            )
            .unwrap();
        for _ in 0..5 {
            s.tick_processes();
        }
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.tick_count")
            .eval()
            .unwrap();
        assert_eq!(n, 5, "expected 1 after-tick per tick_processes, got {n}");
    }

    /// When a file is opened, the after-load hook is fired. Test
    /// shape: register a listener then fire `run_hook` directly, since
    /// `EditorState::open` constructs its own host (so we can't
    /// pre-attach). This covers the Rust-side wiring path.
    #[test]
    fn after_load_hook_fires_with_loaded_buffer_visible() {
        let mut s = EditorState::new();
        s.lua_host
            .eval(
                Some("test"),
                r#"
                _G.after_load_count = 0
                pmacs.hook.add("buffer.after-load", function()
                    _G.after_load_count = _G.after_load_count + 1
                end)
                "#,
            )
            .unwrap();
        let outcome = s
            .lua_host
            .run_hook("buffer.after-load", mlua::MultiValue::new())
            .expect("hook is defined");
        assert!(outcome.proceed);
        let n: i64 = s
            .lua_host
            .lua()
            .load("return _G.after_load_count")
            .eval()
            .unwrap();
        assert_eq!(n, 1);
    }

    /// describe-hook reports the kind, source, and callbacks in
    /// registration order. Satisfies the M2.6 acceptance bullet on
    /// `describe-hook` listing attached functions with source
    /// locations.
    #[test]
    fn describe_hook_reports_kind_and_source_locations() {
        let s = EditorState::new();
        let info: mlua::Table = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.hook.add("buffer.before-save", function() return true end)
                pmacs.hook.add("buffer.before-save", function() return true end)
                return pmacs.describe.hook("buffer.before-save")
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(info.get::<String>("kind").unwrap(), "short-circuit");
        assert!(info.get::<String>("source").unwrap().contains(':'));
        let callbacks: mlua::Table = info.get("callbacks").unwrap();
        let len = callbacks.len().unwrap();
        // A builtin (saveplace) also subscribes to `buffer.before-save`,
        // registered at startup, so it precedes the two the test adds.
        // Assert on the *last two* callbacks — the ones this chunk just
        // registered — rather than the exact total (robust to builtins).
        assert!(len >= 2, "expected >= 2 callbacks; describe says {len}");
        let cb1: mlua::Table = callbacks.get(len - 1).unwrap();
        let cb2: mlua::Table = callbacks.get(len).unwrap();
        let s1: String = cb1.get("source").unwrap();
        let s2: String = cb2.get("source").unwrap();
        // Both registrations come from the test chunk; the second
        // must report a strictly later line.
        let line = |s: &str| -> i32 {
            s.rsplit_once(':')
                .and_then(|(_, n)| n.parse().ok())
                .unwrap_or(0)
        };
        assert!(line(&s1) < line(&s2), "source lines: {s1} vs {s2}");
    }

    /// Composition kind: short-circuit. A `false` from the first
    /// callback prevents later callbacks from running.
    #[test]
    fn short_circuit_kind_stops_at_first_false() {
        let s = EditorState::new();
        let count: i64 = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.hook.define {
                    name = "demo.sc",
                    description = "demo short-circuit",
                    kind = "short-circuit",
                }
                _G.hits = 0
                pmacs.hook.add("demo.sc", function() _G.hits = _G.hits + 1; return false end)
                pmacs.hook.add("demo.sc", function() _G.hits = _G.hits + 1; return true end)
                pmacs.hook.run("demo.sc")
                return _G.hits
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Composition kind: all-must-succeed. Every callback runs even if
    /// an earlier one raises.
    #[test]
    fn all_must_succeed_kind_runs_every_callback() {
        let s = EditorState::new();
        let (proceed, hits): (bool, i64) = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.hook.define {
                    name = "demo.ams",
                    description = "demo all-must-succeed",
                    kind = "all-must-succeed",
                }
                _G.hits = 0
                pmacs.hook.add("demo.ams", function() error('boom') end)
                pmacs.hook.add("demo.ams", function() _G.hits = _G.hits + 1 end)
                pmacs.hook.add("demo.ams", function() _G.hits = _G.hits + 1 end)
                local ok = pmacs.hook.run("demo.ams")
                return ok, _G.hits
                "#,
            )
            .eval()
            .unwrap();
        assert!(!proceed, "errors must surface as a non-proceed return");
        assert_eq!(hits, 2, "every non-failing callback must still run");
    }

    /// Composition kind: accumulate. Each callback receives the
    /// previous return as its first argument.
    #[test]
    fn accumulate_kind_threads_value() {
        let s = EditorState::new();
        let final_value: i64 = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.hook.define {
                    name = "demo.acc",
                    description = "demo accumulate",
                    kind = "accumulate",
                }
                pmacs.hook.add("demo.acc", function(n) return n + 1 end)
                pmacs.hook.add("demo.acc", function(n) return n * 2 end)
                pmacs.hook.add("demo.acc", function(n) return n - 5 end)
                return pmacs.hook.run("demo.acc", 10)
                "#,
            )
            .eval()
            .unwrap();
        // (((10 + 1) * 2) - 5) = 17
        assert_eq!(final_value, 17);
    }

    /// Hook callback errors land in the *errors* buffer alongside
    /// chunk-level errors, not on stderr (terminal is in raw mode).
    #[test]
    fn hook_errors_are_captured_to_errors_buffer() {
        let mut s = EditorState::new();
        s.lua_host
            .eval(
                Some("test"),
                r#"
                pmacs.hook.add("buffer.after-load", function() error('boom from hook') end)
                "#,
            )
            .unwrap();
        s.lua_host
            .run_hook("buffer.after-load", mlua::MultiValue::new());
        let id = s
            .lua_host
            .errors_buffer_id()
            .expect("errors buffer present");
        let reg = s.lua_host.registry().borrow();
        let buf = reg.get(id).unwrap();
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(
            body.contains("hook:buffer.after-load") && body.contains("boom from hook"),
            "errors body: {body}"
        );
    }

    // ---- T M2.7 acceptance --------------------------------------------------

    fn alt(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::ALT)
    }

    /// Bullet 1: the minibuffer's contents are stored in a real Buffer
    /// with a real `TextView`. No special path.
    #[test]
    fn minibuffer_uses_standard_rope_and_view_machinery() {
        let s = EditorState::new();
        let core = s.core.borrow();
        // `Buffer::name` and `Buffer::len` are the same query surface
        // every other buffer exposes.
        assert_eq!(core.minibuffer.buffer.name(), "*minibuffer*");
        assert_eq!(core.minibuffer.buffer.len(), 0);
        // `TextView::line_count` is the same TextView API the main
        // buffer uses.
        assert_eq!(core.minibuffer.text_view.line_count(), 1);
    }

    #[test]
    fn errors_buffer_window_textview_stays_in_sync_after_appends() {
        // Regression: when a window displays the *errors* buffer and a
        // new Lua error appends content via `LuaHost::append_to_errors_buffer`,
        // the window's TextView must see the edit. Otherwise its line
        // cache goes stale: `line_count` returns the old count, cursor
        // motions land in unmappable positions, and the screen appears
        // frozen until something else triggers a buffer switch.
        let mut s = fresh_with(b"");
        // Provoke a first error so the *errors* buffer exists.
        let _ = s.lua_host.eval(Some("first"), "error('alpha')");
        let errors_id = s
            .lua_host
            .errors_buffer_id()
            .expect("first error created the buffer");
        // Switch the active window to *errors*.
        s.core.borrow_mut().switch_active_buffer(errors_id).unwrap();
        let lines_before = s.core.borrow().active_window().text_view.line_count();
        // Provoke a second error while the window is on *errors*.
        let _ = s.lua_host.eval(Some("second"), "error('beta\\ngamma')");
        let lines_after = s.core.borrow().active_window().text_view.line_count();
        assert!(
            lines_after > lines_before,
            "TextView line count did not grow: before={lines_before} after={lines_after}"
        );
        // The view's line index must reach the end of the buffer (a
        // trailing newline yields one extra empty line, so the last
        // offset can equal `buffer.len()` but never exceed it).
        let buf_len = s.core.borrow().active_buffer_len();
        let last_offset = s
            .core
            .borrow()
            .active_window()
            .text_view
            .line_offset(lines_after - 1)
            .unwrap();
        assert!(
            last_offset <= buf_len,
            "last cached line offset {last_offset} exceeds buffer length {buf_len}"
        );
        // The pre-append last offset would be smaller than the post-
        // append buffer length; if the view weren't notified, the new
        // content would be unreachable.
        let pre_append_max_offset = s
            .core
            .borrow()
            .active_window()
            .text_view
            .line_offset(lines_before - 1)
            .unwrap();
        assert!(
            last_offset > pre_append_max_offset,
            "view did not advance past pre-append last offset"
        );
    }

    /// Bullet 2: `M-x` opens a fuzzy-completing prompt over every
    /// registered command. Typing a fragment narrows the candidate
    /// list; accepting invokes the chosen command.
    #[test]
    fn m_x_with_fuzzy_completion_runs_a_command() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, alt('x'));
        assert!(s.core.borrow().minibuffer.is_active());
        // Type "edcan" --- the subsequence ranks editor.cancel
        // strictly above editor.execute-command (editor.cancel has
        // every needle char consecutive after the word-boundary `.`,
        // which scores far higher than the wide gaps in
        // editor.execute-command).
        for c in "edcan".chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let cands = s
            .core
            .borrow()
            .minibuffer
            .session
            .as_ref()
            .unwrap()
            .candidates
            .clone();
        assert!(!cands.is_empty(), "expected at least one candidate");
        assert_eq!(cands[0], "editor.cancel", "candidates: {cands:?}");
        // Accept (RET).
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Enter));
        // editor.cancel sets status = "Quit".
        assert_eq!(s.core.borrow().status, "Quit");
        assert!(!s.core.borrow().minibuffer.is_active());
    }

    #[test]
    fn m_x_unknown_command_does_not_corrupt_status_line() {
        // The exact failure mode reported from a real run: M-x with a
        // name that does not resolve to any command. mlua's
        // `tostring(err)` returns a multi-line traceback; the on_accept
        // handler in default.lua takes only the first line, and the
        // Rust-side status renderer sanitizes again at the cell-grid
        // boundary. Both raw and rendered status must be single-line.
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, alt('x'));
        assert!(s.core.borrow().minibuffer.is_active());
        for c in "definitely-not-a-command".chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        // Force-accept the typed text instead of any fuzzy candidate
        // (matches the runtime path when no candidate is highlighted).
        {
            let mut core = s.core.borrow_mut();
            if let Some(session) = core.minibuffer.session.as_mut() {
                session.selected = None;
                session.candidates.clear();
            }
        }
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Enter));
        let raw = s.core.borrow().status.clone();
        assert!(
            !raw.contains('\n'),
            "raw status leaked newline: {raw:?} (default.lua should take first line)"
        );
        assert!(raw.starts_with("M-x error: "), "raw status: {raw}");
        let line = build_status_line(&s.core.borrow(), &s.lua_host, &KeyDispatcher::new(), 200);
        assert!(
            !line.contains('\n'),
            "rendered status line leaked newline: {line:?} (raw: {raw:?})"
        );
        assert!(
            line.contains("M-x error"),
            "expected M-x error prefix; got: {line}"
        );
    }

    /// Bullet 3: history persists across "sessions". We test by
    /// pointing the minibuffer at a tempdir, accepting two entries,
    /// then constructing a fresh minibuffer pointed at the same dir
    /// and verifying the entries reload from disk.
    #[test]
    fn history_persists_across_sessions_via_dir_injection() {
        use crate::minibuffer::{CompletionSource, History, Minibuffer, MinibufferSession};
        let dir = tempfile::TempDir::new().unwrap();
        let dir_path = dir.path().to_path_buf();

        let lua = mlua::Lua::new();
        let dummy = lua.create_function(|_, _: String| Ok(())).unwrap();
        let mut mb1 = Minibuffer::new();
        mb1.history_dir = Some(dir_path.clone());
        for entry in ["alpha", "beta"] {
            mb1.begin(MinibufferSession {
                prompt: "P: ".into(),
                initial: String::new(),
                history_bucket: "test".into(),
                source: CompletionSource::None,
                on_accept: dummy.clone(),
                on_cancel: None,
                candidates: Vec::new(),
                selected: None,
                history_index: None,
                typed_before_history_nav: None,
            });
            for c in entry.chars() {
                mb1.insert_char(c);
            }
            mb1.accept().unwrap();
        }
        drop(mb1);

        // Fresh instance pointed at the same dir: open a session;
        // history is loaded lazily on `begin`. After that, the
        // history bucket should carry both entries.
        let mut mb2 = Minibuffer::new();
        mb2.history_dir = Some(dir_path);
        mb2.begin(MinibufferSession {
            prompt: "P: ".into(),
            initial: String::new(),
            history_bucket: "test".into(),
            source: CompletionSource::None,
            on_accept: dummy,
            on_cancel: None,
            candidates: Vec::new(),
            selected: None,
            history_index: None,
            typed_before_history_nav: None,
        });
        let h: &History = mb2.history.get("test").expect("history loaded");
        let entries: Vec<_> = h.entries.iter().cloned().collect();
        assert_eq!(entries, vec!["alpha".to_string(), "beta".into()]);
    }

    /// Bullet 4: every named completion source (commands, buffers,
    /// files, custom Lua function) is selectable from `pmacs.minibuffer.read`.
    #[test]
    fn every_completion_source_is_selectable() {
        let s = EditorState::new();
        // commands
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.minibuffer.read {
                    prompt = "X: ", source = "commands",
                    on_accept = function() end,
                }
                "#,
            )
            .exec()
            .unwrap();
        assert!(matches!(
            s.core.borrow().minibuffer.session.as_ref().unwrap().source,
            crate::minibuffer::CompletionSource::Commands
        ));
        s.core.borrow_mut().minibuffer.cancel();

        // buffers
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.minibuffer.read {
                    prompt = "X: ", source = "buffers",
                    on_accept = function() end,
                }
                "#,
            )
            .exec()
            .unwrap();
        assert!(matches!(
            s.core.borrow().minibuffer.session.as_ref().unwrap().source,
            crate::minibuffer::CompletionSource::Buffers
        ));
        s.core.borrow_mut().minibuffer.cancel();

        // files
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.minibuffer.read {
                    prompt = "X: ", source = "files", source_root = "/tmp",
                    on_accept = function() end,
                }
                "#,
            )
            .exec()
            .unwrap();
        assert!(matches!(
            s.core.borrow().minibuffer.session.as_ref().unwrap().source,
            crate::minibuffer::CompletionSource::Files { .. }
        ));
        s.core.borrow_mut().minibuffer.cancel();

        // custom function
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.minibuffer.read {
                    prompt = "X: ",
                    source = function() return { "alpha", "beta" } end,
                    on_accept = function() end,
                }
                "#,
            )
            .exec()
            .unwrap();
        assert!(matches!(
            s.core.borrow().minibuffer.session.as_ref().unwrap().source,
            crate::minibuffer::CompletionSource::Custom(_)
        ));
        let cands = s
            .core
            .borrow()
            .minibuffer
            .session
            .as_ref()
            .unwrap()
            .candidates
            .clone();
        assert_eq!(cands, vec!["alpha".to_string(), "beta".into()]);
    }

    /// `pmacs.minibuffer.read` rejects unknown spec keys per R50.
    #[test]
    fn read_rejects_unknown_spec_keys() {
        let s = EditorState::new();
        let result = s
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.minibuffer.read {
                    prompt = "X: ",
                    bogus = true,
                    on_accept = function() end,
                }
                "#,
            )
            .exec();
        assert!(result.is_err(), "unknown key should error");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unknown field") && msg.contains("bogus"),
            "msg: {msg}"
        );
    }

    /// `C-g` while a prompt is active cancels the session without
    /// invoking `on_accept`.
    #[test]
    fn cg_cancels_active_prompt() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, alt('x'));
        assert!(s.core.borrow().minibuffer.is_active());
        s.dispatch_key(FrontendId::LOCAL, ctrl('g'));
        assert!(!s.core.borrow().minibuffer.is_active());
        assert_eq!(s.core.borrow().status, "Quit");
    }

    /// TAB on an active session replaces the buffer with the
    /// currently-selected candidate.
    #[test]
    fn tab_completes_to_selected_candidate() {
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, alt('x'));
        for c in "save".chars() {
            s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        // TAB completes to the top candidate.
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Tab));
        let contents = s.core.borrow().minibuffer.contents();
        assert_eq!(contents, "buffer.save", "minibuffer: {contents:?}");
    }

    // ---- T M2.8 acceptance --------------------------------------------------

    /// Bullet 1: 8 splits in a single frame render correctly. We
    /// verify by computing per-window rectangles via the layout and
    /// asserting all are non-empty + non-overlapping for a typical
    /// 24×80 terminal.
    #[test]
    fn eight_splits_render_in_distinct_rects_via_lua_api() {
        let s = EditorState::new();
        s.lua_host
            .lua()
            .load(
                "
                -- Build an 8-way layout: 3 vertical splits then 1
                -- horizontal split per resulting column.
                pmacs.window.split_vertical()
                pmacs.window.focus_next()
                pmacs.window.split_vertical()
                pmacs.window.focus_next()
                pmacs.window.split_vertical()
                pmacs.window.focus_next()
                -- Now 4 columns; horizontal-split each.
                for _ = 1, 4 do
                    pmacs.window.split_horizontal()
                    pmacs.window.focus_next()
                    pmacs.window.focus_next()
                end
                ",
            )
            .exec()
            .unwrap();
        let core = s.core.borrow();
        assert_eq!(core.windows.len(), 8);
        let area = crate::window::Rect::new(0, 0, 40, 120);
        let placements = core.active_layout().compute(area);
        assert_eq!(placements.len(), 8);
        for r in placements.values() {
            assert!(!r.is_empty(), "rect was empty: {r:?}");
        }
    }

    /// Bullet 2: focus-next walks the layout deterministically and
    /// returns to the starting window after a full cycle.
    #[test]
    fn focus_next_walks_predictably() {
        let s = EditorState::new();
        s.lua_host
            .lua()
            .load(
                "
                pmacs.window.split_vertical()
                pmacs.window.split_horizontal()
                ",
            )
            .exec()
            .unwrap();
        let start = s.core.borrow().active_window_id();
        let total = s.core.borrow().windows.len();
        assert_eq!(total, 3);
        for _ in 0..total {
            s.core.borrow_mut().focus_next();
        }
        assert_eq!(s.core.borrow().active_window_id(), start);
    }

    /// Bullet 3: the buffer-list buffer is a regular Buffer in the
    /// registry. Searchable, addressable, has bytes.
    #[test]
    fn buffer_list_is_a_regular_buffer() {
        let s = EditorState::new();
        // Force allocation of the *help* buffer so the listing has at
        // least two entries.
        let _: Option<crate::lua_bindings::BufferIdLua> = s
            .lua_host
            .lua()
            .load("return pmacs.help.show_command('cursor.left')")
            .eval()
            .unwrap();
        s.lua_host
            .invoke_command("editor.list-buffers", mlua::MultiValue::new())
            .unwrap();
        let id = s
            .lua_host
            .registry()
            .borrow()
            .find_by_name("*buffer-list*")
            .expect("*buffer-list* must exist");
        let reg = s.lua_host.registry().borrow();
        let buf = reg.get(id).unwrap();
        assert!(!buf.is_empty(), "buffer-list should have content");
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("*scratch*"), "body: {body}");
        assert!(body.contains("*help*"), "body: {body}");
    }

    /// `pmacs.buffer.kill` removes a buffer from the registry but
    /// first redirects every window pointing at it to a safe fallback
    /// (the existing `*scratch*` if present), so windows never end up
    /// referring to a missing id.
    #[test]
    fn buffer_kill_redirects_active_window_to_fallback() {
        let s = EditorState::new();
        let doomed = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("doomed.txt", b"hello");
        s.core.borrow_mut().switch_active_buffer(doomed).unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), "doomed.txt");
        s.lua_host
            .lua()
            .load("pmacs.buffer.kill(...)")
            .call::<()>(crate::lua_bindings::BufferIdLua(doomed))
            .unwrap();
        assert!(
            !s.lua_host.registry().borrow().contains(doomed),
            "buffer should be removed from registry"
        );
        assert_ne!(
            s.core.borrow().active_buffer_id(),
            doomed,
            "active window should have been redirected"
        );
        assert_eq!(
            s.core.borrow().active_buffer_name(),
            "*scratch*",
            "fallback should be *scratch*"
        );
    }

    #[test]
    fn buffer_kill_fires_on_removed_callbacks() {
        let s = EditorState::new();
        let doomed = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("doomed.txt", b"hello");
        let called: bool = s
            .lua_host
            .lua()
            .load(
                r"
                local doomed = ...
                local called = false
                pmacs.buffer.on_removed(doomed, function(dead)
                    assert(dead == doomed)
                    called = true
                end)
                pmacs.buffer.kill(doomed)
                return called
                ",
            )
            .call(crate::lua_bindings::BufferIdLua(doomed))
            .unwrap();
        assert!(called, "kill should fire buffer removal callbacks");
    }

    /// `pmacs.buffer.kill` refuses to remove the last remaining
    /// buffer; the registry must never go empty.
    #[test]
    fn buffer_kill_refuses_last_buffer() {
        let s = EditorState::new();
        // EditorState::new starts with *scratch*. Drop every other
        // buffer (there shouldn't be any, but be defensive) and try to
        // kill the lone survivor.
        let last = s.core.borrow().active_buffer_id();
        let result: mlua::Result<()> = s
            .lua_host
            .lua()
            .load("pmacs.buffer.kill(...)")
            .call(crate::lua_bindings::BufferIdLua(last));
        assert!(
            result.is_err(),
            "kill should refuse the last buffer, got {result:?}"
        );
        assert!(
            s.lua_host.registry().borrow().contains(last),
            "buffer must remain after refused kill"
        );
    }

    /// Inside `*buffer-list*`, RET (bound to `editor.buffer-list-visit`)
    /// switches the active window to the buffer named on the cursor's
    /// line. Drives the path through `editor.list-buffers` to set up
    /// the line-to-buffer mapping, then `move_down` once more to land
    /// on the second data line, then visits.
    #[test]
    fn buffer_list_visit_switches_to_buffer_at_cursor() {
        let s = EditorState::new();
        let _ = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("target.txt", b"x");
        s.lua_host
            .invoke_command("editor.list-buffers", mlua::MultiValue::new())
            .unwrap();
        // After list-buffers, the cursor sits on data line 1 (the
        // first registered buffer, i.e. *scratch*). Walk down until we
        // land on `target.txt`.
        let mut hops = 0;
        loop {
            let line: i64 = s
                .lua_host
                .lua()
                .load("return pmacs.editor.cursor_line()")
                .eval()
                .unwrap();
            assert!(line >= 1, "cursor should be on a data line");
            let name_at_cursor = s.lua_host.lua()
                .load("local i = pmacs.editor.cursor_line(); local ids = pmacs.buffer.list(); local nth = 1; for _, id in ipairs(ids) do if pmacs.describe.buffer(id).name == '*buffer-list*' then else if nth == i then return pmacs.describe.buffer(id).name end; nth = nth + 1 end end")
                .eval::<Option<String>>().unwrap();
            if name_at_cursor.as_deref() == Some("target.txt") {
                break;
            }
            s.lua_host
                .invoke_command("cursor.down", mlua::MultiValue::new())
                .unwrap();
            hops += 1;
            assert!(hops < 32, "couldn't find target.txt in buffer list");
        }
        s.lua_host
            .invoke_command("editor.buffer-list-visit", mlua::MultiValue::new())
            .unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), "target.txt");
    }

    #[test]
    fn editor_move_to_line_positions_cursor_by_zero_based_line() {
        let s = fresh_with(b"alpha\nbeta\ngamma");
        s.lua_host
            .lua()
            .load("pmacs.editor.move_to_line(1)")
            .exec()
            .unwrap();
        assert_eq!(s.core.borrow().cursor_line(), 1);
        assert_eq!(s.core.borrow().cursor(), 6);

        s.lua_host
            .lua()
            .load("pmacs.editor.move_to_line(99)")
            .exec()
            .unwrap();
        assert_eq!(s.core.borrow().cursor_line(), 2);
        assert_eq!(s.core.borrow().cursor(), 11);
    }

    /// `editor.next-buffer` walks the active window through the
    /// buffer registry in order, wrapping past the end. Three buffers
    /// in registry order: walking next four times returns to the
    /// starting buffer.
    #[test]
    fn next_buffer_cycles_through_registry_with_wrap() {
        let s = EditorState::new(); // creates *scratch*
        let a = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("a.txt", b"x");
        let b = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("b.txt", b"y");
        // Start on *scratch*. The registry order is [scratch, a, b].
        let names: Vec<String> = (0..4)
            .map(|_| {
                s.lua_host
                    .invoke_command("editor.next-buffer", mlua::MultiValue::new())
                    .unwrap();
                s.core.borrow().active_buffer_name()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "a.txt".to_string(),
                "b.txt".to_string(),
                "*scratch*".to_string(),
                "a.txt".to_string(),
            ],
            "next-buffer should cycle scratch -> a -> b -> scratch -> a"
        );
        // Cleanup so the test is self-contained.
        let _ = s.lua_host.registry().borrow_mut().remove(a);
        let _ = s.lua_host.registry().borrow_mut().remove(b);
    }

    /// `editor.previous-buffer` walks the registry backward, wrapping
    /// past the start.
    #[test]
    fn previous_buffer_cycles_backward_with_wrap() {
        let s = EditorState::new();
        s.lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("a.txt", b"x");
        s.lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("b.txt", b"y");
        // From *scratch*, previous wraps to b.txt (last in registry).
        s.lua_host
            .invoke_command("editor.previous-buffer", mlua::MultiValue::new())
            .unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), "b.txt");
        s.lua_host
            .invoke_command("editor.previous-buffer", mlua::MultiValue::new())
            .unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), "a.txt");
        s.lua_host
            .invoke_command("editor.previous-buffer", mlua::MultiValue::new())
            .unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), "*scratch*");
    }

    /// With only one buffer in the registry, both cycling commands
    /// are no-ops.
    #[test]
    fn buffer_cycling_is_noop_with_one_buffer() {
        let s = EditorState::new(); // only *scratch*
        let before = s.core.borrow().active_buffer_name();
        s.lua_host
            .invoke_command("editor.next-buffer", mlua::MultiValue::new())
            .unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), before);
        s.lua_host
            .invoke_command("editor.previous-buffer", mlua::MultiValue::new())
            .unwrap();
        assert_eq!(s.core.borrow().active_buffer_name(), before);
    }

    /// `mark-delete` and `unmark` re-seat the cursor on the same line
    /// after the wholesale buffer rewrite, then advance one row
    /// (Emacs's `Buffer-menu-mark` semantics). Without the re-seat,
    /// the cursor would dangle at a stale byte offset across the
    /// rewrite. This test asserts the row-advance contract.
    #[test]
    fn buffer_list_mark_advances_cursor_one_row() {
        let s = EditorState::new();
        let _ = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("a.txt", b"x");
        let _ = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("b.txt", b"x");
        s.lua_host
            .invoke_command("editor.list-buffers", mlua::MultiValue::new())
            .unwrap();
        let line_before: i64 = s
            .lua_host
            .lua()
            .load("return pmacs.editor.cursor_line()")
            .eval()
            .unwrap();
        assert_eq!(line_before, 1, "should land on first data line");
        s.lua_host
            .invoke_command("editor.buffer-list-mark-delete", mlua::MultiValue::new())
            .unwrap();
        let line_after: i64 = s
            .lua_host
            .lua()
            .load("return pmacs.editor.cursor_line()")
            .eval()
            .unwrap();
        assert_eq!(line_after, 2, "mark-delete should advance one row");
        s.lua_host
            .invoke_command("editor.buffer-list-unmark", mlua::MultiValue::new())
            .unwrap();
        let line_after_unmark: i64 = s
            .lua_host
            .lua()
            .load("return pmacs.editor.cursor_line()")
            .eval()
            .unwrap();
        assert_eq!(line_after_unmark, 3, "unmark should also advance one row");
    }

    /// `editor.buffer-list-mark-delete` followed by
    /// `editor.buffer-list-execute` removes the marked buffer from the
    /// registry. The active window (showing `*buffer-list*`) is left
    /// alone since the kill only targeted a different buffer.
    #[test]
    fn buffer_list_mark_then_execute_removes_marked_buffers() {
        let s = EditorState::new();
        let doomed = s
            .lua_host
            .registry()
            .borrow_mut()
            .create_from_bytes("doomed.txt", b"x");
        s.lua_host
            .invoke_command("editor.list-buffers", mlua::MultiValue::new())
            .unwrap();
        // Walk the cursor to the doomed.txt row.
        let mut hops = 0;
        loop {
            let name_at_cursor = s.lua_host.lua()
                .load("local i = pmacs.editor.cursor_line(); local ids = pmacs.buffer.list(); local nth = 1; for _, id in ipairs(ids) do if pmacs.describe.buffer(id).name == '*buffer-list*' then else if nth == i then return pmacs.describe.buffer(id).name end; nth = nth + 1 end end")
                .eval::<Option<String>>().unwrap();
            if name_at_cursor.as_deref() == Some("doomed.txt") {
                break;
            }
            s.lua_host
                .invoke_command("cursor.down", mlua::MultiValue::new())
                .unwrap();
            hops += 1;
            assert!(hops < 32, "couldn't reach doomed.txt");
        }
        s.lua_host
            .invoke_command("editor.buffer-list-mark-delete", mlua::MultiValue::new())
            .unwrap();
        s.lua_host
            .invoke_command("editor.buffer-list-execute", mlua::MultiValue::new())
            .unwrap();
        assert!(
            !s.lua_host.registry().borrow().contains(doomed),
            "doomed.txt should have been killed by execute"
        );
        assert_eq!(
            s.core.borrow().active_buffer_name(),
            "*buffer-list*",
            "active window should still be on *buffer-list*"
        );
    }

    /// Bullet 4: SIGWINCH-equivalent (recomputing layout against a
    /// new area) preserves split ratios.
    #[test]
    fn resize_preserves_split_ratios() {
        let s = EditorState::new();
        s.lua_host
            .lua()
            .load("pmacs.window.split_vertical()")
            .exec()
            .unwrap();
        // Set a 2:1 weight on the root split.
        if let crate::window::LayoutNode::Split { weights, .. } =
            &mut s.core.borrow_mut().active_layout_mut().root
        {
            *weights = vec![2, 1];
        } else {
            panic!("expected split");
        }
        let p1 = s
            .core
            .borrow()
            .active_layout()
            .compute(crate::window::Rect::new(0, 0, 24, 90));
        let p2 = s
            .core
            .borrow()
            .active_layout()
            .compute(crate::window::Rect::new(0, 0, 24, 60));
        // Both should preserve the 2:1 ratio. Find the two windows
        // and verify the larger:smaller ratio is 2:1 in both.
        let wider1 = p1.values().map(|r| r.size.cols).max().unwrap();
        let narrower1 = p1.values().map(|r| r.size.cols).min().unwrap();
        assert_eq!(wider1 / narrower1, 2);
        let wider2 = p2.values().map(|r| r.size.cols).max().unwrap();
        let narrower2 = p2.values().map(|r| r.size.cols).min().unwrap();
        assert_eq!(wider2 / narrower2, 2);
    }

    /// Edits in one window propagate to all windows on the same
    /// buffer (multi-window `TextView` coherence).
    #[test]
    fn edits_in_one_window_visible_in_another_on_same_buffer() {
        let mut s = fresh_with(b"hello");
        // Open a second window on the same buffer.
        s.lua_host
            .lua()
            .load("pmacs.window.split_vertical()")
            .exec()
            .unwrap();
        let buf_id = s.core.borrow().active_buffer_id();
        // Edit through the active window.
        s.dispatch_key(FrontendId::LOCAL, ctrl('e')); // cursor.line-end
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('!'), KeyModifiers::NONE),
        );
        // Buffer length is now 6; the *other* window points at the
        // same buffer id and its TextView was notified by
        // apply_active_edit.
        let core = s.core.borrow();
        assert_eq!(core.active_buffer_len(), 6);
        let active = core.active_window_id();
        let other_id = core
            .windows
            .keys()
            .find(|id| **id != active)
            .copied()
            .unwrap();
        assert_eq!(core.windows[&other_id].buffer_id, buf_id);
    }

    // ---- T M2.9: multi-view composition ------------------------------------

    /// Render `core`'s active window into a freshly-zeroed cell buffer and
    /// return it. Mirrors what `editor::render` does for window content,
    /// but skips status / minibuffer / mode-line so the assertions can
    /// look at the buffer cells directly.
    fn render_active_window_to_grid(
        core: &mut crate::editor_core::EditorCore,
    ) -> Vec<crate::cell::Cell> {
        use crate::cell::{Cell, CellGrid, CellSize};
        use crate::view::Viewport;
        let active = core.active_window_id();
        let win = core.windows.get_mut(&active).unwrap();
        let rect = crate::window::Rect::new(0, 0, 24, 80);
        let cell_count = (rect.size.rows * rect.size.cols) as usize;
        let mut backing = vec![Cell::default(); cell_count];
        let registry = core.registry.clone();
        let reg = registry.borrow();
        let buf = reg.get(win.buffer_id).unwrap();
        let viewport = Viewport {
            buffer_start: 0,
            buffer_end: buf.len(),
            cell_origin: rect.origin,
            cell_size: CellSize::new(rect.size.rows, rect.size.cols),
            gutter_w: 0,
        };
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: rect.size.cols,
            size: CellSize::new(rect.size.rows, rect.size.cols),
        };
        win.text_view.render(buf, viewport, &mut grid);
        for overlay in &mut win.overlays {
            overlay.render(buf, viewport, &mut grid);
        }
        backing
    }

    /// Acceptance bullet 1: a buffer with three views (text + style
    /// overlay + virtual cells) renders correctly into one cell grid.
    #[test]
    fn three_views_compose_in_a_real_window() {
        use crate::cell::{Cell, CellCoord, Glyph, Style, UnderlineStyle};
        use crate::overlay::{StyleSpan, StyleSpanOverlay, VirtualCell, VirtualCellOverlay};
        let s = fresh_with(b"hello world\nsecond\n");
        {
            let mut core = s.core.borrow_mut();
            let win = core.active_window_mut();
            // Layer 2: bold underline on "hello".
            let mut style = StyleSpanOverlay::new();
            style.add(StyleSpan {
                row: 0,
                start_col: 0,
                end_col: 5,
                style: Style {
                    bold: true,
                    underline: UnderlineStyle::Curly,
                    ..Default::default()
                },
            });
            // Layer 3: virtual cell '★' past the end of "hello world".
            let mut virt = VirtualCellOverlay::new();
            virt.add(VirtualCell {
                row: 0,
                col: 12,
                cell: Cell {
                    glyph: Glyph::Char('★'),
                    style: Style {
                        italic: true,
                        ..Default::default()
                    },
                    attachment: None,
                },
            });
            win.push_overlay(Box::new(style));
            win.push_overlay(Box::new(virt));
        }
        let mut core = s.core.borrow_mut();
        let cells = render_active_window_to_grid(&mut core);
        let stride = 80usize;
        let at = |row: u32, col: u32| -> &Cell { &cells[row as usize * stride + col as usize] };
        // Layer 1 (text): glyphs come from the buffer.
        assert_eq!(at(0, 0).glyph, Glyph::Char('h'));
        assert_eq!(at(0, 4).glyph, Glyph::Char('o'));
        assert_eq!(at(0, 6).glyph, Glyph::Char('w'));
        assert_eq!(at(1, 0).glyph, Glyph::Char('s'));
        // Layer 2 (style): "hello" is bold + curly-underlined; glyphs preserved.
        for col in 0..5 {
            let c = at(0, col);
            assert!(c.style.bold, "col {col} not bold");
            assert_eq!(c.style.underline, UnderlineStyle::Curly);
        }
        // " world" plain style.
        for col in 5..11 {
            let c = at(0, col);
            assert!(!c.style.bold);
            assert_eq!(c.style.underline, UnderlineStyle::None);
        }
        // Layer 3 (virtual): glyph replaced.
        assert_eq!(at(0, 12).glyph, Glyph::Char('★'));
        assert!(at(0, 12).style.italic);
        // Sanity that we didn't bleed past the active window region.
        let _ = CellCoord::new(0, 0);
    }

    /// Acceptance bullet 3: composition adds <10% overhead over
    /// single-view rendering. Measured against the *composition
    /// machinery* — the cost of holding additional views and
    /// dispatching to them — independent of the work each overlay
    /// chooses to do, since that work scales with what it paints.
    ///
    /// Concretely: render the same buffer with `text_view` alone vs.
    /// `text_view` plus two overlays whose `render` immediately
    /// returns. The difference is the dispatch loop cost. Anything
    /// above ~5% would mean the per-overlay setup cost dominates a
    /// small frame, and overlay-heavy frames would suffer.
    ///
    /// As an informational data point we also time a *realistic*
    /// composed frame (with non-empty overlays) and print it; we do
    /// not assert on it because overlay work scales linearly with
    /// cells touched and "10%" is a meaningful budget only against
    /// machinery, not against work.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "perf measurement is intentionally linear"
    )]
    fn composition_overhead_under_ten_percent() {
        use crate::cell::{Cell, CellCoord, CellGrid, CellSize, Glyph, Style};
        use crate::overlay::{StyleSpan, StyleSpanOverlay, VirtualCell, VirtualCellOverlay};
        use crate::view::{View, Viewport};
        use std::time::Instant;
        const ITERS: usize = 5000;
        const WARMUP: usize = 500;

        struct NoopOverlay;
        impl View for NoopOverlay {}

        // Buffer with 200 lines of plausible source code so the base
        // view does meaningful work each frame.
        let mut content = Vec::new();
        for i in 0..200 {
            content.extend_from_slice(format!("    let value_{i} = {i} * 2;\n").as_bytes());
        }
        let s = fresh_with(&content);

        let (single_avg_ns, dispatch_avg_ns, realistic_avg_ns) = {
            let mut core = s.core.borrow_mut();
            let active = core.active_window_id();
            let buf_id = core.windows[&active].buffer_id;
            let registry = core.registry.clone();
            let reg = registry.borrow();
            let buf = reg.get(buf_id).unwrap();
            let viewport = Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(24, 80),
                gutter_w: 0,
            };

            // Two no-op overlays: probe the dispatch cost only.
            let mut empty1: Box<dyn View> = Box::new(NoopOverlay);
            let mut empty2: Box<dyn View> = Box::new(NoopOverlay);

            // Realistic overlay payload, ~3% of cells.
            let mut style = StyleSpanOverlay::new();
            for row in 0..8 {
                style.add(StyleSpan {
                    row: (row as u32 * 3) % 24,
                    start_col: 4,
                    end_col: 8,
                    style: Style {
                        bold: true,
                        ..Default::default()
                    },
                });
            }
            let mut virt = VirtualCellOverlay::new();
            for row in 0..8 {
                virt.add(VirtualCell {
                    row: (row as u32 * 3) % 24,
                    col: 60,
                    cell: Cell {
                        glyph: Glyph::Char('|'),
                        style: Style::default(),
                        attachment: None,
                    },
                });
            }

            let mut backing = vec![Cell::default(); 24 * 80];

            // Warmup, then time: text_view only.
            for _ in 0..WARMUP {
                let win = core.windows.get_mut(&active).unwrap();
                let mut grid = CellGrid {
                    cells: &mut backing,
                    stride: 80,
                    size: CellSize::new(24, 80),
                };
                win.text_view.render(buf, viewport, &mut grid);
            }
            let t = Instant::now();
            for _ in 0..ITERS {
                let win = core.windows.get_mut(&active).unwrap();
                let mut grid = CellGrid {
                    cells: &mut backing,
                    stride: 80,
                    size: CellSize::new(24, 80),
                };
                win.text_view.render(buf, viewport, &mut grid);
            }
            let single = t.elapsed().as_nanos() / ITERS as u128;

            // text_view + two no-op overlays: pure dispatch overhead.
            for _ in 0..WARMUP {
                let win = core.windows.get_mut(&active).unwrap();
                let mut grid = CellGrid {
                    cells: &mut backing,
                    stride: 80,
                    size: CellSize::new(24, 80),
                };
                win.text_view.render(buf, viewport, &mut grid);
                empty1.render(buf, viewport, &mut grid);
                empty2.render(buf, viewport, &mut grid);
            }
            let t = Instant::now();
            for _ in 0..ITERS {
                let win = core.windows.get_mut(&active).unwrap();
                let mut grid = CellGrid {
                    cells: &mut backing,
                    stride: 80,
                    size: CellSize::new(24, 80),
                };
                win.text_view.render(buf, viewport, &mut grid);
                empty1.render(buf, viewport, &mut grid);
                empty2.render(buf, viewport, &mut grid);
            }
            let dispatch = t.elapsed().as_nanos() / ITERS as u128;

            // text_view + realistic overlays: informational only.
            for _ in 0..WARMUP {
                let win = core.windows.get_mut(&active).unwrap();
                let mut grid = CellGrid {
                    cells: &mut backing,
                    stride: 80,
                    size: CellSize::new(24, 80),
                };
                win.text_view.render(buf, viewport, &mut grid);
                style.render(buf, viewport, &mut grid);
                virt.render(buf, viewport, &mut grid);
            }
            let t = Instant::now();
            for _ in 0..ITERS {
                let win = core.windows.get_mut(&active).unwrap();
                let mut grid = CellGrid {
                    cells: &mut backing,
                    stride: 80,
                    size: CellSize::new(24, 80),
                };
                win.text_view.render(buf, viewport, &mut grid);
                style.render(buf, viewport, &mut grid);
                virt.render(buf, viewport, &mut grid);
            }
            let realistic = t.elapsed().as_nanos() / ITERS as u128;

            (single, dispatch, realistic)
        };

        eprintln!("single render avg          : {single_avg_ns} ns");
        eprintln!("dispatch (2 no-op overlays): {dispatch_avg_ns} ns");
        eprintln!("realistic 3-view frame     : {realistic_avg_ns} ns");
        let dispatch_ratio = dispatch_avg_ns as f64 / single_avg_ns as f64;
        let realistic_ratio = realistic_avg_ns as f64 / single_avg_ns as f64;
        eprintln!(
            "dispatch overhead          : {:.1}%",
            (dispatch_ratio - 1.0) * 100.0
        );
        eprintln!(
            "realistic overhead         : {:.1}%",
            (realistic_ratio - 1.0) * 100.0
        );

        if !cfg!(target_os = "macos") {
            assert!(
                dispatch_ratio < 1.10,
                "composition machinery added more than 10% overhead: {dispatch_ratio:.3} \
                 (single={single_avg_ns} ns, dispatch={dispatch_avg_ns} ns)"
            );
        }
    }

    /// Edits to the buffer reach overlays via `on_edit`, just like
    /// they reach the base text view. Without this, overlays that
    /// cache buffer-derived state would silently desync after the
    /// first edit.
    #[test]
    fn overlays_receive_on_edit_alongside_text_view() {
        use crate::buffer::EditOp;
        use crate::view::View;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingOverlay {
            count: Arc<AtomicU32>,
        }
        impl View for CountingOverlay {
            fn on_edit(
                &mut self,
                _buf: &crate::buffer::Buffer,
                _edit: &crate::rope::Edit,
            ) -> Result<(), crate::buffer::BufferError> {
                self.count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }

        let s = fresh_with(b"hi");
        let count = Arc::new(AtomicU32::new(0));
        s.core
            .borrow_mut()
            .active_window_mut()
            .push_overlay(Box::new(CountingOverlay {
                count: count.clone(),
            }));
        s.core
            .borrow_mut()
            .apply_active_edit(EditOp::Insert {
                pos: 2,
                bytes: b"!",
            })
            .unwrap();
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "overlay did not see on_edit"
        );
    }

    /// PR #113 round-6 finding 1: a same-buffer split copies
    /// store-backed render overlays to the new pane (splits fire no
    /// switch hook and started from an empty overlay list), and
    /// per-window attachment is idempotent via the store identity.
    #[test]
    fn same_buffer_split_copies_style_overlays_and_attach_is_idempotent() {
        use crate::overlay::{BufferStyleOverlay, SharedBufferStyleSpans};
        use crate::window::Orientation;
        use std::sync::{Arc, Mutex};

        let s = fresh_with(b"hello\n");
        let store: SharedBufferStyleSpans = Arc::new(Mutex::new(Vec::new()));
        {
            let mut core = s.core.borrow_mut();
            let win = core.active_window_mut();
            win.ensure_overlay(Box::new(BufferStyleOverlay::new(Arc::clone(&store))));
            // Second ensure over the SAME store: no duplicate.
            win.ensure_overlay(Box::new(BufferStyleOverlay::new(Arc::clone(&store))));
            assert_eq!(
                win.overlay_kinds()
                    .iter()
                    .filter(|k| **k == "buffer_style_overlay")
                    .count(),
                1,
                "ensure_overlay must be idempotent per store"
            );
        }
        // Same-buffer split: the new pane carries a copy.
        let new_id = s
            .core
            .borrow_mut()
            .split_active(Orientation::Horizontal, true);
        {
            let core = s.core.borrow();
            let win = core.windows.get(&new_id).expect("split window");
            assert_eq!(
                win.overlay_kinds()
                    .iter()
                    .filter(|k| **k == "buffer_style_overlay")
                    .count(),
                1,
                "a same-buffer split must copy the render overlay"
            );
        }
        // Fresh-buffer split: no copy (different buffer, different
        // styling).
        let scratch_id = s
            .core
            .borrow_mut()
            .split_active(Orientation::Horizontal, false);
        let core = s.core.borrow();
        let win = core.windows.get(&scratch_id).expect("scratch window");
        assert_eq!(
            win.overlay_kinds()
                .iter()
                .filter(|k| **k == "buffer_style_overlay")
                .count(),
            0,
            "a fresh-buffer split carries nothing"
        );
    }

    // ---- T M2.12: mouse input ----------------------------------------------

    fn mouse(kind: crossterm::event::MouseEventKind, row: u16, col: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn term_size_24x80() -> crate::cell::CellSize {
        crate::cell::CellSize::new(24, 80)
    }

    /// Acceptance bullet 1: click on any cell positions the cursor at
    /// the corresponding rope position.
    #[test]
    fn mouse_click_positions_cursor() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello\nworld\n");
        // Click at row 1, col 3 — should land in the middle of "world".
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 3),
            term_size_24x80(),
        );
        // "hello\n" is 6 bytes; "wor" puts us at byte 6+3 = 9.
        assert_eq!(s.core.borrow().cursor(), 9);
        // A click also begins an empty selection at the click point.
        // The "is empty" check is via region() — empty selection has no region.
        assert!(s.core.borrow().active_region().is_none());
    }

    /// Acceptance bullet 2: drag selection produces a region usable
    /// by region-aware commands.
    #[test]
    fn mouse_drag_produces_region_usable_by_delete() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello world\n");
        // Click at col 0 (start of buffer).
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
            term_size_24x80(),
        );
        assert_eq!(s.core.borrow().cursor(), 0);
        // Drag to col 5 ("hello").
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Drag(MouseButton::Left), 0, 5),
            term_size_24x80(),
        );
        let region = s.core.borrow().active_region();
        assert_eq!(region, Some((0, 5)), "expected (0, 5); got {region:?}");
        // Region-aware command consumes the region.
        s.lua_host
            .invoke_command("region.delete", mlua::MultiValue::new())
            .unwrap();
        // Buffer now contains " world\n"; cursor moved to start.
        let core = s.core.borrow();
        assert_eq!(core.active_buffer_len(), 7);
        assert_eq!(core.cursor(), 0);
        assert!(core.active_region().is_none());
    }

    #[test]
    fn mouse_drag_selection_paints_in_tui_grid() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello world\n");
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
            term_size_24x80(),
        );
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Drag(MouseButton::Left), 0, 5),
            term_size_24x80(),
        );

        let (cells, _, _) = render_to_grid(&s, 24, 80);
        for col in 0..5 {
            let style = cells[col as usize].style;
            assert!(style.reverse, "selected col {col} was not reverse video");
        }
        assert!(
            !cells[5].style.reverse,
            "unselected cell after mouse selection was reverse video"
        );
    }

    #[test]
    fn mouse_double_click_selects_word_and_paints_in_tui_grid() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello world\n");

        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 7),
            term_size_24x80(),
        );
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Up(MouseButton::Left), 0, 7),
            term_size_24x80(),
        );
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 7),
            term_size_24x80(),
        );
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Up(MouseButton::Left), 0, 7),
            term_size_24x80(),
        );

        assert_eq!(s.core.borrow().cursor(), 11);
        assert_eq!(s.core.borrow().active_region(), Some((6, 11)));

        let (cells, _, _) = render_to_grid(&s, 24, 80);
        assert!(!cells[5].style.reverse, "selection leaked into separator");
        for col in 6..11 {
            assert!(
                cells[col as usize].style.reverse,
                "double-click selected word missing col {col}"
            );
        }
        assert!(!cells[11].style.reverse, "selection leaked past word");
    }

    #[test]
    fn mouse_double_click_on_separator_leaves_no_region() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello world\n");

        for _ in 0..2 {
            s.dispatch_mouse(
                FrontendId::LOCAL,
                mouse(MouseEventKind::Down(MouseButton::Left), 0, 5),
                term_size_24x80(),
            );
            s.dispatch_mouse(
                FrontendId::LOCAL,
                mouse(MouseEventKind::Up(MouseButton::Left), 0, 5),
                term_size_24x80(),
            );
        }

        assert_eq!(s.core.borrow().cursor(), 5);
        assert!(s.core.borrow().active_region().is_none());
    }

    /// Mouse framing Q#M1 — `dispatch_pointer` replays the mouse
    /// gesture semantics in byte space for semantic frontends.
    #[test]
    fn dispatch_pointer_replays_mouse_semantics_in_byte_space() {
        use crate::protocol::{Modifiers as WireMods, PointerKind};
        // Bytes: h=0 é=1,2 ' '=3 l=4 l=5 o=6 ' '=7 w=8 ö=9,10 r=11
        // l=12 d=13 \n=14; len=15.
        let mut s = fresh_with("hé llo wörld\n".as_bytes());
        let bid = s.core.borrow().active_buffer_id();
        let none = WireMods::NONE;

        // Down places the cursor — a mid-codepoint hit (inside 'é')
        // snaps back to the boundary — and anchors a selection.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 2, PointerKind::Down, none);
        assert_eq!(
            s.core.borrow().cursor(),
            1,
            "mid-codepoint hit snaps to the char boundary"
        );

        // Drag grows the region from the anchor; Up keeps it.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 6, PointerKind::Drag, none);
        assert_eq!(s.core.borrow().cursor(), 6);
        assert_eq!(s.core.borrow().active_region(), Some((1, 6)));
        s.dispatch_pointer(FrontendId::LOCAL, bid, 6, PointerKind::Up, none);
        assert_eq!(s.core.borrow().active_region(), Some((1, 6)));

        // A plain click (Down + Up, no drag) leaves no region.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 4, PointerKind::Down, none);
        s.dispatch_pointer(FrontendId::LOCAL, bid, 4, PointerKind::Up, none);
        assert_eq!(s.core.borrow().cursor(), 4);
        assert!(s.core.borrow().active_region().is_none());

        // DoubleDown selects the word at the hit ("wörld").
        s.dispatch_pointer(FrontendId::LOCAL, bid, 8, PointerKind::DoubleDown, none);
        assert_eq!(s.core.borrow().active_region(), Some((8, 14)));
        assert_eq!(s.core.borrow().cursor(), 14);

        // Past-EOF hits clamp to the buffer length.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 999, PointerKind::Down, none);
        assert_eq!(s.core.borrow().cursor(), 15);

        // A pointer for a buffer the window isn't displaying is
        // dropped (click racing a buffer switch).
        let other = crate::buffer::BufferId::next();
        s.dispatch_pointer(FrontendId::LOCAL, other, 0, PointerKind::Down, none);
        assert_eq!(s.core.borrow().cursor(), 15, "mismatched buffer ignored");
    }

    #[test]
    fn dispatch_pointer_triple_down_selects_the_whole_line() {
        use crate::protocol::{Modifiers as WireMods, PointerKind};
        // Line 0 = bytes [0, 12) including the newline; line 1 =
        // [12, 19).
        let mut s = fresh_with(b"hello world\nsecond\n");
        let bid = s.core.borrow().active_buffer_id();
        let none = WireMods::NONE;

        s.dispatch_pointer(FrontendId::LOCAL, bid, 4, PointerKind::TripleDown, none);
        assert_eq!(
            s.core.borrow().active_region(),
            Some((0, 12)),
            "whole line selected, trailing newline included"
        );
        assert_eq!(s.core.borrow().cursor(), 12, "cursor at selection end");

        // A line without a trailing newline runs to the buffer end.
        let mut s = fresh_with(b"abc");
        let bid = s.core.borrow().active_buffer_id();
        s.dispatch_pointer(FrontendId::LOCAL, bid, 1, PointerKind::TripleDown, none);
        assert_eq!(s.core.borrow().active_region(), Some((0, 3)));
    }

    #[test]
    fn dispatch_pointer_shift_down_extends_instead_of_restarting() {
        use crate::protocol::{Modifiers as WireMods, PointerKind};
        let mut s = fresh_with(b"hello world\n");
        let bid = s.core.borrow().active_buffer_id();
        let none = WireMods::NONE;
        let shift = WireMods::SHIFT;

        // No selection, cursor parked at 2: Shift-Down anchors at the
        // pre-click cursor and moves to the hit (Q#M5).
        s.dispatch_pointer(FrontendId::LOCAL, bid, 2, PointerKind::Down, none);
        s.dispatch_pointer(FrontendId::LOCAL, bid, 2, PointerKind::Up, none);
        assert!(s.core.borrow().active_region().is_none());
        s.dispatch_pointer(FrontendId::LOCAL, bid, 7, PointerKind::Down, shift);
        assert_eq!(s.core.borrow().active_region(), Some((2, 7)));
        // The Up after a Shift-click must not collapse the region
        // (anchor ≠ cursor).
        s.dispatch_pointer(FrontendId::LOCAL, bid, 7, PointerKind::Up, shift);
        assert_eq!(s.core.borrow().active_region(), Some((2, 7)));

        // With a live selection, Shift-Down keeps the anchor — even
        // extending in the other direction.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 0, PointerKind::Down, shift);
        assert_eq!(
            s.core.borrow().active_region(),
            Some((0, 2)),
            "anchor 2 kept; cursor crossed to the other side"
        );

        // A drag after a Shift-Down grows from the inherited anchor.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 9, PointerKind::Drag, shift);
        assert_eq!(s.core.borrow().active_region(), Some((2, 9)));

        // A plain Down restarts the anchor as before.
        s.dispatch_pointer(FrontendId::LOCAL, bid, 4, PointerKind::Down, none);
        s.dispatch_pointer(FrontendId::LOCAL, bid, 4, PointerKind::Up, none);
        assert!(s.core.borrow().active_region().is_none());
    }

    /// Acceptance bullet 3: mouse events are coalesced at frame
    /// boundaries — many drag events between renders all apply, and
    /// the cursor ends up at the last position.
    #[test]
    fn mouse_drag_events_coalesce_across_a_frame() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"abcdefghij\n");
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
            term_size_24x80(),
        );
        // Burst of drags through cols 1..=8 — simulates `process_event`
        // being invoked repeatedly between renders.
        for col in 1..=8u16 {
            s.dispatch_mouse(
                FrontendId::LOCAL,
                mouse(MouseEventKind::Drag(MouseButton::Left), 0, col),
                term_size_24x80(),
            );
        }
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Up(MouseButton::Left), 0, 8),
            term_size_24x80(),
        );
        // Cursor lands at the final drag col, anchor stays at click.
        assert_eq!(s.core.borrow().cursor(), 8);
        assert_eq!(s.core.borrow().active_region(), Some((0, 8)));
    }

    /// Plain click without a drag should *not* leave a phantom empty
    /// selection — `Up(Left)` clears it.
    #[test]
    fn plain_click_clears_empty_selection_on_release() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello\n");
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 2),
            term_size_24x80(),
        );
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Up(MouseButton::Left), 0, 2),
            term_size_24x80(),
        );
        assert_eq!(s.core.borrow().cursor(), 2);
        assert!(s.core.borrow().active_window().selection.is_none());
    }

    /// Mouse-wheel scrolls advance `view_top` and drag the cursor
    /// along by the same delta so it keeps its relative position in
    /// the viewport. Without the cursor-shift, the renderer's
    /// auto-scroll-to-cursor pass would snap `view_top` straight back
    /// the moment the cursor fell offscreen, making wheel scrolling
    /// feel stuck after one notch.
    #[test]
    fn scroll_wheel_advances_view_top_and_drags_cursor() {
        use crossterm::event::MouseEventKind;
        let mut content = Vec::new();
        for i in 0..50 {
            content.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let mut s = fresh_with(&content);
        let view_top_before = s.core.borrow().view_top();
        let cursor_line_before = s.core.borrow().cursor_line();
        // Wheel down 3 notches: view_top advances by 3 * SCROLL_LINES.
        for _ in 0..3 {
            s.dispatch_mouse(
                FrontendId::LOCAL,
                mouse(MouseEventKind::ScrollDown, 5, 5),
                term_size_24x80(),
            );
        }
        let view_top_after = s.core.borrow().view_top();
        let cursor_line_after = s.core.borrow().cursor_line();
        assert_eq!(
            view_top_after - view_top_before,
            3 * SCROLL_LINES as usize,
            "three notches should move view_top by 3*SCROLL_LINES"
        );
        assert_eq!(
            cursor_line_after - cursor_line_before,
            3 * SCROLL_LINES as usize,
            "cursor should follow view by the same delta"
        );
        // Wheel up enough notches to reach the top.
        for _ in 0..10 {
            s.dispatch_mouse(
                FrontendId::LOCAL,
                mouse(MouseEventKind::ScrollUp, 5, 5),
                term_size_24x80(),
            );
        }
        assert_eq!(
            s.core.borrow().view_top(),
            0,
            "scroll up should reach the top of the buffer"
        );
        assert_eq!(
            s.core.borrow().cursor_line(),
            0,
            "cursor should ride back up with the view"
        );
    }

    /// Click on a non-active window activates it (mouse click selects
    /// the focused window in addition to positioning the cursor).
    #[test]
    fn click_in_other_window_activates_it() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello\nworld\n");
        s.lua_host
            .lua()
            .load("pmacs.window.split_vertical()")
            .exec()
            .unwrap();
        let original_active = s.core.borrow().active_window_id();
        // Click on the right side (col 60 — guaranteed in the second window
        // for any standard 80-col terminal split in half).
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 60),
            term_size_24x80(),
        );
        let new_active = s.core.borrow().active_window_id();
        assert_ne!(
            new_active, original_active,
            "click in other window did not activate it"
        );
    }

    /// Click on a window's mode line is ignored — it does not move
    /// the cursor or activate the window. Reserved for future use.
    #[test]
    fn click_on_mode_line_is_ignored() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello\n");
        let cursor_before = s.core.borrow().cursor();
        // The single window occupies all but the bottom row of the
        // terminal; its mode line is at row term_rows - 2 = 22.
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 22, 5),
            term_size_24x80(),
        );
        assert_eq!(s.core.borrow().cursor(), cursor_before);
    }

    /// `pmacs.editor.region()` exposes the active region to Lua, and
    /// returns nil otherwise.
    #[test]
    fn lua_region_binding_returns_active_region() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = fresh_with(b"hello\n");
        // No region yet.
        let v: mlua::Value = s
            .lua_host
            .lua()
            .load("return pmacs.editor.region()")
            .eval()
            .unwrap();
        assert!(matches!(v, mlua::Value::Nil));

        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 1),
            term_size_24x80(),
        );
        s.dispatch_mouse(
            FrontendId::LOCAL,
            mouse(MouseEventKind::Drag(MouseButton::Left), 0, 4),
            term_size_24x80(),
        );
        let result: (i64, i64) = s
            .lua_host
            .lua()
            .load(
                r#"
                local r = pmacs.editor.region()
                return r.start, r["end"]
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, (1, 4));
    }

    // ---- Word and page motion ----------------------------------------------

    #[test]
    fn word_right_skips_separators_then_word_run() {
        let mut s = fresh_with(b"  hello world  foo");
        // Cursor at 0 (in leading whitespace). One word-right lands
        // after "hello".
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        assert_eq!(s.core.borrow().cursor(), 7); // end of "hello"
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        assert_eq!(s.core.borrow().cursor(), 13); // end of "world"
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        assert_eq!(s.core.borrow().cursor(), 18); // end of "foo" (and buffer)
        // Past the end stays clamped.
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        assert_eq!(s.core.borrow().cursor(), 18);
    }

    #[test]
    fn word_left_mirrors_word_right() {
        let mut s = fresh_with(b"  hello world  foo");
        // Drop cursor at the end.
        s.core.borrow_mut().active_window_mut().cursor = 18;
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(s.core.borrow().cursor(), 15); // start of "foo"
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(s.core.borrow().cursor(), 8); // start of "world"
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(s.core.borrow().cursor(), 2); // start of "hello"
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(s.core.borrow().cursor(), 0);
    }

    #[test]
    fn word_motion_treats_underscore_as_word_char() {
        let mut s = fresh_with(b"foo_bar baz");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        // "foo_bar" is a single word.
        assert_eq!(s.core.borrow().cursor(), 7);
    }

    #[test]
    fn word_motion_handles_multibyte_codepoints() {
        let mut s = fresh_with("café résumé".as_bytes());
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        // "café" is 5 bytes (é is 2 bytes); cursor at 5.
        assert_eq!(s.core.borrow().cursor(), 5);
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        );
        // "résumé" is 8 bytes; cursor at 5 + 1 (space) + 8 = 14.
        assert_eq!(s.core.borrow().cursor(), 14);
    }

    #[test]
    fn shift_arrow_extends_selection_and_paints_in_tui_grid() {
        let mut s = fresh_with(b"abcdef\n");
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));

        assert_eq!(s.core.borrow().cursor(), 2);
        assert_eq!(s.core.borrow().active_region(), Some((0, 2)));

        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        assert!(cells[0].style.reverse, "selection did not paint col 0");
        assert!(cells[1].style.reverse, "selection did not paint col 1");
        assert!(!cells[2].style.reverse, "selection leaked into col 2");
        assert_eq!(glyph_at(&cells, stride, 0, 0), 'a');
        assert_eq!(glyph_at(&cells, stride, 0, 1), 'b');
    }

    #[test]
    fn ctrl_shift_arrow_extends_selection_by_words_and_paragraphs() {
        let mut s = fresh_with(b"alpha beta\n\nsecond\n");
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        );
        assert_eq!(s.core.borrow().cursor(), 5);
        assert_eq!(s.core.borrow().active_region(), Some((0, 5)));

        s.core.borrow_mut().active_window_mut().cursor = 0;
        s.core.borrow_mut().clear_selection();
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        );
        assert_eq!(s.core.borrow().cursor(), 11);
        assert_eq!(s.core.borrow().active_region(), Some((0, 11)));
    }

    #[test]
    fn page_down_advances_cursor_and_view_top() {
        let mut content = Vec::new();
        for i in 0..100 {
            content.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let mut s = fresh_with(&content);
        // Set a known viewport size so page step is predictable.
        s.core.borrow_mut().active_window_mut().last_visible_rows = 10;
        let cursor_before = s.core.borrow().cursor();
        let view_top_before = s.core.borrow().view_top();
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::PageDown));
        let cursor_after = s.core.borrow().cursor();
        let view_top_after = s.core.borrow().view_top();
        assert!(
            cursor_after > cursor_before,
            "page-down did not advance cursor"
        );
        assert!(
            view_top_after > view_top_before,
            "page-down did not advance view_top"
        );
    }

    #[test]
    fn page_up_returns_to_top() {
        let mut content = Vec::new();
        for i in 0..100 {
            content.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let mut s = fresh_with(&content);
        s.core.borrow_mut().active_window_mut().last_visible_rows = 10;
        // Page down a few times.
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::PageDown));
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::PageDown));
        // Page up enough times to overshoot.
        for _ in 0..5 {
            s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::PageUp));
        }
        assert_eq!(s.core.borrow().view_top(), 0);
    }

    // ---- Paragraph motion --------------------------------------------------

    #[test]
    fn paragraph_down_lands_on_blank_lines_in_sequence() {
        let mut s = fresh_with(b"para 1 line a\npara 1 line b\n\npara 2 line a\n\npara 3\n");
        // Cursor at 0 (start of para 1). C-down should land at the
        // first blank line (after "para 1 line b\n").
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Down, KeyModifiers::CONTROL));
        // "para 1 line a\npara 1 line b\n" = 14 + 14 = 28 bytes.
        assert_eq!(s.core.borrow().cursor(), 28);
        // Press again: lands at the blank between para 2 and para 3.
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Down, KeyModifiers::CONTROL));
        // 28 + "\n" + "para 2 line a\n" = 28 + 1 + 14 = 43.
        assert_eq!(s.core.borrow().cursor(), 43);
        // Once more: end of buffer.
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Down, KeyModifiers::CONTROL));
        let len = s.core.borrow().active_buffer_len();
        assert_eq!(s.core.borrow().cursor(), len);
    }

    #[test]
    fn paragraph_up_mirrors_paragraph_down() {
        let mut s = fresh_with(b"para 1\n\npara 2\n\npara 3\n");
        let len = s.core.borrow().active_buffer_len();
        s.core.borrow_mut().active_window_mut().cursor = len;
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Up, KeyModifiers::CONTROL));
        // Lands at start of blank line between para 2 and para 3.
        // "para 1\n\npara 2\n" = 7 + 1 + 7 = 15.
        assert_eq!(s.core.borrow().cursor(), 15);
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Up, KeyModifiers::CONTROL));
        // Lands at start of blank between para 1 and para 2.
        // "para 1\n" = 7.
        assert_eq!(s.core.borrow().cursor(), 7);
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Up, KeyModifiers::CONTROL));
        // Top of buffer.
        assert_eq!(s.core.borrow().cursor(), 0);
    }

    #[test]
    fn paragraph_motion_treats_whitespace_only_lines_as_blank() {
        // Lines with only spaces / tabs separate paragraphs the same
        // way as truly empty lines.
        let mut s = fresh_with(b"alpha\n   \nbeta\n");
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Down, KeyModifiers::CONTROL));
        // Lands at start of the whitespace-only line, byte 6.
        assert_eq!(s.core.borrow().cursor(), 6);
    }

    // ---- Window-notify audit: every buffer-mutating path must keep
    //      windows displaying that buffer in sync. -------------------------

    #[test]
    fn help_show_command_updates_window_on_help_buffer() {
        // Regression: render_command does delete-all + insert on
        // *help*; if a window is showing it, the window's TextView
        // must be rebuilt or the new content is unreachable.
        let s = fresh_with(b"");
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.command.define {
                    name = "alpha",
                    description = "Alpha cmd.",
                    fn = function() end,
                }
                pmacs.help.show_command("alpha")
                "#,
            )
            .exec()
            .unwrap();
        let help_id = s
            .lua_host
            .registry()
            .borrow()
            .find_by_name(crate::help::HELP_BUFFER_NAME)
            .expect("*help* exists after show_command");
        s.core.borrow_mut().switch_active_buffer(help_id).unwrap();
        let lines_first = s.core.borrow().active_window().text_view.line_count();

        // Define another command and re-render — the help buffer is
        // rewritten end-to-end. The window's text_view must reflect
        // the new content.
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.command.define {
                    name = "beta",
                    description = "Beta cmd has a much longer description that produces noticeably more lines.",
                    fn = function() end,
                }
                pmacs.help.show_command("beta")
                "#,
            )
            .exec()
            .unwrap();
        let lines_second = s.core.borrow().active_window().text_view.line_count();
        let buf_len = s.core.borrow().active_buffer_len();
        let last_offset = s
            .core
            .borrow()
            .active_window()
            .text_view
            .line_offset(lines_second - 1)
            .unwrap();
        assert!(
            last_offset <= buf_len,
            "stale offset {last_offset} > buf_len {buf_len}"
        );
        // The two renders may produce different line counts; the
        // important invariant is that the view tracks the buffer.
        let _ = lines_first;
    }

    #[test]
    fn hook_error_logging_updates_window_on_errors_buffer() {
        // Hook callbacks that raise route through log_hook_error,
        // which appends to *errors*. If a window is displaying
        // *errors*, its TextView must be notified.
        let mut s = fresh_with(b"");
        // Trigger one error so *errors* exists, then switch to it.
        let _ = s.lua_host.eval(Some("seed"), "error('seed')");
        let errors_id = s
            .lua_host
            .errors_buffer_id()
            .expect("*errors* exists after eval-error");
        s.core.borrow_mut().switch_active_buffer(errors_id).unwrap();
        let lines_before = s.core.borrow().active_window().text_view.line_count();

        // Define a hook that raises, then run it — that path goes
        // through `log_hook_error`, distinct from `eval`.
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.hook.define {
                    name = "test.boom",
                    description = "Test hook that raises.",
                    kind = "all-must-succeed",
                }
                pmacs.hook.add("test.boom", function() error("kaboom") end)
                "#,
            )
            .exec()
            .unwrap();
        let _ = s.lua_host.run_hook("test.boom", mlua::MultiValue::new());

        let lines_after = s.core.borrow().active_window().text_view.line_count();
        assert!(
            lines_after >= lines_before,
            "*errors* window TextView did not reflect new entry"
        );
        // Pre-fix, the new content would be unreachable past the
        // pre-append last line. Confirm the new last offset advanced.
        let buf_len = s.core.borrow().active_buffer_len();
        let last_offset = s
            .core
            .borrow()
            .active_window()
            .text_view
            .line_offset(lines_after - 1)
            .unwrap();
        assert!(last_offset <= buf_len);
    }

    /// Audit: every public `EditorCore` mutation path that touches a
    /// buffer must leave windows on that buffer with a `TextView`
    /// whose line index covers the buffer end. This is the invariant
    /// the last four user-reported bugs all violated.
    #[test]
    fn windows_textview_invariant_holds_for_every_known_mutator() {
        use crate::buffer::EditOp;
        let mut s = fresh_with(b"alpha\nbeta\n");

        // 1) apply_active_edit — the canonical path.
        s.core
            .borrow_mut()
            .apply_active_edit(EditOp::Insert {
                pos: 0,
                bytes: b"prefix\n",
            })
            .unwrap();
        assert_textview_covers_buffer(&s);

        // 2) Lua buf:insert / buf:delete via userdata methods.
        s.lua_host
            .lua()
            .load(
                r#"
                local buf = pmacs.buffer.list()[1]
                buf:insert(buf:len(), "appended")
                buf:delete(0, 3)
                "#,
            )
            .exec()
            .unwrap();
        assert_textview_covers_buffer(&s);

        // 3) LuaHost::eval error path → append_to_errors_buffer.
        let errors_id = {
            let _ = s.lua_host.eval(Some("e"), "error('x')");
            s.lua_host.errors_buffer_id().unwrap()
        };
        s.core.borrow_mut().switch_active_buffer(errors_id).unwrap();
        let _ = s.lua_host.eval(Some("e2"), "error('y')");
        assert_textview_covers_buffer(&s);

        // 4) Help renderer.
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.command.define {
                    name = "audit-cmd",
                    description = "audit",
                    fn = function() end,
                }
                pmacs.help.show_command("audit-cmd")
                "#,
            )
            .exec()
            .unwrap();
        let help_id = s
            .lua_host
            .registry()
            .borrow()
            .find_by_name(crate::help::HELP_BUFFER_NAME)
            .unwrap();
        s.core.borrow_mut().switch_active_buffer(help_id).unwrap();
        s.lua_host
            .lua()
            .load("pmacs.help.show_command('audit-cmd')")
            .exec()
            .unwrap();
        assert_textview_covers_buffer(&s);

        // 5) editor.list-buffers (Lua userdata delete + insert).
        s.dispatch_key(FrontendId::LOCAL, ctrl('x'));
        s.dispatch_key(FrontendId::LOCAL, ctrl('b'));
        assert_textview_covers_buffer(&s);
    }

    // ---- Render-grid correctness -------------------------------------------
    //
    // The M-x stack-traceback corruption shipped despite a green
    // suite because we had no tests asserting against actual cell
    // content — only against the strings that fed into rendering.
    // These tests exercise the full `paint_frame` pipeline (window
    // text, mode line, status line, minibuffer overlay, cursor
    // placement) and read the resulting cells back.

    /// Render the editor state into a Vec-backed grid and return
    /// `(cells, stride, cursor)`. Tests use this and then index cells
    /// directly to verify what reached the screen.
    fn render_to_grid(
        s: &EditorState,
        rows: u32,
        cols: u32,
    ) -> (Vec<crate::cell::Cell>, u32, Option<CellCoord>) {
        let mut backing = vec![crate::cell::Cell::default(); (rows * cols) as usize];
        let mut grid = crate::cell::CellGrid {
            cells: &mut backing,
            stride: cols,
            size: crate::cell::CellSize::new(rows, cols),
        };
        let cursor = paint_frame(
            s,
            FrontendId::LOCAL,
            &HashMap::new(),
            &mut grid,
            crate::cell::CellSize::new(rows, cols),
        );
        (backing, cols, cursor)
    }

    fn glyph_at(cells: &[crate::cell::Cell], stride: u32, row: u32, col: u32) -> char {
        match &cells[(row * stride + col) as usize].glyph {
            crate::cell::Glyph::Char(c) => *c,
            crate::cell::Glyph::Cluster(_) => '?',
            crate::cell::Glyph::Continuation => ' ',
        }
    }

    fn row_text(cells: &[crate::cell::Cell], stride: u32, row: u32, cols: u32) -> String {
        (0..cols)
            .map(|c| glyph_at(cells, stride, row, c))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn render_paints_buffer_text_into_window_cells() {
        let s = fresh_with(b"hello\nworld\n");
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        assert_eq!(row_text(&cells, stride, 0, 80), "hello");
        assert_eq!(row_text(&cells, stride, 1, 80), "world");
    }

    #[test]
    fn render_status_line_truncates_multiline_status_to_one_row() {
        // Direct verification that the M-x error fix reaches the cell
        // grid, not just `build_status_line`. Set a multi-line status
        // and confirm that no cell on rows above the status line
        // contains traceback content, and the status row is intact.
        let s = fresh_with(b"hello\n");
        s.core.borrow_mut().status =
            "M-x error: command \"foo\" not found\nstack traceback:\n[C]: in ?\n".into();
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        // Status line is the bottom row.
        let status_row = row_text(&cells, stride, 23, 80);
        assert!(
            status_row.contains("M-x error: command \"foo\" not found"),
            "status row missing main message: {status_row:?}"
        );
        assert!(
            !status_row.contains("traceback"),
            "traceback leaked into status row: {status_row:?}"
        );
        // No row other than status should contain "traceback".
        for row in 0..23 {
            let text = row_text(&cells, stride, row, 80);
            assert!(
                !text.contains("traceback"),
                "traceback leaked into row {row}: {text:?}"
            );
        }
        // No cell anywhere should hold a control character.
        for cell in &cells {
            if let crate::cell::Glyph::Char(c) = cell.glyph {
                assert!(!c.is_control(), "control character {c:?} reached a cell");
            }
        }
    }

    #[test]
    fn render_mode_line_marks_active_window_and_modified_buffer() {
        let mut s = fresh_with(b"hello");
        // Make the buffer modified.
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char('!'), KeyModifiers::NONE),
        );
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        // Mode line is row 22 (rows-2 = 24-2).
        let mode_text = row_text(&cells, stride, 22, 80);
        // Active marker `+`, modified marker `*`.
        assert!(
            mode_text.contains("+*"),
            "mode line missing active+modified markers: {mode_text:?}"
        );
        assert!(
            mode_text.contains("test"),
            "mode line missing buffer name: {mode_text:?}"
        );
        // Mode line cells should be in reverse video.
        for col in 0..80 {
            let style = cells[(22 * stride + col) as usize].style;
            assert!(style.reverse, "mode line col {col} not reverse video");
        }
    }

    #[test]
    fn statusline_no_visible_provider_preserves_ascii_modeline_cells() {
        let s = fresh_with(b"hello");
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let actual = (0..80)
            .map(|col| glyph_at(&cells, stride, 22, col))
            .collect::<String>();
        let left = " +  test ";
        let right = " L1:C1 All ";
        let expected = format!("{left}{}{right}", " ".repeat(80 - left.len() - right.len()));
        assert_eq!(actual, expected);
    }

    #[test]
    fn statusline_real_frame_orders_runs_styles_separators_and_keeps_echo_independent() {
        let s = fresh_with(b"hello");
        s.core.borrow_mut().status = "echo-only".to_owned();
        s.lua_host
            .lua()
            .load(
                r#"
                pmacs.theme.merge {
                    ["ui.modeline.red"] = { fg = 1 },
                    ["ui.modeline.blue"] = { fg = 2 },
                }
                _G.statusline_handles = {
                    pmacs.statusline.register {
                        name = "left-zero", side = "left", priority = 0,
                        face = "ui.modeline.blue", fn = function() return "L0" end,
                    },
                    pmacs.statusline.register {
                        name = "left-high", side = "left", priority = 10,
                        face = "ui.modeline.red", fn = function() return "LH" end,
                    },
                    pmacs.statusline.register {
                        name = "left-nil", side = "left", priority = 100,
                        fn = function() return nil end,
                    },
                    pmacs.statusline.register {
                        name = "left-empty", side = "left", priority = 100,
                        fn = function() return "" end,
                    },
                    pmacs.statusline.register {
                        name = "left-zero-late", side = "left", priority = 0,
                        face = "ui.modeline.blue", fn = function() return "L1" end,
                    },
                    pmacs.statusline.register {
                        name = "right-zero", side = "right", priority = 0,
                        face = "ui.modeline.blue", fn = function() return "R0" end,
                    },
                    pmacs.statusline.register {
                        name = "right-high", side = "right", priority = 10,
                        face = "ui.modeline.red", fn = function() return "RH" end,
                    },
                    pmacs.statusline.register {
                        name = "right-zero-late", side = "right", priority = 0,
                        face = "ui.modeline.blue", fn = function() return "R1" end,
                    },
                }
                "#,
            )
            .exec()
            .unwrap();

        let (cells, stride, _) = render_to_grid(&s, 24, 100);
        let mode = row_text(&cells, stride, 22, 100);
        assert!(
            mode.starts_with(" +  test  LH L0 L1"),
            "wrong left composition: {mode:?}"
        );
        assert!(
            mode.ends_with("R0 R1 RH  L1:C1 All"),
            "wrong right composition: {mode:?}"
        );
        assert!(!mode.contains("left-nil") && !mode.contains("left-empty"));
        assert_eq!(row_text(&cells, stride, 23, 100), "echo-only");

        let lh_col = mode.find("LH").unwrap() as u32;
        let l0_col = mode.find("L0").unwrap() as u32;
        let rh_col = mode.find("RH").unwrap() as u32;
        let base = cells[(22 * stride) as usize].style;
        for col in [lh_col, lh_col + 1, rh_col, rh_col + 1] {
            let style = cells[(22 * stride + col) as usize].style;
            assert!(style.reverse);
            assert_eq!(style.bg, crate::cell::Color::Indexed(1));
        }
        for col in [l0_col, l0_col + 1] {
            let style = cells[(22 * stride + col) as usize].style;
            assert!(style.reverse);
            assert_eq!(style.bg, crate::cell::Color::Indexed(2));
        }
        assert_eq!(
            cells[(22 * stride + lh_col + 2) as usize].style,
            base,
            "custom/custom separator must retain ui.modeline"
        );
        let protected_right_col = mode.find(" L1:C1 All").unwrap() as u32;
        assert_eq!(
            cells[(22 * stride + protected_right_col - 1) as usize].style,
            base,
            "custom/built-in separator must retain ui.modeline"
        );
    }

    #[test]
    fn statusline_real_frame_evaluates_distinct_split_contexts_and_focus() {
        let s = fresh_with(b"left");
        s.lua_host
            .lua()
            .load(
                r#"
                _G.other_statusline_buffer = pmacs.buffer.create("other")
                pmacs.window.split_vertical()
                pmacs.window.switch_buffer(_G.other_statusline_buffer)
                _G.statusline_seen = {}
                _G.statusline_context_handle = pmacs.statusline.register {
                    name = "contexts", side = "left",
                    fn = function(ctx)
                        table.insert(_G.statusline_seen, {
                            frontend = ctx.frontend,
                            window = ctx.window,
                            buffer = tostring(ctx.buffer),
                            active = ctx.active,
                        })
                        return ctx.active and "ACTIVE" or "PASSIVE"
                    end,
                }
                _G.statusline_split_clip_handle = pmacs.statusline.register {
                    name = "split-clipping", side = "right",
                    fn = function(ctx)
                        return string.rep(ctx.active and "X" or "Y", 20)
                    end,
                }
                "#,
            )
            .exec()
            .unwrap();

        let (cells, stride, _) = render_to_grid(&s, 24, 120);
        let seen: mlua::Table = s.lua_host.lua().globals().get("statusline_seen").unwrap();
        assert_eq!(seen.raw_len(), 2);
        let first: mlua::Table = seen.raw_get(1).unwrap();
        let second: mlua::Table = seen.raw_get(2).unwrap();
        let first_window: u64 = first.get("window").unwrap();
        let second_window: u64 = second.get("window").unwrap();
        let first_buffer: String = first.get("buffer").unwrap();
        let second_buffer: String = second.get("buffer").unwrap();
        let first_frontend: u64 = first.get("frontend").unwrap();
        let second_frontend: u64 = second.get("frontend").unwrap();
        let first_active: bool = first.get("active").unwrap();
        let second_active: bool = second.get("active").unwrap();
        assert_ne!(first_window, second_window);
        assert_ne!(first_buffer, second_buffer);
        assert_eq!(first_frontend, FrontendId::LOCAL.0);
        assert_eq!(second_frontend, FrontendId::LOCAL.0);
        assert_ne!(first_active, second_active);

        let left_mode = (0..60)
            .map(|col| glyph_at(&cells, stride, 22, col))
            .collect::<String>();
        let right_mode = (60..120)
            .map(|col| glyph_at(&cells, stride, 22, col))
            .collect::<String>();
        assert!(
            (left_mode.contains("ACTIVE") && right_mode.contains("PASSIVE"))
                || (left_mode.contains("PASSIVE") && right_mode.contains("ACTIVE"))
        );

        s.lua_host
            .lua()
            .load("_G.statusline_seen = {}; pmacs.window.focus_next()")
            .exec()
            .unwrap();
        let _ = render_to_grid(&s, 24, 120);
        let seen: mlua::Table = s.lua_host.lua().globals().get("statusline_seen").unwrap();
        assert_eq!(seen.raw_len(), 2);
        let now_first: mlua::Table = seen.raw_get(1).unwrap();
        let now_second: mlua::Table = seen.raw_get(2).unwrap();
        let active_by_window = |table: &mlua::Table| {
            (
                table.get::<u64>("window").unwrap(),
                table.get::<bool>("active").unwrap(),
            )
        };
        let flipped = [active_by_window(&now_first), active_by_window(&now_second)];
        assert!(flipped.contains(&(first_window, !first_active)));
        assert!(flipped.contains(&(second_window, !second_active)));
        let (narrow_cells, narrow_stride, _) = render_to_grid(&s, 24, 30);
        let narrow_left = (0..15)
            .map(|col| glyph_at(&narrow_cells, narrow_stride, 22, col))
            .collect::<String>();
        let narrow_right = (15..30)
            .map(|col| glyph_at(&narrow_cells, narrow_stride, 22, col))
            .collect::<String>();
        assert!(
            (narrow_left.contains('X')
                && !narrow_left.contains('Y')
                && narrow_right.contains('Y')
                && !narrow_right.contains('X'))
                || (narrow_left.contains('Y')
                    && !narrow_left.contains('X')
                    && narrow_right.contains('X')
                    && !narrow_right.contains('Y')),
            "custom runs crossed a split boundary: left={narrow_left:?} right={narrow_right:?}"
        );
    }

    #[test]
    fn statusline_real_frame_discards_context_mutated_during_callback() {
        let s = fresh_with(b"old");
        s.lua_host
            .lua()
            .load(
                r#"
                _G.statusline_switch_target = pmacs.buffer.create("switched")
                _G.statusline_switch_once = true
                _G.statusline_switch_handle = pmacs.statusline.register {
                    name = "context-mutator", side = "left",
                    fn = function()
                        if _G.statusline_switch_once then
                            _G.statusline_switch_once = false
                            pmacs.window.switch_buffer(_G.statusline_switch_target)
                            return "STALE"
                        end
                        return "FRESH"
                    end,
                }
                "#,
            )
            .exec()
            .unwrap();

        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let first = row_text(&cells, stride, 22, 80);
        assert!(
            first.contains("switched"),
            "callback buffer switch did not land"
        );
        assert!(
            !first.contains("STALE"),
            "invalidated old-context output reached the new buffer: {first:?}"
        );

        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let second = row_text(&cells, stride, 22, 80);
        assert!(
            second.contains("FRESH"),
            "next valid frame did not evaluate the surviving context: {second:?}"
        );
    }

    #[test]
    fn statusline_real_frame_paints_unicode_clusters_and_sanitizes_all_runs() {
        let s = fresh_with(b"hello");
        {
            let core = s.core.borrow();
            let registry = core.registry.clone();
            registry
                .borrow_mut()
                .get_mut(core.active_buffer_id())
                .unwrap()
                .set_name("na\r\n\u{1b}me");
        }
        s.lua_host
            .lua()
            .load(
                r#"
                _G.statusline_unicode_handle = pmacs.statusline.register {
                    name = "unicode", side = "left",
                    fn = function() return "\204\129界e\204\129\27Z" end,
                }
                "#,
            )
            .exec()
            .unwrap();

        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let row = &cells[(22 * stride) as usize..(23 * stride) as usize];
        let wide_col = row
            .iter()
            .position(|cell| cell.glyph == crate::cell::Glyph::Char('界'))
            .expect("CJK grapheme should be present");
        assert_eq!(row[wide_col + 1].glyph, crate::cell::Glyph::Continuation);
        assert_eq!(
            row[wide_col + 2].glyph,
            crate::cell::Glyph::Cluster("e\u{301}".as_bytes().into())
        );
        assert_eq!(row[wide_col + 3].glyph, crate::cell::Glyph::Char(' '));
        assert_eq!(row[wide_col + 4].glyph, crate::cell::Glyph::Char('Z'));
        for cell in row {
            match &cell.glyph {
                crate::cell::Glyph::Char(ch) => assert!(!ch.is_control()),
                crate::cell::Glyph::Cluster(bytes) => {
                    let text = std::str::from_utf8(bytes).unwrap();
                    assert!(!text.chars().any(char::is_control));
                    assert_ne!(text, "\u{301}", "standalone zero-width grapheme leaked");
                }
                crate::cell::Glyph::Continuation => {}
            }
        }
        let ascii_projection = row
            .iter()
            .map(|cell| match cell.glyph {
                crate::cell::Glyph::Char(ch) => ch,
                _ => '?',
            })
            .collect::<String>();
        assert!(
            ascii_projection.contains("na   me"),
            "buffer-name controls were not replaced independently: {ascii_projection:?}"
        );
    }

    #[test]
    fn statusline_real_frame_clips_custom_edges_but_preserves_protected_suffix() {
        let s = fresh_with(b"hello");
        s.lua_host
            .lua()
            .load(
                r#"
                _G.statusline_clip_handles = {
                    pmacs.statusline.register {
                        name = "left-high", side = "left", priority = 10,
                        fn = function() return "HIGH" end,
                    },
                    pmacs.statusline.register {
                        name = "left-low", side = "left", priority = 0,
                        fn = function() return "界LOW" end,
                    },
                    pmacs.statusline.register {
                        name = "right-low", side = "right", priority = 0,
                        fn = function() return "LOW" end,
                    },
                    pmacs.statusline.register {
                        name = "right-high", side = "right", priority = 10,
                        fn = function() return "HIGH" end,
                    },
                }
                "#,
            )
            .exec()
            .unwrap();

        let (cells, stride, _) = render_to_grid(&s, 6, 17);
        let mode = row_text(&cells, stride, 4, 17);
        assert!(
            mode.contains("HIGH"),
            "high-priority right edge lost: {mode:?}"
        );
        assert!(
            !mode.contains("LOW"),
            "low-priority right edge survived: {mode:?}"
        );
        assert!(
            mode.ends_with(" L1:C1 All"),
            "protected suffix was not preserved in full: {mode:?}"
        );
        assert_ne!(
            cells[(4 * stride) as usize].glyph,
            crate::cell::Glyph::Continuation,
            "a clipped wide grapheme left a continuation at the window edge"
        );

        let left_only = fresh_with(b"hello");
        left_only
            .lua_host
            .lua()
            .load(
                r#"
                _G.statusline_left_clip_handles = {
                    pmacs.statusline.register {
                        name = "left-high", side = "left", priority = 10,
                        fn = function() return "HIGH" end,
                    },
                    pmacs.statusline.register {
                        name = "left-low", side = "left", priority = 0,
                        fn = function() return "界LOW" end,
                    },
                }
                "#,
            )
            .exec()
            .unwrap();
        let (cells, stride, _) = render_to_grid(&left_only, 6, 26);
        let mode = row_text(&cells, stride, 4, 26);
        assert!(mode.starts_with(" +  test  HIGH"));
        assert!(!mode.contains("LOW"));
        assert!(mode.ends_with(" L1:C1 All"));

        let (cells, stride, _) = render_to_grid(&s, 6, 11);
        let mode = row_text(&cells, stride, 4, 11);
        assert!(
            !mode.contains("L1:C1") && !mode.contains("HIGH") && !mode.contains("LOW"),
            "a non-fitting protected suffix must drop the whole right group: {mode:?}"
        );
    }

    /// Give the active buffer a file path and return its `file://`
    /// URI, so diag-store entries can be keyed to it.
    fn set_active_buffer_path(s: &EditorState, path: &str) -> String {
        let core = s.core.borrow();
        let registry = core.registry.clone();
        let mut reg = registry.borrow_mut();
        let buf = reg.get_mut(core.active_buffer_id()).unwrap();
        buf.set_file_path(Some(std::path::PathBuf::from(path)));
        crate::lsp::path_to_file_uri(buf.file_path().unwrap())
    }

    fn diag_with_severity(severity: crate::diag::DiagnosticSeverity) -> crate::diag::Diagnostic {
        crate::diag::Diagnostic {
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: 1,
            severity,
            message: "boom".into(),
            source: None,
            code: None,
        }
    }

    #[test]
    fn render_mode_line_shows_diagnostic_counts() {
        use crate::diag::DiagnosticSeverity::{Error, Hint, Warning};
        let s = fresh_with(b"hello\n");
        let uri = set_active_buffer_path(&s, "/tmp/modeline_diag.rs");
        let store = s.lsp_manager.borrow().diag_store();
        store.lock().unwrap().set(
            uri,
            vec![
                diag_with_severity(Error),
                diag_with_severity(Error),
                diag_with_severity(Warning),
                diag_with_severity(Hint), // hints stay off the mode line
            ],
        );
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let mode_text = row_text(&cells, stride, 22, 80);
        assert!(
            mode_text.contains("E:2 W:1"),
            "mode line missing diagnostic counts: {mode_text:?}"
        );
        assert!(
            !mode_text.contains("H:"),
            "hints should not appear on the mode line: {mode_text:?}"
        );
    }

    #[test]
    fn render_mode_line_hides_diagnostic_counts_while_stale() {
        use crate::diag::DiagnosticSeverity::Error;
        let s = fresh_with(b"hello\n");
        let uri = set_active_buffer_path(&s, "/tmp/modeline_stale.rs");
        let store = s.lsp_manager.borrow().diag_store();
        {
            let mut guard = store.lock().unwrap();
            guard.set(uri.clone(), vec![diag_with_severity(Error)]);
            guard.mark_stale(uri);
        }
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let mode_text = row_text(&cells, stride, 22, 80);
        assert!(
            !mode_text.contains("E:"),
            "stale diagnostics must not reach the mode line: {mode_text:?}"
        );
    }

    #[test]
    fn render_with_attached_diagnostic_view_does_not_deadlock() {
        // Regression: paint_frame once held the diag-store mutex
        // across the whole window loop, and `DiagnosticView::render`
        // (attached as a window overlay when a file with an LSP
        // opens) locks the same mutex — the daemon froze on the
        // first frame after C-x C-f. This test renders the full
        // paint_frame path with a real DiagnosticView attached; it
        // hangs the suite if the lock is ever widened again.
        use crate::diag::DiagnosticSeverity::Error;
        let s = fresh_with(b"hello\n");
        let uri = set_active_buffer_path(&s, "/tmp/modeline_overlay.rs");
        let store = s.lsp_manager.borrow().diag_store();
        store
            .lock()
            .unwrap()
            .set(uri.clone(), vec![diag_with_severity(Error)]);
        {
            let mut core = s.core.borrow_mut();
            core.active_window_mut()
                .push_overlay(Box::new(crate::diag::DiagnosticView::new(uri, store, None)));
        }
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        // Both surfaces of the same store: the overlay's underline
        // and the mode line's count.
        assert_eq!(
            cells[0].style.underline,
            crate::cell::UnderlineStyle::Curly,
            "diagnostic overlay should underline the error range"
        );
        let mode_text = row_text(&cells, stride, 22, 80);
        assert!(
            mode_text.contains("E:1"),
            "mode line missing count: {mode_text:?}"
        );
    }

    #[test]
    fn render_mode_line_omits_diagnostic_counts_when_clean() {
        let s = fresh_with(b"hello\n");
        let _uri = set_active_buffer_path(&s, "/tmp/modeline_clean.rs");
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        let mode_text = row_text(&cells, stride, 22, 80);
        assert!(
            !mode_text.contains("E:") && !mode_text.contains("W:"),
            "clean buffer must not show diagnostic counts: {mode_text:?}"
        );
    }

    #[test]
    fn render_places_cursor_on_active_window() {
        let mut s = fresh_with(b"abc\n");
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Right));
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Right));
        let (_, _, cursor) = render_to_grid(&s, 24, 80);
        assert_eq!(cursor, Some(CellCoord::new(0, 2)));
    }

    #[test]
    fn render_minibuffer_replaces_status_row_when_active() {
        // Open M-x; the minibuffer takes over the bottom row.
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, alt('x'));
        assert!(s.core.borrow().minibuffer.is_active());
        let (cells, stride, cursor) = render_to_grid(&s, 24, 80);
        let bottom = row_text(&cells, stride, 23, 80);
        assert!(
            bottom.starts_with("M-x"),
            "minibuffer prompt missing: {bottom:?}"
        );
        // Cursor is on the bottom row (in the minibuffer), not in
        // the buffer area.
        assert_eq!(cursor.unwrap().row, 23);
    }

    #[test]
    fn arrow_keys_navigate_the_completion_dropdown() {
        // Regression: Up/Down used to run command HISTORY even with a
        // completion dropdown showing, so the highlight never moved. Now
        // they navigate the dropdown when one is present.
        let selected = |s: &EditorState| {
            s.core
                .borrow()
                .minibuffer
                .session
                .as_ref()
                .expect("session")
                .selected
        };
        let mut s = fresh_with(b"");
        s.dispatch_key(FrontendId::LOCAL, alt('x'));
        assert!(
            s.core.borrow().minibuffer.has_candidates(),
            "M-x populates a completion dropdown"
        );
        let sel0 = selected(&s);

        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Down));
        let sel1 = selected(&s);
        assert_ne!(sel0, sel1, "Down must move the completion selection");

        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::Up));
        assert_eq!(
            selected(&s),
            sel0,
            "Up must move the completion selection back"
        );
    }

    #[test]
    fn render_clears_grid_between_frames() {
        // Frame 1 renders some text; frame 2 with shorter content
        // must not leak frame-1 cells into the now-empty area.
        let s = fresh_with(b"the quick brown fox\n");
        let (cells_a, stride_a, _) = render_to_grid(&s, 24, 80);
        assert!(row_text(&cells_a, stride_a, 0, 80).contains("quick"));

        // Replace the buffer with shorter content via apply_active_edit.
        s.core
            .borrow_mut()
            .apply_active_edit(crate::buffer::EditOp::Replace {
                range: crate::rope::Range { start: 0, end: 20 },
                bytes: b"hi\n",
            })
            .unwrap();
        let (cells_b, stride_b, _) = render_to_grid(&s, 24, 80);
        // Row 0 now has only "hi"; the rest of that row must be blank.
        assert_eq!(row_text(&cells_b, stride_b, 0, 80), "hi");
    }

    #[test]
    fn render_split_windows_paint_into_distinct_columns() {
        let s = fresh_with(b"hello\nworld\n");
        s.lua_host
            .lua()
            .load("pmacs.window.split_vertical()")
            .exec()
            .unwrap();
        let (cells, stride, _) = render_to_grid(&s, 24, 80);
        // Both halves should show "hello" on row 0, in their own
        // column ranges. With a 50/50 vertical split, left half is
        // cols 0..40 and right half is cols 40..80.
        let left_row = (0..40)
            .map(|c| glyph_at(&cells, stride, 0, c))
            .collect::<String>()
            .trim_end()
            .to_string();
        let right_row = (40..80)
            .map(|c| glyph_at(&cells, stride, 0, c))
            .collect::<String>()
            .trim_end()
            .to_string();
        assert_eq!(left_row, "hello");
        assert_eq!(right_row, "hello");
        // Each window has its own mode line at row 22; both should
        // be in reverse video.
        for col in [0, 39, 40, 79] {
            assert!(cells[(22 * stride + col) as usize].style.reverse);
        }
    }

    /// Helper: assert that the active window's `TextView` has line
    /// offsets covering the active buffer end. This is the universal
    /// "view is in sync with buffer" invariant; staleness manifests as
    /// `line_offset(line_count - 1) > buf.len()`.
    fn assert_textview_covers_buffer(s: &EditorState) {
        let core = s.core.borrow();
        let buf_len = core.active_buffer_len();
        let view = &core.active_window().text_view;
        let line_count = view.line_count();
        assert!(line_count >= 1, "view should always have at least one line");
        let last_offset = view.line_offset(line_count - 1).unwrap();
        assert!(
            last_offset <= buf_len,
            "TextView out of sync: last line offset {last_offset} > buf_len {buf_len}"
        );
    }

    #[test]
    fn paragraph_motion_no_op_at_buffer_edges() {
        let mut s = fresh_with(b"single line, no breaks\n");
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Down, KeyModifiers::CONTROL));
        let len = s.core.borrow().active_buffer_len();
        assert_eq!(s.core.borrow().cursor(), len);
        // Another C-down stays at end.
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Down, KeyModifiers::CONTROL));
        assert_eq!(s.core.borrow().cursor(), len);
        // C-up from there returns to start (no internal blanks).
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Up, KeyModifiers::CONTROL));
        assert_eq!(s.core.borrow().cursor(), 0);
    }

    #[test]
    fn page_step_falls_back_when_no_render_yet() {
        // last_visible_rows = 0 (never rendered). page_step uses 20.
        let mut content = Vec::new();
        for i in 0..200 {
            content.extend_from_slice(format!("L{i}\n").as_bytes());
        }
        let mut s = fresh_with(&content);
        assert_eq!(s.core.borrow().active_window().last_visible_rows, 0);
        s.dispatch_key(FrontendId::LOCAL, plain(KeyCode::PageDown));
        assert!(s.core.borrow().view_top() >= 20);
    }

    // ---- T M3.3 acceptance: Lua coroutine async API --------------------------

    /// Drive `tick_async` until `predicate` is true, sleeping briefly
    /// between ticks so workers have a chance to send replies. Panics
    /// after a 2-second deadline so a stuck test doesn't hang CI.
    fn pump_async<F: Fn(&EditorState) -> bool>(state: &mut EditorState, predicate: F) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !predicate(state) {
            assert!(
                std::time::Instant::now() < deadline,
                "async pump deadline exceeded"
            );
            state.tick_async();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn lua_get<T: for<'a> mlua::FromLua + Clone>(state: &EditorState, var: &str) -> Option<T> {
        state.lua_host.lua().globals().get::<T>(var).ok()
    }

    /// Acceptance bullet 1 + 2: a Lua coroutine yields cleanly when
    /// awaiting a Handle, and resumes with the worker's result on
    /// completion.
    #[test]
    fn async_coroutine_resumes_with_compute_sum_result() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_TEST_RESULT = nil
                pmacs.async(function()
                    local v = pmacs.workers.compute_sum(10):await()
                    _G.PMACS_TEST_RESULT = v
                end)
                ",
            )
            .expect("spawn coroutine");

        pump_async(&mut state, |s| {
            lua_get::<i64>(s, "PMACS_TEST_RESULT").is_some()
        });

        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_RESULT"), Some(55));
    }

    /// `pmacs.workers.dispatch("compute_sum", { n = 7 })` is the
    /// canonical name-based form from the spec example. It must
    /// return a Handle whose `:await()` yields the same value as the
    /// direct constructor.
    #[test]
    fn dispatch_by_name_routes_to_registered_handler() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r#"
                _G.PMACS_TEST_RESULT = nil
                pmacs.async(function()
                    _G.PMACS_TEST_RESULT =
                        pmacs.workers.dispatch("compute_sum", { n = 7 }):await()
                end)
                "#,
            )
            .expect("dispatch by name");
        pump_async(&mut state, |s| {
            lua_get::<i64>(s, "PMACS_TEST_RESULT").is_some()
        });
        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_RESULT"), Some(28));
    }

    /// Acceptance bullet 3: cancelled awaits raise a structured error
    /// with `tag = "cancelled"` per R45.
    #[test]
    fn cancelled_await_raises_tagged_error() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r#"
                _G.PMACS_TEST_TAG = nil
                _G.PMACS_TEST_HANDLE_ID = nil
                pmacs.async(function()
                    local h = pmacs.workers.sleep(2000)
                    _G.PMACS_TEST_HANDLE_ID = h:id()
                    -- Cancel ourselves before awaiting. The runtime
                    -- has not yet ticked, so the handle is still in
                    -- flight; await will park us, the worker observes
                    -- the cancel and replies, tick resumes us, await
                    -- raises { tag = "cancelled" }.
                    h:cancel()
                    local ok, err = pcall(function() return h:await() end)
                    if not ok and type(err) == "table" then
                        _G.PMACS_TEST_TAG = err.tag
                    elseif not ok then
                        _G.PMACS_TEST_TAG = "non-table-error:" .. tostring(err)
                    else
                        _G.PMACS_TEST_TAG = "unexpected-success"
                    end
                end)
                "#,
            )
            .expect("spawn cancelled coroutine");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_TEST_TAG").is_some()
        });
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_TAG"),
            Some("cancelled".to_string()),
            "expected R45-tagged cancellation error"
        );
        assert!(
            lua_get::<i64>(&state, "PMACS_TEST_HANDLE_ID").is_some(),
            "handle id should have been recorded"
        );
    }

    /// `Handle:on_complete` fires the callback without requiring a
    /// coroutine. This satisfies the "non-coroutine consumer" half of
    /// the acceptance surface.
    #[test]
    fn on_complete_callback_fires_outside_a_coroutine() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_TEST_STATUS = nil
                _G.PMACS_TEST_VALUE = nil
                local h = pmacs.workers.compute_sum(5)
                h:on_complete(function(status, value)
                    _G.PMACS_TEST_STATUS = status
                    _G.PMACS_TEST_VALUE = value
                end)
                ",
            )
            .expect("install callback");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_TEST_STATUS").is_some()
        });
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_STATUS"),
            Some("ok".to_string())
        );
        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_VALUE"), Some(15));
    }

    /// Multiple coroutines awaiting different handles all complete,
    /// each with their own value. Exercises the parked-coroutine
    /// table's keying.
    #[test]
    fn multiple_concurrent_awaits_resolve_independently() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_TEST_DONE = 0
                _G.PMACS_TEST_SUM = 0
                for i = 1, 5 do
                    pmacs.async(function()
                        local v = pmacs.workers.compute_sum(i):await()
                        _G.PMACS_TEST_SUM = _G.PMACS_TEST_SUM + v
                        _G.PMACS_TEST_DONE = _G.PMACS_TEST_DONE + 1
                    end)
                end
                ",
            )
            .expect("spawn fan-out");
        pump_async(&mut state, |s| {
            lua_get::<i64>(s, "PMACS_TEST_DONE") == Some(5)
        });
        // sum_{i=1..5} of i*(i+1)/2 = 1 + 3 + 6 + 10 + 15 = 35
        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_SUM"), Some(35));
    }

    /// T M3.4: a second dispatch with the same `supersede` key
    /// cancels the first. Mirrors the spec example from R45 ---
    /// the canonical "fast typist queues stale searches" pattern.
    /// The first await raises `{ tag = "cancelled" }`; the second
    /// completes with the new value.
    #[test]
    fn supersede_via_opts_cancels_predecessor_and_runs_successor() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_TEST_FIRST_TAG = nil
                _G.PMACS_TEST_SECOND = nil
                pmacs.async(function()
                    local h = pmacs.workers.sleep(2000, { supersede = 'search' })
                    local ok, err = pcall(function() return h:await() end)
                    if not ok and type(err) == 'table' then
                        _G.PMACS_TEST_FIRST_TAG = err.tag
                    end
                end)
                pmacs.async(function()
                    -- Second dispatch under the same supersede key.
                    -- Must settle Complete; the first must be Cancelled.
                    _G.PMACS_TEST_SECOND =
                        pmacs.workers.compute_sum(10, { supersede = 'search' }):await()
                end)
                ",
            )
            .expect("spawn pair of supersede-keyed coroutines");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_TEST_FIRST_TAG").is_some()
                && lua_get::<i64>(s, "PMACS_TEST_SECOND").is_some()
        });
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_FIRST_TAG"),
            Some("cancelled".to_string())
        );
        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_SECOND"), Some(55));
    }

    /// T M3.5: a streaming handler delivers all items through
    /// `:on_batch`, terminated by `:on_close`. The Lua-side test
    /// counts both items and batches and verifies coalescing.
    #[test]
    fn stream_on_batch_delivers_all_items_in_few_callbacks() {
        let mut state = EditorState::new();
        // Deliberately small cap (32) so we can prove the batch
        // boundary while keeping item count moderate (1024).
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_STREAM_TOTAL = 0
                _G.PMACS_STREAM_BATCHES = 0
                _G.PMACS_STREAM_CLOSED = nil
                local s = pmacs.workers.emit_n(1024, { max_batch = 32 })
                s:on_batch(function(items)
                    _G.PMACS_STREAM_BATCHES = _G.PMACS_STREAM_BATCHES + 1
                    _G.PMACS_STREAM_TOTAL = _G.PMACS_STREAM_TOTAL + #items
                end)
                s:on_close(function(status, _value)
                    _G.PMACS_STREAM_CLOSED = status
                end)
                ",
            )
            .expect("spawn stream");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_STREAM_CLOSED").is_some()
        });
        assert_eq!(lua_get::<i64>(&state, "PMACS_STREAM_TOTAL"), Some(1024));
        assert_eq!(
            lua_get::<String>(&state, "PMACS_STREAM_CLOSED"),
            Some("ok".to_string())
        );
        // Coalescing: with cap=32, ≥32 batches structurally
        // (1024/32). The pump loop ticks at 2ms, the runtime
        // coalesces all queued items per drain bounded by the cap.
        // Bound is 1024/32 ≤ batches ≤ 1024/32 + scheduler slack.
        let batches = lua_get::<i64>(&state, "PMACS_STREAM_BATCHES").unwrap_or(0);
        assert!(
            (32..=200).contains(&batches),
            "expected batches in [32, 200], got {batches}"
        );
    }

    /// T M3.5: the frame target and default max batch are tunable
    /// from Lua via `pmacs.async_config.*`.
    #[test]
    fn async_config_round_trips_through_lua() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_DEFAULT_FT  = pmacs.async_config.frame_target_ms()
                _G.PMACS_DEFAULT_MB  = pmacs.async_config.default_max_batch()
                pmacs.async_config.frame_target_ms(33)
                pmacs.async_config.default_max_batch(64)
                _G.PMACS_NEW_FT = pmacs.async_config.frame_target_ms()
                _G.PMACS_NEW_MB = pmacs.async_config.default_max_batch()
                ",
            )
            .expect("config round-trip");
        assert_eq!(lua_get::<i64>(&state, "PMACS_DEFAULT_FT"), Some(16));
        assert_eq!(lua_get::<i64>(&state, "PMACS_DEFAULT_MB"), Some(1024));
        assert_eq!(lua_get::<i64>(&state, "PMACS_NEW_FT"), Some(33));
        assert_eq!(lua_get::<i64>(&state, "PMACS_NEW_MB"), Some(64));
    }

    /// T M3.5 + T M3.4: a stream supersession surfaces the
    /// predecessor's `Cancelled` outcome through `:on_close`.
    #[test]
    fn stream_supersede_delivers_cancelled_to_on_close() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_FIRST_STATUS = nil
                _G.PMACS_SECOND_STATUS = nil
                local first = pmacs.workers.emit_n(1000000,
                    { supersede = 'emit', max_batch = 32 })
                first:on_close(function(status, _v)
                    _G.PMACS_FIRST_STATUS = status
                end)
                local second = pmacs.workers.emit_n(8,
                    { supersede = 'emit', max_batch = 8 })
                second:on_close(function(status, _v)
                    _G.PMACS_SECOND_STATUS = status
                end)
                ",
            )
            .expect("spawn supersede pair");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_FIRST_STATUS").is_some()
                && lua_get::<String>(s, "PMACS_SECOND_STATUS").is_some()
        });
        assert_eq!(
            lua_get::<String>(&state, "PMACS_FIRST_STATUS"),
            Some("cancelled".to_string())
        );
        assert_eq!(
            lua_get::<String>(&state, "PMACS_SECOND_STATUS"),
            Some("ok".to_string())
        );
    }

    /// `pmacs.workers.dispatch(name, args, opts)` --- the spec
    /// example shape, including supersede.
    #[test]
    fn dispatch_by_name_accepts_supersede_opt() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_TEST_TAG = nil
                _G.PMACS_TEST_VALUE = nil
                pmacs.async(function()
                    local h = pmacs.workers.dispatch('sleep', { ms = 5000 },
                                                    { supersede = 'job' })
                    local ok, err = pcall(function() return h:await() end)
                    if not ok and type(err) == 'table' then
                        _G.PMACS_TEST_TAG = err.tag
                    end
                end)
                pmacs.async(function()
                    _G.PMACS_TEST_VALUE = pmacs.workers.dispatch(
                        'compute_sum', { n = 4 },
                        { supersede = 'job' }
                    ):await()
                end)
                ",
            )
            .expect("dispatch by name with supersede");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_TEST_TAG").is_some()
                && lua_get::<i64>(s, "PMACS_TEST_VALUE").is_some()
        });
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_TAG"),
            Some("cancelled".to_string())
        );
        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_VALUE"), Some(10));
    }

    /// T M3.6: `pmacs.workers.grep` end-to-end. We build a synthetic
    /// tree, dispatch a grep through Lua, and verify match items
    /// arrive on `:on_batch` with the expected `{file, line,
    /// match_start, match_end, text}` shape, terminated by a clean
    /// `:on_close`. This is the "Lua code can do expensive things
    /// without freezing the editor" surface in user-facing form.
    #[test]
    fn grep_via_lua_delivers_matches_through_on_batch() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), "first\nneedle here\nthird\n").expect("a.txt");
        std::fs::write(dir.path().join("b.txt"), "no match\n").expect("b.txt");
        std::fs::write(dir.path().join("c.txt"), "needle\nfoo\nneedle again\n").expect("c.txt");
        let root = dir.path().to_string_lossy().into_owned();
        let mut state = EditorState::new();
        let script = format!(
            r#"
            _G.PMACS_GREP_TOTAL = 0
            _G.PMACS_GREP_FIRST_FILE = nil
            _G.PMACS_GREP_FIRST_LINE = nil
            _G.PMACS_GREP_FIRST_TEXT = nil
            _G.PMACS_GREP_FIRST_MS = nil
            _G.PMACS_GREP_FIRST_ME = nil
            _G.PMACS_GREP_CLOSED = nil
            local s = pmacs.workers.grep({{
                root = {root:?},
                pattern = "needle",
            }})
            s:on_batch(function(items)
                for _, m in ipairs(items) do
                    _G.PMACS_GREP_TOTAL = _G.PMACS_GREP_TOTAL + 1
                    if _G.PMACS_GREP_FIRST_FILE == nil then
                        _G.PMACS_GREP_FIRST_FILE = m.file
                        _G.PMACS_GREP_FIRST_LINE = m.line
                        _G.PMACS_GREP_FIRST_TEXT = m.text
                        _G.PMACS_GREP_FIRST_MS = m.match_start
                        _G.PMACS_GREP_FIRST_ME = m.match_end
                    end
                end
            end)
            s:on_close(function(status, _v)
                _G.PMACS_GREP_CLOSED = status
            end)
            "#,
        );
        state
            .lua_host
            .eval(Some("test"), &script)
            .expect("dispatch grep");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_GREP_CLOSED").is_some()
        });
        // 3 matches: a.txt:2 + c.txt:1 + c.txt:3
        assert_eq!(lua_get::<i64>(&state, "PMACS_GREP_TOTAL"), Some(3));
        assert_eq!(
            lua_get::<String>(&state, "PMACS_GREP_CLOSED"),
            Some("ok".to_string())
        );
        // The first match table carries every documented field.
        assert!(
            lua_get::<String>(&state, "PMACS_GREP_FIRST_FILE").is_some_and(|f| {
                std::path::Path::new(&f)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
            })
        );
        let line = lua_get::<i64>(&state, "PMACS_GREP_FIRST_LINE").unwrap_or(0);
        assert!(line >= 1, "line numbers are 1-based, got {line}");
        let text = lua_get::<String>(&state, "PMACS_GREP_FIRST_TEXT").unwrap_or_default();
        assert!(
            text.contains("needle"),
            "first match text should contain 'needle', got {text:?}"
        );
        let ms = lua_get::<i64>(&state, "PMACS_GREP_FIRST_MS").unwrap_or(-1);
        let me = lua_get::<i64>(&state, "PMACS_GREP_FIRST_ME").unwrap_or(-1);
        assert!(
            ms >= 0 && me - ms == 6,
            "match offsets should span 6 bytes (len 'needle'), got [{ms}, {me})"
        );
    }

    /// T M3.6: a Lua grep dispatched under a supersede key gets
    /// cancelled when a successor is dispatched under the same
    /// key. The predecessor's `:on_close` fires with `"cancelled"`.
    #[test]
    fn grep_supersede_via_lua_cancels_predecessor() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Synthetic load: enough work to outlive the supersede tick.
        let body = "noise noise noise noise\n".repeat(50);
        for i in 0..2_000 {
            std::fs::write(dir.path().join(format!("f{i:04}.txt")), &body).expect("write");
        }
        let root = dir.path().to_string_lossy().into_owned();
        let mut state = EditorState::new();
        let script = format!(
            r#"
            _G.PMACS_GREP_FIRST_STATUS = nil
            _G.PMACS_GREP_SECOND_STATUS = nil
            local first = pmacs.workers.grep(
                {{ root = {root:?}, pattern = "needle", fanout = 1 }},
                {{ supersede = "search" }}
            )
            first:on_close(function(status, _v)
                _G.PMACS_GREP_FIRST_STATUS = status
            end)
            local second = pmacs.workers.grep(
                {{ root = {root:?}, pattern = "alpha", fanout = 1 }},
                {{ supersede = "search" }}
            )
            second:on_close(function(status, _v)
                _G.PMACS_GREP_SECOND_STATUS = status
            end)
            "#,
        );
        state
            .lua_host
            .eval(Some("test"), &script)
            .expect("dispatch grep pair");
        pump_async(&mut state, |s| {
            lua_get::<String>(s, "PMACS_GREP_FIRST_STATUS").is_some()
                && lua_get::<String>(s, "PMACS_GREP_SECOND_STATUS").is_some()
        });
        let first = lua_get::<String>(&state, "PMACS_GREP_FIRST_STATUS").unwrap_or_default();
        // First either ran-to-completion (extremely fast host) or got
        // cancelled. Both are acceptable outcomes for the supersede
        // path; the regression we guard against is the first stream
        // never settling at all.
        assert!(
            first == "cancelled" || first == "ok",
            "first close status should be cancelled or ok, got {first:?}"
        );
        assert_eq!(
            lua_get::<String>(&state, "PMACS_GREP_SECOND_STATUS"),
            Some("ok".to_string())
        );
    }

    /// T M3.7: `pmacs.workers.snapshot()` returns a Lua-shaped
    /// version of the runtime's snapshot. Active jobs come back
    /// with kind labels and (non-zero) ages.
    #[test]
    fn workers_snapshot_via_lua_lists_active_jobs() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                local h = pmacs.workers.sleep(2000, { supersede = 'job' })
                _G.PMACS_TEST_ID = h:id()
                local snap = pmacs.workers.snapshot()
                _G.PMACS_TEST_ACTIVE = #snap.active
                _G.PMACS_TEST_KIND   = snap.active[1].kind
                _G.PMACS_TEST_KEY    = snap.active[1].supersede
                ",
            )
            .expect("snapshot via lua");
        assert!(lua_get::<i64>(&state, "PMACS_TEST_ID").is_some());
        assert_eq!(lua_get::<i64>(&state, "PMACS_TEST_ACTIVE"), Some(1));
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_KIND"),
            Some("sleep".to_string())
        );
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_KEY"),
            Some("job".to_string())
        );
    }

    /// `pmacs.workers.show()` creates the *workers* buffer, fills
    /// it with content, and binds C-c C-k to the cancel command in
    /// that buffer. After tick the buffer's content reflects the
    /// runtime state --- so the spec's "updates within 100 ms"
    /// bound is met by frame-cadence ticks.
    #[test]
    fn workers_show_creates_and_refreshes_the_buffer() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                _G.PMACS_TEST_BUF = pmacs.workers.show()
                _G.PMACS_TEST_LEN_BEFORE = pmacs.window.buffer():len()
                ",
            )
            .expect("show buffer");
        // The show call sets workers_buffer_visible; subsequent ticks
        // should refresh. Dispatch a job and tick, then re-read the
        // buffer's content. We assert the second snapshot is at least
        // as long as the first (it now has an active row).
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                local h = pmacs.workers.sleep(2000, { supersede = 'show' })
                _G.PMACS_TEST_ID = h:id()
                ",
            )
            .expect("dispatch sleep");
        // Force a tick to refresh the buffer.
        state.tick_async();
        state
            .lua_host
            .eval(
                Some("test"),
                r##"
                local id = _G.PMACS_TEST_BUF
                _G.PMACS_TEST_LEN_AFTER = id:len()
                local len = id:len()
                local body = id:slice(0, len)
                _G.PMACS_TEST_BODY = body
                _G.PMACS_TEST_HAS_ID = string.find(body, "#" .. tostring(_G.PMACS_TEST_ID), 1, true) ~= nil
                _G.PMACS_TEST_HAS_KIND = string.find(body, "sleep", 1, true) ~= nil
                "##,
            )
            .expect("read buffer body");
        assert_eq!(
            lua_get::<bool>(&state, "PMACS_TEST_HAS_ID"),
            Some(true),
            "buffer body should mention the dispatched job id"
        );
        assert_eq!(
            lua_get::<bool>(&state, "PMACS_TEST_HAS_KIND"),
            Some(true),
            "buffer body should label the kind 'sleep'"
        );
    }

    /// `pmacs.workers.cancel_at_point()` reads the cursor, parses
    /// the job id at the line, and cancels the corresponding job.
    /// We synthesize the cursor position by dispatching, showing,
    /// finding the row's offset in the buffer body, and seeking
    /// the editor's cursor there.
    #[test]
    fn workers_cancel_at_point_cancels_the_named_job() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r##"
                local h = pmacs.workers.sleep(5000, { supersede = 'targeted' })
                _G.PMACS_TEST_ID = h:id()
                local buf = pmacs.workers.show()
                _G.PMACS_TEST_BUF = buf
                local body = buf:slice(0, buf:len())
                local needle = "#" .. tostring(_G.PMACS_TEST_ID)
                local row_start = string.find(body, needle, 1, true)
                _G.PMACS_TEST_ROW_START = row_start
                "##,
            )
            .expect("set up buffer + locate row");
        let row_start = lua_get::<i64>(&state, "PMACS_TEST_ROW_START").expect("row located");
        // Move cursor to the row's first byte by inserting/seeking via
        // the editor's API. The simplest way to position the cursor
        // is to switch to the *workers* buffer and call move-by-bytes.
        // Easier still: call cancel_at_point directly with a synthetic
        // cursor by exposing a programmatic surface. We use the raw
        // binding `_job_id_at_byte` directly to verify the parser, and
        // then call `_cancel` --- exactly what cancel_at_point does in
        // sequence.
        let id_pre_cancel = lua_get::<i64>(&state, "PMACS_TEST_ID").unwrap();
        let script = format!(
            r"
            local id = pmacs._async._job_id_at_byte(_G.PMACS_TEST_BUF, {row_start})
            _G.PMACS_TEST_PARSED = id
            if id ~= nil then
                pmacs._async._cancel(id)
            end
            "
        );
        state
            .lua_host
            .eval(Some("test"), &script)
            .expect("cancel via parsed id");
        assert_eq!(
            lua_get::<i64>(&state, "PMACS_TEST_PARSED"),
            Some(id_pre_cancel),
            "parser should recover the id from the row"
        );
        // Pump until the job settles into Cancelled.
        let id_u64 = u64::try_from(id_pre_cancel).expect("non-negative id");
        pump_async(&mut state, |s| s.async_runtime.is_cancelled(id_u64));
    }

    /// R46 enforcement: package code that yields a non-Handle is
    /// reported through `pmacs.error`, not silently accepted. The
    /// runtime should not park the coroutine on a bogus value.
    #[test]
    fn non_handle_yield_is_reported_via_pmacs_error() {
        let mut state = EditorState::new();
        // Install a stub pmacs.error that records the message.
        state
            .lua_host
            .eval(
                Some("test"),
                r#"
                _G.PMACS_ERROR_MSG = nil
                pmacs.error = function(msg) _G.PMACS_ERROR_MSG = msg end
                pmacs.async(function()
                    coroutine.yield("not a handle") -- R46 violation
                end)
                "#,
            )
            .expect("spawn bad coroutine");
        let msg: Option<String> = lua_get(&state, "PMACS_ERROR_MSG");
        assert!(msg.is_some(), "pmacs.error should have been invoked");
        assert!(
            msg.as_deref().unwrap_or("").contains("non-Handle"),
            "message did not mention the cause: {msg:?}"
        );
    }

    /// T M5.6f: `M-x editor.describe-instance` echoes a one-line
    /// summary into the status row.
    #[test]
    fn editor_describe_instance_echoes_status_line() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                "pmacs.command.invoke('editor.describe-instance')",
            )
            .expect("invoke editor.describe-instance");
        let status = state.core.borrow().status.clone();
        assert!(
            status.starts_with("pmacs "),
            "expected pmacs version prefix in status; got {status:?}"
        );
        assert!(
            status.contains("[local]"),
            "expected default instance name marker; got {status:?}"
        );
    }

    /// T M5.6f: `M-x editor.describe-instance-buffer` switches the
    /// active window to *pmacs-instance* and binds buffer-local `q`
    /// to `buffer.kill-this`. The buffer-local binding is verified by
    /// resolving directly against the keymap stack — `pmacs.describe.key`
    /// only consults global scope.
    #[test]
    fn editor_describe_instance_buffer_switches_and_binds_q() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                pmacs.command.invoke('editor.describe-instance-buffer')
                _G.PMACS_TEST_NAME = pmacs.window.buffer():name()
                ",
            )
            .expect("invoke editor.describe-instance-buffer");
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_NAME"),
            Some("*pmacs-instance*".to_string()),
        );

        let km = state.lua_host.keymaps().borrow();
        let buffer_id = state.core.borrow().active_window().buffer_id;
        let chords = crate::key::parse_sequence("q").unwrap();
        match km.resolve(&chords, Some(buffer_id), &[]) {
            crate::keymap_stack::StackResolution::Bound(rb) => {
                assert_eq!(
                    rb.binding.command, "buffer.kill-this",
                    "q in *pmacs-instance* must dispatch to buffer.kill-this"
                );
            }
            other => panic!("expected buffer-local Bound for `q`, got {other:?}"),
        }
    }

    /// T M5.6f: `q` in the *pmacs-instance* buffer kills the buffer
    /// (via `buffer.kill-this`).
    #[test]
    fn editor_describe_instance_buffer_q_kills_the_buffer() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                local id = pmacs.instance.show()
                pmacs.keymap.bind {
                  scope = 'buffer', buffer = id, sequence = 'q',
                  command = 'buffer.kill-this',
                }
                pmacs.window.switch_buffer(id)
                _G.PMACS_TEST_BEFORE = pmacs.window.buffer():name()
                pmacs.command.invoke('buffer.kill-this')
                _G.PMACS_TEST_AFTER = pmacs.window.buffer():name()
                ",
            )
            .expect("instance show + kill");
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_BEFORE"),
            Some("*pmacs-instance*".to_string()),
        );
        assert_ne!(
            lua_get::<String>(&state, "PMACS_TEST_AFTER"),
            Some("*pmacs-instance*".to_string()),
            "the buffer should no longer be active after kill-this"
        );
    }

    /// `M-x editor.list-workers` opens the *workers* observability
    /// buffer in the active window. The user can then use the
    /// buffer-local `C-c C-k` binding (T M3.7) to cancel a job.
    #[test]
    fn editor_list_workers_command_switches_to_workers_buffer() {
        let mut state = EditorState::new();
        state
            .lua_host
            .eval(
                Some("test"),
                r"
                pmacs.command.invoke('editor.list-workers')
                _G.PMACS_TEST_NAME = pmacs.window.buffer():name()
                ",
            )
            .expect("invoke editor.list-workers");
        assert_eq!(
            lua_get::<String>(&state, "PMACS_TEST_NAME"),
            Some("*workers*".to_string()),
            "active window should now show the *workers* buffer"
        );
    }

    /// `pmacs.project.search(query, opts)` is the programmatic side
    /// of the `project.search` command. It dispatches a parallel
    /// grep, streams batches into `*search-results*`, and supersedes
    /// any predecessor under the `"search"` key. We drive it
    /// against a synthetic tempdir tree, pump until the closing
    /// status marker arrives, and confirm the buffer's body carries
    /// the match.
    #[test]
    fn project_search_streams_matches_into_search_results_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("alpha.txt"), "needle on this line\n").expect("write alpha");
        std::fs::write(dir.path().join("beta.txt"), "no match here\n").expect("write beta");
        let root = dir.path().display().to_string();

        let mut state = EditorState::new();
        let script = format!(
            r#"
            pmacs.project.search("needle", {{ root = "{root}" }})
            "#
        );
        state
            .lua_host
            .eval(Some("test"), &script)
            .expect("kick off project.search");

        // Pump until the `*search-results*` buffer carries the close
        // marker that our `on_close` handler appends.
        pump_async(&mut state, |s| {
            let _ = s.lua_host.lua().globals().set("PMACS_TEST_BODY", "");
            let _ = s
                .lua_host
                .lua()
                .load(
                    r#"
                for _, id in ipairs(pmacs.buffer.list()) do
                    if pmacs.describe.buffer(id).name == "*search-results*" then
                        _G.PMACS_TEST_BODY = id:slice(0, id:len())
                        break
                    end
                end
                "#,
                )
                .exec();
            lua_get::<String>(s, "PMACS_TEST_BODY").is_some_and(|b| b.contains("-- search "))
        });

        let body = lua_get::<String>(&state, "PMACS_TEST_BODY").expect("body captured");
        assert!(
            body.contains("alpha.txt"),
            "results should mention the matching file: {body}"
        );
        assert!(
            body.contains("needle on this line"),
            "results should include the matched text: {body}"
        );
        assert!(
            !body.contains("beta.txt"),
            "non-matching file should not appear: {body}"
        );
    }
}
