// lua_bindings.rs --- Hand-curated Lua surface (R51).

//! The Rust/Lua boundary surface: `pmacs.buffer.*`, `pmacs.command.*`,
//! and `pmacs.describe.*`.
//!
//! Per R51 we do not auto-derive `UserData` on core types --- the Lua
//! API is its own design rather than a leaked Rust shape. Bindings live
//! here; core types stay where they belong.
//!
//! # Lifetime contracts (R53)
//!
//! * [`BufferIdLua`] is a `Copy` handle. Lua may store, pass, and re-use
//!   it freely, but the underlying [`Buffer`] lives in the registry. If
//!   the buffer is removed (`pmacs.buffer.remove(id)`), all live handles
//!   become stale; the next method call on a stale handle returns a
//!   typed error ([`BindingError::StaleId`]). There is never a
//!   use-after-free.
//! * The registry itself lives behind a [`SharedRegistry`]
//!   (`Rc<RefCell<BufferRegistry>>`). The single-threaded main-thread
//!   invariant means borrow conflicts only arise from re-entrant Lua
//!   calls; those are caller bugs and will surface as panics from
//!   [`RefCell::borrow_mut`]. M2.5+ may revisit if Lua-from-Lua
//!   re-entry becomes a real pattern.
//!
//! # Ownership (R52)
//!
//! Bytes flowing across the boundary are copied. When Lua passes a
//! string to `id:insert`, the Rust side copies the contents into the
//! rope's leaf chunks; the Lua string remains owned by Lua. When Rust
//! returns bytes via `id:slice`, a fresh Lua string is created --- the
//! Rust slice does not escape.
//!
//! # Error mapping (R52, R53)
//!
//! Every Rust error visible to Lua is wrapped via
//! [`mlua::Error::external`]. The structured fields of the original
//! error (e.g. [`RopeError::OutOfBounds { pos, len }`]) are preserved
//! by Display, so `tostring(err)` carries them, and the original error
//! chain is reachable from Rust via `error.source()`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use mlua::{FromLua, Function, Lua, Table, UserData, UserDataMethods, Value, Variadic};
use thiserror::Error;

use std::sync::{Arc, Mutex};

use crate::async_runtime::{
    AsyncRuntime, GrepMatch, GrepSpec, JobOutcome, JobResult, SharedAsyncRuntime, StreamPayload,
};
use crate::buffer::{BufferId, EditOp, MarkGravity, MarkId};
use crate::buffer_registry::BufferRegistry;
use crate::cell::{Color, Style, UnderlineStyle};
use crate::command::{Command, CommandError, CommandRegistry, SourceLocation};
use crate::editor::InteractiveCommandOrigin;
use crate::editor_core::EditorCore;
use crate::highlight::SyntaxHighlightView;
use crate::hook::{Hook, HookRegistry};
use crate::key::{display_sequence, parse_sequence};
use crate::keymap_stack::KeymapStack;
use crate::menu::{MenuItem, MenuRegistry};
use crate::packages::{
    Address, Fetcher, InstallError, InstallPin, InstallScope, InstallSpec, InstalledPackage,
    Installer, LookupOutcome, ResolvedKind, lookup_in_roster,
};
use crate::protocol::{AttachTarget, AttachmentHandle, InstanceIdentity};
use crate::rope::Range;
use crate::statusline::{
    SharedStatuslineRegistry, StatuslineProviderFailure, StatuslineProviderId, StatuslineRegistry,
    StatuslineSide,
};
use crate::syntax::{self, ParseTreeBundle, ParseView, ParseViewHandle, SharedSyntaxRegistry};
use crate::workers_buffer;

// Domain submodules split out of this file (audit F-016). Each owns one
// `pmacs.<domain>` API surface and is installed from the `install()` spine
// / editor wiring below; the shared core (registry alias, `BindingError`,
// `BufferIdLua`, state holders, helpers, and `install()`) stays here.
// Submodules reach shared-core items via `super::` (a child module can see
// its ancestors' private items), so the split needs no visibility widening
// beyond call seams. Public entry points a domain owns are re-exported here
// so external `crate::lua_bindings::<item>` paths (and in-file uses) stay
// stable.
mod config;
mod diag;
mod fold;
mod index;
mod mcp;
// Every `pub` item a moved domain owned is re-exported so its prior
// `crate::lua_bindings::<item>` path still resolves — the split must not
// shrink the public API surface. That includes the `install_*` wiring fns:
// they take crate-internal handle types (so external callers can't invoke
// them), but they were `pub`, so their paths are preserved for
// compile-compatibility; any deliberate narrowing is a separate change.
pub use diag::install_diag;
pub use fold::install_fold;
pub use index::{SharedProjectIndexer, install_project_index, make_project_indexer};
pub use mcp::{McpServerIdLua, install_mcp, make_mcp_manager};

// ---------------------------------------------------------------------------
// Shared registry alias
// ---------------------------------------------------------------------------

/// Shared, single-threaded handle to the editor's buffer registry.
///
/// Held by [`crate::lua::LuaHost`] and by every closure captured during
/// [`install`]. `Rc<RefCell<...>>` is correct for the main-thread
/// invariant: not `Send`, but cheaply cloneable and interior-mutable
/// for the closure soup that mlua's `create_function` produces.
pub type SharedRegistry = Rc<RefCell<BufferRegistry>>;

/// Shared, single-threaded handle to the command registry. Same
/// rationale as [`SharedRegistry`] --- single-thread, interior
/// mutability for closure capture.
pub type SharedCommandRegistry = Rc<RefCell<CommandRegistry>>;

/// Shared, single-threaded handle to the keymap stack.
pub type SharedKeymapStack = Rc<RefCell<KeymapStack>>;

/// Shared handle to the context-menu registry. Cloned into the Lua
/// `pmacs.menu.*` closures and stored as app data alongside the command
/// and keymap registries.
pub type SharedMenuRegistry = Rc<RefCell<MenuRegistry>>;

/// Shared, single-threaded handle to the editor core --- the world
/// state mutated by `pmacs.editor.*` primitives invoked from inside
/// command bodies.
pub type SharedCore = Rc<RefCell<EditorCore>>;

/// Shared, single-threaded handle to the hook registry. Same
/// rationale as the other `Rc<RefCell<...>>` aliases.
pub type SharedHookRegistry = Rc<RefCell<HookRegistry>>;

#[derive(Clone)]
struct BufferRemoveCallbacks(Rc<RefCell<BufferRemoveCallbackState>>);

struct BufferRemoveCallbackState {
    next_id: u64,
    callbacks: HashMap<BufferId, Vec<BufferRemoveCallback>>,
}

#[derive(Clone)]
struct BufferRemoveCallback {
    id: u64,
    body: Function,
    source: SourceLocation,
}

impl BufferRemoveCallbacks {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(BufferRemoveCallbackState {
            next_id: 1,
            callbacks: HashMap::new(),
        })))
    }

    fn add(&self, buffer: BufferId, body: Function, source: SourceLocation) -> u64 {
        let mut state = self.0.borrow_mut();
        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        state
            .callbacks
            .entry(buffer)
            .or_default()
            .push(BufferRemoveCallback { id, body, source });
        id
    }

    fn remove(&self, buffer: BufferId, callback_id: u64) -> bool {
        let mut state = self.0.borrow_mut();
        let Some(callbacks) = state.callbacks.get_mut(&buffer) else {
            return false;
        };
        let before = callbacks.len();
        callbacks.retain(|callback| callback.id != callback_id);
        let removed = callbacks.len() != before;
        if callbacks.is_empty() {
            state.callbacks.remove(&buffer);
        }
        removed
    }

    fn take(&self, buffer: BufferId) -> Vec<BufferRemoveCallback> {
        self.0
            .borrow_mut()
            .callbacks
            .remove(&buffer)
            .unwrap_or_default()
    }
}

struct BufferRemoveCallbackHandleLua {
    buffer: BufferId,
    callback_id: u64,
}

impl UserData for BufferRemoveCallbackHandleLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |lua, this, ()| {
            Ok(remove_buffer_removed_callback(lua, this))
        });
    }
}

/// Init-phase tracker. The user's `init.lua` runs while this is `false`;
/// [`crate::editor::EditorState::new`] flips it to `true` after the
/// init chunk returns. Lua bindings that gate on init phase
/// (e.g. `pmacs.attach`) read it via [`Lua::app_data_ref`] and use
/// [`require_init_phase`] to short-circuit with a typed error.
///
/// Newtype around `Rc<Cell<bool>>` so the typed app-data lookup is
/// unambiguous; raw `Rc<Cell<bool>>` would collide with any other
/// flag using the same primitive shape.
#[derive(Debug, Clone)]
pub struct InitCompleteFlag(Rc<Cell<bool>>);

impl InitCompleteFlag {
    /// Construct a fresh flag in the "init in progress" state.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(Cell::new(false)))
    }

    /// Whether the init phase has finished.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.0.get()
    }

    /// Mark init complete. Idempotent — calling twice is fine.
    pub fn set_complete(&self) {
        self.0.set(true);
    }

    /// Re-open the init phase. Test/dev-only escape hatch:
    /// integration tests that exercise init-only Lua APIs
    /// (`pmacs.packages.install_local`, `pmacs.attach`) against a
    /// fully-constructed [`crate::editor::EditorState`] need a way
    /// to reset the flag the editor flips during startup.
    /// Production code flips this once and never re-opens; the
    /// `_for_testing` suffix and the `#[doc(hidden)]` mark this
    /// as not part of the user-facing surface.
    #[doc(hidden)]
    pub fn reopen_for_testing(&self) {
        self.0.set(false);
    }
}

impl Default for InitCompleteFlag {
    fn default() -> Self {
        Self::new()
    }
}

/// Slot for an init-time attach request. Written by `pmacs.attach{...}`
/// (M5.6d), read by the post-init dispatcher (M5.6g) to decide whether
/// the local frontend should run against its own [`crate::editor_core::EditorCore`]
/// or hand off to attach mode against a remote daemon.
///
/// v0.1 supports a single attach request per init.lua: a second call
/// errors via [`BindingError::AttachAlreadyRequested`] so a typo in
/// the user's config can't silently override an earlier choice.
#[derive(Debug, Clone, Default)]
pub struct RequestedAttach(Rc<RefCell<Option<AttachTarget>>>);

impl RequestedAttach {
    /// Construct an empty request slot.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(None)))
    }

    /// Read the currently-requested target without consuming it.
    #[must_use]
    pub fn get(&self) -> Option<AttachTarget> {
        self.0.borrow().clone()
    }

    /// Set the request iff the slot is empty. Returns `Err(existing)`
    /// if a prior request is already recorded; the slot is unchanged
    /// in that case.
    ///
    /// # Errors
    ///
    /// Returns `Err(existing)` carrying the prior request when the
    /// slot is already populated.
    pub fn try_set(&self, target: AttachTarget) -> Result<(), AttachTarget> {
        let mut slot = self.0.borrow_mut();
        if let Some(prev) = slot.as_ref() {
            return Err(prev.clone());
        }
        *slot = Some(target);
        Ok(())
    }

    /// Consume the requested target. Subsequent calls return `None`
    /// until a new request is recorded — but post-init Lua calls are
    /// gated by [`require_init_phase`], so re-population is unreachable
    /// in normal flow.
    #[must_use]
    pub fn take(&self) -> Option<AttachTarget> {
        self.0.borrow_mut().take()
    }
}

/// Slot for the current outbound attachment, if any.
///
/// Read by `pmacs.current_attachment()` (M5.6e). v0.1 has no
/// production-side producer — Local mode runs as its own instance
/// (no remote attachment), Attach mode has no `LuaHost`, and Daemon
/// mode is the *target* of attachments rather than the source. The
/// slot exists so the Lua API has a stable shape now; future modes
/// (e.g. a Lua VM running on the daemon side that reflects which
/// frontend is currently dispatching, or a v0.2 flow where a daemon
/// chains upstream) can populate it without changing the binding.
///
/// `LuaHost::set_current_attachment` / `clear_current_attachment`
/// drive the slot from Rust; in v0.1 these are used primarily in
/// tests and by the future M5.6g dispatcher.
#[derive(Debug, Clone, Default)]
pub struct CurrentAttachmentSlot(Rc<RefCell<Option<AttachmentHandle>>>);

impl CurrentAttachmentSlot {
    /// Construct an empty slot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the current attachment, if any.
    #[must_use]
    pub fn get(&self) -> Option<AttachmentHandle> {
        self.0.borrow().clone()
    }

    /// Set (or replace) the current attachment.
    pub fn set(&self, handle: AttachmentHandle) {
        *self.0.borrow_mut() = Some(handle);
    }

    /// Clear the current attachment. No-op if already empty.
    pub fn clear(&self) {
        self.0.borrow_mut().take();
    }
}

/// Identity facts about the running pmacs process.
///
/// Populated by [`install`] with `(name: None, started: Instant::now())`
/// — correct for the Local-mode editor whose `LuaHost` is constructed
/// at process boot. Daemon mode overrides via
/// [`crate::lua::LuaHost::set_local_instance_info`] so the uptime
/// reported by `pmacs.instance.identity()` matches what the daemon
/// hands back over its `Hello`.
///
/// Read by `pmacs.instance.identity()` (M5.6f) to build the
/// [`InstanceIdentity`] returned to Lua.
#[derive(Debug, Clone)]
pub struct LocalInstanceInfo(Rc<RefCell<LocalInstanceData>>);

#[derive(Debug, Clone)]
struct LocalInstanceData {
    name: Option<String>,
    started: std::time::Instant,
}

impl LocalInstanceInfo {
    /// Construct with `(name: None, started: Instant::now())`.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(LocalInstanceData {
            name: None,
            started: std::time::Instant::now(),
        })))
    }

    /// Set the user-facing instance name (typically `--socket NAME`).
    pub fn set_name(&self, name: Option<String>) {
        self.0.borrow_mut().name = name;
    }

    /// Override the `started` anchor. Daemon mode uses this so the
    /// uptime reported by `pmacs.instance.identity()` matches the
    /// `DaemonState`'s own clock.
    pub fn set_started(&self, started: std::time::Instant) {
        self.0.borrow_mut().started = started;
    }

    /// Build an [`InstanceIdentity`] reflecting the running process at
    /// the moment of the call. Subsequent calls re-evaluate uptime
    /// against the same anchor.
    #[must_use]
    pub fn build_identity(&self) -> InstanceIdentity {
        let data = self.0.borrow();
        InstanceIdentity::for_running_process(data.name.clone(), data.started)
    }
}

impl Default for LocalInstanceInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory roster of packages installed during the init phase.
///
/// Populated by `pmacs.packages.install{...}` and
/// `install_project{...}` (T M7.3). Read by
/// `pmacs.packages.installed()` for introspection, by the M7.7
/// require-searcher to resolve `require("<pkg>")`, and by
/// `pmacs.packages.update` to determine which on-disk installs
/// need to be reinstalled or pruned. Single-threaded
/// `Rc<RefCell<...>>` per the boundary's main-thread invariant.
#[derive(Debug, Clone, Default)]
pub struct InstalledPackages(Rc<RefCell<Vec<InstalledPackage>>>);

impl InstalledPackages {
    /// Construct an empty roster.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful install. If a previous entry has the
    /// same `install_path` it is replaced in place; otherwise the
    /// new package is appended. Replacement (rather than blind
    /// append) keeps the roster a unique set of currently-installed
    /// packages, which is what the M7.7 searcher and
    /// `pmacs.packages.update` both rely on --- a stale duplicate
    /// would surface either through `pmacs.packages.installed()` or
    /// through the searcher's most-recent-first lookup.
    ///
    /// Keying by `install_path` (not just by basename) preserves the
    /// legitimate case of the same package installed at both user
    /// and project scope: both rosters reside in the same slot but
    /// at different paths, and both should be visible to
    /// `pmacs.packages.installed()`.
    pub fn record(&self, pkg: InstalledPackage) {
        let path = pkg.install_path.clone();
        let mut roster = self.0.borrow_mut();
        if let Some(slot) = roster.iter_mut().find(|p| p.install_path == path) {
            *slot = pkg;
        } else {
            roster.push(pkg);
        }
    }

    /// Remove every roster entry whose `install_path` matches
    /// `path` exactly. Used by `pmacs.packages.update` when a
    /// transitive dependency drops out of the new resolve plan:
    /// the on-disk install dir is removed, and the matching roster
    /// entry must follow so the searcher stops finding it.
    ///
    /// Path-scoped (not basename-scoped) so a project-scope
    /// install sharing the basename of a pruned user-scope install
    /// is preserved --- the two paths are distinct, and `update`
    /// only owns the user-scope set.
    pub fn remove_by_install_path(&self, path: &std::path::Path) {
        self.0.borrow_mut().retain(|p| p.install_path != path);
    }

    /// Replace the roster wholesale from a previously captured
    /// snapshot. Used by `pmacs.packages.update` rollback: if a later
    /// mutation or lockfile write fails, the in-memory search roster
    /// must return to the same state as the still-current lockfile.
    pub fn replace_snapshot(&self, packages: Vec<InstalledPackage>) {
        *self.0.borrow_mut() = packages;
    }

    /// Snapshot the current roster for read-only consumers.
    #[must_use]
    pub fn snapshot(&self) -> Vec<InstalledPackage> {
        self.0.borrow().clone()
    }
}

/// Stack of currently-loading package basenames (T M8.1d).
///
/// Pushed by the wrapped loader returned from
/// [`load_package_chunk`] before the package's chunk runs;
/// popped after the chunk returns (or errors). Used by
/// [`pmacs.packages.on_unload`] as a fallback when
/// [`mlua::Function::environment`] returns `None` --- under Lua
/// 5.4, a closure that doesn't reference any global doesn't
/// capture `_ENV` as an upvalue, so the env-identity check can't
/// find the owning package. The stack lets the binding still
/// recover the basename for the typical case (`on_unload` called
/// at chunk top-level or from a chunk-direct function call).
///
/// A stack rather than a single slot because `require()` chains
/// can be re-entrant (package A's chunk requires B; B's chunk
/// runs nested inside A's). Each push corresponds to one chunk
/// invocation.
#[derive(Default)]
pub struct CurrentlyLoadingPackage(Rc<RefCell<Vec<String>>>);

impl CurrentlyLoadingPackage {
    /// Construct an empty stack. Installed once per Lua state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&self, basename: String) {
        self.0.borrow_mut().push(basename);
    }

    fn pop(&self) {
        self.0.borrow_mut().pop();
    }

    fn top(&self) -> Option<String> {
        self.0.borrow().last().cloned()
    }
}

/// Registry of `pmacs.packages.on_unload` hooks (T M8.1d).
///
/// Keyed by package basename --- each package registers zero or more
/// callbacks via `pmacs.packages.on_unload(fn)`, and
/// `pmacs.packages.reload(name)` runs them in registration order
/// before invalidating `package.loaded` and re-`require`-ing.
///
/// Hooks are *consumed* on reload: after running, the entry is
/// cleared, so a re-loaded package re-registers fresh hooks. This
/// keeps each reload cycle self-contained --- a stale closure that
/// captures the prior chunk's locals can't fire on a later reload.
#[derive(Default)]
pub struct PackageUnloadHooks(Rc<RefCell<HashMap<String, Vec<mlua::Function>>>>);

impl PackageUnloadHooks {
    /// Construct an empty registry. Installed once per Lua state via
    /// `lua.set_app_data` during the binding bootstrap.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `hook` for `basename`. Order matters --- hooks run in
    /// registration order on reload, so a "shut down workers, then
    /// flush state" pair stays in that order.
    pub fn register(&self, basename: &str, hook: mlua::Function) {
        self.0
            .borrow_mut()
            .entry(basename.to_string())
            .or_default()
            .push(hook);
    }

    /// Drain the entire hook list for `basename`, returning it
    /// as a Vec. The registry's slot for `basename` is empty
    /// afterward.
    ///
    /// Used by [`run_unload_hooks`] to snapshot the cycle's hooks
    /// at start; new `on_unload` registrations during the cycle
    /// land in the (now empty) registry slot instead of extending
    /// the current queue. A successful reload / replacement then
    /// clears that old-env slot before the fresh chunk registers
    /// its next-cycle hooks. This prevents a self-replicating hook
    /// from extending the current unload cycle indefinitely.
    pub fn drain(&self, basename: &str) -> Vec<mlua::Function> {
        self.0.borrow_mut().remove(basename).unwrap_or_default()
    }

    /// Insert `hooks` at the front of the existing list for
    /// `basename`. Existing entries (typically registered by the
    /// chunk during the cycle that just failed) shift to follow
    /// the prepended list.
    ///
    /// Used by [`run_unload_hooks`] on a hook failure: the unrun
    /// tail (including the failed hook at index 0) is pushed back
    /// to the front of the registry so a retry re-attempts them
    /// in order before any newly-registered hooks fire.
    pub fn prepend(&self, basename: &str, mut hooks: Vec<mlua::Function>) {
        if hooks.is_empty() {
            return;
        }
        let mut map = self.0.borrow_mut();
        let existing = map.remove(basename).unwrap_or_default();
        hooks.extend(existing);
        map.insert(basename.to_string(), hooks);
    }
}

/// Source label of the currently-evaluating chunk, populated by
/// [`crate::lua::LuaHost::eval`] before each evaluation.
///
/// The label follows Lua's `@<path>` convention for file-loaded
/// chunks (see [`crate::config::load_user_config_at`]); the
/// install-API binding strips the `@` and takes the parent
/// directory to resolve relative `project_root` values in
/// `pmacs.packages.install_project`. Without this slot we'd be
/// unable to recover the chunk source from a Rust callback because
/// pmacs's Lua state intentionally omits the `debug` library
/// (`forbid(unsafe_code)` rules out `Lua::unsafe_new`, and
/// `debug.getinfo` is not available in the safe stdlib subset).
///
/// Single-slot state, no stack: nested `eval` calls overwrite the
/// outer chunk's source for the duration of the inner call. v0.1
/// has no nested-eval flow that consults this slot, so the
/// simplification is sound.
#[derive(Debug, Clone, Default)]
pub struct CurrentEvalSource(pub Option<String>);

/// Override hook for the `pmacs.packages.install{...}` machinery.
///
/// In production this slot is empty: `install` builds a [`Fetcher`]
/// rooted at `$XDG_CACHE_HOME/pmacs/git/` and an [`InstallScope::User`]
/// rooted at `$XDG_DATA_HOME/pmacs/packages/`. Tests cannot mutate
/// `XDG_CACHE_HOME` / `XDG_DATA_HOME` because `std::env::set_var` is
/// `unsafe` since Rust 2024 and the project forbids unsafe; instead
/// they install a [`PackageInstallOverride`] with explicit paths.
///
/// Set via [`crate::lua::LuaHost::set_package_install_override`]; read
/// by [`do_install`].
#[derive(Debug, Clone, Default)]
pub struct PackageInstallOverride {
    /// Override the bare-mirror cache dir. Defaults to
    /// `$XDG_CACHE_HOME/pmacs/git/` when absent.
    pub cache_dir: Option<std::path::PathBuf>,
    /// Override the user-scope install root. Defaults to
    /// `$XDG_DATA_HOME/pmacs/packages/` when absent.
    pub user_install_root: Option<std::path::PathBuf>,
}

impl PackageInstallOverride {
    /// Empty override (production default behavior).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the cache-dir override. Builder-style.
    #[must_use]
    pub fn with_cache_dir(mut self, p: std::path::PathBuf) -> Self {
        self.cache_dir = Some(p);
        self
    }

    /// Set the user-install-root override. Builder-style.
    #[must_use]
    pub fn with_user_install_root(mut self, p: std::path::PathBuf) -> Self {
        self.user_install_root = Some(p);
        self
    }
}

/// Short-circuit a binding when the init phase has completed.
///
/// Lifecycle-affecting Lua APIs (currently just `pmacs.attach`; M5.6d+)
/// must be called from `init.lua` so they take effect before the
/// editor's main loop starts. Calls after init produce
/// [`BindingError::InitOnlyApi`] with the named op for diagnostics.
///
/// `op_name` is the `pmacs.foo` style identifier the user would
/// recognize from their config (e.g. `"pmacs.attach"`).
///
/// # Errors
///
/// Returns [`BindingError::InitOnlyApi`] wrapped via
/// [`mlua::Error::external`] when init has completed. Returns
/// [`BindingError::NoInitFlag`] (also wrapped) if the flag is missing
/// from app data — that indicates a setup ordering bug, not a user
/// error.
pub fn require_init_phase(lua: &Lua, op_name: &'static str) -> mlua::Result<()> {
    let flag = lua
        .app_data_ref::<InitCompleteFlag>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoInitFlag))?;
    if flag.is_complete() {
        return Err(mlua::Error::external(BindingError::InitOnlyApi {
            op: op_name,
        }));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Boundary errors
// ---------------------------------------------------------------------------

/// Errors specific to the Lua boundary, distinct from the buffer/rope
/// layer's own errors. These surface to Lua via [`mlua::Error::external`]
/// and preserve their structured fields through `Display`.
#[derive(Debug, Error)]
pub enum BindingError {
    /// The Lua state is missing its `BufferRegistry` app data. Indicates
    /// [`install`] was never called for this state, which is a
    /// programming error rather than user input.
    #[error("Lua app data missing: BufferRegistry was not installed on this Lua state")]
    NoRegistry,

    /// The supplied [`BufferId`] no longer resolves --- the buffer was
    /// removed. Surface this rather than `RegistryError::Missing`
    /// directly so the boundary message reads naturally to Lua users.
    #[error("stale buffer handle: id {id:?} (the buffer was removed)")]
    StaleId {
        /// The offending ID, preserved for callers that want to report
        /// it (most commonly via `tostring(err)`).
        id: BufferId,
    },

    /// A Lua mark handle refers to a mark that has already been removed.
    #[error("stale mark handle: mark {mark:?} no longer exists on buffer {buffer:?}")]
    StaleMark {
        /// Buffer that originally owned the mark.
        buffer: BufferId,
        /// Removed mark ID.
        mark: MarkId,
    },

    /// A position argument was negative; positions are byte offsets and
    /// must be `>= 0`.
    #[error("position must be non-negative; got {got}")]
    NegativePosition {
        /// The offending Lua integer.
        got: i64,
    },

    /// A range had `start > end` after coercion to byte offsets.
    #[error("invalid range: start {start} > end {end}")]
    InvalidRange {
        /// Start byte offset.
        start: u64,
        /// End byte offset.
        end: u64,
    },

    /// A command spec table contained a non-string key. Lua tables can
    /// be keyed by anything; the command spec is named-args only
    /// (R49/R50) so we reject other key types.
    #[error("command spec key must be a string; got {got}")]
    NonStringSpecKey {
        /// The Lua type of the offending key.
        got: String,
    },

    /// A command spec field was missing or had the wrong type. Used
    /// when the field's absence isn't covered by a more specific
    /// [`CommandError`] (e.g. `name` being absent when we need to
    /// build the error message).
    #[error("command spec field `{field}` is missing or not a {expected}")]
    SpecFieldType {
        /// The offending field name.
        field: &'static str,
        /// The expected type name.
        expected: &'static str,
    },

    /// A keymap spec contained an unknown `scope` value. Accept-list:
    /// `global`, `buffer`, `mode`.
    #[error("unknown keymap scope `{got}`; expected one of: global, buffer, mode")]
    UnknownScope {
        /// The offending scope name.
        got: String,
    },

    /// A buffer-local bind/unbind didn't supply a `buffer` field, or
    /// a mode bind didn't supply `mode`.
    #[error("keymap scope `{scope}` requires field `{field}`")]
    MissingScopeField {
        /// The scope that needed the field.
        scope: &'static str,
        /// The missing field name.
        field: &'static str,
    },

    /// `pmacs.editor.*` was called before [`install_editor`] attached
    /// the [`SharedCore`] to the Lua app data. Indicates a setup
    /// ordering bug rather than user input.
    #[error("Lua app data missing: editor core was not installed on this Lua state")]
    NoCore,

    /// A Lua integer passed to `pmacs.editor.insert_char` did not
    /// represent a valid Unicode scalar value.
    #[error("invalid codepoint: {value}")]
    InvalidCodepoint {
        /// The integer that was supplied (cast back to `i64`).
        value: i64,
    },

    /// A `pmacs.minibuffer.read { source = "..." }` argument was a
    /// string outside the accepted vocabulary.
    #[error(
        "unknown completion source `{got}`; expected one of: none, commands, buffers, files, or a function"
    )]
    UnknownCompletionSource {
        /// The offending string.
        got: String,
    },

    /// A lifecycle-affecting Lua API (e.g. `pmacs.attach`) was called
    /// after the init phase completed. v0.1 routes these through
    /// [`require_init_phase`] so they only run while `init.lua` is
    /// executing; mid-session calls error here.
    ///
    /// The message names a workaround (the equivalent CLI flag) so
    /// users have a path forward, per the project's "errors point at
    /// the workaround" convention.
    #[error(
        "{op} must be called from init.lua, before the editor starts \
         (after-init calls are not supported in v0.1; \
         restart pmacs with the equivalent CLI flag to change attachment)"
    )]
    InitOnlyApi {
        /// The Lua-facing name of the op being gated, e.g. `"pmacs.attach"`.
        op: &'static str,
    },

    /// The Lua state is missing its [`InitCompleteFlag`] app data.
    /// Indicates [`install`] was never called for this state — a
    /// programming error rather than user input.
    #[error("Lua app data missing: InitCompleteFlag was not installed on this Lua state")]
    NoInitFlag,

    /// The Lua state is missing its [`RequestedAttach`] app data.
    /// Programming error, not user input.
    #[error("Lua app data missing: RequestedAttach slot was not installed on this Lua state")]
    NoRequestedAttachSlot,

    /// The Lua state is missing its [`CurrentAttachmentSlot`] app data.
    /// Programming error, not user input.
    #[error("Lua app data missing: CurrentAttachmentSlot was not installed on this Lua state")]
    NoCurrentAttachmentSlot,

    /// The Lua state is missing its [`LocalInstanceInfo`] app data.
    /// Programming error, not user input.
    #[error("Lua app data missing: LocalInstanceInfo was not installed on this Lua state")]
    NoLocalInstanceInfo,

    /// `pmacs.attach{...}` was given a spec table with neither a
    /// `target` string nor a `kind` string. The user has to provide
    /// one or the other; the message names both forms.
    #[error(
        "pmacs.attach: spec must contain either `target` (e.g. \"local:/path/to.sock\" or \"ssh:host\") \
         or `kind` (one of \"local\", \"ssh\", \"tls\", \"custom\")"
    )]
    AttachSpecMissingKindOrTarget,

    /// A `pmacs.attach{...}` spec used a `kind` that isn't one of the
    /// four recognized values.
    #[error(
        "pmacs.attach: unknown kind `{got}` (expected one of: \"local\", \"ssh\", \"tls\", \"custom\")"
    )]
    AttachSpecUnknownKind {
        /// The offending kind string.
        got: String,
    },

    /// A `pmacs.attach{ kind = ... }` spec was missing a required
    /// field for that kind, or a field had the wrong Lua type.
    #[error("pmacs.attach{{ kind = \"{kind}\" }}: field `{field}` is missing or not a {expected}")]
    AttachSpecField {
        /// The kind whose schema requires this field.
        kind: &'static str,
        /// The field name.
        field: &'static str,
        /// The expected Lua type.
        expected: &'static str,
    },

    /// A second `pmacs.attach{...}` was made while a request from an
    /// earlier call site is still pending. v0.1 supports a single
    /// attach per init.lua to make typos visible.
    #[error(
        "pmacs.attach has already been called in this init phase (current request: `{prior}`); \
         remove the earlier call before adding a new one"
    )]
    AttachAlreadyRequested {
        /// `Display` form of the existing target, for diagnostics.
        prior: String,
    },

    /// The Lua state is missing its [`InstalledPackages`] roster.
    /// Programming error, not user input.
    #[error("Lua app data missing: InstalledPackages roster was not installed on this Lua state")]
    NoInstalledPackagesSlot,

    /// `pmacs.packages.install{...}` was passed something that wasn't
    /// a string (shorthand) or a table (kwargs).
    #[error(
        "pmacs.packages.install: spec must be a string \
         (e.g. \"github:user/repo@^1.0.0\") or a table \
         (e.g. {{ \"github:user/repo\", version = \"^1.0.0\" }}); got {got}"
    )]
    InstallSpecWrongType {
        /// The Lua type of the offending value.
        got: String,
    },

    /// A `pmacs.packages.install{...}` table form omitted the address
    /// (no positional `[1]` and no `address = "..."` kwarg).
    #[error(
        "pmacs.packages.install: spec table must contain either a \
         positional address at [1] or an `address` field"
    )]
    InstallSpecMissingAddress,

    /// A `pmacs.packages.install{...}` spec table specified more than
    /// one of `version`, `branch`, `commit`. Each install must pin
    /// exactly one revision; combining pin kinds is ambiguous (which
    /// one wins?). The error message names every conflicting field
    /// the spec actually carried.
    #[error(
        "pmacs.packages.install: spec must specify exactly one of \
         `version`, `branch`, or `commit`; got: {fields}"
    )]
    InstallSpecConflictingPins {
        /// Comma-separated list of the offending field names, in
        /// the order they appeared on the table.
        fields: String,
    },

    /// `install_project` was called without an explicit
    /// `project_root` field. The CWD-fallback was removed because at
    /// init time CWD is whatever directory the user happened to
    /// invoke pmacs from --- almost never a meaningful project
    /// root. The message names two concrete patterns for filling in
    /// a value, so users hitting this in a CI log or stack trace
    /// can fix it without context.
    #[error(
        "pmacs.packages.install_project requires an explicit \
         `project_root` field. \
         Pass `project_root = \"/path/to/your/project\"` (often \
         `os.getenv(\"PMACS_PROJECT\")` or a path relative to the \
         directory containing your init.lua)."
    )]
    InstallProjectMissingProjectRoot,

    /// The package install layer surfaced a typed error. The display
    /// chain reproduces the inner [`InstallError`]'s message verbatim,
    /// so callers see e.g. "no tag for X satisfies ^1.0".
    #[error("{0}")]
    PackageInstall(#[from] InstallError),

    /// The dependency resolver surfaced a typed error.
    #[error("{0}")]
    PackageResolve(#[from] crate::packages::ResolveError),

    /// The lockfile machinery surfaced a typed error (parse, I/O,
    /// content-hash mismatch, missing manifest, etc.).
    #[error("{0}")]
    PackageLockfile(#[from] crate::packages::LockfileError),

    /// `pmacs.packages.update` was called but the lockfile contains
    /// no top-level entries to re-resolve. Either no `install` has
    /// completed yet, or the lockfile was hand-edited.
    #[error(
        "pmacs.packages.update: no top-level packages in lockfile to update. \
         Run `pmacs.packages.install` first."
    )]
    PackagesUpdateNoEntries,
    /// `pmacs.packages.update("name")` was passed a value that
    /// failed [`PackageName`](crate::packages::PackageName)
    /// validation.
    #[error("pmacs.packages.update: invalid package name `{name}`: {reason}")]
    PackagesUpdateBadName {
        /// The offending value.
        name: String,
        /// Why it was rejected.
        reason: String,
    },
    /// `pmacs.packages.update("name")` was called for a package
    /// that isn't in the lockfile. Surfaces the typo loudly rather
    /// than silently no-op-ing.
    #[error(
        "pmacs.packages.update: package `{name}` is not in the lockfile. \
         Available names come from `pmacs.packages.installed()`."
    )]
    PackagesUpdateUnknownName {
        /// The unknown name the caller passed.
        name: String,
    },

    /// `pmacs.packages.reload("name")` was called but no installed
    /// package has that basename. The roster is the source of
    /// truth; if the user expects the package to be there, they
    /// either misspelled the name or the package failed to
    /// install at startup.
    #[error(
        "pmacs.packages.reload: no installed package named `{name}`. \
         Available names come from `pmacs.packages.installed()`."
    )]
    PackagesReloadUnknownName {
        /// The unknown name the caller passed.
        name: String,
    },

    /// `pmacs.packages.on_unload(fn)` was called but the runtime
    /// can't recover the calling package's basename via identity
    /// against the registered per-package env tables. This
    /// shouldn't fire under normal use --- it indicates the call
    /// ran outside any package's chunk (e.g. directly from
    /// `init.lua` or a non-package Lua chunk), where there's no
    /// owning package to attach the hook to. The error message
    /// names the workaround.
    #[error(
        "pmacs.packages.on_unload must be called from inside a package's chunk; \
         the calling function's environment isn't one of the registered \
         per-package _ENV tables, so there's no owning package to attach the \
         hook to. If you need a teardown hook for editor-shutdown cleanup \
         from non-package code, use \
         `pmacs.hook.add('editor.before-quit', function() ... end)`."
    )]
    PackagesOnUnloadOutsidePackage,

    /// The `on_unload` registry slot wasn't installed. Programming
    /// error like [`Self::NoInstalledPackagesSlot`].
    #[error("Lua app data missing: PackageUnloadHooks slot was not installed on this Lua state")]
    NoUnloadHooksSlot,

    /// `require("pkg.x")` for a submodule the package's manifest does
    /// not list in `exports`. The error surfaces both the missing
    /// export and the available ones so the user can fix their
    /// require or update the manifest.
    #[error(
        "pmacs package `{package}` does not export `{requested}`. \
         Available exports: {exports_display}. \
         If `{requested}` should be public, add it to the package's \
         `exports` list in pmacs.toml; otherwise this require is \
         reaching into the package's internals.",
        exports_display = format_exports_for_error(exports),
    )]
    PackageNotExported {
        /// Package basename.
        package: String,
        /// The full require name as written.
        requested: String,
        /// Sorted exports list from the manifest.
        exports: Vec<String>,
    },

    /// `require("pkg.x")` named an export the manifest declared, but
    /// neither `<dir>/x.lua` nor `<dir>/x/init.lua` exists on disk.
    /// Indicates the package's tree is missing a file the manifest
    /// promised — broken upstream, not a user error.
    #[error(
        "pmacs package export `{requested}` is declared in the manifest \
         but the file is missing on disk (expected `{expected_path}`). \
         The package's installed snapshot is incomplete; reinstall or \
         report this to the package maintainer."
    )]
    PackageExportFileMissing {
        /// The full require name.
        requested: String,
        /// The path the searcher tried to read first (the `<dir>/x.lua`
        /// form; the `init.lua` fallback was also absent).
        expected_path: String,
    },

    /// Lua app data missing: `pmacs.packages.describe` was called but
    /// the [`InstalledPackages`] roster slot is unpopulated. Same
    /// programming-error category as [`Self::NoInstalledPackagesSlot`].
    #[error("pmacs.packages.describe: InstalledPackages roster is not installed on this Lua state")]
    DescribeNoRoster,

    /// A Lua intercept returned a table that was missing one of the
    /// required position/range fields. The intercept contract requires
    /// the returned table to carry the same fields as the input
    /// (`pos` for insert, `start`/`end` for delete, `start`/`end` +
    /// optional `bytes_len` for replace).
    #[error("intercept return table missing required field `{field}`")]
    InterceptResultMissingField {
        /// The missing field name.
        field: &'static str,
    },

    /// A Lua intercept tried to change the op's `kind` (e.g. return
    /// `delete` from a `replace` input). M6.4 forbids kind-changing
    /// intercepts: the lifetime contract on
    /// [`crate::buffer::EditOp`] is preserved only when bytes are
    /// passed through unchanged, and a kind change would require
    /// inventing or dropping bytes. Same-kind transforms (modifying
    /// `pos` / `start` / `end`) are permitted.
    ///
    /// The message names a workaround (the input kind that the
    /// returned table must use) so users have a path forward, per
    /// the project's "errors point at the workaround" convention.
    #[error(
        "intercept changed op kind from `{from}` to `{to}`; M6.4 intercepts may only modify \
         positions/ranges, not the op kind. Return a table with kind=\"{from}\" or raise an \
         error to reject the edit."
    )]
    InterceptKindChange {
        /// The original kind (the one Lua must return).
        from: &'static str,
        /// The kind Lua tried to return.
        to: String,
    },

    /// The buffer registry is already borrowed --- typically because a
    /// `pmacs.buffer.X` call was made from inside a buffer intercept
    /// callback. Intercepts run while the registry is locked so the
    /// edit can be applied atomically; calling back into
    /// `pmacs.buffer.X` from the intercept body would deadlock. We
    /// detect the recursive borrow attempt and surface a typed error
    /// instead of letting `RefCell::borrow_mut` panic.
    ///
    /// The structural fix (let intercepts re-enter the buffer API
    /// safely) is tracked as a deferred audit task; until then,
    /// intercept bodies must operate only on the `op` parameter and
    /// any state captured in their closure --- not call back through
    /// the public surface synchronously.
    #[error(
        "buffer registry already borrowed (likely a re-entrant call from \
         inside a buffer intercept); intercepts cannot call pmacs.buffer.X \
         synchronously --- defer the work to a hook or callback that runs \
         after the edit completes"
    )]
    Reentrant,

    /// Style-overlay teardown was requested from a callback that is
    /// still running under an editor-core or buffer-registry borrow.
    /// Disposal touches both stores, so it must acquire both before
    /// changing the shared disposed flag or removing either view.
    #[error(
        "style overlay disposal cannot run while editor state is borrowed; defer dispose() until \
         after the current callback completes"
    )]
    StyleOverlayDisposeReentrant,
}

// ---------------------------------------------------------------------------
// BufferIdLua: the userdata wrapper
// ---------------------------------------------------------------------------

/// Lua-facing wrapper around [`BufferId`].
///
/// We could implement `UserData` directly on `BufferId`, but R51 wants
/// the Lua surface to be its own design. Wrapping lets the Lua API
/// evolve independently of the Rust shape and lets us add Lua-only
/// metamethods (e.g., `__tostring`, equality) without touching
/// [`BufferId`].
///
/// `Copy` because `BufferId` is `Copy`. Lua scripts can freely pass
/// the handle around.
#[derive(Copy, Clone)]
pub struct BufferIdLua(pub BufferId);

impl BufferIdLua {
    /// The wrapped [`BufferId`].
    #[must_use]
    pub fn id(self) -> BufferId {
        self.0
    }
}

impl FromLua for BufferIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "BufferIdLua".to_string(),
                message: Some(
                    "expected a buffer handle (returned by pmacs.buffer.create / from_bytes)"
                        .to_string(),
                ),
            }),
        }
    }
}

impl UserData for BufferIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        add_query_methods(methods);
        add_mutation_methods(methods);
        add_history_methods(methods);
        add_meta_methods(methods);
    }
}

fn add_query_methods<M: UserDataMethods<BufferIdLua>>(methods: &mut M) {
    methods.add_method("len", |lua, this, ()| {
        with_registry(lua, |r| {
            // Buffer lengths in practice fit comfortably in i64; we pin
            // the boundary at i64::MAX rather than wrapping because Lua
            // integers are i64 on every supported backend.
            i64::try_from(resolve(r, this.0)?.len()).map_err(mlua::Error::external)
        })
    });

    methods.add_method("name", |lua, this, ()| {
        with_registry(lua, |r| Ok(resolve(r, this.0)?.name().to_owned()))
    });

    // Backing file path, or nil for pathless buffers (scratch,
    // generated). The per-buffer twin of `pmacs.editor.file_path()`:
    // consumers that hold a buffer handle from earlier in a hook
    // fan-out (auto-pairing's typed-edit record) must resolve
    // language/URIs against THAT buffer, not whatever is active by
    // the time their callback runs.
    methods.add_method("path", |lua, this, ()| {
        with_registry(lua, |r| {
            Ok(resolve(r, this.0)?
                .file_path()
                .map(|p| p.display().to_string()))
        })
    });

    // Edit revision: bumped by every edit, undo, and redo. The
    // compile-mode external-edit guard (Q#CM2) records this after
    // each of its own writes and resyncs on mismatch — byte length
    // is not an edit-integrity token (a same-length replace changes
    // content while preserving length).
    methods.add_method("revision", |lua, this, ()| {
        with_registry(lua, |r| {
            i64::try_from(resolve(r, this.0)?.revision()).map_err(mlua::Error::external)
        })
    });

    methods.add_method("is_modified", |lua, this, ()| {
        with_registry(lua, |r| Ok(resolve(r, this.0)?.is_modified()))
    });

    methods.add_method("is_valid", |lua, this, ()| {
        with_registry(lua, |r| Ok(r.contains(this.0)))
    });

    methods.add_method("slice", |lua, this, (start, end): (i64, i64)| {
        let bytes = with_registry(lua, |r| slice_buffer(r, this.0, start, end))?;
        lua.create_string(&bytes)
    });
}

fn add_mutation_methods<M: UserDataMethods<BufferIdLua>>(methods: &mut M) {
    methods.add_method(
        "insert",
        |lua, this, (pos, bytes, opts): (i64, mlua::String, Option<Table>)| {
            let pos = u64_from_lua(pos)?;
            let bypass_intercept = parse_bypass_intercept(opts.as_ref())?;
            let payload = bytes.as_bytes();
            let edit = run_buffer_edit(
                lua,
                this.0,
                EditOp::Insert {
                    pos,
                    bytes: &payload,
                },
                bypass_intercept,
            )?;
            notify_buffer_edit_to_windows(lua, this.0, &edit);
            effective_edit_triple(&edit)
        },
    );

    methods.add_method(
        "delete",
        |lua, this, (start, end, opts): (i64, i64, Option<Table>)| {
            let range = checked_range(start, end)?;
            let bypass_intercept = parse_bypass_intercept(opts.as_ref())?;
            let edit = run_buffer_edit(lua, this.0, EditOp::Delete { range }, bypass_intercept)?;
            notify_buffer_edit_to_windows(lua, this.0, &edit);
            effective_edit_triple(&edit)
        },
    );

    methods.add_method(
        "replace",
        |lua, this, (start, end, bytes, opts): (i64, i64, mlua::String, Option<Table>)| {
            let range = checked_range(start, end)?;
            let bypass_intercept = parse_bypass_intercept(opts.as_ref())?;
            let payload = bytes.as_bytes();
            let edit = run_buffer_edit(
                lua,
                this.0,
                EditOp::Replace {
                    range,
                    bytes: &payload,
                },
                bypass_intercept,
            )?;
            notify_buffer_edit_to_windows(lua, this.0, &edit);
            effective_edit_triple(&edit)
        },
    );
}

fn parse_bypass_intercept(opts: Option<&Table>) -> mlua::Result<bool> {
    Ok(match opts {
        Some(opts) => opts
            .get::<Option<bool>>("bypass_intercept")?
            .unwrap_or(false),
        None => false,
    })
}

/// The mutators' Lua return value: the **effective** edit after buffer
/// intercepts ran — `(start, end, inserted_len)` of the operation that
/// was actually applied (kill ring review round 4). An intercept may
/// legally rewrite an op's range; callers that must know exactly what
/// happened (killring's C-k / M-y) compare these against what they
/// requested instead of inferring from length deltas, which an
/// equal-length rewrite defeats.
fn effective_edit_triple(edit: &crate::rope::Edit) -> mlua::Result<(i64, i64, i64)> {
    let cvt = |v: u64| i64::try_from(v).map_err(mlua::Error::external);
    Ok((
        cvt(edit.range.start)?,
        cvt(edit.range.end)?,
        cvt(edit.inserted_len)?,
    ))
}

fn run_buffer_edit(
    lua: &Lua,
    id: BufferId,
    op: EditOp<'_>,
    bypass_intercept: bool,
) -> mlua::Result<crate::rope::Edit> {
    if bypass_intercept {
        run_bypass_edit(lua, id, op)
    } else {
        run_managed_edit(lua, id, op)
    }
}

fn run_bypass_edit(lua: &Lua, id: BufferId, op: EditOp<'_>) -> mlua::Result<crate::rope::Edit> {
    with_registry_mut(lua, |r| {
        let buf = resolve_mut(r, id)?;
        buf.begin_edit().map_err(mlua::Error::external)?;
        let result = buf
            .apply_edit_skip_intercepts(op)
            .map_err(mlua::Error::external);
        buf.end_edit();
        result
    })
}

/// Three-phase edit flow that lets intercepts re-enter `pmacs.buffer.X`
/// safely (T M7.4).
///
/// Phase 1: borrow the registry, mark the buffer mid-edit
/// (`begin_edit`), take its views out, snapshot the
/// [`InterceptContext`], drop the borrow.
///
/// Phase 2: run the intercept chain against the snapshot context,
/// without holding the registry borrow. An intercept body that calls
/// back into `pmacs.buffer.X` on a different buffer succeeds
/// transparently; on the same buffer it hits the `editing_in_progress`
/// gate and returns `BufferError::ConcurrentEdit`.
///
/// Phase 3: re-borrow, restore the views (preserving any new views
/// added during phase 2), clear the mid-edit flag, and run
/// `apply_edit_skip_intercepts` --- which performs the rope edit, the
/// undo bookkeeping, the revision bump, and the `on_edit` broadcast.
fn run_managed_edit(lua: &Lua, id: BufferId, op: EditOp<'_>) -> mlua::Result<crate::rope::Edit> {
    // Phase 1: borrow, begin_edit, take views, snapshot context.
    let (mut views, ctx) = with_registry_mut(lua, |r| {
        let buf = resolve_mut(r, id)?;
        buf.begin_edit().map_err(mlua::Error::external)?;
        let ctx = crate::view::InterceptContext::snapshot(buf);
        let views = buf.take_views();
        Ok((views, ctx))
    })?;

    // Phase 2: run intercepts. Registry borrow is released, so the
    // intercept body may re-enter `pmacs.buffer.X`. The bytes
    // referenced by `op` are owned by the caller's `mlua::String`,
    // which lives across the whole call --- the borrow stays valid.
    let intercept_result: Result<EditOp<'_>, crate::buffer::BufferError> = (|| {
        let mut current = op;
        for (_, view) in &mut views {
            current = view.intercept_edit(&ctx, current)?;
        }
        Ok(current)
    })();

    // Phase 3: re-borrow, restore views, clear mid-edit flag, apply.
    // We restore views and clear the flag even on intercept error,
    // so the buffer is left in a usable state.
    with_registry_mut(lua, |r| {
        let buf = resolve_mut(r, id)?;
        buf.restore_views(views);
        buf.end_edit();
        match intercept_result {
            Ok(final_op) => buf
                .apply_edit_skip_intercepts(final_op)
                .map_err(mlua::Error::external),
            Err(e) => Err(mlua::Error::external(e)),
        }
    })
}

fn add_history_methods<M: UserDataMethods<BufferIdLua>>(methods: &mut M) {
    methods.add_method("undo", |lua, this, ()| {
        let edit = with_registry_mut(lua, |r| Ok(resolve_mut(r, this.0)?.undo().ok()))?;
        if let Some(edit) = edit.as_ref() {
            notify_buffer_edit_to_windows(lua, this.0, edit);
        }
        Ok(edit.is_some())
    });

    methods.add_method("redo", |lua, this, ()| {
        let edit = with_registry_mut(lua, |r| Ok(resolve_mut(r, this.0)?.redo().ok()))?;
        if let Some(edit) = edit.as_ref() {
            notify_buffer_edit_to_windows(lua, this.0, edit);
        }
        Ok(edit.is_some())
    });
}

/// Notify every window currently displaying `buffer_id` that the
/// buffer was just edited via the Lua surface, AND queue the edit's
/// CRDT op (if any) for broadcast to replica frontends.
///
/// Without the window notification, a window already displaying the
/// edited buffer would keep a stale
/// [`crate::text_view::TextView`] line cache — cursor motions stop
/// updating the screen until the window switches buffers.
///
/// # Post-audit-round-5 F28: daemon-origin CRDT op broadcast
///
/// Lua-driven edits (`buf:insert`, `buf:delete`, `buf:replace`,
/// `buf:undo`, `buf:redo`) on CRDT-backed buffers produce Edits with
/// `crdt_op` populated. Without explicit broadcast queueing, those
/// ops never reach replica frontends — their `BufferMirror`s see the
/// resulting `CellDelta` repaint but never import the CRDT op, so
/// subsequent optimistic edits on the replica are generated against
/// stale mirror content.
///
/// We push the op as
/// [`crate::editor_core::CrdtOpOrigin::DaemonKey`] (via
/// `EditorCore::queue_daemon_origin_crdt_op`) so the broadcast sweep
/// includes every replica with no sender exclusion: no frontend
/// applied the op locally; every replica's mirror needs the bytes.
///
/// # No-op cases
///
/// No-op when no [`SharedCore`] has been registered as Lua app data
/// (the shape used by the early-stage tests that exercise the
/// registry without an editor core).
fn notify_buffer_edit_to_windows(lua: &Lua, buffer_id: BufferId, edit: &crate::rope::Edit) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    let mut core = core.borrow_mut();
    core.notify_buffer_edit(buffer_id, edit);
    // F28 — queue for broadcast. `queue_daemon_origin_crdt_op` is a
    // no-op when the edit doesn't carry a `crdt_op` (the buffer
    // wasn't CRDT-backed at edit time).
    core.queue_daemon_origin_crdt_op(buffer_id, edit);
}

fn remove_buffer_removed_callback(lua: &Lua, handle: &BufferRemoveCallbackHandleLua) -> bool {
    let Some(callbacks) = lua.app_data_ref::<BufferRemoveCallbacks>() else {
        return false;
    };
    callbacks.remove(handle.buffer, handle.callback_id)
}

fn remove_buffer_and_fire(lua: &Lua, registry: &SharedRegistry, id: BufferId) -> mlua::Result<()> {
    registry
        .borrow_mut()
        .remove(id)
        .map(|_| ())
        .map_err(mlua::Error::external)?;
    after_buffer_removed(lua, id);
    Ok(())
}

fn after_buffer_removed(lua: &Lua, id: BufferId) {
    if let Some(keymaps) = lua.app_data_ref::<SharedKeymapStack>() {
        keymaps.borrow_mut().remove_buffer(id);
    }
    if let Some(config) = lua.app_data_ref::<config::SharedConfigRegistry>() {
        config.borrow_mut().remove_buffer(id);
    }
    let callbacks = match lua.app_data_ref::<BufferRemoveCallbacks>() {
        Some(callbacks) => callbacks.take(id),
        None => Vec::new(),
    };
    for callback in callbacks {
        if let Err(err) = callback.body.call::<()>(BufferIdLua(id)) {
            log_buffer_removed_error(lua, &callback.source, &err);
        }
    }
}

fn run_hook_if_defined(lua: &Lua, name: &str, args: mlua::MultiValue) {
    let snapshot = match lua.app_data_ref::<SharedHookRegistry>() {
        Some(hooks) => hooks.borrow().snapshot(name),
        None => None,
    };
    let Some((kind, callbacks)) = snapshot else {
        return;
    };
    let outcome = crate::hook::run_snapshot(kind, &callbacks, args);
    for err in &outcome.errors {
        log_hook_error(lua, name, err);
    }
}

/// Force any window showing `buffer_id` to rebuild its `TextView`.
///
/// Called by `pmacs.help.*` after a render rewrites `*help*` end to
/// end. The render does delete-all + insert; tracking the individual
/// edits would be more code than this is worth, and a full rebuild
/// is what was implicitly happening anyway. Without this, the
/// regression we hit on `*errors*` and `*buffer-list*` recurs on
/// `*help*`: a window already displaying it keeps the pre-render
/// line cache and cursor motions stop updating the screen.
fn rebuild_help_buffer_views(lua: &Lua, buffer_id: BufferId) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    core.borrow_mut().rebuild_views_for(buffer_id);
}

fn add_meta_methods<M: UserDataMethods<BufferIdLua>>(methods: &mut M) {
    methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
        // `BufferId` has a private inner u64 (R22), so we don't expose
        // the value; the Debug repr is for human-readable identity.
        Ok(format!("BufferId({:?})", this.0))
    });

    methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: BufferIdLua| {
        Ok(this.0 == other.0)
    });
}

fn slice_buffer(r: &BufferRegistry, id: BufferId, start: i64, end: i64) -> mlua::Result<Vec<u8>> {
    let buf = resolve(r, id)?;
    let len = buf.len();
    let start = u64_from_lua(start)?;
    let end = u64_from_lua(end)?;
    if start > end {
        return Err(mlua::Error::external(BindingError::InvalidRange {
            start,
            end,
        }));
    }
    if end > len {
        return Err(mlua::Error::external(crate::rope::RopeError::OutOfBounds {
            pos: end,
            len,
        }));
    }
    let mut out = vec![0u8; (end - start) as usize];
    if !out.is_empty() {
        buf.snapshot_rope().slice(start, end, &mut out);
    }
    Ok(out)
}

fn checked_range(start: i64, end: i64) -> mlua::Result<Range> {
    let start = u64_from_lua(start)?;
    let end = u64_from_lua(end)?;
    if start > end {
        return Err(mlua::Error::external(BindingError::InvalidRange {
            start,
            end,
        }));
    }
    Ok(Range::new(start, end))
}

// ---------------------------------------------------------------------------
// LuaInterceptView: a Lua function as a buffer intercept_edit chain entry
// ---------------------------------------------------------------------------

/// Wraps a Lua function as a [`crate::view::View`] whose only behavior
/// is to participate in the buffer's `intercept_edit` chain. Other
/// view callbacks (`on_edit`, `render`) take their default no-op
/// implementations.
///
/// # Lua-side contract
///
/// On every `apply_edit`, the wrapped Lua function is invoked with
/// one argument: a table describing the proposed op. The table's
/// `kind` is one of `"insert"`, `"delete"`, `"replace"`. Position
/// fields:
///
/// - `kind = "insert"`:  `pos: integer`, `bytes: string`, `bytes_len: integer`
/// - `kind = "delete"`:  `start: integer`, `end: integer`
/// - `kind = "replace"`: `start: integer`, `end: integer`, `bytes: string`, `bytes_len: integer`
///
/// `bytes` is the literal byte content the caller proposes to insert
/// (for `insert`) or replace into the range (for `replace`). It is
/// the inserted/incoming bytes, *not* the bytes being overwritten;
/// `delete` carries no `bytes` field because deletion has no
/// payload. Surfaced as a Lua string (which is byte-clean: Lua
/// strings hold arbitrary 8-bit data, not UTF-8).
///
/// **Why M6.4 punted on `bytes`, and why M8.3 added it.** The
/// concern was per-edit FFI cost: every keystroke would copy the
/// inserted bytes across the boundary. In practice typical inserts
/// are 1 byte (a typed character), and the wdired pattern (M8.3)
/// genuinely needs the bytes — the dired-class package validates
/// permission-column edits against the rwx alphabet by inspecting
/// the proposed bytes, which the spec explicitly requires
/// ("rejection at the `intercept_edit` layer, not at the syscall").
/// `bytes_len` is retained for the rare case where an intercept
/// only needs the size and wants to skip a length lookup.
///
/// The function returns one of:
///
/// - `nil` --- pass through the original op unchanged. The common
///   case for "this edit is fine, I have nothing to say."
/// - a table with the same shape as the input --- override `pos` /
///   `start` / `end` (bytes preserved). The `kind` field on the
///   returned table must equal the input kind: M6.4 does not support
///   kind-changing intercepts (lifetime-clean only because bytes are
///   immutable; see above).
/// - raise an error --- reject the edit with the raised message
///   surfaced as [`crate::buffer::BufferError::Intercepted`].
///
/// Multiple `LuaInterceptView`s may be attached to the same buffer.
/// They run in attach order, threading the (possibly-modified) op
/// through the chain; the first to raise stops the chain. This
/// matches the buffer's existing view-chain semantics --- the M6.4
/// REPL package just happens to be the first user.
struct LuaInterceptView {
    /// The owning Lua state. `Lua` is `Clone` in mlua 0.10 (refcounted
    /// internally), so each view holds its own handle without forcing
    /// the View trait to thread a `&Lua` parameter through.
    lua: Lua,
    /// The Lua function. Held alive across calls; refcounted by mlua.
    body: Function,
}

impl crate::view::View for LuaInterceptView {
    fn intercept_edit<'a>(
        &mut self,
        _ctx: &crate::view::InterceptContext,
        op: EditOp<'a>,
    ) -> Result<EditOp<'a>, crate::buffer::BufferError> {
        let input = build_intercept_input(&self.lua, &op).map_err(|e| {
            crate::buffer::BufferError::Intercepted {
                reason: format!("failed to build intercept input table: {e}"),
            }
        })?;
        let result: Value = self.body.call(input).map_err(|e| {
            // Lua-raised errors and Rust-side coercion errors land
            // here. We surface the Lua message verbatim so the user
            // sees the intercept's reason, not an opaque "intercept
            // failed."
            crate::buffer::BufferError::Intercepted {
                reason: format!("{e}"),
            }
        })?;
        match result {
            Value::Nil => Ok(op),
            Value::Table(t) => {
                apply_op_overrides(op, &t).map_err(|e| crate::buffer::BufferError::Intercepted {
                    reason: format!("{e}"),
                })
            }
            other => Err(crate::buffer::BufferError::Intercepted {
                reason: format!(
                    "intercept must return nil or a table; got {}",
                    other.type_name()
                ),
            }),
        }
    }
}

/// Build the table passed to a Lua intercept on each `apply_edit`.
///
/// `bytes` is surfaced as a Lua string for `insert` and `replace`
/// (M8.3 enhancement, see [`LuaInterceptView`] doc). Lua strings are
/// byte-clean — `lua.create_string(&[u8])` preserves arbitrary
/// non-UTF-8 content — so a `name` field containing a non-UTF-8
/// byte from an exotic filesystem still round-trips correctly.
fn build_intercept_input(lua: &Lua, op: &EditOp<'_>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    match *op {
        EditOp::Insert { pos, bytes } => {
            t.set("kind", "insert")?;
            t.set("pos", i64_clamp(pos))?;
            t.set("bytes", lua.create_string(bytes)?)?;
            t.set("bytes_len", i64_clamp(bytes.len() as u64))?;
        }
        EditOp::Delete { range } => {
            t.set("kind", "delete")?;
            t.set("start", i64_clamp(range.start))?;
            t.set("end", i64_clamp(range.end))?;
        }
        EditOp::Replace { range, bytes } => {
            t.set("kind", "replace")?;
            t.set("start", i64_clamp(range.start))?;
            t.set("end", i64_clamp(range.end))?;
            t.set("bytes", lua.create_string(bytes)?)?;
            t.set("bytes_len", i64_clamp(bytes.len() as u64))?;
        }
    }
    Ok(t)
}

/// Coerce a `u64` to `i64`, saturating at `i64::MAX`. Buffer positions
/// in practice fit comfortably in `i64`; we pin the boundary rather
/// than wrapping because Lua integers are `i64` on every supported
/// backend.
fn i64_clamp(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Read the override table returned by a Lua intercept and produce a
/// new [`EditOp`] with the same lifetime as the input. Same `kind`
/// only: M6.4 forbids kind-changing transforms (see [`LuaInterceptView`]
/// docs for the lifetime rationale).
fn apply_op_overrides<'a>(op: EditOp<'a>, t: &Table) -> mlua::Result<EditOp<'a>> {
    let kind: String = t.get("kind").map_err(|_| {
        mlua::Error::external(BindingError::InterceptResultMissingField { field: "kind" })
    })?;
    match (kind.as_str(), op) {
        ("insert", EditOp::Insert { bytes, .. }) => {
            let pos: i64 = t.get("pos").map_err(|_| {
                mlua::Error::external(BindingError::InterceptResultMissingField { field: "pos" })
            })?;
            Ok(EditOp::Insert {
                pos: u64_from_lua(pos)?,
                bytes,
            })
        }
        ("delete", EditOp::Delete { .. }) => {
            let start: i64 = t.get("start").map_err(|_| {
                mlua::Error::external(BindingError::InterceptResultMissingField { field: "start" })
            })?;
            let end: i64 = t.get("end").map_err(|_| {
                mlua::Error::external(BindingError::InterceptResultMissingField { field: "end" })
            })?;
            Ok(EditOp::Delete {
                range: checked_range(start, end)?,
            })
        }
        ("replace", EditOp::Replace { bytes, .. }) => {
            let start: i64 = t.get("start").map_err(|_| {
                mlua::Error::external(BindingError::InterceptResultMissingField { field: "start" })
            })?;
            let end: i64 = t.get("end").map_err(|_| {
                mlua::Error::external(BindingError::InterceptResultMissingField { field: "end" })
            })?;
            Ok(EditOp::Replace {
                range: checked_range(start, end)?,
                bytes,
            })
        }
        (other_kind, op) => {
            let original_kind = match op {
                EditOp::Insert { .. } => "insert",
                EditOp::Delete { .. } => "delete",
                EditOp::Replace { .. } => "replace",
            };
            Err(mlua::Error::external(BindingError::InterceptKindChange {
                from: original_kind,
                to: other_kind.to_owned(),
            }))
        }
    }
}

/// Userdata returned by `pmacs.buffer.add_intercept`; consumed by
/// `pmacs.buffer.remove_intercept`. Holds the buffer ID and the
/// view ID so a stale handle (referring to a removed buffer or
/// already-detached intercept) can be detected and reported via
/// [`BindingError::StaleId`] rather than silently no-op-ing.
#[derive(Copy, Clone)]
pub struct InterceptHandleLua {
    buffer: BufferId,
    view: crate::buffer::ViewId,
}

#[derive(Clone)]
/// Lua handle for a shared buffer-byte style overlay.
///
/// Lifetime contract (PR #113 round-6 finding 3): the buffer-attached
/// translator lives until the buffer dies OR `dispose()` is called.
/// One handle per buffer incarnation (the compile-mode and REPL
/// discipline) needs no disposal — the buffer's death frees it;
/// repeated `add_style_overlay` calls on a LONG-LIVED buffer must
/// `dispose()` retired handles, or every edit keeps paying for every
/// abandoned translator.
pub struct StyleOverlayHandleLua {
    /// Shared style spans rendered by every attached overlay view.
    spans: crate::overlay::SharedBufferStyleSpans,
    /// Buffer the translator was attached to. Attachment is
    /// validated against this (round-7 finding 1): a render view on
    /// any OTHER buffer would show coordinates translated only by
    /// edits to this one.
    buffer: BufferId,
    /// The buffer-attached translator's view id — retained so
    /// `dispose()` can detach it.
    translator: crate::buffer::ViewId,
    /// Shared across handle clones (`FromLua` clones): set by
    /// `dispose()`, checked by attachment — re-attaching a disposed
    /// handle would resurrect rendering without its translator
    /// (round-7 finding 1).
    disposed: Arc<std::sync::atomic::AtomicBool>,
}

impl FromLua for StyleOverlayHandleLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(ud.borrow::<Self>()?.clone()),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "StyleOverlayHandleLua".to_string(),
                message: Some(
                    "expected a style overlay handle (returned by pmacs.buffer.add_style_overlay)"
                        .to_string(),
                ),
            }),
        }
    }
}

impl UserData for StyleOverlayHandleLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "add",
            |_, this, (start, end, style): (i64, i64, Table)| -> mlua::Result<()> {
                let start = u64_from_lua(start)?;
                let end = u64_from_lua(end)?;
                if start > end {
                    return Err(mlua::Error::external(BindingError::InvalidRange {
                        start,
                        end,
                    }));
                }
                if start == end {
                    return Ok(());
                }
                this.spans
                    .lock()
                    .expect("style overlay mutex poisoned")
                    .push(crate::overlay::BufferStyleSpan {
                        start,
                        end,
                        style: lua_to_style(&style)?,
                    });
                Ok(())
            },
        );

        methods.add_method("clear", |_, this, ()| {
            this.spans
                .lock()
                .expect("style overlay mutex poisoned")
                .clear();
            Ok(())
        });

        methods.add_method("clear_before", |_, this, pos: i64| -> mlua::Result<()> {
            let pos = u64_from_lua(pos)?;
            this.spans
                .lock()
                .expect("style overlay mutex poisoned")
                .retain(|span| span.end > pos);
            Ok(())
        });

        methods.add_method("spans", |lua, this, ()| {
            let spans = this.spans.lock().expect("style overlay mutex poisoned");
            let out = lua.create_table_with_capacity(spans.len(), 0)?;
            for (i, span) in spans.iter().enumerate() {
                let row = lua.create_table_with_capacity(0, 3)?;
                row.set("start", i64::try_from(span.start).unwrap_or(i64::MAX))?;
                row.set("end", i64::try_from(span.end).unwrap_or(i64::MAX))?;
                row.set("style", style_to_lua(lua, span.style)?)?;
                out.set(i + 1, row)?;
            }
            Ok(out)
        });

        // Idempotent teardown (round-6 finding 3): detaches the
        // buffer-attached translator (so later edits stop paying for
        // it) and removes every window render view over this store.
        // Without this, a retired handle's translator lived until
        // the buffer died — permanent per-edit cost growth for
        // repeated creation on a long-lived buffer. Safe to call
        // twice; safe after the buffer is gone.
        methods.add_method("dispose", |lua, this, ()| {
            // Preflight every borrow before changing shared state.
            // A callback may run while the editor core or registry is
            // already borrowed; panicking (or removing the window
            // views before discovering a registry conflict) would
            // leave a partially disposed handle. Returning a pointed
            // error keeps the operation retryable after the callback.
            let core_handle = lua.app_data_ref::<SharedCore>();
            let mut core = match core_handle.as_deref() {
                Some(core) => Some(core.try_borrow_mut().map_err(|_| {
                    mlua::Error::external(BindingError::StyleOverlayDisposeReentrant)
                })?),
                None => None,
            };
            let registry_handle = lua
                .app_data_ref::<SharedRegistry>()
                .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
            let mut registry = registry_handle
                .try_borrow_mut()
                .map_err(|_| mlua::Error::external(BindingError::StyleOverlayDisposeReentrant))?;

            this.disposed
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let id = crate::overlay::style_store_identity(&this.spans);
            // Window cleanup needs the editor core, which is
            // optional app data...
            if let Some(core) = core.as_mut() {
                for win in core.windows.values_mut() {
                    win.overlays.retain(|v| v.overlay_identity() != Some(id));
                }
            }
            // ...but the translator detach must not go through it:
            // an install-only/headless host registers the registry
            // WITHOUT a core, and returning success while the
            // translator stays attached would leak per-edit work for
            // the buffer's lifetime (round-7 finding 2).
            if let Ok(buf) = registry.get_mut(this.buffer) {
                buf.detach_view(this.translator);
            }
            Ok(())
        });
    }
}

impl FromLua for InterceptHandleLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "InterceptHandleLua".to_string(),
                message: Some(
                    "expected an intercept handle (returned by pmacs.buffer.add_intercept)"
                        .to_string(),
                ),
            }),
        }
    }
}

impl UserData for InterceptHandleLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "InterceptHandle({:?},{:?})",
                this.buffer, this.view
            ))
        });
    }
}

/// Userdata handle for a buffer-owned mark.
#[derive(Copy, Clone)]
pub struct MarkHandleLua {
    buffer: BufferId,
    mark: MarkId,
}

impl FromLua for MarkHandleLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "MarkHandleLua".to_string(),
                message: Some(
                    "expected a mark handle (returned by pmacs.buffer.mark_create)".to_string(),
                ),
            }),
        }
    }
}

impl UserData for MarkHandleLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", |lua, this, ()| {
            with_registry(lua, |r| {
                let buf = resolve(r, this.buffer)?;
                let pos = buf.mark_pos(this.mark).ok_or_else(|| {
                    mlua::Error::external(BindingError::StaleMark {
                        buffer: this.buffer,
                        mark: this.mark,
                    })
                })?;
                Ok(i64_clamp(pos))
            })
        });

        methods.add_method("pos", |lua, this, ()| {
            with_registry(lua, |r| {
                let buf = resolve(r, this.buffer)?;
                let pos = buf.mark_pos(this.mark).ok_or_else(|| {
                    mlua::Error::external(BindingError::StaleMark {
                        buffer: this.buffer,
                        mark: this.mark,
                    })
                })?;
                Ok(i64_clamp(pos))
            })
        });

        methods.add_method("set", |lua, this, pos: i64| {
            let pos = u64_from_lua(pos)?;
            with_registry_mut(lua, |r| {
                let buf = resolve_mut(r, this.buffer)?;
                let ok = buf
                    .set_mark(this.mark, pos)
                    .map_err(mlua::Error::external)?;
                if !ok {
                    return Err(mlua::Error::external(BindingError::StaleMark {
                        buffer: this.buffer,
                        mark: this.mark,
                    }));
                }
                Ok(())
            })
        });

        methods.add_method("remove", |lua, this, ()| {
            with_registry_mut(lua, |r| {
                let buf = resolve_mut(r, this.buffer)?;
                Ok(buf.remove_mark(this.mark))
            })
        });

        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("MarkHandle({:?},{:?})", this.buffer, this.mark))
        });
    }
}

/// Opaque Lua handle for a statusline provider registration.
#[derive(Copy, Clone)]
pub struct StatuslineProviderIdLua(pub StatuslineProviderId);

impl FromLua for StatuslineProviderIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(data) => Ok(*data.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "StatuslineProviderIdLua".to_owned(),
                message: Some("expected a statusline provider handle".to_owned()),
            }),
        }
    }
}

impl UserData for StatuslineProviderIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("raw", |_, this, ()| Ok(this.0.raw()));
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(this.0.to_string())
        });
        methods.add_meta_method(
            mlua::MetaMethod::Eq,
            |_, this, other: StatuslineProviderIdLua| Ok(this.0 == other.0),
        );
    }
}

/// Retrieve the statusline registry installed with the base `pmacs` table.
///
/// `EditorState` and semantic/TUI constructors use this exact shared handle;
/// bare Lua hosts always receive an empty registry rather than an absent API.
pub fn statusline_registry(lua: &Lua) -> mlua::Result<SharedStatuslineRegistry> {
    lua.app_data_ref::<SharedStatuslineRegistry>()
        .map(|registry| registry.clone())
        .ok_or_else(|| mlua::Error::external("statusline registry is not installed"))
}

/// Install the strict `pmacs.statusline` registration/lifecycle surface.
#[allow(
    clippy::too_many_lines,
    reason = "one strict table parser followed by four small lifecycle bindings; splitting obscures the all-fields-before-mutation contract"
)]
pub fn install_statusline_module(
    lua: &Lua,
    registry: &SharedStatuslineRegistry,
) -> mlua::Result<Table> {
    let module = lua.create_table()?;
    {
        let registry = registry.clone();
        module.set(
            "register",
            lua.create_function(move |lua, spec: Table| {
                let mut unknown = None;
                spec.clone().for_each(|key: Value, _: Value| {
                    let name = match &key {
                        Value::String(value) => value.to_str().map_or_else(
                            |_| "<invalid UTF-8>".to_owned(),
                            |value| value.to_owned(),
                        ),
                        other => format!("{other:?}"),
                    };
                    if !matches!(name.as_str(), "name" | "side" | "priority" | "face" | "fn")
                        && unknown.is_none()
                    {
                        unknown = Some(name);
                    }
                    Ok(())
                })?;
                if let Some(key) = unknown {
                    return Err(mlua::Error::external(format!(
                        "pmacs.statusline.register: unknown field `{key}`"
                    )));
                }

                let name =
                    strict_statusline_string(spec.raw_get("name")?, "name", false)?
                        .expect("required statusline name");
                let side_value =
                    strict_statusline_string(spec.raw_get("side")?, "side", false)?
                        .expect("required statusline side");
                let side = match side_value.as_str() {
                    "left" => StatuslineSide::Left,
                    "right" => StatuslineSide::Right,
                    other => {
                        return Err(mlua::Error::external(format!(
                            "pmacs.statusline.register: `side` must be \"left\" or \"right\", got {other:?}"
                        )));
                    }
                };
                let priority = strict_statusline_priority(spec.raw_get("priority")?)?;
                let face = strict_statusline_string(spec.raw_get("face")?, "face", true)?
                    .unwrap_or_else(|| "ui.modeline".to_owned());
                let callback = match spec.raw_get::<Value>("fn")? {
                    Value::Function(function) => function,
                    other => {
                        return Err(mlua::Error::external(format!(
                            "pmacs.statusline.register: `fn` must be a function, got {}",
                            other.type_name()
                        )));
                    }
                };
                // Every raw field is now parsed and typed; only this final call
                // mutates the registry.
                let id = registry.borrow_mut().register(
                    name,
                    side,
                    priority,
                    face,
                    callback,
                    caller_source(lua, 2),
                )
                    .map_err(mlua::Error::external)?;
                Ok(StatuslineProviderIdLua(id))
            })?,
        )?;
    }
    {
        let registry = registry.clone();
        module.set(
            "unregister",
            lua.create_function(move |_, id: StatuslineProviderIdLua| {
                Ok(registry.borrow_mut().unregister(id.0))
            })?,
        )?;
    }
    {
        let registry = registry.clone();
        module.set(
            "set_priority",
            lua.create_function(move |_, (id, value): (StatuslineProviderIdLua, Value)| {
                let priority = strict_statusline_priority(value)?;
                Ok(registry.borrow_mut().set_priority(id.0, priority))
            })?,
        )?;
    }
    {
        let registry = registry.clone();
        module.set(
            "set_enabled",
            lua.create_function(move |_, (id, value): (StatuslineProviderIdLua, Value)| {
                let Value::Boolean(enabled) = value else {
                    return Err(mlua::Error::external(
                        "pmacs.statusline.set_enabled: `enabled` must be a boolean",
                    ));
                };
                Ok(registry.borrow_mut().set_enabled(id.0, enabled))
            })?,
        )?;
    }
    {
        let registry = registry.clone();
        module.set(
            "providers",
            lua.create_function(move |lua, ()| {
                let providers = registry.borrow().providers();
                let output = lua.create_table_with_capacity(providers.len(), 0)?;
                for (index, provider) in providers.iter().enumerate() {
                    let metadata = lua.create_table_with_capacity(0, 6)?;
                    metadata.raw_set("handle", StatuslineProviderIdLua(provider.id))?;
                    metadata.raw_set("name", provider.name.as_str())?;
                    metadata.raw_set("side", provider.side.as_str())?;
                    metadata.raw_set("priority", provider.priority)?;
                    metadata.raw_set("face", provider.face.as_str())?;
                    metadata.raw_set("enabled", provider.enabled)?;
                    output.raw_set(index + 1, metadata)?;
                }
                Ok(output)
            })?,
        )?;
    }
    Ok(module)
}

fn strict_statusline_string(
    value: Value,
    field: &'static str,
    optional: bool,
) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil if optional => Ok(None),
        Value::String(value) => value
            .to_str()
            .map(|value| Some(value.to_owned()))
            .map_err(|_| {
                mlua::Error::external(format!(
                    "pmacs.statusline.register: `{field}` must be valid UTF-8"
                ))
            }),
        other => Err(mlua::Error::external(format!(
            "pmacs.statusline.register: `{field}` must be a string, got {}",
            other.type_name()
        ))),
    }
}

fn strict_statusline_priority(value: Value) -> mlua::Result<i32> {
    match value {
        Value::Nil => Ok(0),
        Value::Integer(value) => i32::try_from(value).map_err(|_| {
            mlua::Error::external(
                "pmacs.statusline priority must be an integer in the signed 32-bit range",
            )
        }),
        Value::Number(value)
            if value.is_finite()
                && value.fract() == 0.0
                && value >= f64::from(i32::MIN)
                && value <= f64::from(i32::MAX) =>
        {
            Ok(value as i32)
        }
        Value::Number(_) => Err(mlua::Error::external(
            "pmacs.statusline priority must be an integer in the signed 32-bit range",
        )),
        other => Err(mlua::Error::external(format!(
            "pmacs.statusline priority must be a number, got {}",
            other.type_name()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Module install
// ---------------------------------------------------------------------------

/// Install the `pmacs.buffer.*` table and register the registry on the
/// Lua state's app data.
///
/// Idempotent within a single process *modulo the registry*: calling
/// `install` again replaces the app-data registry with the new one and
/// rebuilds the `pmacs` global. Tests rely on this; production code
/// calls it exactly once at [`crate::lua::LuaHost`] construction.
///
/// `registry`, `commands`, and `keymaps` are taken by reference and
/// cloned internally for each captured closure --- the caller keeps
/// ownership of its handles.
pub fn install(
    lua: &Lua,
    registry: &SharedRegistry,
    commands: &SharedCommandRegistry,
    keymaps: &SharedKeymapStack,
    menus: &SharedMenuRegistry,
    hooks: &SharedHookRegistry,
) -> mlua::Result<()> {
    lua.set_app_data(registry.clone());
    lua.set_app_data(commands.clone());
    lua.set_app_data(keymaps.clone());
    lua.set_app_data(menus.clone());
    lua.set_app_data(hooks.clone());
    lua.set_app_data(InitCompleteFlag::new());
    lua.set_app_data(RequestedAttach::new());
    lua.set_app_data(CurrentAttachmentSlot::new());
    lua.set_app_data(LocalInstanceInfo::new());
    lua.set_app_data(InstalledPackages::new());
    lua.set_app_data(PackageUnloadHooks::new());
    lua.set_app_data(CurrentlyLoadingPackage::new());
    lua.set_app_data(BufferRemoveCallbacks::new());
    let statusline = Rc::new(RefCell::new(StatuslineRegistry::new()));
    lua.set_app_data(statusline.clone());
    let config_registry: config::SharedConfigRegistry =
        Rc::new(RefCell::new(crate::config_registry::ConfigRegistry::new()));
    lua.set_app_data(config_registry.clone());

    let pmacs = lua.create_table()?;
    pmacs.set("buffer", install_buffer_module(lua, registry)?)?;
    pmacs.set("command", install_command_module(lua, commands)?)?;
    pmacs.set("keymap", install_keymap_module(lua, keymaps)?)?;
    pmacs.set("menu", install_menu_module(lua, menus)?)?;
    pmacs.set("hook", install_hook_module(lua, hooks)?)?;
    pmacs.set("statusline", install_statusline_module(lua, &statusline)?)?;
    pmacs.set("config", config::install_config(lua, &config_registry)?)?;
    // Wall-clock millis (since UNIX epoch). Used by builtin runtime
    // chunks for timeout loops; `os.clock()` only counts CPU time and
    // is a poor fit for "wait until something arrives over I/O".
    pmacs.set(
        "now_ms",
        lua.create_function(|_, ()| {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            Ok(i64::try_from(ms).unwrap_or(i64::MAX))
        })?,
    )?;
    pmacs.set(
        "describe",
        install_describe_module(lua, registry, commands, keymaps, hooks)?,
    )?;
    pmacs.set(
        "help",
        install_help_module(lua, registry, commands, keymaps, hooks)?,
    )?;
    pmacs.set("attach", install_attach_binding(lua)?)?;
    pmacs.set(
        "current_attachment",
        install_current_attachment_binding(lua)?,
    )?;
    pmacs.set("instance", install_instance_module(lua, registry)?)?;
    pmacs.set("ansi", install_ansi_module(lua)?)?;
    pmacs.set("packages", install_packages_module(lua)?)?;
    pmacs.set("state", install_state_module(lua)?)?;
    pmacs.set("session", install_session_module(lua)?)?;
    pmacs.set("autosave", install_autosave_module(lua)?)?;
    lua.globals().set("pmacs", pmacs)?;
    Ok(())
}

/// `pmacs.autosave.*` — the Rust half of autosave + crash recovery
/// (Arc 3 phase 3). Lua cannot enumerate non-active buffers' paths, and
/// `FileMeta` is neither Lua-visible nor serde, so the sweep and the
/// external-change guard live in Rust (Q#AS1). `autosave.lua` layers the
/// cadence, the configurable interval, and the recovery UX on top.
///
/// The `_`-prefixed names are the raw primitives; `autosave.lua` adds the
/// public `enable` / `interval_ms` / `sweep` onto the same table.
fn install_autosave_module(lua: &Lua) -> mlua::Result<Table> {
    // The skip cache lives for the life of the VM.
    lua.set_app_data(crate::autosave::AutosaveCache::default());
    let m = lua.create_table()?;

    // _sweep() -> (written, blocked). `blocked` counts buffers whose
    // sweep was refused because unclaimed crash data sits at their key.
    m.set(
        "_sweep",
        lua.create_function(|lua, ()| crate::autosave::sweep(lua).map_err(mlua::Error::external))?,
    )?;

    // _adopt(buf): claim a buffer's recovery file for this session, so
    // later sweeps may overwrite it and a kill can retire it.
    // `recover-file` calls this once the contents are installed.
    m.set(
        "_adopt",
        lua.create_function(|lua, id: BufferIdLua| {
            crate::autosave::adopt(lua, id.0);
            Ok(())
        })?,
    )?;

    // _discard_buffer(buf): retire a buffer's recovery copy by BufferId —
    // removes both its current-path key and the key its last sweep wrote
    // (they differ after a rename).
    m.set(
        "_discard_buffer",
        lua.create_function(|lua, id: BufferIdLua| {
            crate::autosave::discard_buffer(lua, id.0);
            Ok(())
        })?,
    )?;

    m.set(
        "_status",
        lua.create_function(|lua, path: String| {
            let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
                return Ok("none");
            };
            Ok(crate::autosave::status(&base, std::path::Path::new(&path)).as_str())
        })?,
    )?;

    m.set(
        "_recover_bytes",
        lua.create_function(|lua, path: String| {
            let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
                return Ok(None);
            };
            match crate::autosave::recover_bytes(&base, std::path::Path::new(&path)) {
                Some(bytes) => Ok(Some(lua.create_string(&bytes)?)),
                None => Ok(None),
            }
        })?,
    )?;

    // _discard(path): delete a recovery file and drop any claim on it.
    m.set(
        "_discard",
        lua.create_function(|lua, path: String| {
            Ok(crate::autosave::discard_path(
                lua,
                std::path::Path::new(&path),
            ))
        })?,
    )?;

    // _pending() -> (fresh_paths, corrupt_count). Enumerates in Rust so
    // argv `[new file]` buffers — which fire no hook — are covered too.
    m.set(
        "_pending",
        lua.create_function(|lua, ()| {
            let (fresh, corrupt) = crate::autosave::pending(lua);
            Ok((fresh, corrupt))
        })?,
    )?;

    Ok(m)
}

/// Marker app-data set by `pmacs.session.arm_restore()` (Arc 3 phase 2,
/// Q#DS7). Its presence tells the `RunLocal` startup trigger to attempt
/// a desktop restore; `desktop_mode(true)` in init.lua arms it.
pub struct DesktopRestoreArmed;

/// Marker app-data set by `run_daemon` (Arc 3 phase 2, Q#DS9). Desktop
/// save/restore is local-only in v1 (the daemon has a layout per
/// attached frontend and no frontend at construction), so `desktop.lua`
/// checks `pmacs.session.is_daemon()` and no-ops there.
pub struct DaemonMode;

/// Fire `buffer.after-load` from Rust with the current active buffer —
/// the seam desktop-restore uses (Q#DS3). `pub(crate)` so
/// [`crate::desktop`] can drive it.
pub(crate) fn fire_after_load_hook(lua: &Lua) {
    run_hook_if_defined(lua, "buffer.after-load", mlua::MultiValue::new());
}

/// `pmacs.session.*` — desktop-save (Arc 3 phase 2). All-Rust because
/// the layout serde + structural rebuild can't live in Lua (Q#DS1).
/// The thin `desktop.lua` builtin wires `desktop_mode` on top of these.
fn install_session_module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    // arm_restore(on): arm (or, with `false`, unarm) restore-on-startup.
    // A boolean app-data path so `desktop_mode(false)` can undo a prior
    // `desktop_mode(true)` — the marker is not one-way.
    m.set(
        "arm_restore",
        lua.create_function(|lua, on: Option<bool>| {
            if on.unwrap_or(true) {
                lua.set_app_data(DesktopRestoreArmed);
            } else {
                lua.remove_app_data::<DesktopRestoreArmed>();
            }
            Ok(())
        })?,
    )?;

    // is_daemon(): keep desktop save/restore local-only in v1 (Q#DS9).
    m.set(
        "is_daemon",
        lua.create_function(|lua, ()| Ok(lua.app_data_ref::<DaemonMode>().is_some()))?,
    )?;

    // save_desktop(): serialize the current session. Returns true when
    // a desktop was written (false = nothing to save / no state dir).
    m.set(
        "save_desktop",
        lua.create_function(|lua, ()| {
            crate::desktop::save_session(lua).map_err(mlua::Error::external)
        })?,
    )?;

    // restore_desktop(): rebuild the saved session (manual command;
    // the startup path goes through EditorState::restore_desktop_if_armed).
    m.set(
        "restore_desktop",
        lua.create_function(|lua, ()| {
            crate::desktop::restore_session(lua).map_err(mlua::Error::external)
        })?,
    )?;

    Ok(m)
}

/// The configured base state directory (Arc 3, Q#PS2). Present as Lua
/// app-data only when a real dir was resolved at startup; its absence
/// (the `cfg(test)` case, and any host without `HOME`/`XDG_STATE_HOME`)
/// makes every `pmacs.state.*` call a no-op, so default-on persistence
/// builtins never touch disk in `cargo test`.
pub struct StateDir(pub std::path::PathBuf);

/// `pmacs.state.{write,read,remove,path}` — the confined key→file store
/// (Q#PS2). All keys pass [`crate::state::validate_name`], so a state
/// call can never read or write outside the state directory. When the
/// state dir is unconfigured every call is inert: `write`/`remove`
/// return `false`, `read`/`path` return `nil`.
fn install_state_module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    m.set(
        "write",
        lua.create_function(|lua, (name, content): (String, mlua::String)| {
            let Some(base) = lua.app_data_ref::<StateDir>() else {
                return Ok(false);
            };
            crate::state::write(&base.0, &name, &content.as_bytes())
                .map_err(mlua::Error::external)?;
            Ok(true)
        })?,
    )?;

    m.set(
        "read",
        lua.create_function(|lua, name: String| {
            let Some(base) = lua.app_data_ref::<StateDir>() else {
                return Ok(None);
            };
            crate::state::read(&base.0, &name).map_err(mlua::Error::external)
        })?,
    )?;

    m.set(
        "remove",
        lua.create_function(|lua, name: String| {
            let Some(base) = lua.app_data_ref::<StateDir>() else {
                return Ok(false);
            };
            crate::state::remove(&base.0, &name).map_err(mlua::Error::external)?;
            Ok(true)
        })?,
    )?;

    m.set(
        "path",
        lua.create_function(|lua, name: String| {
            let Some(base) = lua.app_data_ref::<StateDir>() else {
                return Ok(None);
            };
            match crate::state::resolve(&base.0, &name) {
                Ok(p) => Ok(Some(p.display().to_string())),
                Err(e) => Err(mlua::Error::external(e)),
            }
        })?,
    )?;

    // True when a state directory is configured — lets Lua modules tell
    // "unconfigured (test / no HOME)" from "configured but empty".
    m.set(
        "available",
        lua.create_function(|lua, ()| Ok(lua.app_data_ref::<StateDir>().is_some()))?,
    )?;

    Ok(m)
}

/// Build the `pmacs.attach` Lua function (T M5.6d).
///
/// Init-time-only: refuses to run after [`InitCompleteFlag`] has been
/// flipped. Accepts either a `target` string (parsed via
/// [`AttachTarget::parse`]) or kwargs of the form `{ kind = "...", ... }`.
/// On success, records the validated target in the [`RequestedAttach`]
/// slot for the post-init dispatcher (M5.6g) to consume.
///
/// # v0.1 stub posture
///
/// Per the project's "validate locally, defer activation" rule, all
/// four kinds (`local`, `ssh`, `tls`, `custom`) parse and validate
/// here. Activation-time errors for the not-yet-implemented transports
/// surface from [`AttachTarget::check_v01`] when the dispatcher
/// (M5.6g, M5.7) tries to act on the stored target — not from this
/// binding. This keeps the upgrade path "v0.2 swaps the activation
/// path; init.lua doesn't change."
fn install_attach_binding(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(|lua, spec: Table| {
        require_init_phase(lua, "pmacs.attach")?;
        let target = parse_attach_spec(&spec)?;
        let slot = lua
            .app_data_ref::<RequestedAttach>()
            .ok_or_else(|| mlua::Error::external(BindingError::NoRequestedAttachSlot))?;
        if let Err(prev) = slot.try_set(target) {
            return Err(mlua::Error::external(
                BindingError::AttachAlreadyRequested {
                    prior: prev.to_string(),
                },
            ));
        }
        Ok(())
    })
}

/// Parse a `pmacs.attach{...}` spec table into an [`AttachTarget`].
///
/// Two accepted forms:
///
/// - **Target string:** `{ target = "kind:body" }` — delegates to
///   [`AttachTarget::parse`], which round-trips through
///   [`AttachTarget::Display`].
/// - **Kwargs:** `{ kind = "...", ... }` with kind-specific fields.
///   See the per-kind branches below for the schema.
///
/// All four kinds are accepted by this parser. Whether they activate
/// in v0.1 is decided later by [`AttachTarget::check_v01`].
fn parse_attach_spec(spec: &Table) -> mlua::Result<AttachTarget> {
    if let Ok(s) = spec.get::<String>("target") {
        return AttachTarget::parse(&s).map_err(mlua::Error::external);
    }

    let Ok(kind) = spec.get::<String>("kind") else {
        return Err(mlua::Error::external(
            BindingError::AttachSpecMissingKindOrTarget,
        ));
    };

    let target = match kind.as_str() {
        "local" => {
            let socket = spec.get::<String>("socket").map_err(|_| {
                mlua::Error::external(BindingError::AttachSpecField {
                    kind: "local",
                    field: "socket",
                    expected: "string",
                })
            })?;
            AttachTarget::LocalSocket(std::path::PathBuf::from(socket))
        }
        "ssh" => {
            let host = spec.get::<String>("host").map_err(|_| {
                mlua::Error::external(BindingError::AttachSpecField {
                    kind: "ssh",
                    field: "host",
                    expected: "string",
                })
            })?;
            let user = spec.get::<String>("user").ok();
            let instance_name = spec.get::<String>("instance").ok();
            AttachTarget::Ssh {
                host,
                user,
                instance_name,
            }
        }
        "tls" => {
            let endpoint = spec.get::<String>("endpoint").map_err(|_| {
                mlua::Error::external(BindingError::AttachSpecField {
                    kind: "tls",
                    field: "endpoint",
                    expected: "string",
                })
            })?;
            let cert = spec.get::<String>("cert").map_err(|_| {
                mlua::Error::external(BindingError::AttachSpecField {
                    kind: "tls",
                    field: "cert",
                    expected: "string",
                })
            })?;
            AttachTarget::Tls {
                endpoint,
                cert: std::path::PathBuf::from(cert),
            }
        }
        "custom" => {
            let cmd = spec.get::<Table>("command").map_err(|_| {
                mlua::Error::external(BindingError::AttachSpecField {
                    kind: "custom",
                    field: "command",
                    expected: "table (sequence of strings)",
                })
            })?;
            let mut command = Vec::with_capacity(cmd.raw_len());
            for v in cmd.sequence_values::<String>() {
                command.push(v.map_err(|_| {
                    mlua::Error::external(BindingError::AttachSpecField {
                        kind: "custom",
                        field: "command",
                        expected: "table of strings (each element a string)",
                    })
                })?);
            }
            AttachTarget::Custom { command }
        }
        other => {
            return Err(mlua::Error::external(BindingError::AttachSpecUnknownKind {
                got: other.to_string(),
            }));
        }
    };

    target.validate().map_err(mlua::Error::external)?;
    Ok(target)
}

/// Build the `pmacs.current_attachment` Lua function (T M5.6e).
///
/// Returns `nil` when no outbound attachment is recorded; otherwise
/// returns a freshly-built Lua table mirroring the [`AttachmentHandle`]
/// fields. The table is regenerated on each call — there is no
/// stable handle reference per the v0.1 stability disclaimer in the
/// [`AttachmentHandle`] doc comment.
///
/// In v0.1 the slot is empty in the typical lifecycle (Local mode is
/// its own instance, Daemon mode is a target not a source, Attach
/// mode has no Lua), so the function virtually always returns `nil`.
/// The shape exists for forward compatibility and so describe-instance
/// (M5.6f) can use a single getter that gracefully degrades to `nil`
/// on the in-process case.
fn install_current_attachment_binding(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(|lua, ()| -> mlua::Result<Value> {
        let slot = lua
            .app_data_ref::<CurrentAttachmentSlot>()
            .ok_or_else(|| mlua::Error::external(BindingError::NoCurrentAttachmentSlot))?;
        match slot.get() {
            Some(h) => Ok(Value::Table(handle_to_lua_table(lua, &h)?)),
            None => Ok(Value::Nil),
        }
    })
}

/// Convert an [`AttachmentHandle`] into a fresh Lua table snapshot.
fn handle_to_lua_table(lua: &Lua, h: &AttachmentHandle) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "frontend_id",
        i64::try_from(h.frontend_id.0).unwrap_or(i64::MAX),
    )?;
    t.set("identity", identity_to_lua_table(lua, &h.identity)?)?;
    t.set("target", target_to_lua_table(lua, &h.target)?)?;
    Ok(t)
}

fn identity_to_lua_table(lua: &Lua, id: &InstanceIdentity) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("pmacs_version", id.pmacs_version.as_str())?;
    // Optional fields are encoded as the underlying string or `nil`.
    // mlua maps Option<&str> to nil-or-string, which matches Lua's
    // idiom of "missing field" for absent metadata.
    match id.build_hash.as_deref() {
        Some(s) => t.set("build_hash", s)?,
        None => t.set("build_hash", Value::Nil)?,
    }
    match id.instance_name.as_deref() {
        Some(s) => t.set("instance_name", s)?,
        None => t.set("instance_name", Value::Nil)?,
    }
    t.set(
        "uptime_secs",
        i64::try_from(id.uptime_secs).unwrap_or(i64::MAX),
    )?;
    t.set("working_directory", id.working_directory.as_str())?;
    Ok(t)
}

fn target_to_lua_table(lua: &Lua, target: &AttachTarget) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("kind", target.kind_name())?;
    // `display` round-trips through `AttachTarget::parse`, so a Lua
    // caller can serialize / persist it and feed it back into
    // `pmacs.attach{ target = ... }` without ambiguity.
    t.set("display", target.to_string())?;
    match target {
        AttachTarget::LocalSocket(p) => {
            t.set("path", p.display().to_string())?;
        }
        AttachTarget::Ssh {
            host,
            user,
            instance_name,
        } => {
            t.set("host", host.as_str())?;
            match user.as_deref() {
                Some(s) => t.set("user", s)?,
                None => t.set("user", Value::Nil)?,
            }
            match instance_name.as_deref() {
                Some(s) => t.set("instance", s)?,
                None => t.set("instance", Value::Nil)?,
            }
        }
        AttachTarget::Tls { endpoint, cert } => {
            t.set("endpoint", endpoint.as_str())?;
            t.set("cert", cert.display().to_string())?;
        }
        AttachTarget::Custom { command } => {
            t.set(
                "command",
                lua.create_sequence_from(command.iter().cloned())?,
            )?;
        }
    }
    Ok(t)
}

/// Build the `pmacs.instance.*` Lua surface (T M5.6f).
///
/// Three functions are exposed:
///
/// * `pmacs.instance.identity()` — always returns a Lua table mirroring
///   [`InstanceIdentity::for_running_process`], built from the
///   [`LocalInstanceInfo`] app-data slot. Uptime is re-evaluated on
///   each call; the rest of the fields are stable across the process.
/// * `pmacs.instance.echo_line()` — returns the single-line summary
///   string ([`crate::instance_buffer::format_echo_line`]) used by the
///   `editor.describe-instance` command. The Lua command body owns
///   the choice of how to surface the string (status row, log, etc.).
/// * `pmacs.instance.show()` — populates / refreshes the
///   `*pmacs-instance*` buffer ([`crate::instance_buffer::render`])
///   and returns its `BufferIdLua` so the caller can switch to it.
fn install_instance_module(lua: &Lua, registry: &SharedRegistry) -> mlua::Result<Table> {
    let instance = lua.create_table()?;

    instance.set("identity", install_instance_identity_binding(lua)?)?;
    instance.set("echo_line", install_instance_echo_line_binding(lua)?)?;
    instance.set("show", install_instance_show_binding(lua, registry)?)?;

    Ok(instance)
}

/// `pmacs.instance.identity()` -> table.
fn install_instance_identity_binding(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(|lua, ()| -> mlua::Result<Table> {
        let info = lua
            .app_data_ref::<LocalInstanceInfo>()
            .ok_or_else(|| mlua::Error::external(BindingError::NoLocalInstanceInfo))?;
        let id = info.build_identity();
        identity_to_lua_table(lua, &id)
    })
}

/// `pmacs.instance.echo_line()` -> string.
fn install_instance_echo_line_binding(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(|lua, ()| -> mlua::Result<String> {
        let info = lua
            .app_data_ref::<LocalInstanceInfo>()
            .ok_or_else(|| mlua::Error::external(BindingError::NoLocalInstanceInfo))?;
        let identity = info.build_identity();
        let attachment = lua
            .app_data_ref::<CurrentAttachmentSlot>()
            .and_then(|s| s.get());
        Ok(crate::instance_buffer::format_echo_line(
            &identity,
            attachment.as_ref(),
        ))
    })
}

/// `pmacs.instance.show()` -> `BufferIdLua`.
fn install_instance_show_binding(lua: &Lua, registry: &SharedRegistry) -> mlua::Result<Function> {
    let reg = registry.clone();
    lua.create_function(move |lua, ()| -> mlua::Result<BufferIdLua> {
        let info = lua
            .app_data_ref::<LocalInstanceInfo>()
            .ok_or_else(|| mlua::Error::external(BindingError::NoLocalInstanceInfo))?;
        let identity = info.build_identity();
        let attachment = lua
            .app_data_ref::<CurrentAttachmentSlot>()
            .and_then(|s| s.get());
        let (id, edits) =
            crate::instance_buffer::render(&mut reg.borrow_mut(), &identity, attachment.as_ref());
        queue_generated_buffer_edits(lua, id, &edits);
        if !edits.is_empty() {
            rebuild_generated_buffer_views(lua, id);
        }
        Ok(BufferIdLua(id))
    })
}

/// T M10.10 post-audit-round-6 F31 — queue every CRDT op produced
/// by a generated-buffer render to the daemon's broadcast queue.
///
/// The three generated buffers (`*help*`, `*workers*`,
/// `*pmacs-instance*`) get upgraded to CRDT-backed at every
/// replica's attach via `send_buffer_snapshots`. Each subsequent
/// regenerate (delete-all + insert-new) produces zero, one, or two
/// `Edit`s carrying `crdt_op`. Replicas need every `CrdtOp` so their
/// `BufferMirror`s converge with the daemon's new content for
/// these buffers; without queueing, the replicas see the
/// `CellDelta` repaint but their mirrors permanently desync.
///
/// Caller: every site in `lua_bindings.rs` that drives one of the
/// render functions. The render functions return their Edits
/// alongside the `BufferId` so this helper can queue them via
/// `EditorCore::queue_daemon_origin_crdt_op`.
///
/// No-op when:
/// - No `SharedCore` is registered as Lua app data (early-stage
///   tests use the registry without an editor core).
/// - The edits' buffer wasn't CRDT-backed (`queue_daemon_origin_crdt_op`
///   itself early-returns when the edit has no `crdt_op`).
fn queue_generated_buffer_edits(lua: &Lua, buffer_id: BufferId, edits: &[crate::rope::Edit]) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    let mut core = core.borrow_mut();
    for edit in edits {
        core.queue_daemon_origin_crdt_op(buffer_id, edit);
    }
}

fn rebuild_generated_buffer_views(lua: &Lua, buffer_id: BufferId) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    core.borrow_mut().rebuild_views_for(buffer_id);
}

#[allow(clippy::too_many_lines)]
fn install_buffer_module(lua: &Lua, registry: &SharedRegistry) -> mlua::Result<Table> {
    let buffer = lua.create_table()?;

    {
        let reg = registry.clone();
        buffer.set(
            "create",
            lua.create_function(move |_, name: String| {
                let id = reg.borrow_mut().create(name);
                Ok(BufferIdLua(id))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "from_bytes",
            lua.create_function(move |_, (name, bytes): (String, mlua::String)| {
                let id = reg.borrow_mut().create_from_bytes(name, &bytes.as_bytes());
                Ok(BufferIdLua(id))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "from_file",
            lua.create_function(move |lua, path: String| -> mlua::Result<BufferIdLua> {
                let path_buf = std::path::PathBuf::from(&path);
                let (bytes, meta) = crate::file_io::load_file(&path_buf).map_err(|source| {
                    mlua::Error::external(std::io::Error::new(
                        source.kind(),
                        format!("failed to load {path}: {source}"),
                    ))
                })?;
                let id = reg.borrow_mut().create_from_bytes(path.clone(), &bytes);
                if let Some(core) = lua.app_data_ref::<SharedCore>() {
                    let mut core = core.borrow_mut();
                    core.switch_active_buffer(id)
                        .map_err(mlua::Error::external)?;
                    core.set_buffer_path(id, Some(path_buf));
                    core.set_buffer_meta(id, Some(meta));
                }
                run_hook_if_defined(lua, "buffer.after-load", mlua::MultiValue::new());
                Ok(BufferIdLua(id))
            })?,
        )?;
    }

    {
        // Arc 1b Q#P6: mark (or unmark) a buffer as requiring
        // round-trip input. While a marked buffer is active,
        // `dispatch_idle` reports false, so semantic frontends'
        // optimistic-apply stays off — RET reaches the buffer-local
        // bindings (a panel's visit) and typing reaches the read-only
        // intercept instead of landing as a CRDT import that bypasses
        // it. `pmacs.listview` marks every panel it creates.
        buffer.set(
            "set_round_trip_input",
            lua.create_function(move |lua, (id, on): (BufferIdLua, bool)| {
                if let Some(core) = lua.app_data_ref::<SharedCore>() {
                    core.borrow_mut().set_round_trip_input(id.0, on);
                }
                Ok(())
            })?,
        )?;
    }

    {
        // T M4.5 L1: find-or-open. If a buffer is already bound to
        // `path`, switch to it (preserving unsaved edits — no
        // reload); otherwise behave like `from_file`. The dedup is
        // what makes cross-file navigation reuse an open file
        // instead of spawning a duplicate buffer (SP-4 Gap A).
        let reg = registry.clone();
        buffer.set(
            "find_or_open",
            lua.create_function(move |lua, path: String| -> mlua::Result<BufferIdLua> {
                let path_buf = std::path::PathBuf::from(&path);
                if let Some(existing) = reg.borrow().find_by_path(&path_buf) {
                    if let Some(core) = lua.app_data_ref::<SharedCore>() {
                        core.borrow_mut()
                            .switch_active_buffer(existing)
                            .map_err(mlua::Error::external)?;
                    }
                    // Arc 1b: switching clears the window's overlays;
                    // subscribers (syntax highlight, LSP style/diag
                    // views) re-attach theirs. The fresh-load branch
                    // below fires `buffer.after-load` instead.
                    run_hook_if_defined(lua, "buffer.after-switch", mlua::MultiValue::new());
                    return Ok(BufferIdLua(existing));
                }
                let (bytes, meta) = crate::file_io::load_file(&path_buf).map_err(|source| {
                    mlua::Error::external(std::io::Error::new(
                        source.kind(),
                        format!("failed to load {path}: {source}"),
                    ))
                })?;
                let id = reg.borrow_mut().create_from_bytes(path.clone(), &bytes);
                if let Some(core) = lua.app_data_ref::<SharedCore>() {
                    let mut core = core.borrow_mut();
                    core.switch_active_buffer(id)
                        .map_err(mlua::Error::external)?;
                    core.set_buffer_path(id, Some(path_buf));
                    core.set_buffer_meta(id, Some(meta));
                }
                run_hook_if_defined(lua, "buffer.after-load", mlua::MultiValue::new());
                Ok(BufferIdLua(id))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "list",
            lua.create_function(move |lua, ()| {
                let r = reg.borrow();
                let t = lua.create_table()?;
                for (i, id) in r.ids().iter().enumerate() {
                    t.set(i + 1, BufferIdLua(*id))?;
                }
                Ok(t)
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "major_mode",
            lua.create_function(move |_, id: BufferIdLua| {
                let r = reg.borrow();
                Ok(resolve(&r, id.0)?.major_mode().map(str::to_owned))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "set_major_mode",
            lua.create_function(move |_, (id, mode): (BufferIdLua, Value)| {
                let mode = match mode {
                    Value::Nil => None,
                    Value::String(mode) => Some(mode.to_str()?.to_owned()),
                    other => {
                        return Err(mlua::Error::external(format!(
                            "pmacs.buffer.set_major_mode: mode must be a string or nil, got {}",
                            other.type_name()
                        )));
                    }
                };
                let mut r = reg.borrow_mut();
                resolve_mut(&mut r, id.0)?.set_major_mode(mode);
                Ok(())
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "remove",
            lua.create_function(move |lua, id: BufferIdLua| {
                remove_buffer_and_fire(lua, &reg, id.0)
            })?,
        )?;
    }

    {
        // T M4.5 L4 — apply one LSP `WorkspaceEdit` resource op.
        // Callers (the `apply_workspace_edit` Lua applier) resolve
        // URIs to paths first, so this works in plain filesystem
        // paths and also reconciles any open buffer: a renamed file's
        // buffer is rebound to the new path; a deleted file's buffer
        // is removed. `spec.kind` is "create" | "rename" | "delete".
        let reg = registry.clone();
        buffer.set(
            "apply_resource_op",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
                let io_err = |ctx: &str, e: std::io::Error| {
                    mlua::Error::external(std::io::Error::new(
                        e.kind(),
                        format!("apply_resource_op {ctx}: {e}"),
                    ))
                };
                let kind: String = spec.get("kind")?;
                match kind.as_str() {
                    "create" => {
                        let path: String = spec.get("path")?;
                        let pb = std::path::PathBuf::from(&path);
                        let overwrite: bool = spec.get("overwrite").unwrap_or(false);
                        let ignore_if_exists: bool = spec.get("ignore_if_exists").unwrap_or(false);
                        if pb.exists() && ignore_if_exists && !overwrite {
                            return Ok(());
                        }
                        if let Some(parent) = pb.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| io_err("create (parents)", e))?;
                        }
                        // Create, or truncate when overwrite is set /
                        // implied (no options ⇒ overwrite per spec).
                        std::fs::write(&pb, b"").map_err(|e| io_err("create", e))?;
                    }
                    "rename" => {
                        let old_p: String = spec.get("old_path")?;
                        let new_p: String = spec.get("new_path")?;
                        let from = std::path::PathBuf::from(&old_p);
                        let to = std::path::PathBuf::from(&new_p);
                        let overwrite: bool = spec.get("overwrite").unwrap_or(false);
                        let ignore_if_exists: bool = spec.get("ignore_if_exists").unwrap_or(false);
                        if to.exists() && ignore_if_exists && !overwrite {
                            return Ok(());
                        }
                        if let Some(parent) = to.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| io_err("rename (parents)", e))?;
                        }
                        std::fs::rename(&from, &to).map_err(|e| io_err("rename", e))?;
                        let bid = reg.borrow().find_by_path(&from);
                        if let Some(id) = bid
                            && let Some(core) = lua.app_data_ref::<SharedCore>()
                        {
                            core.borrow_mut().set_buffer_path(id, Some(to.clone()));
                        }
                    }
                    "delete" => {
                        let path: String = spec.get("path")?;
                        let pb = std::path::PathBuf::from(&path);
                        let recursive: bool = spec.get("recursive").unwrap_or(false);
                        let ignore_if_not_exists: bool =
                            spec.get("ignore_if_not_exists").unwrap_or(false);
                        match std::fs::symlink_metadata(&pb) {
                            Ok(md) => {
                                let r = if md.is_dir() {
                                    if recursive {
                                        std::fs::remove_dir_all(&pb)
                                    } else {
                                        std::fs::remove_dir(&pb)
                                    }
                                } else {
                                    std::fs::remove_file(&pb)
                                };
                                r.map_err(|e| io_err("delete", e))?;
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                if !ignore_if_not_exists {
                                    return Err(io_err("delete", e));
                                }
                            }
                            Err(e) => return Err(io_err("delete (stat)", e)),
                        }
                        let bid = reg.borrow().find_by_path(&pb);
                        if let Some(id) = bid {
                            remove_buffer_and_fire(lua, &reg, id)?;
                        }
                    }
                    other => {
                        return Err(mlua::Error::external(format!(
                            "apply_resource_op: unknown kind {other:?}"
                        )));
                    }
                }
                Ok(())
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "on_removed",
            lua.create_function(
                move |lua,
                      (id, body): (BufferIdLua, Function)|
                      -> mlua::Result<BufferRemoveCallbackHandleLua> {
                    {
                        let r = reg.borrow();
                        resolve(&r, id.0)?;
                    }
                    let callbacks = lua
                        .app_data_ref::<BufferRemoveCallbacks>()
                        .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
                    let callback_id = callbacks.add(id.0, body, caller_source(lua, 2));
                    Ok(BufferRemoveCallbackHandleLua {
                        buffer: id.0,
                        callback_id,
                    })
                },
            )?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "mark_create",
            lua.create_function(
                move |_, (id, pos, opts): (BufferIdLua, i64, Option<Table>)| {
                    let gravity = parse_mark_gravity(opts.as_ref())?;
                    let pos = u64_from_lua(pos)?;
                    let mut r = reg.borrow_mut();
                    let buf = resolve_mut(&mut r, id.0)?;
                    let mark = buf
                        .create_mark(pos, gravity)
                        .map_err(mlua::Error::external)?;
                    Ok(MarkHandleLua { buffer: id.0, mark })
                },
            )?,
        )?;
    }

    // M6.4: chained intercept registration. The view chain in
    // `crate::buffer::Buffer` is the underlying primitive; each
    // registered Lua function becomes a `LuaInterceptView` attached
    // via `Buffer::attach_view`. Multiple intercepts run in attach
    // order, threading the (possibly transformed) op through; any
    // intercept may reject by raising. The M6.4 REPL package uses
    // exactly one of these per REPL buffer.
    {
        let reg = registry.clone();
        buffer.set(
            "add_intercept",
            lua.create_function(
                move |lua,
                      (id, body): (BufferIdLua, Function)|
                      -> mlua::Result<InterceptHandleLua> {
                    let mut r = reg.borrow_mut();
                    let buf = resolve_mut(&mut r, id.0)?;
                    let view = LuaInterceptView {
                        lua: lua.clone(),
                        body,
                    };
                    let view_id = buf.attach_view(Box::new(view));
                    Ok(InterceptHandleLua {
                        buffer: id.0,
                        view: view_id,
                    })
                },
            )?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "remove_intercept",
            lua.create_function(move |_, handle: InterceptHandleLua| -> mlua::Result<bool> {
                let mut r = reg.borrow_mut();
                // Stale buffer handle — treat as already-removed,
                // matching the contract that detach_view returns
                // None for unknown view IDs (idempotent removal).
                let Ok(buf) = r.get_mut(handle.buffer) else {
                    return Ok(false);
                };
                Ok(buf.detach_view(handle.view).is_some())
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        buffer.set(
            "add_style_overlay",
            lua.create_function(
                move |lua, id: BufferIdLua| -> mlua::Result<StyleOverlayHandleLua> {
                    let spans = Arc::new(Mutex::new(Vec::new()));
                    // Coordinate translation lives on the BUFFER
                    // (PR #113 round-5 finding 1): buffer-attached
                    // views see every edit exactly once — Lua bypass
                    // writes, undo/redo, remote CRDT ops — whether or
                    // not any window shows the buffer. The window
                    // attachments below are render-only; per-window
                    // translation ran once per split and zero times
                    // hidden.
                    let translator = {
                        let mut r = reg.borrow_mut();
                        let buf = resolve_mut(&mut r, id.0)?;
                        buf.attach_view(Box::new(crate::overlay::BufferStyleSpanTranslator::new(
                            Arc::clone(&spans),
                        )))
                    };
                    let handle = StyleOverlayHandleLua {
                        spans: Arc::clone(&spans),
                        buffer: id.0,
                        translator,
                        disposed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    };
                    attach_style_overlay_to_visible_windows(lua, id.0, &spans);
                    Ok(handle)
                },
            )?,
        )?;
    }

    {
        buffer.set(
            "attach_style_overlay",
            lua.create_function(
                move |lua, (id, handle): (BufferIdLua, StyleOverlayHandleLua)| {
                    // Round-7 finding 1: a disposed handle's
                    // translator is gone — re-attaching would
                    // resurrect rendering with frozen coordinates —
                    // and a handle's translator follows edits to ITS
                    // buffer only, so attaching to any other buffer
                    // shows unmaintained spans.
                    if handle.disposed.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(mlua::Error::external(
                            "this style overlay handle was disposed; create a fresh \
                             one with pmacs.buffer.add_style_overlay",
                        ));
                    }
                    if id.0 != handle.buffer {
                        return Err(mlua::Error::external(format!(
                            "this style overlay handle belongs to buffer {:?}; its \
                             spans are not translated by edits to {:?} — create an \
                             overlay for that buffer with pmacs.buffer.add_style_overlay",
                            handle.buffer, id.0
                        )));
                    }
                    // The recorded owner may have been removed since
                    // the handle was created. Buffer IDs are
                    // generational, so resolving it is the only way
                    // to distinguish a live owner from a stale handle;
                    // silently scanning the windows would otherwise
                    // report a successful no-op.
                    with_registry(lua, |r| {
                        resolve(r, id.0)?;
                        Ok(())
                    })?;
                    attach_style_overlay_to_visible_windows(lua, id.0, &handle.spans);
                    Ok(())
                },
            )?,
        )?;
    }

    Ok(buffer)
}

fn parse_mark_gravity(opts: Option<&Table>) -> mlua::Result<MarkGravity> {
    let Some(opts) = opts else {
        return Ok(MarkGravity::Right);
    };
    let gravity = opts.get::<Option<String>>("gravity")?;
    match gravity.as_deref().unwrap_or("right") {
        "left" => Ok(MarkGravity::Left),
        "right" => Ok(MarkGravity::Right),
        other => Err(mlua::Error::external(format!(
            "pmacs.buffer.mark_create: opts.gravity must be \"left\" or \"right\"; got {other:?}"
        ))),
    }
}

fn attach_style_overlay_to_visible_windows(
    lua: &Lua,
    buffer_id: BufferId,
    spans: &crate::overlay::SharedBufferStyleSpans,
) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    let mut core = core.borrow_mut();
    for win in core.windows.values_mut() {
        if win.buffer_id == buffer_id {
            // ensure_overlay: idempotent per window via the store's
            // identity (round-6 finding 1) — repeated switches into
            // the buffer stacked duplicate render views on passive
            // panes, each cloning every span and rescanning the
            // buffer per frame.
            win.ensure_overlay(Box::new(crate::overlay::BufferStyleOverlay::new(
                Arc::clone(spans),
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// pmacs.ansi: M6.4-side exposure of the M6.3 parser
// ---------------------------------------------------------------------------

/// Lua-facing wrapper around [`crate::ansi::AnsiParser`].
///
/// Constructed via `pmacs.ansi.parser()`; methods `feed(bytes)`,
/// `reset()`, and `finish()` mirror the Rust API. `feed` and
/// `finish` return an array of event tables --- see
/// [`event_to_lua_table`] for the schema; `finish` drains stream-end
/// state and resets the parser for a fresh stream. The wrapper
/// is `RefCell`-internal so multiple Lua-side methods can borrow
/// safely; the Lua VM is single-threaded so the borrow can never
/// race.
pub struct AnsiParserLua(RefCell<crate::ansi::AnsiParser>);

impl UserData for AnsiParserLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("feed", |lua, this, bytes: mlua::String| {
            let raw = bytes.as_bytes();
            let events = this.0.borrow_mut().feed(&raw);
            let out = lua.create_table_with_capacity(events.len(), 0)?;
            for (i, ev) in events.iter().enumerate() {
                out.set(i + 1, event_to_lua_table(lua, ev)?)?;
            }
            Ok(out)
        });

        methods.add_method("reset", |_, this, ()| {
            this.0.borrow_mut().reset();
            Ok(())
        });

        // Stream-end finalization: flushes cross-feed state (an
        // incomplete UTF-8 sequence becomes the replacement
        // character). Compile-mode calls this once at the process's
        // terminal event (Q#CM4; PR #113 round-1 finding 9).
        methods.add_method("finish", |lua, this, ()| {
            let events = this.0.borrow_mut().finish();
            let out = lua.create_table_with_capacity(events.len(), 0)?;
            for (i, ev) in events.iter().enumerate() {
                out.set(i + 1, event_to_lua_table(lua, ev)?)?;
            }
            Ok(out)
        });

        methods.add_meta_method(mlua::MetaMethod::ToString, |_, _this, ()| {
            Ok("AnsiParser".to_string())
        });
    }
}

/// Build the `pmacs.ansi.*` table. The only entry today is
/// `parser()`; future additions (e.g. an event-table-validator
/// helper) live alongside it.
fn install_ansi_module(lua: &Lua) -> mlua::Result<Table> {
    let ansi = lua.create_table()?;
    ansi.set(
        "parser",
        lua.create_function(|_, ()| {
            Ok(AnsiParserLua(RefCell::new(crate::ansi::AnsiParser::new())))
        })?,
    )?;
    Ok(ansi)
}

// ---------------------------------------------------------------------------
// pmacs.packages module (T M7.3)
// ---------------------------------------------------------------------------

/// Build the `pmacs.packages.*` table.
///
/// Surface:
///
/// - `pmacs.packages.install(spec)` --- install to user-config root
///   (`$XDG_DATA_HOME/pmacs/packages/`).
/// - `pmacs.packages.install_project(spec)` --- install to the project
///   root (`<cwd>/.pmacs/packages/`, override with `project_root` in
///   the spec).
/// - `pmacs.packages.installed()` --- snapshot of packages that
///   completed install during the init phase.
/// - `pmacs.packages.update(name?)` --- re-resolve top-level
///   packages against the current upstream and replace the
///   on-disk install. With no argument, updates every top-level
///   entry recorded in `<install_root>/pmacs.lock`; with a name,
///   updates only that entry (`UpdatePolicy::UpdateOne`). Returns
///   a Lua summary table reporting `version`, `commit`,
///   `prior_commit`, and `changed` per package.
///
/// Both install variants are init-time-only via [`require_init_phase`];
/// mid-session calls produce [`BindingError::InitOnlyApi`] naming
/// the equivalent CLI flag (none yet --- restart with an updated
/// init.lua). Each install is synchronous: errors raise back at the
/// call site so the offending init.lua line is named in the traceback.
/// Comma-separated quoted list of available exports, with `(none)`
/// when the manifest declares no exports. Used by
/// [`BindingError::PackageNotExported`]'s `Display`.
fn format_exports_for_error(exports: &[String]) -> String {
    if exports.is_empty() {
        return "(none)".to_string();
    }
    let mut quoted: Vec<String> = exports.iter().map(|e| format!("`{e}`")).collect();
    quoted.sort();
    quoted.dedup();
    quoted.join(", ")
}

#[allow(
    clippy::too_many_lines,
    reason = "linear list of packages.set(...) bindings, each a small closure; \
              splitting into helpers fragments the surface without removing decisions"
)]
fn install_packages_module(lua: &Lua) -> mlua::Result<Table> {
    let packages = lua.create_table()?;

    packages.set(
        "install",
        lua.create_function(|lua, spec: Value| -> mlua::Result<Table> {
            require_init_phase(lua, "pmacs.packages.install")?;
            let install_spec = parse_lua_install_spec(&spec)?;
            do_install(lua, &install_spec, &InstallScope::User)
        })?,
    )?;

    packages.set(
        "install_project",
        lua.create_function(|lua, spec: Value| -> mlua::Result<Table> {
            require_init_phase(lua, "pmacs.packages.install_project")?;
            let install_spec = parse_lua_install_spec(&spec)?;
            // Allow `project_root = "..."` override in the table form.
            let project_root = install_spec_project_root(lua, &spec)?;
            do_install(lua, &install_spec, &InstallScope::Project { project_root })
        })?,
    )?;

    packages.set(
        "installed",
        lua.create_function(|lua, ()| -> mlua::Result<Table> {
            let slot = lua
                .app_data_ref::<InstalledPackages>()
                .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;
            let snapshot = slot.snapshot();
            let t = lua.create_table()?;
            for (i, pkg) in snapshot.iter().enumerate() {
                t.set(i + 1, installed_package_to_lua(lua, pkg)?)?;
            }
            Ok(t)
        })?,
    )?;

    packages.set(
        "update",
        lua.create_function(|lua, target: Option<String>| -> mlua::Result<Table> {
            require_init_phase(lua, "pmacs.packages.update")?;
            do_update(lua, target.as_deref())
        })?,
    )?;

    packages.set(
        "install_local",
        lua.create_function(|lua, source_path: String| -> mlua::Result<Table> {
            require_init_phase(lua, "pmacs.packages.install_local")?;
            do_install_local(lua, std::path::Path::new(&source_path))
        })?,
    )?;

    packages.set(
        "on_unload",
        lua.create_function(|lua, callback: Function| -> mlua::Result<()> {
            // Recover the calling package's basename via *identity*
            // comparison against the cached per-package _ENV
            // tables. The M7.7 searcher attaches each package's
            // private env (`pmacs.pkgenvs/<basename>`) as the
            // chunk's `_ENV`; a closure created inside that chunk
            // inherits that exact table by reference. Walking the
            // cache and matching `candidate == callback_env` finds
            // the basename without trusting any field the chunk
            // could fabricate.
            //
            // Calls from non-package code (init.lua's top level,
            // the REPL) hit a callback whose env is _G or some
            // non-package table; that doesn't match anything in the
            // cache, so the lookup falls through to the
            // PackagesOnUnloadOutsidePackage error. A user setting
            // `_PACKAGE = { name = "victim" }` in _G doesn't
            // matter --- _G is never a registered env table.
            // Primary: identity-check the callback's env against
            // the cached per-package envs. Works whenever the
            // closure references any global (so Lua compiles it
            // with `_ENV` as an upvalue) --- the typical case.
            let basename_from_env = match callback.environment() {
                Some(env) => env_table_basename(lua, &env)?,
                None => None,
            };
            // Fallback: under Lua 5.4 a closure that touches only
            // locals doesn't capture `_ENV`, so
            // `Function::environment` returns `None` and the
            // identity check has nothing to compare. Recover the
            // owning package from the [`CurrentlyLoadingPackage`]
            // stack: if we're inside a chunk-load (the typical
            // moment for `on_unload` registration), the wrapped
            // loader has pushed the basename for us. Calls from
            // outside any package's chunk have an empty stack and
            // surface the standard error.
            let basename = if let Some(b) = basename_from_env {
                b
            } else {
                let from_stack = lua
                    .app_data_ref::<CurrentlyLoadingPackage>()
                    .and_then(|s| s.top());
                from_stack.ok_or_else(|| {
                    mlua::Error::external(BindingError::PackagesOnUnloadOutsidePackage)
                })?
            };
            let slot = lua
                .app_data_ref::<PackageUnloadHooks>()
                .ok_or_else(|| mlua::Error::external(BindingError::NoUnloadHooksSlot))?;
            slot.register(&basename, callback);
            Ok(())
        })?,
    )?;

    packages.set(
        "reload",
        lua.create_function(|lua, name: String| -> mlua::Result<Value> { do_reload(lua, &name) })?,
    )?;

    packages.set(
        "load",
        lua.create_function(|lua, name: String| -> mlua::Result<bool> {
            // T M7.8 isolation boundary for load-time errors. Wraps
            // `require(name)` in a Rust-side catch so a single broken
            // package doesn't abort the surrounding init.lua: caller
            // gets `false` and the error lands in `*errors*` tagged
            // with the package name. Successful loads return `true`.
            //
            // Cancellations propagate (return Err) rather than being
            // logged as a load error: a C-g during `require` is a
            // user-initiated abort, not a package failure, and the
            // outer eval's error path resets the flag.
            let require: Function = lua.globals().get("require")?;
            match require.call::<Value>(name.clone()) {
                Ok(_) => Ok(true),
                Err(e) => {
                    if crate::lua_isolation::is_cancellation(&e) {
                        return Err(e);
                    }
                    log_package_load_error(lua, &name, &e);
                    Ok(false)
                }
            }
        })?,
    )?;

    packages.set(
        "describe",
        lua.create_function(|lua, name: String| -> mlua::Result<mlua::Value> {
            let slot = lua
                .app_data_ref::<InstalledPackages>()
                .ok_or_else(|| mlua::Error::external(BindingError::DescribeNoRoster))?;
            let snapshot = slot.snapshot();
            // Most-recent-first match, mirroring searcher precedence.
            for pkg in snapshot.iter().rev() {
                if pkg.install_basename() != name {
                    continue;
                }
                return Ok(mlua::Value::Table(installed_package_describe_table(
                    lua, pkg,
                )?));
            }
            Ok(mlua::Value::Nil)
        })?,
    )?;

    register_package_searcher(lua)?;

    Ok(packages)
}

/// Register a custom searcher in `package.searchers` (Lua 5.4) /
/// `package.loaders` (Lua 5.1, `LuaJIT`) that consults the
/// [`InstalledPackages`] roster at require time.
///
/// # Three responsibilities (T M7.7)
///
/// 1. **Resolve `require("<basename>")` to the manifest's `entry`.**
///    Carried over from T M7.3: a package whose entry isn't at the
///    standard `<basename>/init.lua` path needs the searcher to map
///    the require to the manifest's declared file.
/// 2. **Gate access via the `exports` whitelist.** Per spec, only
///    submodules listed in `manifest.exports` are visible to other
///    packages. `require("<basename>.<sub>")` for an unlisted
///    `<sub>` raises a clear error naming the package and the
///    available exports — the searcher takes responsibility for the
///    require-name (rather than returning "not found" and letting
///    `package.path` find the file anyway, which would defeat the
///    whitelist).
/// 3. **Set a per-package environment table** on every loaded
///    chunk. Each package gets its own `_ENV` (cached by basename in
///    the Lua registry), with `__index = _G` so reads still see the
///    standard library and pmacs API but writes stay local. This
///    enforces the "package A cannot accidentally pollute package B's
///    globals" boundary called out in the M7.7 spec without requiring
///    a Lua sandbox.
///
/// # Precedence
///
/// The searcher is **inserted at the front** of the searchers list
/// (position 1, before the path-based searcher). This makes the
/// pmacs roster authoritative for installed packages: `require("foo")`
/// where `foo` is an installed package goes through this searcher
/// even if a `foo.lua` exists somewhere on `package.path`. Requires
/// for non-installed names return a string (Lua's "not found" idiom)
/// so subsequent searchers — preload, path-based — get a turn.
///
/// In M7.3 the searcher was *appended*; that ordering predates
/// `exports` enforcement. The path searcher would happily find
/// `<install>/internal.lua` regardless of whether `internal` was in
/// the exports list, defeating the whitelist. M7.7 needs the pmacs
/// searcher to win first or `exports` is decorative.
///
/// # 5.1 vs 5.4 names
///
/// Lua 5.1 / `LuaJIT` exposes the searcher list as `package.loaders`;
/// Lua 5.2+ renamed it to `package.searchers`. Both are tables of
/// functions with the same callback shape. We probe `searchers`
/// first and fall back to `loaders` so the same code works under
/// both feature flags.
fn register_package_searcher(lua: &Lua) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let searchers: Table = match package.get::<Option<Table>>("searchers")? {
        Some(t) => t,
        None => package.get::<Table>("loaders")?,
    };

    let searcher = lua.create_function(|lua, name: String| -> mlua::Result<mlua::Value> {
        let Some(slot) = lua.app_data_ref::<InstalledPackages>() else {
            // Slot uninstalled (shouldn't happen under production
            // wiring; defensive nil keeps require working under
            // unusual test setups).
            return Ok(mlua::Value::Nil);
        };
        let snapshot = slot.snapshot();

        match lookup_in_roster(&name, &snapshot) {
            LookupOutcome::NotInstalled => {
                // Fall through to the next searcher. Lua appends
                // this string to the aggregate require-error
                // message if no later searcher succeeds either.
                let s =
                    lua.create_string(format!("\n\tno installed pmacs package named '{name}'"))?;
                Ok(mlua::Value::String(s))
            }
            LookupOutcome::EntryModule { entry_path } => {
                load_package_chunk(lua, &name, &entry_path, package_basename_from_name(&name))
            }
            LookupOutcome::ExportedModule { file_path, kind } => {
                if matches!(kind, ResolvedKind::MissingBoth) {
                    // Manifest promises this export, but neither
                    // `<dir>/x.lua` nor `<dir>/x/init.lua` exists.
                    // Surface the broken-package state with the
                    // declared-but-absent file path so the
                    // packager can fix the manifest or ship the
                    // missing file.
                    return Err(mlua::Error::external(
                        BindingError::PackageExportFileMissing {
                            requested: name.clone(),
                            expected_path: file_path.display().to_string(),
                        },
                    ));
                }
                load_package_chunk(lua, &name, &file_path, package_basename_from_name(&name))
            }
            LookupOutcome::NotExported {
                package,
                requested,
                exports,
            } => Err(mlua::Error::external(BindingError::PackageNotExported {
                package,
                requested,
                exports,
            })),
        }
    })?;

    // Insert at position 1, before all existing searchers, so the
    // pmacs roster is authoritative for installed packages. Lua
    // tables are 1-indexed; we shift the existing entries up by one.
    let len = searchers.raw_len();
    for i in (1..=len).rev() {
        let existing: mlua::Value = searchers.get(i)?;
        searchers.set(i + 1, existing)?;
    }
    searchers.set(1, searcher)?;
    Ok(())
}

/// First segment of a dotted require name (or the whole name if no
/// dot). Used to identify which package a chunk belongs to so its
/// environment table can be cached and shared across the package's
/// modules.
fn package_basename_from_name(name: &str) -> &str {
    name.split_once('.').map_or(name, |(h, _)| h)
}

/// Load a chunk from disk, set its environment to the package's
/// per-package env table, and return it wrapped as a Lua function
/// that the searcher hands back to `require`.
fn load_package_chunk(
    lua: &Lua,
    require_name: &str,
    file_path: &std::path::Path,
    package_basename: &str,
) -> mlua::Result<mlua::Value> {
    let bytes = match std::fs::read(file_path) {
        Ok(b) => b,
        Err(e) => {
            // Searcher convention: a string return becomes a "not
            // found, here's why" reason appended to the require
            // error. The package was correctly identified but its
            // file isn't readable; that's a packager / disk error,
            // not a "wrong basename" error, so we still surface a
            // searcher-style message rather than raising.
            let s = lua.create_string(format!(
                "\n\tpmacs package '{require_name}' \
                 file `{}` could not be read: {e}",
                file_path.display(),
            ))?;
            return Ok(mlua::Value::String(s));
        }
    };
    let chunk_name = format!("@{}", file_path.display());
    let func = lua.load(&bytes).set_name(&chunk_name).into_function()?;
    let env = package_env_for(lua, package_basename)?;
    func.set_environment(env)?;

    // Wrap the chunk function so the package's basename is pushed
    // onto the [`CurrentlyLoadingPackage`] stack before the chunk
    // runs and popped after it returns (or errors). This is the
    // fallback path `pmacs.packages.on_unload` uses when the
    // callback doesn't carry an `_ENV` upvalue --- a Lua 5.4
    // closure that touches only locals/upvalues compiles without
    // an `_ENV` capture, and `Function::environment` then returns
    // `None`, defeating the env-identity ownership check. The push
    // makes "I am loading <basename> right now" recoverable from
    // the binding without depending on closure-time env capture.
    //
    // The stack is popped on both success and error paths so a
    // failing chunk can't leak a basename into the next load.
    let basename_owned = package_basename.to_string();
    let wrapped = lua.create_function(
        move |lua, args: mlua::MultiValue| -> mlua::Result<mlua::MultiValue> {
            if let Some(slot) = lua.app_data_ref::<CurrentlyLoadingPackage>() {
                slot.push(basename_owned.clone());
            }
            let result = func.call::<mlua::MultiValue>(args);
            if let Some(slot) = lua.app_data_ref::<CurrentlyLoadingPackage>() {
                slot.pop();
            }
            result
        },
    )?;
    Ok(mlua::Value::Function(wrapped))
}

/// Registry key under which per-package `_ENV` tables are cached.
/// Owned by [`package_env_for`] and cleared by
/// [`clear_package_env`] on reload / `install_local` replacement.
const PACKAGE_ENVS_REGISTRY_KEY: &str = "pmacs.pkgenvs";

/// Get (or lazily create) the per-package environment table for
/// `basename`. Cached in the Lua registry under
/// [`PACKAGE_ENVS_REGISTRY_KEY`]`/<basename>`.
///
/// The env table has `__index = _G` so reads see the standard
/// library and the pmacs API; writes stay local. A `_PACKAGE` table
/// inside the env carries the package's basename for introspection.
fn package_env_for(lua: &Lua, basename: &str) -> mlua::Result<Table> {
    let envs = package_envs_table(lua)?;
    if let Some(env) = envs.get::<Option<Table>>(basename)? {
        return Ok(env);
    }
    let env = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set("__index", lua.globals())?;
    env.set_metatable(Some(mt));
    let info = lua.create_table_with_capacity(0, 1)?;
    info.set("name", basename)?;
    env.set("_PACKAGE", info)?;
    envs.set(basename, env.clone())?;
    Ok(env)
}

/// Lazily get-or-create the env-cache table at
/// [`PACKAGE_ENVS_REGISTRY_KEY`]. Shared between
/// [`package_env_for`] and [`clear_package_env`] /
/// [`callback_belongs_to_package`] so the registry layout is owned
/// in exactly one place.
fn package_envs_table(lua: &Lua) -> mlua::Result<Table> {
    if let Some(t) = lua.named_registry_value::<Option<Table>>(PACKAGE_ENVS_REGISTRY_KEY)? {
        return Ok(t);
    }
    let t = lua.create_table()?;
    lua.set_named_registry_value(PACKAGE_ENVS_REGISTRY_KEY, t.clone())?;
    Ok(t)
}

/// Drop the cached `_ENV` table for `basename`, so the next
/// [`package_env_for`] call constructs a fresh one. Called from
/// `pmacs.packages.reload(name)` (after `on_unload` hooks run,
/// before re-`require`) and from `pmacs.packages.install_local`
/// when it replaces an existing symlink to a different source.
///
/// Without this, removed-from-source globals stay visible after
/// reload because the env table outlives the chunk that wrote
/// them. The dev-loop story ("edit on disk, see new behavior") is
/// only honest if env globals reset on reload.
///
/// Also drops any `on_unload` hooks still registered for
/// `basename`. Hooks that survived the just-run cycle were
/// registered by closures whose env-table is the *old* env we're
/// about to discard --- they reference a chunk that no longer
/// exists. Firing them on the next cycle would either trip the
/// identity check in `pmacs.packages.on_unload` (because the env
/// is no longer in the cache) or call into resources the dead
/// chunk thinks are gone. The freshly-required chunk will
/// re-register whatever hooks it still wants.
fn clear_package_env(lua: &Lua, basename: &str) -> mlua::Result<()> {
    let envs = package_envs_table(lua)?;
    envs.set(basename, mlua::Value::Nil)?;
    if let Some(slot) = lua.app_data_ref::<PackageUnloadHooks>() {
        let _ = slot.drain(basename);
    }
    Ok(())
}

/// Run the registered `on_unload` hooks for `basename` in
/// registration order. The cycle's hooks are *snapshotted* at
/// start (drained from the live registry into a local queue);
/// new `on_unload` registrations made by hook bodies land in the
/// now-empty live registry slot instead of extending the current
/// queue. On a successful reload / replacement, [`clear_package_env`]
/// drops those old-env survivors before the freshly-required chunk
/// registers next-cycle hooks. This prevents a self-replicating hook
/// from looping the current cycle indefinitely.
///
/// Each queued hook is called in order. A successful call drops
/// the hook from the queue and is returned to the caller as
/// completed. A failing hook stays at the front of the queue; the
/// unrun queue (including the failed hook) is prepended back onto
/// the live registry so a retry of the surrounding operation
/// re-attempts the failed cleanup in order before any newly-
/// registered hooks fire.
///
/// **Idempotence contract.** `on_unload` hooks must be safe to
/// call more than once. Under retry-preserving semantics, a hook
/// that fails will be re-attempted on the next reload /
/// `install_local` replacement. A hook that's not idempotent
/// (e.g. `worker:terminate()` followed by something that asserts
/// the worker is still alive) will see a different observable
/// state on the retry than on the original attempt; the package
/// author has to handle that.
///
/// Used by both [`do_reload`] and [`do_install_local`] so the
/// hook semantics are identical whether the package is being
/// re-loaded against fresh disk or being swapped out for a
/// different working tree at the same name.
fn run_unload_hooks(lua: &Lua, basename: &str) -> mlua::Result<Vec<mlua::Function>> {
    let mut queue: Vec<mlua::Function> = {
        let slot = lua
            .app_data_ref::<PackageUnloadHooks>()
            .ok_or_else(|| mlua::Error::external(BindingError::NoUnloadHooksSlot))?;
        slot.drain(basename)
    };
    let mut completed = Vec::new();

    while !queue.is_empty() {
        // Clone the front (cheap — Function is a Lua reference
        // handle). Don't pop yet; we only consume on success so
        // a failing hook stays at the front of the queue for
        // restoration to the registry.
        let hook = queue[0].clone();
        if let Err(e) = hook.call::<()>(()) {
            // Push the unrun queue back onto the live registry's
            // front. Any hooks the chunk registered during the
            // cycle (in the now-non-empty registry slot) shift
            // to follow them — they'll fire after the retry
            // completes the original queue.
            let slot = lua
                .app_data_ref::<PackageUnloadHooks>()
                .ok_or_else(|| mlua::Error::external(BindingError::NoUnloadHooksSlot))?;
            slot.prepend(basename, queue);
            return Err(e);
        }
        completed.push(queue.remove(0));
    }
    Ok(completed)
}

/// If `env` is one of the cached per-package `_ENV` tables, return
/// the basename it's stored under. Otherwise `Ok(None)`.
///
/// Identity-based comparison (mlua's `Table: PartialEq` is reference
/// equality on the underlying Lua reference). This is what makes
/// `pmacs.packages.on_unload` unspoofable: a chunk in `_G` can
/// fabricate a table with `_PACKAGE.name = "victim"`, but it can't
/// fabricate the actual cached env-table reference for `victim`,
/// because that table is constructed by the searcher in Rust and
/// only handed out as the `_ENV` of `victim`'s chunk.
fn env_table_basename(lua: &Lua, env: &Table) -> mlua::Result<Option<String>> {
    let envs = package_envs_table(lua)?;
    for pair in envs.pairs::<String, Table>() {
        let (basename, candidate) = pair?;
        if candidate == *env {
            return Ok(Some(basename));
        }
    }
    Ok(None)
}

/// Parse the Lua-side `install(...)` argument into an [`InstallSpec`].
///
/// Two accepted forms:
///
/// - **Shorthand string**: `"github:user/repo@^1.0.0"`. Split on the
///   last `@` (so SSH-style addresses like `git:git@host:path@=1.2.3`
///   parse as expected).
/// - **Table**: `{ "github:user/repo", version = "^1.0.0" }`. The
///   address may also be passed as `address = "..."`. The `version`
///   field defaults to `"*"` if omitted (any tag).
fn parse_lua_install_spec(value: &Value) -> mlua::Result<InstallSpec> {
    match value {
        Value::String(s) => {
            let s = s.to_string_lossy();
            InstallSpec::parse_shorthand(&s)
                .map_err(|e| mlua::Error::external(BindingError::from(e)))
        }
        Value::Table(t) => {
            let address_str: String = match t.get::<String>(1) {
                Ok(s) => s,
                Err(_) => match t.get::<String>("address") {
                    Ok(s) => s,
                    Err(_) => {
                        return Err(mlua::Error::external(
                            BindingError::InstallSpecMissingAddress,
                        ));
                    }
                },
            };
            let address = Address::parse(&address_str)
                .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Address(e))))?;
            let pin = parse_install_pin(t)?;
            Ok(InstallSpec { address, pin })
        }
        other => Err(mlua::Error::external(BindingError::InstallSpecWrongType {
            got: other.type_name().to_string(),
        })),
    }
}

/// Parse the pin fields from a `pmacs.packages.install{...}` table.
///
/// A spec table may carry exactly one of:
/// - `version = "<semver constraint>"` (e.g. `"^1.0.0"`, `"=2.3.4"`).
/// - `branch = "<branch name>"` (e.g. `"main"`).
/// - `commit = "<sha>"` (full or partial; the fetcher accepts either).
///
/// If none are present the pin defaults to `version = "*"` (any
/// tag). If two or more are present the parse fails with
/// [`BindingError::InstallSpecConflictingPins`] naming every field
/// that conflicted.
fn parse_install_pin(t: &Table) -> mlua::Result<InstallPin> {
    let version: Option<String> = t.get::<Option<String>>("version").unwrap_or(None);
    let branch: Option<String> = t.get::<Option<String>>("branch").unwrap_or(None);
    let commit: Option<String> = t.get::<Option<String>>("commit").unwrap_or(None);
    let mut present: Vec<&'static str> = Vec::new();
    let version = version.filter(|s| !s.is_empty());
    let branch = branch.filter(|s| !s.is_empty());
    let commit = commit.filter(|s| !s.is_empty());
    if version.is_some() {
        present.push("version");
    }
    if branch.is_some() {
        present.push("branch");
    }
    if commit.is_some() {
        present.push("commit");
    }
    if present.len() > 1 {
        return Err(mlua::Error::external(
            BindingError::InstallSpecConflictingPins {
                fields: present.join(", "),
            },
        ));
    }
    if let Some(b) = branch {
        return Ok(InstallPin::Branch(b));
    }
    if let Some(c) = commit {
        return Ok(InstallPin::Commit(c));
    }
    let value = version.unwrap_or_else(|| "*".to_string());
    let req = semver::VersionReq::parse(&value).map_err(|e| {
        mlua::Error::external(BindingError::from(InstallError::InvalidVersionReq {
            value,
            cause: e.to_string(),
        }))
    })?;
    Ok(InstallPin::Version(req))
}

/// Read the required `project_root = "..."` field from the table form
/// of `install_project`'s spec.
///
/// Absolute paths are returned as-is. Relative paths are resolved
/// against the directory of the currently-evaluating chunk
/// (typically the user's `init.lua`) --- *not* against
/// `std::env::current_dir()`. The init-script's directory is stable
/// across invocations; CWD is whatever shell directory the user
/// happened to start pmacs from and is rarely the right anchor.
///
/// Returns [`BindingError::InstallProjectMissingProjectRoot`] when
/// the field is absent or the spec was given as a shorthand string.
/// Pre-v0.1.0 the fallback was `current_dir()`; that surprise was
/// removed because CWD-at-startup is almost never the user's project
/// root in any meaningful sense (see reviewer item 10).
///
/// # How the chunk directory is recovered
///
/// pmacs's Lua state is built with `Lua::new()`, which loads the
/// safe stdlib subset and intentionally omits `debug` (the project
/// forbids `unsafe_code`, so `Lua::unsafe_new` is not an option).
/// Without `debug.getinfo` we cannot walk Lua's call stack at
/// runtime. Instead, [`crate::lua::LuaHost::eval`] writes the
/// chunk's source label into a [`CurrentEvalSource`] app-data
/// slot before evaluating; this function reads it. The label
/// follows Lua's `@<path>` convention for file-loaded chunks (see
/// [`crate::config::load_user_config_at`]), so stripping the `@`
/// and taking the parent directory is well-defined.
///
/// # Forward-planning note
///
/// When project-local `init.lua` lands (post-v0.1; tracked
/// separately in the milestone plan), this function should consult
/// a thread-local "current project root" set by the project loader
/// before falling through to the missing-field error. Until that
/// machinery exists, `project_root` is unconditionally required;
/// the global init.lua path is the only init.lua path, and there
/// is no implicit "current project" to draw on.
fn install_spec_project_root(lua: &Lua, value: &Value) -> mlua::Result<std::path::PathBuf> {
    let field = match value {
        Value::Table(t) => t.get::<String>("project_root").ok(),
        _ => None,
    };
    let raw = match field {
        Some(s) if !s.is_empty() => s,
        _ => {
            return Err(mlua::Error::external(
                BindingError::InstallProjectMissingProjectRoot,
            ));
        }
    };
    let candidate = std::path::PathBuf::from(&raw);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    if let Some(chunk_dir) = current_eval_dir(lua) {
        return Ok(chunk_dir.join(&candidate));
    }
    // Fallback for evaluations without a file-shaped source label
    // (string-loaded test chunks, REPL one-liners, the M-x
    // command-line evaluator): the relative path is taken as-is.
    // The user's value is non-empty so they explicitly opted in;
    // this branch matches the pre-v0.1 CWD interpretation.
    Ok(candidate)
}

/// Read the parent directory of the currently-evaluating chunk's
/// source label, if any. Returns `None` when no source has been
/// pushed (e.g., the call stack came in via a non-`eval` entry
/// point), or when the source label is not in `@<path>` shape.
///
/// The slot is populated by [`crate::lua::LuaHost::eval`] before
/// it runs the chunk; see the docstring on
/// [`install_spec_project_root`] for why we use this rather than
/// `debug.getinfo`.
fn current_eval_dir(lua: &Lua) -> Option<std::path::PathBuf> {
    let slot = lua.app_data_ref::<CurrentEvalSource>()?;
    let label = slot.0.as_deref()?;
    let path_str = label.strip_prefix('@')?;
    let path = std::path::PathBuf::from(path_str);
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    Some(parent.to_path_buf())
}

/// Run the install end-to-end: build a fetcher rooted at
/// `$XDG_CACHE_HOME/pmacs/git/`, resolve the spec's full dependency
/// closure via [`crate::packages::Resolver`], call
/// [`Installer::install_at_commit`] for every package in the
/// resulting plan (so the installer honors the resolver's commit
/// choice rather than independently re-running tag matching),
/// extend `package.path` for each, and record each result in the
/// [`InstalledPackages`] roster.
///
/// The Lua-callable spec is a single top-level package; transitive
/// dependencies declared in that package's manifest are pulled in
/// by the resolver and installed in topological order. The Lua
/// caller gets back the *top-level* package's metadata table (the
/// thing they asked to install); transitive dependencies are
/// observable through `pmacs.packages.installed()`.
///
/// A [`PackageInstallOverride`] in app data, if present, redirects
/// the fetcher's cache dir and the user-scope install root. Tests
/// use this instead of mutating `XDG_*` env vars (which would
/// require `unsafe`).
fn do_install(lua: &Lua, spec: &InstallSpec, scope: &InstallScope) -> mlua::Result<Table> {
    let override_data = lua.app_data_ref::<PackageInstallOverride>();
    let cache_override = override_data.as_ref().and_then(|o| o.cache_dir.clone());
    let user_root_override = override_data
        .as_ref()
        .and_then(|o| o.user_install_root.clone());
    drop(override_data);

    // Build a fetcher to share with the resolver and the installer.
    // The resolver enumerates tags / reads manifests through it; the
    // installer reuses the same cache for the actual checkout.
    let resolver_fetcher = match &cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir.clone()),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };
    let installer_fetcher = match &cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir.clone()),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };

    // Resolve the full plan including transitive deps.
    let resolver = crate::packages::Resolver::new(resolver_fetcher);
    let request = crate::packages::ResolveRequest {
        address: spec.address.clone(),
        pin: spec.pin.clone(),
    };
    let plan = resolver
        .resolve(&[request])
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    // Install every plan entry in topological order. The plan
    // already orders dependencies before dependents, so an
    // installer that reads a freshly-installed dependency's
    // manifest finds it on disk by the time it's needed.
    let mut installer = Installer::new(installer_fetcher, scope.clone());
    if let (InstallScope::User, Some(root)) = (scope, user_root_override) {
        installer = installer.with_install_root_override(root);
    }

    let top_level_url = spec.address.to_git_url();
    let mut top_level_installed: Option<InstalledPackage> = None;

    let slot = lua
        .app_data_ref::<InstalledPackages>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;

    for rp in &plan.packages {
        // Top-level: install with the user's original pin so the
        // returned `tag` / `pin` fields reflect what the user
        // asked for ("version `^1.0.0`" or "branch `main`").
        // Transitive deps: install at the resolver's chosen commit
        // (the commit is what's reproducible; transitive pins
        // don't carry forward branch/version semantics).
        //
        // In both cases we route through `install_at_commit` so
        // the installer honors the resolver's commit choice
        // rather than independently re-running its own tag
        // matching --- the resolver may have rejected a newer
        // upstream tag for compatibility reasons, and a divergent
        // installer pick would defeat that constraint.
        let is_top_level = rp.address.to_git_url() == top_level_url;
        let install_spec = if is_top_level {
            InstallSpec {
                address: rp.address.clone(),
                pin: spec.pin.clone(),
            }
        } else {
            InstallSpec {
                address: rp.address.clone(),
                pin: crate::packages::InstallPin::Commit(rp.revision.clone()),
            }
        };
        let installed = installer
            .install_at_commit(&install_spec, &rp.revision)
            .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

        if let Some(parent) = installed.install_path.parent() {
            prepend_package_path(lua, parent)?;
        }
        slot.record(installed.clone());
        if is_top_level {
            top_level_installed = Some(installed);
        }
    }
    drop(slot);

    let top = top_level_installed
        .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;

    // Lockfile write. Build a Lockfile from the just-installed plan,
    // merge any pre-existing lockfile entries that aren't in this
    // plan (so multiple installs in the same init.lua accumulate
    // rather than clobber), and write back. Failure to write here is
    // surfaced --- the install bytes are already on disk; an unwritten
    // lockfile means a future Frozen install can't reproduce this
    // state and the user should know about it.
    let install_root = installer
        .install_root()
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
    let lockfile_fetcher = match &cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir.clone()),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };
    write_merged_lockfile(&plan, &lockfile_fetcher, &install_root)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    installed_package_to_lua(lua, &top)
}

/// `pmacs.packages.update(name?)` --- re-resolve top-level
/// packages against the current upstream and reinstall the result.
///
/// Reads the user-scope lockfile at
/// `<install_root>/pmacs.lock` for the package set: every entry
/// whose `top_level_pin` is set is treated as a user-stated
/// install. Builds [`ResolveRequest`]s from those, dispatches to
/// [`crate::packages::Resolver::resolve_with_policy`] under
/// [`crate::packages::UpdatePolicy::UpdateOne`] (when `target` is
/// `Some(name)`) or [`crate::packages::UpdatePolicy::UpdateAll`]
/// (when `target` is `None`), reinstalls the resulting plan, and
/// writes the new lockfile (replacing the old one --- update is a
/// full re-resolve).
///
/// Returns a Lua table with one entry per updated package, naming
/// the prior commit, the new commit, and the new version. The
/// `pmacs.packages.installed()` snapshot also reflects the change.
#[allow(
    clippy::too_many_lines,
    reason = "single linear flow: read lockfile → build requests → resolve → install → write. \
              Splitting helpers fragments the read without removing complexity."
)]
fn do_update(lua: &Lua, target: Option<&str>) -> mlua::Result<Table> {
    use crate::packages::{
        Address, LOCKFILE_FILENAME, Lockfile, LockfileEntry, PackageName, ResolveRequest, Resolver,
        UpdatePolicy,
    };

    let override_data = lua.app_data_ref::<PackageInstallOverride>();
    let cache_override = override_data.as_ref().and_then(|o| o.cache_dir.clone());
    let user_root_override = override_data
        .as_ref()
        .and_then(|o| o.user_install_root.clone());
    drop(override_data);

    let resolver_fetcher = match &cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir.clone()),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };
    let installer_fetcher = match &cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir.clone()),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };
    let lockfile_fetcher = match &cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir.clone()),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };

    // The Lua surface always targets user scope --- project-scope
    // updates would need a `project_root` argument, deferred to a
    // future `pmacs.packages.update_project`.
    let scope = InstallScope::User;
    let mut installer = Installer::new(installer_fetcher, scope.clone());
    if let Some(root) = user_root_override {
        installer = installer.with_install_root_override(root);
    }
    let install_root = installer
        .install_root()
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
    let lock_path = install_root.join(LOCKFILE_FILENAME);

    let lock = Lockfile::read_from(&lock_path)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    // Collect ResolveRequests from top-level entries.
    let mut requests = Vec::new();
    for entry in &lock.packages {
        if let Some(top_pin) = &entry.top_level_pin {
            let pin = top_pin
                .to_install_pin()
                .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
            requests.push(ResolveRequest {
                address: Address::Url(entry.url.clone()),
                pin,
            });
        }
    }
    if requests.is_empty() {
        return Err(mlua::Error::external(BindingError::PackagesUpdateNoEntries));
    }

    // Capture prior commits so we can report what moved.
    let prior_commit_by_name: std::collections::BTreeMap<PackageName, String> = lock
        .packages
        .iter()
        .map(|e| (e.name.clone(), e.commit.clone()))
        .collect();

    let policy = match target {
        None => UpdatePolicy::UpdateAll,
        Some(name) => {
            let pkg_name = PackageName::new(name).map_err(|e| {
                mlua::Error::external(BindingError::PackagesUpdateBadName {
                    name: name.to_string(),
                    reason: e.to_string(),
                })
            })?;
            // Refuse to update a name that isn't in the lockfile ---
            // surfaces the typo loudly rather than silently doing
            // nothing.
            if lock.entry(&pkg_name).is_none() {
                return Err(mlua::Error::external(
                    BindingError::PackagesUpdateUnknownName {
                        name: name.to_string(),
                    },
                ));
            }
            UpdatePolicy::UpdateOne(pkg_name)
        }
    };

    let resolver = Resolver::new(resolver_fetcher);
    let plan = resolver
        .resolve_with_policy(&requests, Some(&lock), &policy)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    // Build and serialize the new lockfile before mutating the
    // install tree. Hashing/serialization failures should leave disk
    // and the in-memory roster exactly as they were.
    let new_lock = Lockfile::from_plan(&plan, &lockfile_fetcher)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
    let new_lock_bytes = new_lock
        .to_bytes()
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    // Build a path-keyed view of the OLD lockfile: each prior entry
    // resolves to `<install_root>/<basename>`. Pruning by exact path
    // (rather than by basename) avoids touching a project-scope
    // roster entry whose basename happens to collide with a
    // user-scope dep that's about to disappear.
    let prior_by_path: std::collections::BTreeMap<std::path::PathBuf, &LockfileEntry> = lock
        .packages
        .iter()
        .map(|e| {
            let basename = crate::packages::installer::package_basename(e.name.as_str());
            (install_root.join(basename), e)
        })
        .collect();

    // Reinstall every plan entry, tracking which paths the new plan
    // covers and which basenames need their `package.loaded` cache
    // cleared. A package is "stale in Lua" if (a) its commit moved
    // (an updated package whose old code is still cached in
    // package.loaded would otherwise return the prior module table)
    // or (b) it disappeared entirely from the plan (handled in the
    // prune loop below).
    let slot = lua
        .app_data_ref::<InstalledPackages>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;
    let old_roster = slot.snapshot();
    let mut new_paths: std::collections::BTreeSet<std::path::PathBuf> =
        std::collections::BTreeSet::new();
    let mut invalidate: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    let update_result: mlua::Result<()> = (|| {
        for rp in &plan.packages {
            // For top-level entries, prefer the original top_level_pin
            // recorded in the lockfile (so the displayed `pin` field
            // matches what the user originally asked for); for
            // transitives, install at the resolver's commit.
            //
            // `replace_at_commit` is the update-aware path: when the
            // resolver picks a different commit than the on-disk
            // install, the existing tree is staged-and-swapped rather
            // than rejected as `AlreadyInstalled`. Without this, an
            // update couldn't actually move a package to a new version.
            let pin_to_use = rp
                .top_level_pin
                .clone()
                .unwrap_or_else(|| crate::packages::InstallPin::Commit(rp.revision.clone()));
            let install_spec = InstallSpec {
                address: rp.address.clone(),
                pin: pin_to_use,
            };
            let installed = installer
                .replace_at_commit(&install_spec, &rp.revision)
                .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
            if let Some(parent) = installed.install_path.parent() {
                prepend_package_path(lua, parent)?;
            }
            let path = installed.install_path.clone();
            if prior_by_path
                .get(&path)
                .is_some_and(|old| old.commit != installed.commit)
            {
                invalidate.insert(installed.install_basename().to_string());
            }
            new_paths.insert(path);
            slot.record(installed);
        }

        // Prune anything that disappeared from the plan: a transitive
        // dep the resolver no longer needs, or a package the user
        // dropped from their top-level set. Without this the on-disk
        // tree, the roster, and the searcher all keep stale state and
        // `require("dropped-pkg")` would still succeed against the old
        // install --- defeating the lockfile's claim of authority.
        for stale_path in prior_by_path.keys().filter(|p| !new_paths.contains(*p)) {
            if stale_path.exists() {
                std::fs::remove_dir_all(stale_path).map_err(|source| {
                    mlua::Error::external(BindingError::from(crate::packages::InstallError::Io {
                        path: stale_path.clone(),
                        source,
                    }))
                })?;
            }
            if let Some(basename) = stale_path.file_name().and_then(|s| s.to_str()) {
                invalidate.insert(basename.to_string());
            }
            slot.remove_by_install_path(stale_path);
        }

        // Update is a full re-resolve --- write a fresh lockfile, no
        // merge. Any package that was in the old lockfile but isn't in
        // the new plan was a transitive dep that the resolver decided
        // is no longer needed. The bytes were precomputed above so the
        // only remaining failure class here is filesystem I/O, and the
        // write itself is atomic.
        Lockfile::write_bytes_to(&lock_path, &new_lock_bytes)
            .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
        Ok(())
    })();

    if let Err(err) = update_result {
        restore_user_update_state(
            &mut installer,
            &lock,
            &old_roster,
            &install_root,
            &new_paths,
            &slot,
        );
        drop(slot);
        return Err(err);
    }
    drop(slot);

    // Now that disk + roster + lockfile are durable, drop stale
    // entries from `package.loaded` so a subsequent `require()`
    // reroutes through the searcher and picks up the new code (or
    // fails if the package was pruned). Doing this *after* the
    // lockfile write means a partial failure earlier in update
    // doesn't leave Lua's module cache out of sync with what's on
    // disk.
    for basename in &invalidate {
        invalidate_loaded_package(lua, basename)?;
    }

    // Build the change summary table.
    let summary = lua.create_table()?;
    for (i, entry) in new_lock.packages.iter().enumerate() {
        let row = lua.create_table()?;
        row.set("name", entry.name.as_str())?;
        row.set("version", entry.version.to_string())?;
        row.set("commit", entry.commit.as_str())?;
        if let Some(prior) = prior_commit_by_name.get(&entry.name) {
            row.set("prior_commit", prior.as_str())?;
            row.set("changed", *prior != entry.commit)?;
        } else {
            // New entry --- e.g., a transitive dep that wasn't in
            // the prior lockfile. (Could happen if the previous
            // install used an older version that didn't depend on
            // this package.)
            row.set("changed", true)?;
        }
        summary.set(i + 1, row)?;
    }
    Ok(summary)
}

/// Best-effort rollback for `pmacs.packages.update`.
///
/// `update` computes the new lockfile before touching disk, but the
/// later install/prune/write steps still involve fallible filesystem
/// operations. If any of them fails, the old lockfile remains the
/// source of truth; this helper tries to make disk and the in-memory
/// roster match it again before the original error is returned.
fn restore_user_update_state(
    installer: &mut Installer,
    old_lock: &crate::packages::Lockfile,
    old_roster: &[InstalledPackage],
    install_root: &std::path::Path,
    new_paths: &std::collections::BTreeSet<std::path::PathBuf>,
    slot: &InstalledPackages,
) {
    let old_paths: std::collections::BTreeSet<std::path::PathBuf> = old_lock
        .packages
        .iter()
        .map(|entry| {
            let basename = crate::packages::installer::package_basename(entry.name.as_str());
            install_root.join(basename)
        })
        .collect();

    for entry in &old_lock.packages {
        let spec = InstallSpec {
            address: Address::Url(entry.url.clone()),
            pin: InstallPin::Commit(entry.commit.clone()),
        };
        let _ = installer.replace_at_commit(&spec, &entry.commit);
    }

    for path in new_paths.difference(&old_paths) {
        if path.exists() {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    slot.replace_snapshot(old_roster.to_vec());
}

/// Run `install_local` end-to-end: validate the source has a
/// `pmacs.toml`, symlink `<install_root>/<basename>` to a
/// canonicalized form of the source path, record into the
/// [`InstalledPackages`] roster, and skip the lockfile (Local pins
/// are explicitly ephemeral).
///
/// Replaces an existing symlink at the install path. Refuses if
/// the install path holds a real directory (a previous fetched
/// install) --- the user must clear that first to avoid surprising
/// loss of an installed tree.
///
/// Always invalidates `package.loaded[<basename>]` before returning,
/// so a re-call against a different source picks up the new code on
/// the next `require()`. (Strictly speaking, `init.lua` doesn't
/// have a load order that gets two `install_local` calls into the
/// same name --- the API is init-only --- but invalidating
/// unconditionally is cheap and protects against user error.)
fn do_install_local(lua: &Lua, source_path: &std::path::Path) -> mlua::Result<Table> {
    let override_data = lua.app_data_ref::<PackageInstallOverride>();
    let user_root_override = override_data
        .as_ref()
        .and_then(|o| o.user_install_root.clone());
    drop(override_data);

    // install_local uses a fetcher only as a constructor argument
    // for Installer; the install_local() method itself never
    // touches the network. We pass the XDG-rooted fetcher (or its
    // override) for symmetry with the other install paths.
    let fetcher = match lua
        .app_data_ref::<PackageInstallOverride>()
        .as_ref()
        .and_then(|o| o.cache_dir.clone())
    {
        Some(dir) => Fetcher::with_cache_dir(dir),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };
    let mut installer = Installer::new(fetcher, InstallScope::User);
    if let Some(root) = user_root_override {
        installer = installer.with_install_root_override(root);
    }

    // Phase 1: plan. Validates the manifest, computes the install
    // path, but makes no disk changes. If validation fails we
    // surface the error immediately; nothing's been mutated.
    let plan = installer
        .plan_local(source_path)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;
    let basename = plan.basename.clone();

    // Phase 2: stage the replacement symlink BEFORE unloading the
    // prior package. This front-loads fallible symlink creation while
    // the old package is still live; if staging fails, no hooks have
    // run and disk / roster / runtime state remain aligned.
    let staged = installer
        .stage_local(plan)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    // Phase 3: run the prior install's on_unload hooks (if any)
    // BEFORE publishing the staged symlink. If a hook fails, we
    // discard the staged symlink, propagate the error, and leave the
    // live install path untouched. The failed hook stays in the
    // registry so retry re-attempts it.
    let completed_hooks = match run_unload_hooks(lua, &basename) {
        Ok(hooks) => hooks,
        Err(e) => {
            installer.discard_staged_local(staged);
            return Err(e);
        }
    };

    // Phase 4: publish, then cache invalidation. If even the final
    // same-directory rename fails, restore the completed hooks so a
    // retry can re-run the prior package's idempotent teardown before
    // trying the publish again.
    let installed = installer.publish_local(staged).map_err(|e| {
        if let Some(slot) = lua.app_data_ref::<PackageUnloadHooks>() {
            slot.prepend(&basename, completed_hooks);
        }
        mlua::Error::external(BindingError::from(e))
    })?;

    if let Some(parent) = installed.install_path.parent() {
        prepend_package_path(lua, parent)?;
    }

    let slot = lua
        .app_data_ref::<InstalledPackages>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;
    slot.record(installed.clone());
    drop(slot);

    // Invalidate package.loaded so a previous require() against an
    // earlier install_local target doesn't shadow the freshly
    // symlinked tree. Mirrors the post-update invalidation;
    // unconditional here because the cost is one Lua table walk.
    invalidate_loaded_package(lua, &basename)?;

    // Drop the cached per-package _ENV too. Same rationale as
    // `do_reload`: without this, globals from a prior install
    // (e.g. install_local against an old source, then again
    // against an updated one) would persist into the freshly
    // symlinked tree's chunk, and the dev-loop "edit on disk and
    // see only what's currently in the source" promise would
    // quietly fail.
    clear_package_env(lua, &basename)?;

    installed_package_to_lua(lua, &installed)
}

/// `pmacs.packages.reload(name)` --- run the package's `on_unload`
/// hooks, drop its `package.loaded` cache entries, and call
/// `require(name)` to load the freshly-readable bytes (T M8.1d).
///
/// Returns the new module table the re-`require` produced.
///
/// Reload is the dev-loop counterpart to `update`: where `update`
/// re-resolves the package set against upstream and replaces the
/// install bytes, `reload` works against whatever's already on disk
/// (typically a working tree symlinked in via `install_local`,
/// freshly edited). It does *not* re-run install machinery.
///
/// Hooks run in registration order. Errors raised by an `on_unload`
/// hook propagate to the caller --- a partial-teardown reload is a
/// programming error in the package, not something the runtime can
/// paper over. The hook list is consumed on reload, so a re-`require`
/// re-registers fresh hooks; this keeps each reload cycle
/// self-contained.
fn do_reload(lua: &Lua, name: &str) -> mlua::Result<Value> {
    // Walk the roster to confirm the name exists (the searcher
    // would also catch a missing name on the require, but a clearer
    // up-front error helps users who typo).
    let slot = lua
        .app_data_ref::<InstalledPackages>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;
    let snapshot = slot.snapshot();
    drop(slot);
    if !snapshot.iter().any(|p| p.install_basename() == name) {
        return Err(mlua::Error::external(
            BindingError::PackagesReloadUnknownName {
                name: name.to_string(),
            },
        ));
    }

    // Run on_unload hooks in registration order via the shared
    // peek-call-pop runner. A failing hook stays at the front of
    // the registry so a retry re-attempts the same cleanup; only
    // hooks that actually completed are popped.
    let _ = run_unload_hooks(lua, name)?;

    // Invalidate the module cache so the next require runs the
    // chunk against the freshly-readable bytes (the
    // install_local-symlinked working tree, or the
    // last-update-replaced install dir).
    invalidate_loaded_package(lua, name)?;

    // Drop the cached per-package _ENV table. Without this,
    // removed-or-renamed globals from the prior chunk would still
    // be visible after reload, because the env table outlives the
    // chunk that wrote them. The next package_env_for() call
    // (during the re-require below) constructs a fresh env.
    clear_package_env(lua, name)?;

    // Re-require. The M7.7 searcher resolves against the same
    // roster entry the original load used; we don't re-resolve via
    // the package system because reload's contract is "pick up
    // disk changes," not "switch packages."
    let require: Function = lua.globals().get("require")?;
    require.call::<Value>(name.to_string())
}

/// Build a fresh [`Lockfile`] from `plan`, merge it with any
/// pre-existing lockfile at `<install_root>/pmacs.lock` (entries
/// not in the new plan are preserved), and write the result back.
///
/// Merge policy: by package name. A new-plan entry replaces a
/// same-named existing entry; non-overlapping existing entries are
/// preserved. Output is sorted alphabetically (matching
/// [`Lockfile::from_plan`]'s contract).
#[allow(
    clippy::result_large_err,
    reason = "Mirrors LockfileError's own carrier-of-diagnostic-context posture; \
              boxing here would only hide the cost without changing the surface."
)]
fn write_merged_lockfile(
    plan: &crate::packages::ResolvePlan,
    fetcher: &Fetcher,
    install_root: &std::path::Path,
) -> Result<(), crate::packages::LockfileError> {
    use crate::packages::{LOCKFILE_FILENAME, Lockfile};

    let mut new_lock = Lockfile::from_plan(plan, fetcher)?;
    let lock_path = install_root.join(LOCKFILE_FILENAME);
    if let Ok(existing) = Lockfile::read_from(&lock_path) {
        let new_names: std::collections::HashSet<_> =
            new_lock.packages.iter().map(|e| e.name.clone()).collect();
        for entry in existing.packages {
            if !new_names.contains(&entry.name) {
                new_lock.packages.push(entry);
            }
        }
        new_lock.packages.sort_by(|a, b| a.name.cmp(&b.name));
    }
    new_lock.write_to(&lock_path)
}

/// Drop `package.loaded[basename]` and every `package.loaded[basename.<sub>]`
/// so a subsequent `require(basename)` re-runs the M7.7 searcher
/// against the freshly-installed bytes (or fails if the package was
/// pruned). Called from `pmacs.packages.update` for any basename
/// whose commit moved or that disappeared from the new plan ---
/// without this step, a `require()` after `update()` would return
/// the cached module table from before the update, defeating the
/// whole point of an in-process update.
///
/// Submodules are matched by `<basename>.` prefix so a package with
/// nested exports (e.g. `mypkg.submod` in
/// `package.loaded["mypkg.submod"]`) is fully invalidated, not just
/// at its top-level entry. Other packages' entries are untouched
/// because the prefix match terminates at the dot boundary.
fn invalidate_loaded_package(lua: &Lua, basename: &str) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let loaded: Table = package.get("loaded")?;

    let prefix = format!("{basename}.");
    let mut keys: Vec<String> = Vec::new();
    for pair in loaded.clone().pairs::<mlua::Value, mlua::Value>() {
        let (key, _) = pair?;
        if let mlua::Value::String(s) = key {
            let k = s.to_str()?;
            if *k == *basename || k.starts_with(&prefix) {
                keys.push(k.to_string());
            }
        }
    }
    for key in keys {
        loaded.set(key, mlua::Value::Nil)?;
    }
    Ok(())
}

/// Idempotently prepend `<root>/?.lua;<root>/?/init.lua` to
/// `package.path`. The standard Lua require pattern: a package with
/// `entry = "init.lua"` installed at `<root>/<basename>/init.lua`
/// becomes findable as `require("<basename>")`.
fn prepend_package_path(lua: &Lua, root: &std::path::Path) -> mlua::Result<()> {
    let package_global = lua.globals().get::<Table>("package")?;
    let current_path: String = package_global.get::<String>("path").unwrap_or_default();
    let root_str = root.display().to_string();
    let new_entries = format!("{root_str}/?.lua;{root_str}/?/init.lua");
    if current_path
        .split(';')
        .any(|seg| seg == format!("{root_str}/?.lua") || seg == format!("{root_str}/?/init.lua"))
    {
        return Ok(());
    }
    let combined = if current_path.is_empty() {
        new_entries
    } else {
        format!("{new_entries};{current_path}")
    };
    package_global.set("path", combined)?;
    Ok(())
}

/// Render the manifest's metadata + exports list as a Lua table,
/// suitable for `pmacs.packages.describe(name)`. Extends the standard
/// [`installed_package_to_lua`] record with `pmacs_required` and a
/// 1-indexed `exports` array; the union of fields is what a
/// "describe-package" caller needs to render a complete view of the
/// package without reaching back through any other API.
fn installed_package_describe_table(lua: &Lua, pkg: &InstalledPackage) -> mlua::Result<Table> {
    let t = installed_package_to_lua(lua, pkg)?;
    t.set("pmacs_required", pkg.manifest.pmacs_required.to_string())?;
    let exports = lua.create_table_with_capacity(pkg.manifest.exports.len(), 0)?;
    for (i, e) in pkg.manifest.exports.iter().enumerate() {
        exports.set(i + 1, e.as_str())?;
    }
    t.set("exports", exports)?;
    Ok(t)
}

/// Translate an [`InstalledPackage`] into the Lua-facing record
/// returned by `pmacs.packages.install` and `pmacs.packages.installed`.
fn installed_package_to_lua(lua: &Lua, pkg: &InstalledPackage) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", pkg.manifest.name.as_str())?;
    t.set("version", pkg.version.to_string())?;
    t.set("tag", pkg.tag.as_str())?;
    t.set("commit", pkg.commit.as_str())?;
    t.set("install_path", pkg.install_path.display().to_string())?;
    t.set("entry", pkg.entry_path().display().to_string())?;
    t.set(
        "scope",
        match &pkg.scope {
            InstallScope::User => "user",
            InstallScope::Project { .. } => "project",
        },
    )?;
    t.set("summary", pkg.manifest.summary.as_str())?;
    // Structured pin info: `{ kind = "version"|"branch"|"commit", value = <user-supplied string> }`.
    // Existing flat fields (`tag`, `version`, `commit`) remain
    // populated for backward-compatible introspection; the `pin`
    // table is the source of truth for "what did the user request",
    // distinct from "what got resolved".
    let pin_table = lua.create_table_with_capacity(0, 2)?;
    pin_table.set("kind", pkg.pin.kind())?;
    pin_table.set("value", pkg.pin.value())?;
    t.set("pin", pin_table)?;
    Ok(t)
}

/// Translate a single [`crate::ansi::AnsiEvent`] into a Lua table.
///
/// The `kind` field is the discriminator; per-variant fields follow
/// the M6.4 spec contract:
///
/// - `text`:  `{ kind="text", text=<string> }`
/// - `set_style`: `{ kind="set_style", style=<style table> }`
/// - `carriage_return` / `backspace` / `erase_to_eol` / `erase_line` /
///   `prompt_start` / `prompt_end` / `command_start` / `output_start` /
///   `bracketed_paste_begin` / `bracketed_paste_end` /
///   `alt_screen_enter` / `alt_screen_exit`: `{ kind=<name> }` only
/// - `set_title`: `{ kind="set_title", title=<string> }`
#[allow(
    clippy::too_many_lines,
    reason = "exhaustive wire-to-Lua conversion keeps every ANSI variant and field visible in one audited match"
)]
fn event_to_lua_table(lua: &Lua, ev: &crate::ansi::AnsiEvent) -> mlua::Result<Table> {
    use crate::ansi::AnsiEvent;
    let t = lua.create_table()?;
    match ev {
        AnsiEvent::Text(s) => {
            t.set("kind", "text")?;
            t.set("text", s.as_str())?;
        }
        AnsiEvent::SetStyle(style) => {
            t.set("kind", "set_style")?;
            t.set("style", style_to_lua_table(lua, style)?)?;
        }
        AnsiEvent::CarriageReturn => {
            t.set("kind", "carriage_return")?;
        }
        AnsiEvent::Backspace => {
            t.set("kind", "backspace")?;
        }
        AnsiEvent::EraseToEol => {
            t.set("kind", "erase_to_eol")?;
        }
        AnsiEvent::EraseLine => {
            t.set("kind", "erase_line")?;
        }
        AnsiEvent::SetTitle(title) => {
            t.set("kind", "set_title")?;
            t.set("title", title.as_str())?;
        }
        AnsiEvent::PromptStart => {
            t.set("kind", "prompt_start")?;
        }
        AnsiEvent::PromptEnd => {
            t.set("kind", "prompt_end")?;
        }
        AnsiEvent::CommandStart => {
            t.set("kind", "command_start")?;
        }
        AnsiEvent::OutputStart => {
            t.set("kind", "output_start")?;
        }
        AnsiEvent::BracketedPasteBegin => {
            t.set("kind", "bracketed_paste_begin")?;
        }
        AnsiEvent::BracketedPasteEnd => {
            t.set("kind", "bracketed_paste_end")?;
        }
        AnsiEvent::AlternateScreenEnter => {
            t.set("kind", "alt_screen_enter")?;
        }
        AnsiEvent::AlternateScreenExit => {
            t.set("kind", "alt_screen_exit")?;
        }
        AnsiEvent::Bell => t.set("kind", "bell")?,
        AnsiEvent::LineFeed => t.set("kind", "line_feed")?,
        AnsiEvent::Index => t.set("kind", "index")?,
        AnsiEvent::NextLine => t.set("kind", "next_line")?,
        AnsiEvent::ReverseIndex => t.set("kind", "reverse_index")?,
        AnsiEvent::HorizontalTab => t.set("kind", "horizontal_tab")?,
        AnsiEvent::SetTabStop => t.set("kind", "set_tab_stop")?,
        AnsiEvent::ClearTabStop => t.set("kind", "clear_tab_stop")?,
        AnsiEvent::ClearAllTabStops => t.set("kind", "clear_all_tab_stops")?,
        AnsiEvent::CursorUp(count)
        | AnsiEvent::CursorDown(count)
        | AnsiEvent::CursorForward(count)
        | AnsiEvent::CursorBackward(count)
        | AnsiEvent::CursorNextLine(count)
        | AnsiEvent::CursorPreviousLine(count)
        | AnsiEvent::EraseCharacters(count)
        | AnsiEvent::InsertCharacters(count)
        | AnsiEvent::DeleteCharacters(count)
        | AnsiEvent::InsertLines(count)
        | AnsiEvent::DeleteLines(count)
        | AnsiEvent::ScrollUp(count)
        | AnsiEvent::ScrollDown(count) => {
            let kind = match ev {
                AnsiEvent::CursorUp(_) => "cursor_up",
                AnsiEvent::CursorDown(_) => "cursor_down",
                AnsiEvent::CursorForward(_) => "cursor_forward",
                AnsiEvent::CursorBackward(_) => "cursor_backward",
                AnsiEvent::CursorNextLine(_) => "cursor_next_line",
                AnsiEvent::CursorPreviousLine(_) => "cursor_previous_line",
                AnsiEvent::EraseCharacters(_) => "erase_characters",
                AnsiEvent::InsertCharacters(_) => "insert_characters",
                AnsiEvent::DeleteCharacters(_) => "delete_characters",
                AnsiEvent::InsertLines(_) => "insert_lines",
                AnsiEvent::DeleteLines(_) => "delete_lines",
                AnsiEvent::ScrollUp(_) => "scroll_up",
                AnsiEvent::ScrollDown(_) => "scroll_down",
                _ => unreachable!("outer match restricts the event"),
            };
            t.set("kind", kind)?;
            t.set("count", *count)?;
        }
        AnsiEvent::CursorHorizontalAbsolute(col) => {
            t.set("kind", "cursor_horizontal_absolute")?;
            t.set("col", *col)?;
        }
        AnsiEvent::CursorVerticalAbsolute(row) => {
            t.set("kind", "cursor_vertical_absolute")?;
            t.set("row", *row)?;
        }
        AnsiEvent::CursorPosition { row, col } => {
            t.set("kind", "cursor_position")?;
            t.set("row", *row)?;
            t.set("col", *col)?;
        }
        AnsiEvent::EraseDisplay(mode) | AnsiEvent::EraseLineMode(mode) => {
            t.set(
                "kind",
                if matches!(ev, AnsiEvent::EraseDisplay(_)) {
                    "erase_display"
                } else {
                    "erase_line_mode"
                },
            )?;
            t.set(
                "mode",
                match mode {
                    crate::ansi::EraseMode::ToEnd => "to_end",
                    crate::ansi::EraseMode::ToStart => "to_start",
                    crate::ansi::EraseMode::All => "all",
                    crate::ansi::EraseMode::Saved => "saved",
                },
            )?;
        }
        AnsiEvent::SetScrollingRegion { top, bottom } => {
            t.set("kind", "set_scrolling_region")?;
            t.set("top", *top)?;
            t.set("bottom", *bottom)?;
        }
        AnsiEvent::SaveCursor => t.set("kind", "save_cursor")?,
        AnsiEvent::RestoreCursor => t.set("kind", "restore_cursor")?,
        AnsiEvent::AlternateScreen { mode, enabled } => {
            t.set("kind", "alternate_screen")?;
            t.set(
                "mode",
                match mode {
                    crate::ansi::AlternateScreenMode::Mode47 => 47,
                    crate::ansi::AlternateScreenMode::Mode1047 => 1047,
                    crate::ansi::AlternateScreenMode::Mode1049 => 1049,
                },
            )?;
            t.set("enabled", *enabled)?;
        }
        AnsiEvent::SetMode { mode, enabled } => {
            t.set("kind", "set_mode")?;
            t.set(
                "mode",
                match mode {
                    crate::ansi::TerminalMode::Insert => "insert",
                    crate::ansi::TerminalMode::Origin => "origin",
                    crate::ansi::TerminalMode::AutoWrap => "auto_wrap",
                    crate::ansi::TerminalMode::ApplicationCursor => "application_cursor",
                    crate::ansi::TerminalMode::ApplicationKeypad => "application_keypad",
                    crate::ansi::TerminalMode::CursorVisible => "cursor_visible",
                    crate::ansi::TerminalMode::BracketedPaste => "bracketed_paste",
                    crate::ansi::TerminalMode::FocusReporting => "focus_reporting",
                    crate::ansi::TerminalMode::SynchronizedOutput => "synchronized_output",
                    crate::ansi::TerminalMode::MouseX10 => "mouse_x10",
                    crate::ansi::TerminalMode::MouseButton => "mouse_button",
                    crate::ansi::TerminalMode::MouseAny => "mouse_any",
                    crate::ansi::TerminalMode::MouseSgr => "mouse_sgr",
                },
            )?;
            t.set("enabled", *enabled)?;
        }
        AnsiEvent::DesignateCharacterSet { slot, charset } => {
            t.set("kind", "designate_character_set")?;
            t.set(
                "slot",
                match slot {
                    crate::ansi::CharacterSetSlot::G0 => "g0",
                    crate::ansi::CharacterSetSlot::G1 => "g1",
                },
            )?;
            t.set(
                "charset",
                match charset {
                    crate::ansi::CharacterSet::Ascii => "ascii",
                    crate::ansi::CharacterSet::DecSpecialGraphics => "dec_special_graphics",
                },
            )?;
        }
        AnsiEvent::ShiftOut => t.set("kind", "shift_out")?,
        AnsiEvent::ShiftIn => t.set("kind", "shift_in")?,
        AnsiEvent::DeviceRequest(request) => {
            t.set("kind", "device_request")?;
            t.set(
                "request",
                match request {
                    crate::ansi::DeviceRequest::PrimaryAttributes => "primary_attributes",
                    crate::ansi::DeviceRequest::SecondaryAttributes => "secondary_attributes",
                    crate::ansi::DeviceRequest::OperatingStatus => "operating_status",
                    crate::ansi::DeviceRequest::CursorPosition => "cursor_position",
                },
            )?;
        }
    }
    Ok(t)
}

/// Translate a [`crate::cell::Style`] into a Lua table.
///
/// The shape is a flat record so Lua-side consumers can read fields
/// without indirection. Underline is a string enum
/// (`"none"`/`"single"`/`"double"`/`"curly"`/`"dotted"`/`"dashed"`).
/// Colors reuse the [`color_to_lua`] convention from the theme
/// surface (`"default"` string / palette integer / `{r,g,b}` table)
/// so a Lua user who learns one style table can read the other
/// without re-learning a parallel encoding.
fn style_to_lua_table(lua: &Lua, style: &crate::cell::Style) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("fg", color_to_lua(lua, style.fg)?)?;
    t.set("bg", color_to_lua(lua, style.bg)?)?;
    t.set("bold", style.bold)?;
    t.set("italic", style.italic)?;
    t.set(
        "underline",
        match style.underline {
            crate::cell::UnderlineStyle::None => "none",
            crate::cell::UnderlineStyle::Single => "single",
            crate::cell::UnderlineStyle::Double => "double",
            crate::cell::UnderlineStyle::Curly => "curly",
            crate::cell::UnderlineStyle::Dotted => "dotted",
            crate::cell::UnderlineStyle::Dashed => "dashed",
        },
    )?;
    t.set("reverse", style.reverse)?;
    Ok(t)
}

/// Late-bound: `pmacs.buffer.kill` needs an [`EditorCore`] handle to
/// redirect any windows showing the doomed buffer before removal. Only
/// available after [`install_editor`] has registered the core.
fn install_buffer_kill(lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let buffer: Table = pmacs.get("buffer")?;
    let cc = core.clone();
    buffer.set(
        "kill",
        lua.create_function(move |lua, id: BufferIdLua| -> mlua::Result<()> {
            cc.borrow_mut()
                .kill_buffer(id.0)
                .map_err(mlua::Error::external)?;
            after_buffer_removed(lua, id.0);
            Ok(())
        })?,
    )?;
    Ok(())
}

fn rotate_interactive_command(lua: &Lua, name: &str) -> mlua::Result<()> {
    let origin = lua
        .app_data_ref::<crate::editor::InteractiveCommandOrigin>()
        .ok_or_else(|| {
            mlua::Error::external(
                "pmacs.command.invoke_interactive: interactive frontend context is unavailable",
            )
        })?;
    let frontend_id = origin.current().ok_or_else(|| {
        mlua::Error::external(
            "pmacs.command.invoke_interactive: requires an active interactive frontend context",
        )
    })?;
    let core = lua.app_data_ref::<SharedCore>().ok_or_else(|| {
        mlua::Error::external("pmacs.command.invoke_interactive: editor core is unavailable")
    })?;
    core.borrow_mut().rotate_command(frontend_id, name);
    Ok(())
}

fn install_command_module(lua: &Lua, commands: &SharedCommandRegistry) -> mlua::Result<Table> {
    let command = lua.create_table()?;

    {
        let cmds = commands.clone();
        command.set(
            "define",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
                let cmd = build_command_from_spec(lua, &spec)?;
                cmds.borrow_mut()
                    .define(cmd)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let cmds = commands.clone();
        command.set(
            "list",
            lua.create_function(move |lua, ()| {
                let r = cmds.borrow();
                let t = lua.create_table()?;
                for (i, name) in r.names().iter().enumerate() {
                    t.set(i + 1, name.clone())?;
                }
                Ok(t)
            })?,
        )?;
    }

    {
        let cmds = commands.clone();
        command.set(
            "invoke",
            lua.create_function(move |_, (name, args): (String, Variadic<Value>)| {
                // Lift the function out, drop the borrow, *then* call ---
                // a command body that itself calls back into
                // pmacs.command.invoke would otherwise hit a
                // double-mut-borrow panic.
                let body = {
                    let r = cmds.borrow();
                    r.get(&name)
                        .ok_or_else(|| {
                            mlua::Error::external(CommandError::NotFound { name: name.clone() })
                        })?
                        .body
                        .clone()
                };
                body.call::<Variadic<Value>>(args)
            })?,
        )?;
    }

    {
        // invoke_interactive(name, ...): like `invoke`, but records a
        // command boundary first (kill ring Q#KR2) — `last = this;
        // this = name` for the active frontend. Used by
        // `editor.execute-command` (M-x) so the invoked command's
        // chain semantics match Emacs's `execute-extended-command`
        // (which sets `this-command`): `M-x edit.kill-line` then `C-k`
        // appends, while `C-k` then `M-x edit.kill-line` does not.
        //
        // Plain `invoke` deliberately stamps NOTHING: it is a public
        // programmatic API called from wrappers, hooks, and async
        // callbacks, and must never pollute interactive command
        // history.
        let cmds = commands.clone();
        command.set(
            "invoke_interactive",
            lua.create_function(move |lua, (name, args): (String, Variadic<Value>)| {
                rotate_interactive_command(lua, &name)?;
                let body = {
                    let r = cmds.borrow();
                    r.get(&name)
                        .ok_or_else(|| {
                            mlua::Error::external(CommandError::NotFound { name: name.clone() })
                        })?
                        .body
                        .clone()
                };
                body.call::<Variadic<Value>>(args)
            })?,
        )?;
    }

    {
        let cmds = commands.clone();
        command.set(
            "exists",
            lua.create_function(move |_, name: String| Ok(cmds.borrow().contains(&name)))?,
        )?;
    }

    {
        // `pmacs.command.unregister(name)` is the inverse of `define`.
        // It exists for the M8.1 dev-loop story: a package that defines
        // commands at top level cannot be reloaded otherwise (the second
        // chunk run hits `DuplicateName`). Packages call this from their
        // `pmacs.packages.on_unload` hook to drop ownership before the
        // chunk is re-executed.
        //
        // Not init-phase gated. `define` is also not gated (the REPL,
        // audit-lint, and packages all register commands during normal
        // operation), and `unregister` is its symmetric inverse:
        // both are registry-CRUD, not the attach/detach-style
        // lifecycle calls that the init-phase gate exists for.
        // Crucially, `pmacs.packages.reload(name)` is itself not
        // init-gated --- the dev-loop story is "edit, save, reload" at
        // any time --- so an init-gated `unregister` would break every
        // package that defines commands and tries to clean them up
        // from `on_unload` post-init.
        //
        // Returns `true` if a command was removed, `false` if `name`
        // wasn't registered. We chose return-bool over erroring on
        // `NotFound` so package authors can write idempotent teardown
        // (`pcall` works too, but `if pmacs.command.exists(...)` reads
        // worse than the natural drop-and-ignore).
        let cmds = commands.clone();
        command.set(
            "unregister",
            lua.create_function(move |_, name: String| -> mlua::Result<bool> {
                let mut r = cmds.borrow_mut();
                match r.remove(&name) {
                    Ok(_) => Ok(true),
                    Err(CommandError::NotFound { .. }) => Ok(false),
                    Err(e) => Err(mlua::Error::external(e)),
                }
            })?,
        )?;
    }

    Ok(command)
}

/// Install `pmacs.menu.*` --- the context-menu item registry (Q#CM2).
///
/// Mirrors [`install_command_module`]: each closure clones the shared
/// `Rc` and borrows on demand. `item` registers, `list` introspects,
/// `remove`/`clear` tear down. Menu items reference commands by name
/// (resolved at invoke time), so this module has no dependency on the
/// command registry.
fn install_menu_module(lua: &Lua, menus: &SharedMenuRegistry) -> mlua::Result<Table> {
    let menu = lua.create_table()?;

    {
        let ms = menus.clone();
        menu.set(
            "item",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
                let item = build_menu_item_from_spec(lua, &spec)?;
                ms.borrow_mut().add(item).map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let ms = menus.clone();
        menu.set(
            "list",
            lua.create_function(move |lua, ()| {
                let r = ms.borrow();
                let out = lua.create_table()?;
                for (i, item) in r.items().iter().enumerate() {
                    let t = lua.create_table()?;
                    if let Some(id) = &item.id {
                        t.set("id", id.clone())?;
                    }
                    t.set("label", item.label.clone())?;
                    t.set("command", item.command.clone())?;
                    if let Some(context) = &item.context {
                        t.set("context", context.clone())?;
                    }
                    t.set("group", item.group.clone())?;
                    t.set("order", item.order)?;
                    t.set("has_predicate", item.predicate.is_some())?;
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // `pmacs.menu.remove(id)` drops the item(s) carrying `id`.
        // Returns `true` if anything was removed --- the symmetric
        // inverse of `item`, mirroring `pmacs.command.unregister`. Lets
        // a user config hide a builtin item idempotently.
        let ms = menus.clone();
        menu.set(
            "remove",
            lua.create_function(move |_, id: String| Ok(ms.borrow_mut().remove(&id)))?,
        )?;
    }

    {
        // `pmacs.menu.clear()` empties the registry --- the reset used
        // when a config wants to rebuild the menu from scratch.
        let ms = menus.clone();
        menu.set(
            "clear",
            lua.create_function(move |_, ()| {
                ms.borrow_mut().clear();
                Ok(())
            })?,
        )?;
    }

    {
        // `pmacs.menu._raw()` --- internal accessor returning items
        // *with* their predicate functions (which `list` omits), so the
        // Lua menu builder (`pmacs.menu.build`) can evaluate visibility.
        // Underscore-prefixed: not part of the user-facing surface.
        let ms = menus.clone();
        menu.set(
            "_raw",
            lua.create_function(move |lua, ()| {
                let r = ms.borrow();
                let out = lua.create_table()?;
                for (i, item) in r.items().iter().enumerate() {
                    let t = lua.create_table()?;
                    t.set("label", item.label.clone())?;
                    t.set("command", item.command.clone())?;
                    if let Some(context) = &item.context {
                        t.set("context", context.clone())?;
                    }
                    t.set("group", item.group.clone())?;
                    t.set("order", item.order)?;
                    if let Some(predicate) = &item.predicate {
                        t.set("predicate", predicate.clone())?;
                    }
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
    }

    Ok(menu)
}

#[allow(
    clippy::too_many_lines,
    reason = "seven help bindings each follow the same pattern; splitting them adds ceremony without clarity"
)]
fn install_help_module(
    lua: &Lua,
    registry: &SharedRegistry,
    commands: &SharedCommandRegistry,
    keymaps: &SharedKeymapStack,
    hooks: &SharedHookRegistry,
) -> mlua::Result<Table> {
    use crate::help;
    let help_t = lua.create_table()?;

    {
        let reg = registry.clone();
        let cmds = commands.clone();
        let kms = keymaps.clone();
        help_t.set(
            "show_command",
            lua.create_function(move |lua, name: String| {
                let result = {
                    let mut r = reg.borrow_mut();
                    let c = cmds.borrow();
                    let k = kms.borrow();
                    help::render_command(&mut r, &c, &k, &name)
                };
                if let Some((id, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *id, edits);
                    rebuild_help_buffer_views(lua, *id);
                }
                Ok(result.map(|(id, _)| BufferIdLua(id)))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        let cmds = commands.clone();
        let kms = keymaps.clone();
        help_t.set(
            "show_key",
            lua.create_function(move |lua, sequence: String| {
                let active_buffer = lua
                    .app_data_ref::<SharedCore>()
                    .map(|core| core.borrow().active_buffer_id());
                // Copy the mode name before taking the mutable registry
                // borrow used to replace *help*. The borrowed slice below
                // then cannot outlive or conflict with help-buffer mutation.
                let active_mode = {
                    let r = reg.borrow();
                    active_buffer
                        .and_then(|id| r.get(id).ok())
                        .and_then(crate::buffer::Buffer::major_mode)
                        .map(str::to_owned)
                };
                let active_mode = active_mode.as_deref();
                let active_modes = active_mode.as_slice();
                let result = {
                    let mut r = reg.borrow_mut();
                    let c = cmds.borrow();
                    let k = kms.borrow();
                    help::render_key(&mut r, &c, &k, active_buffer, active_modes, &sequence)
                };
                if let Some((id, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *id, edits);
                    rebuild_help_buffer_views(lua, *id);
                }
                Ok(result.map(|(id, _)| BufferIdLua(id)))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        help_t.set(
            "show_buffer",
            lua.create_function(move |lua, id: BufferIdLua| {
                let result = {
                    let mut r = reg.borrow_mut();
                    help::render_buffer(&mut r, id.0)
                };
                if let Some((rid, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *rid, edits);
                    rebuild_help_buffer_views(lua, *rid);
                }
                Ok(result.map(|(rid, _)| BufferIdLua(rid)))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        let kms = keymaps.clone();
        help_t.set(
            "show_mode",
            lua.create_function(move |lua, name: String| {
                let result = {
                    let mut r = reg.borrow_mut();
                    let k = kms.borrow();
                    help::render_mode(&mut r, &k, &name)
                };
                if let Some((id, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *id, edits);
                    rebuild_help_buffer_views(lua, *id);
                }
                Ok(result.map(|(id, _)| BufferIdLua(id)))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        let hks = hooks.clone();
        help_t.set(
            "show_hook",
            lua.create_function(move |lua, name: String| {
                let result = {
                    let mut r = reg.borrow_mut();
                    let h = hks.borrow();
                    help::render_hook(&mut r, &h, &name)
                };
                if let Some((id, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *id, edits);
                    rebuild_help_buffer_views(lua, *id);
                }
                Ok(result.map(|(id, _)| BufferIdLua(id)))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        help_t.set(
            "show_view",
            lua.create_function(move |lua, id: BufferIdLua| {
                let result = {
                    let mut r = reg.borrow_mut();
                    help::render_view(&mut r, id.0)
                };
                if let Some((rid, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *rid, edits);
                    rebuild_help_buffer_views(lua, *rid);
                }
                Ok(result.map(|(rid, _)| BufferIdLua(rid)))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        let cmds = commands.clone();
        let kms = keymaps.clone();
        let hks = hooks.clone();
        help_t.set(
            "follow_link",
            lua.create_function(move |lua, cursor: i64| {
                let cursor = u64::try_from(cursor).map_err(mlua::Error::external)?;
                let result = {
                    let mut r = reg.borrow_mut();
                    let c = cmds.borrow();
                    let k = kms.borrow();
                    let h = hks.borrow();
                    help::follow_link_at(&mut r, &c, &k, &h, cursor)
                };
                if let Some((id, edits)) = result.as_ref() {
                    queue_generated_buffer_edits(lua, *id, edits);
                    rebuild_help_buffer_views(lua, *id);
                }
                Ok(result.map(|(id, _)| BufferIdLua(id)))
            })?,
        )?;
    }

    Ok(help_t)
}

fn install_hook_module(lua: &Lua, hooks: &SharedHookRegistry) -> mlua::Result<Table> {
    let hook = lua.create_table()?;

    {
        let hks = hooks.clone();
        hook.set(
            "define",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
                // R50: typo-detection on the spec keys.
                for pair in spec.clone().pairs::<Value, Value>() {
                    let (k, _) = pair?;
                    let key = require_string_key(k)?;
                    if !matches!(key.as_str(), "name" | "description" | "kind") {
                        return Err(mlua::Error::external(
                            crate::hook::HookError::UnknownField { field: key },
                        ));
                    }
                }
                let name: String = spec.get("name")?;
                let description: Option<String> = spec.get("description")?;
                let description =
                    description
                        .filter(|d| !d.trim().is_empty())
                        .ok_or_else(|| {
                            mlua::Error::external(crate::hook::HookError::MissingDescription {
                                name: name.clone(),
                            })
                        })?;
                let kind = match spec.get::<Option<String>>("kind")? {
                    Some(s) => crate::hook::HookKind::parse(&s).map_err(mlua::Error::external)?,
                    None => crate::hook::HookKind::AllMustSucceed,
                };
                hks.borrow_mut()
                    .define(name, description, kind, caller_source(lua, 2))
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let hks = hooks.clone();
        hook.set(
            "add",
            lua.create_function(
                move |lua, (name, body): (String, Function)| -> mlua::Result<()> {
                    hks.borrow_mut()
                        .add(&name, body, caller_source(lua, 2))
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let hks = hooks.clone();
        hook.set(
            "list",
            lua.create_function(move |lua, ()| {
                let r = hks.borrow();
                let t = lua.create_table()?;
                for (i, name) in r.names().iter().enumerate() {
                    t.set(i + 1, name.clone())?;
                }
                Ok(t)
            })?,
        )?;
    }

    {
        let hks = hooks.clone();
        hook.set("run", lua.create_function(run_hook_from_lua(hks))?)?;
    }

    Ok(hook)
}

/// `pmacs.hook.run(name, args...)` --- snapshot the hook's callbacks,
/// drop the registry borrow, dispatch per the hook's kind. The Lua
/// return value is:
///
/// * For `short-circuit`: `true` if the run proceeded, `false` if a
///   callback vetoed (returned `false` or raised).
/// * For `all-must-succeed`: `true` if every callback succeeded,
///   `false` otherwise.
/// * For `accumulate`: the final accumulated value (multi-return).
///
/// Errors from individual callbacks are not returned to the caller ---
/// they are captured into the host's `*errors*` log so that listeners
/// don't take down the whole pipeline.
fn run_hook_from_lua(
    hks: SharedHookRegistry,
) -> impl Fn(&Lua, (String, Variadic<Value>)) -> mlua::Result<Variadic<Value>> + 'static {
    move |lua, (name, args)| {
        let snapshot = hks.borrow().snapshot(&name);
        let Some((kind, callbacks)) = snapshot else {
            return Err(mlua::Error::external(crate::hook::HookError::NotFound {
                name,
            }));
        };
        let mut margs = mlua::MultiValue::new();
        for v in args {
            margs.push_back(v);
        }
        let outcome = crate::hook::run_snapshot(kind, &callbacks, margs);
        for err in &outcome.errors {
            log_hook_error(lua, &name, err);
        }
        let mut out = Variadic::new();
        if kind == crate::hook::HookKind::Accumulate {
            out.push(if outcome.proceed {
                outcome.value
            } else {
                Value::Nil
            });
        } else {
            out.push(Value::Boolean(outcome.proceed));
        }
        Ok(out)
    }
}

/// Append a one-line entry to the `*errors*` buffer naming the hook
/// and the failing callback's source. Mirrors the behaviour of
/// [`crate::lua::LuaHost::eval`] for chunk-level errors --- failures
/// in user-attached hooks should land in the same place as syntax
/// errors in `init.lua`.
fn log_hook_error(lua: &Lua, hook_name: &str, err: &crate::hook::HookCallbackError) {
    let line = format!(
        "[hook:{hook_name}] callback at {} raised: {}\n",
        err.source.render(),
        err.error
    );
    let result = {
        let Some(app) = lua.app_data_ref::<SharedRegistry>() else {
            return;
        };
        let mut reg = app.borrow_mut();
        let id = match reg.find_by_name(crate::lua::ERRORS_BUFFER_NAME) {
            Some(id) => id,
            None => reg.create(crate::lua::ERRORS_BUFFER_NAME),
        };
        let Ok(buf) = reg.get_mut(id) else {
            return;
        };
        let pos = buf.len();
        let edit = buf
            .apply_edit(EditOp::Insert {
                pos,
                bytes: line.as_bytes(),
            })
            .ok();
        edit.map(|e| (id, e))
    };
    // Notify any window viewing *errors* — same staleness fix as
    // `LuaHost::append_to_errors_buffer`.
    if let Some((id, edit)) = result {
        notify_buffer_edit_to_windows(lua, id, &edit);
    }
}
/// Append a first-in-run statusline provider failure to `*errors*`.
///
/// The latch decision lives in [`crate::statusline::StatuslineRegistry`];
/// this function owns only the repository-standard durable sink and window
/// invalidation.
pub(crate) fn log_statusline_provider_error(lua: &Lua, failure: &StatuslineProviderFailure) {
    let message = crate::statusline::sanitize_provider_error_text(&failure.message);
    let line = format!(
        "[statusline:{}] provider registered at {} failed for {:?}/{:?}/{:?}/active={}: {}\n",
        failure.provider_name,
        failure.source.render(),
        failure.context.frontend_id,
        failure.context.window_id,
        failure.context.buffer_id,
        failure.context.active,
        message,
    );
    let result = {
        let Some(app) = lua.app_data_ref::<SharedRegistry>() else {
            return;
        };
        let mut registry = app.borrow_mut();
        let id = match registry.find_by_name(crate::lua::ERRORS_BUFFER_NAME) {
            Some(id) => id,
            None => registry.create(crate::lua::ERRORS_BUFFER_NAME),
        };
        let Ok(buffer) = registry.get_mut(id) else {
            return;
        };
        let position = buffer.len();
        let edit = buffer
            .apply_edit(EditOp::Insert {
                pos: position,
                bytes: line.as_bytes(),
            })
            .ok();
        edit.map(|edit| (id, edit))
    };
    if let Some((id, edit)) = result {
        notify_buffer_edit_to_windows(lua, id, &edit);
    }
}

/// T M7.8: append a `[package <name>]` entry to `*errors*`.
///
/// Mirrors `log_hook_error`'s implementation. Used by
/// `pmacs.packages.load` so a single failing package's error lands in
/// the canonical sink without abandoning the rest of the load list.
fn log_package_load_error(lua: &Lua, package: &str, err: &mlua::Error) {
    let line = format!("[package {package}] load failed: {err}\n");
    let result = {
        let Some(app) = lua.app_data_ref::<SharedRegistry>() else {
            return;
        };
        let mut reg = app.borrow_mut();
        let id = match reg.find_by_name(crate::lua::ERRORS_BUFFER_NAME) {
            Some(id) => id,
            None => reg.create(crate::lua::ERRORS_BUFFER_NAME),
        };
        let Ok(buf) = reg.get_mut(id) else {
            return;
        };
        let pos = buf.len();
        let edit = buf
            .apply_edit(EditOp::Insert {
                pos,
                bytes: line.as_bytes(),
            })
            .ok();
        edit.map(|e| (id, e))
    };
    if let Some((id, edit)) = result {
        notify_buffer_edit_to_windows(lua, id, &edit);
    }
}

fn log_buffer_removed_error(lua: &Lua, source: &SourceLocation, err: &mlua::Error) {
    let line = format!(
        "[buffer.on_removed] callback at {} raised: {err}\n",
        source.render()
    );
    let result = {
        let Some(app) = lua.app_data_ref::<SharedRegistry>() else {
            return;
        };
        let mut reg = app.borrow_mut();
        let id = match reg.find_by_name(crate::lua::ERRORS_BUFFER_NAME) {
            Some(id) => id,
            None => reg.create(crate::lua::ERRORS_BUFFER_NAME),
        };
        let Ok(buf) = reg.get_mut(id) else {
            return;
        };
        let pos = buf.len();
        let edit = buf
            .apply_edit(EditOp::Insert {
                pos,
                bytes: line.as_bytes(),
            })
            .ok();
        edit.map(|e| (id, e))
    };
    if let Some((id, edit)) = result {
        notify_buffer_edit_to_windows(lua, id, &edit);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "six describe bindings share one coherent registry surface; splitting them adds ceremony without clarifying borrow lifetimes"
)]
fn install_describe_module(
    lua: &Lua,
    registry: &SharedRegistry,
    commands: &SharedCommandRegistry,
    keymaps: &SharedKeymapStack,
    hooks: &SharedHookRegistry,
) -> mlua::Result<Table> {
    let describe = lua.create_table()?;

    {
        let cmds = commands.clone();
        let kms = keymaps.clone();
        describe.set(
            "command",
            lua.create_function(move |lua, name: String| {
                let r = cmds.borrow();
                let km = kms.borrow();
                match r.get(&name) {
                    Some(cmd) => Ok(Value::Table(command_info_table(lua, cmd, &km)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        let cmds = commands.clone();
        let kms = keymaps.clone();
        describe.set(
            "key",
            lua.create_function(move |lua, sequence: String| {
                let chords = parse_sequence(&sequence).map_err(mlua::Error::external)?;
                let active_buffer = lua
                    .app_data_ref::<SharedCore>()
                    .map(|core| core.borrow().active_buffer_id());
                // Keep the registry borrow only across pure resolution.
                // `ResolvedBinding` owns its scope, so creating the Lua
                // result table cannot retain a Buffer borrow or re-enter
                // Lua while one is live.
                let resolution = {
                    let r = reg.borrow();
                    let active_mode = active_buffer
                        .and_then(|id| r.get(id).ok())
                        .and_then(crate::buffer::Buffer::major_mode);
                    let active_modes = active_mode.as_slice();
                    let km = kms.borrow();
                    km.resolve(&chords, active_buffer, active_modes)
                };
                match resolution {
                    crate::keymap_stack::StackResolution::Bound(rb) => {
                        let cmds = cmds.borrow();
                        Ok(Value::Table(key_info_table(
                            lua,
                            &chords,
                            &rb,
                            cmds.get(&rb.binding.command),
                        )?))
                    }
                    _ => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        describe.set(
            "buffer",
            lua.create_function(move |lua, id: BufferIdLua| {
                let r = reg.borrow();
                match r.get(id.0) {
                    Ok(buf) => Ok(Value::Table(buffer_info_table(lua, buf)?)),
                    Err(_) => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        describe.set(
            "view",
            lua.create_function(move |lua, id: BufferIdLua| {
                let r = reg.borrow();
                match r.get(id.0) {
                    Ok(buf) => Ok(Value::Table(view_info_table(lua, buf)?)),
                    Err(_) => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let kms = keymaps.clone();
        describe.set(
            "mode",
            lua.create_function(move |lua, name: String| {
                let km = kms.borrow();
                match km.modes.iter().find(|(n, _)| n == &name) {
                    Some((_, map)) => Ok(Value::Table(mode_info_table(lua, &name, map)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let hks = hooks.clone();
        describe.set(
            "hook",
            lua.create_function(move |lua, name: String| {
                let r = hks.borrow();
                match r.get(&name) {
                    Some(hook) => Ok(Value::Table(hook_info_table(lua, hook)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    Ok(describe)
}

fn command_info_table(lua: &Lua, cmd: &Command, keymaps: &KeymapStack) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", cmd.name.clone())?;
    t.set("description", cmd.description.clone())?;
    t.set("source", cmd.source.render())?;
    // Populate key_bindings by reverse-scanning the keymap stack for
    // every binding whose command matches `cmd.name`.
    let bindings = lua.create_table()?;
    let mut idx = 1;
    for (scope, seq, binding) in keymaps.iter_all() {
        if binding.command == cmd.name {
            let entry = lua.create_table()?;
            entry.set("sequence", display_sequence(&seq))?;
            entry.set("scope", scope.render())?;
            entry.set("source", binding.source.render())?;
            bindings.set(idx, entry)?;
            idx += 1;
        }
    }
    t.set("key_bindings", bindings)?;
    Ok(t)
}

fn buffer_info_table(lua: &Lua, buf: &crate::buffer::Buffer) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", buf.name().to_owned())?;
    t.set(
        "length",
        i64::try_from(buf.len()).map_err(mlua::Error::external)?,
    )?;
    t.set("modified", buf.is_modified())?;
    t.set(
        "view_count",
        i64::try_from(buf.view_count()).map_err(mlua::Error::external)?,
    )?;
    Ok(t)
}

fn view_info_table(lua: &Lua, buf: &crate::buffer::Buffer) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("buffer_name", buf.name().to_owned())?;
    let view_ids = lua.create_table()?;
    for (i, vid) in buf.view_ids().enumerate() {
        view_ids.set(i + 1, format!("{vid:?}"))?;
    }
    t.set("view_ids", view_ids)?;
    t.set(
        "view_count",
        i64::try_from(buf.view_count()).map_err(mlua::Error::external)?,
    )?;
    Ok(t)
}

fn mode_info_table(lua: &Lua, name: &str, map: &crate::keymap_tree::Keymap) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", name.to_owned())?;
    let bindings = lua.create_table()?;
    for (i, (seq, binding)) in map.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("sequence", display_sequence(&seq))?;
        entry.set("command", binding.command)?;
        entry.set("source", binding.source.render())?;
        bindings.set(i + 1, entry)?;
    }
    t.set("bindings", bindings)?;
    Ok(t)
}

fn hook_info_table(lua: &Lua, hook: &Hook) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", hook.name.clone())?;
    t.set("description", hook.description.clone())?;
    t.set("kind", hook.kind.as_str())?;
    t.set("source", hook.source.render())?;
    let callbacks = lua.create_table()?;
    for (i, cb) in hook.callbacks.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("source", cb.source.render())?;
        callbacks.set(i + 1, entry)?;
    }
    t.set("callbacks", callbacks)?;
    Ok(t)
}

fn key_info_table(
    lua: &Lua,
    chords: &[crate::key::Chord],
    rb: &crate::keymap_stack::ResolvedBinding,
    cmd: Option<&Command>,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("sequence", display_sequence(chords))?;
    t.set("command", rb.binding.command.clone())?;
    t.set("scope", rb.scope.render())?;
    t.set("source", rb.binding.source.render())?;
    if let Some(cmd) = cmd {
        t.set("description", cmd.description.clone())?;
    }
    Ok(t)
}

fn install_keymap_module(lua: &Lua, keymaps: &SharedKeymapStack) -> mlua::Result<Table> {
    let keymap = lua.create_table()?;

    {
        let kms = keymaps.clone();
        keymap.set(
            "bind",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
                let bind_args = parse_bind_spec(lua, &spec)?;
                bind_args.apply(&mut kms.borrow_mut())
            })?,
        )?;
    }

    {
        let kms = keymaps.clone();
        keymap.set(
            "unbind",
            lua.create_function(move |_, spec: Table| -> mlua::Result<()> {
                let unbind_args = parse_unbind_spec(&spec)?;
                unbind_args.apply(&mut kms.borrow_mut())
            })?,
        )?;
    }

    {
        let kms = keymaps.clone();
        keymap.set(
            "lookup",
            lua.create_function(move |lua, sequence: String| {
                let chords = parse_sequence(&sequence).map_err(mlua::Error::external)?;
                let km = kms.borrow();
                match km.resolve(&chords, None, &[]) {
                    crate::keymap_stack::StackResolution::Bound(rb) => {
                        Ok(Value::Table(key_info_table(lua, &chords, &rb, None)?))
                    }
                    _ => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let kms = keymaps.clone();
        keymap.set(
            "list",
            lua.create_function(move |lua, ()| {
                let km = kms.borrow();
                let out = lua.create_table()?;
                for (i, (scope, seq, binding)) in km.iter_all().into_iter().enumerate() {
                    let entry = lua.create_table()?;
                    entry.set("sequence", display_sequence(&seq))?;
                    entry.set("command", binding.command)?;
                    entry.set("scope", scope.render())?;
                    out.set(i + 1, entry)?;
                }
                Ok(out)
            })?,
        )?;
    }

    Ok(keymap)
}

// ---------------------------------------------------------------------------
// pmacs.editor: world-state primitives.
// ---------------------------------------------------------------------------

/// Install the `pmacs.editor.*` table on top of an already-installed
/// `pmacs` global, and register the editor [`SharedCore`] on the Lua
/// state's app data.
///
/// Must be called *after* [`install`] has set up `pmacs.{buffer,
/// command, keymap, describe}`. The editor primitives borrow the
/// [`EditorCore`] through the captured `Rc<RefCell<...>>`, so they
/// can mutate it from inside command bodies invoked through
/// [`crate::lua::LuaHost::invoke_command`] without colliding with the
/// run loop's outstanding `&mut EditorState`.
///
/// # Errors
///
/// Returns the underlying [`mlua::Error`] if either the existing
/// `pmacs` global is missing or any closure registration fails.
pub fn install_editor(lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    lua.set_app_data(core.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let editor = lua.create_table()?;

    {
        let cc = core.clone();
        let registry = core.borrow().registry.clone();
        editor.set(
            "active_modes",
            lua.create_function(move |lua, ()| {
                let active_buffer = cc.borrow().active_buffer_id();
                let mode = {
                    let r = registry.borrow();
                    resolve(&r, active_buffer)?.major_mode().map(str::to_owned)
                };
                let modes = lua.create_table()?;
                if let Some(mode) = mode {
                    modes.set(1, mode)?;
                }
                Ok(modes)
            })?,
        )?;
    }

    install_motion(&editor, lua, core)?;
    install_editing(&editor, lua, core)?;
    install_history(&editor, lua, core)?;
    install_session(&editor, lua, core)?;
    install_search(&editor, lua, core)?;

    pmacs.set("editor", editor)?;
    pmacs.set("frontend", install_frontend_module(lua, core)?)?;
    pmacs.set("minibuffer", install_minibuffer_module(lua, core)?)?;
    pmacs.set("window", install_window_module(lua, core)?)?;
    install_buffer_kill(lua, core)?;
    Ok(())
}

/// Build the `pmacs.frontend` table (T M5.4).
///
/// v0.1 surface:
/// * `pmacs.frontend.id()` returns the integer ID of the frontend
///   that produced the most recent dispatched input event. Single-
///   frontend instances (the v0.1 norm) always return
///   [`crate::protocol::FrontendId::LOCAL`]'s inner integer.
fn install_frontend_module(lua: &Lua, core: &SharedCore) -> mlua::Result<Table> {
    let frontend = lua.create_table()?;

    let core_clone = core.clone();
    frontend.set(
        "id",
        lua.create_function(move |_, ()| {
            let id = core_clone.borrow().active_frontend.0;
            Ok(i64::try_from(id).unwrap_or(i64::MAX))
        })?,
    )?;

    Ok(frontend)
}

// ---------------------------------------------------------------------------
// pmacs._async: raw helpers for the Lua-side coroutine runtime (T M3.3).
//
// The friendly surface --- `pmacs.async`, `Handle:await`, `:on_complete`,
// `:cancel`, the `{ tag = "cancelled" }` error shape --- lives in
// `builtin/runtime/async.lua`. This module exposes only the primitives
// the Lua chunk builds on:
//
// * `_dispatch_sleep(ms)` --- returns a `JobId`.
// * `_dispatch_sum(n)`    --- returns a `JobId`.
// * `_cancel(id)`         --- requests cancellation.
// * `_is_complete(id)`    --- has the runtime observed a settled state?
// * `_is_cancelled(id)`   --- did the worker observe cancellation?
// * `_take_result(id)`    --- consumes the entry and returns
//                            `(status, value)` where `status` is one of
//                            `"ok"`, `"cancelled"`, `"failed"`, `"pending"`.
// * `_tick()`             --- drains the worker reply bus and returns
//                            the array of newly-settled ids.
// ---------------------------------------------------------------------------

/// Read a Lua table into a [`GrepSpec`]. `root` and `pattern` are
/// required; every other field falls back to [`GrepSpec::new`]'s
/// defaults. Type errors raise via `mlua::Error::external`.
fn grep_spec_from_table(t: &Table) -> mlua::Result<GrepSpec> {
    let root: String = t.get("root").map_err(|_| {
        mlua::Error::external("pmacs.workers.grep: spec.root (string path) is required")
    })?;
    let pattern: String = t.get("pattern").map_err(|_| {
        mlua::Error::external("pmacs.workers.grep: spec.pattern (string) is required")
    })?;
    let mut spec = GrepSpec::new(std::path::PathBuf::from(root), pattern);
    if let Ok(case_sensitive) = t.get::<bool>("case_sensitive") {
        spec.case_sensitive = case_sensitive;
    }
    if let Ok(max_file_bytes) = t.get::<u64>("max_file_bytes") {
        spec.max_file_bytes = max_file_bytes;
    }
    if let Ok(max_match_text) = t.get::<u32>("max_match_text") {
        spec.max_match_text = max_match_text;
    }
    if let Ok(max_results) = t.get::<u32>("max_results") {
        spec.max_results = max_results;
    }
    if let Ok(fanout) = t.get::<usize>("fanout") {
        spec.fanout = fanout.max(1);
    }
    Ok(spec)
}

/// Translate one [`StreamPayload`] into the Lua value the user
/// callback sees. The shape is per-variant: `U64` becomes a Lua
/// integer; `Match` becomes a `{ file, line, text, match_start,
/// match_end }` table. New variants extend this match exhaustively.
/// Convert a [`crate::fs::FsDirEntry`] into the Lua-side table
/// `pmacs.fs.read_dir`'s callers consume. The shape pins the v0.1
/// keys --- adding fields is non-breaking, removing or renaming
/// them is a breaking change for every dired/wdired-class package.
fn fs_dir_entry_to_lua(lua: &Lua, entry: &crate::fs::FsDirEntry) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 7)?;
    t.set("name", entry.name.as_str())?;
    t.set("kind", entry.kind.as_str())?;
    t.set("size", i64::try_from(entry.size).unwrap_or(i64::MAX))?;
    t.set("mtime", entry.mtime_secs)?;
    t.set("mtime_nsec", i64::from(entry.mtime_nsec))?;
    t.set("mode", i64::from(entry.mode))?;
    if let Some(target) = &entry.symlink_target {
        t.set("symlink_target", target.as_str())?;
    }
    Ok(t)
}

fn stream_payload_to_lua(lua: &Lua, payload: StreamPayload) -> mlua::Result<mlua::Value> {
    match payload {
        StreamPayload::U64(v) => Ok(mlua::Value::Integer(i64::try_from(v).unwrap_or(i64::MAX))),
        StreamPayload::Match(GrepMatch {
            file,
            line,
            match_start,
            match_end,
            text,
        }) => {
            let t = lua.create_table_with_capacity(0, 5)?;
            t.set("file", file)?;
            t.set("line", line)?;
            t.set("match_start", match_start)?;
            t.set("match_end", match_end)?;
            t.set("text", text)?;
            Ok(mlua::Value::Table(t))
        }
    }
}

/// Install the async runtime's raw helpers onto `pmacs._async`. Must
/// run after [`install`] so the `pmacs` table already exists.
///
/// `registry` is the shared buffer registry the `*workers*`
/// observability buffer renders into ([T M3.7]). It threads through
/// `_show_workers_buffer` --- callers that don't need the buffer
/// surface (a few in-process tests) can still use `install_async`
/// without it: the bindings short-circuit if the registry is `None`.
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings, each following the same Rc-borrow shape; splitting into helpers adds ceremony without clarity"
)]
pub fn install_async(
    lua: &Lua,
    runtime: &SharedAsyncRuntime,
    registry: Option<&SharedRegistry>,
) -> mlua::Result<()> {
    lua.set_app_data(runtime.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let async_mod = lua.create_table()?;

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_sleep",
            lua.create_function(move |_, (ms, key): (i64, Option<String>)| {
                Ok(rt.dispatch_sleep(ms, key.as_deref()))
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_sum",
            lua.create_function(move |_, (n, key): (u64, Option<String>)| {
                Ok(rt.dispatch_compute_sum(n, key.as_deref()))
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_emit_n",
            lua.create_function(
                move |_, (count, key, max_batch): (u64, Option<String>, Option<usize>)| {
                    Ok(rt.dispatch_emit_n(count, key.as_deref(), max_batch))
                },
            )?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_grep",
            lua.create_function(
                move |_, (spec_table, key, max_batch): (Table, Option<String>, Option<usize>)| {
                    let spec = grep_spec_from_table(&spec_table)?;
                    Ok(rt.dispatch_grep(spec, key.as_deref(), max_batch))
                },
            )?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_fs_read_dir",
            lua.create_function(move |_, (path, key): (String, Option<String>)| {
                Ok(rt.dispatch_fs_read_dir(std::path::PathBuf::from(path), key.as_deref()))
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_fs_stat",
            lua.create_function(move |_, (path, key): (String, Option<String>)| {
                Ok(rt.dispatch_fs_stat(std::path::PathBuf::from(path), key.as_deref()))
            })?,
        )?;
    }

    // Mutating fs raw dispatchers: no supersede parameter exposed
    // at the Lua surface. The Rust runtime methods still accept
    // `Option<&str>` for symmetry, but pmacs._async hard-codes
    // `None` so packages reaching for the raw bindings can't
    // bypass the no-supersede-on-mutation safety decision the
    // wrappers in `builtin/runtime/fs.lua` document. Rationale:
    // a "cancelled" syscall may have already run and changed
    // disk; supersede semantics are misleading for ops that mutate.
    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_fs_rename",
            lua.create_function(move |_, (from, to): (String, String)| {
                Ok(rt.dispatch_fs_rename(
                    std::path::PathBuf::from(from),
                    std::path::PathBuf::from(to),
                    None,
                ))
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_fs_chmod",
            lua.create_function(move |_, (path, mode): (String, u32)| {
                Ok(rt.dispatch_fs_chmod(std::path::PathBuf::from(path), mode, None))
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_dispatch_fs_remove",
            lua.create_function(move |_, path: String| {
                Ok(rt.dispatch_fs_remove(std::path::PathBuf::from(path), None))
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_take_stream_batches",
            lua.create_function(move |lua, ()| {
                let batches = rt.take_stream_batches();
                let out = lua.create_table_with_capacity(batches.len(), 0)?;
                for (i, batch) in batches.into_iter().enumerate() {
                    let entry = lua.create_table()?;
                    entry.set("id", batch.id)?;
                    entry.set("closed", batch.closed)?;
                    let items = lua.create_table_with_capacity(batch.items.len(), 0)?;
                    for (j, payload) in batch.items.into_iter().enumerate() {
                        let value = stream_payload_to_lua(lua, payload)?;
                        items.set(j + 1, value)?;
                    }
                    entry.set("items", items)?;
                    if batch.closed {
                        let (status, value): (&'static str, mlua::Value) = match batch.outcome {
                            Some(JobOutcome::Complete(JobResult::Sum(v))) => (
                                "ok",
                                mlua::Value::Integer(i64::try_from(v).unwrap_or(i64::MAX)),
                            ),
                            Some(JobOutcome::Cancelled) => ("cancelled", mlua::Value::Nil),
                            Some(JobOutcome::Failed(msg)) => {
                                ("failed", mlua::Value::String(lua.create_string(&msg)?))
                            }
                            // Unit, Parse, ReadDir, and "no recorded
                            // outcome" all surface as a clean
                            // ok-with-nil to Lua. Streams that close
                            // without an explicit outcome (the
                            // typical emit_n case) are
                            // indistinguishable from ones that
                            // returned `Unit`. Parse and ReadDir
                            // jobs aren't streams; the arms are
                            // here for exhaustiveness, since a
                            // settled non-stream job's outcome
                            // could in principle reach this branch
                            // if the runtime were extended to ship
                            // a stream-closed reply for them.
                            Some(JobOutcome::Complete(
                                JobResult::Unit
                                | JobResult::Parse { .. }
                                | JobResult::ReadDir(_)
                                | JobResult::Stat(_)
                                | JobResult::Json(_),
                            ))
                            | None => ("ok", mlua::Value::Nil),
                        };
                        entry.set("status", status)?;
                        entry.set("value", value)?;
                    }
                    out.set(i + 1, entry)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_frame_target_ms",
            lua.create_function(move |_, ()| Ok(rt.frame_target_ms()))?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_set_frame_target_ms",
            lua.create_function(move |_, ms: u64| {
                rt.set_frame_target_ms(ms);
                Ok(())
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_default_max_batch",
            lua.create_function(move |_, ()| Ok(rt.default_max_batch()))?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_set_default_max_batch",
            lua.create_function(move |_, n: usize| {
                rt.set_default_max_batch(n);
                Ok(())
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_cancel",
            lua.create_function(move |_, id: u64| {
                rt.cancel(id);
                Ok(())
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_is_complete",
            lua.create_function(move |_, id: u64| Ok(rt.is_complete(id)))?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_is_cancelled",
            lua.create_function(move |_, id: u64| Ok(rt.is_cancelled(id)))?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_take_result",
            lua.create_function(move |lua, id: u64| {
                let mut out = mlua::MultiValue::new();
                match rt.take_result(id) {
                    Some(JobOutcome::Complete(JobResult::Unit)) => {
                        out.push_back(mlua::Value::String(lua.create_string("ok")?));
                        out.push_back(mlua::Value::Nil);
                    }
                    Some(JobOutcome::Complete(JobResult::Sum(v))) => {
                        out.push_back(mlua::Value::String(lua.create_string("ok")?));
                        out.push_back(mlua::Value::Integer(i64::try_from(v).unwrap_or(i64::MAX)));
                    }
                    Some(JobOutcome::Complete(JobResult::Parse { duration_ms })) => {
                        // Lua surface for parse settle: status "ok",
                        // value = parse-only duration in ms. The tree
                        // itself is fetched separately via
                        // `pmacs.parse._take_tree(id)` (T M4.1).
                        out.push_back(mlua::Value::String(lua.create_string("ok")?));
                        out.push_back(mlua::Value::Integer(
                            i64::try_from(duration_ms).unwrap_or(i64::MAX),
                        ));
                    }
                    Some(JobOutcome::Complete(JobResult::ReadDir(entries))) => {
                        // Lua surface for fs.read_dir settle:
                        // status "ok", value = array of per-entry
                        // tables. T M8.1.
                        out.push_back(mlua::Value::String(lua.create_string("ok")?));
                        let t = lua.create_table_with_capacity(entries.len(), 0)?;
                        for (i, entry) in entries.into_iter().enumerate() {
                            t.set(i + 1, fs_dir_entry_to_lua(lua, &entry)?)?;
                        }
                        out.push_back(mlua::Value::Table(t));
                    }
                    Some(JobOutcome::Complete(JobResult::Stat(entry))) => {
                        // Lua surface for fs.stat settle: status
                        // "ok", value = single per-entry table
                        // (same shape as a read_dir entry). T M8.1.
                        out.push_back(mlua::Value::String(lua.create_string("ok")?));
                        out.push_back(mlua::Value::Table(fs_dir_entry_to_lua(lua, &entry)?));
                    }
                    Some(JobOutcome::Complete(JobResult::Json(value))) => {
                        // Lua surface for externally-completed JSON
                        // jobs (M9.1 MCP requests): status "ok",
                        // value = JSON-translated Lua table.
                        out.push_back(mlua::Value::String(lua.create_string("ok")?));
                        out.push_back(json_to_lua(lua, &value)?);
                    }
                    Some(JobOutcome::Cancelled) => {
                        out.push_back(mlua::Value::String(lua.create_string("cancelled")?));
                        out.push_back(mlua::Value::Nil);
                    }
                    Some(JobOutcome::Failed(msg)) => {
                        out.push_back(mlua::Value::String(lua.create_string("failed")?));
                        out.push_back(mlua::Value::String(lua.create_string(&msg)?));
                    }
                    None => {
                        out.push_back(mlua::Value::String(lua.create_string("pending")?));
                        out.push_back(mlua::Value::Nil);
                    }
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_tick",
            lua.create_function(move |lua, ()| {
                let ids = rt.tick();
                let t = lua.create_table_with_capacity(ids.len(), 0)?;
                for (i, id) in ids.into_iter().enumerate() {
                    t.set(i + 1, id)?;
                }
                Ok(t)
            })?,
        )?;
    }

    {
        let rt = runtime.clone();
        async_mod.set(
            "_pending_len",
            lua.create_function(move |_, ()| Ok(rt.pending_len()))?,
        )?;
    }

    // T M3.7: snapshot the runtime's job tables. Returned as
    // `{ active = {...}, completed = {...} }` where each entry is a
    // table the Lua side can render directly.
    {
        let rt = runtime.clone();
        async_mod.set(
            "_workers_snapshot",
            lua.create_function(move |lua, ()| workers_snapshot_to_lua(lua, &rt))?,
        )?;
    }

    if let Some(registry) = registry {
        let rt = runtime.clone();
        let reg = registry.clone();
        async_mod.set(
            "_show_workers_buffer",
            lua.create_function(move |lua, ()| {
                let snap = rt.workers_snapshot();
                let (id, edits) = workers_buffer::render(&mut reg.borrow_mut(), &snap);
                queue_generated_buffer_edits(lua, id, &edits);
                if !edits.is_empty() {
                    rebuild_generated_buffer_views(lua, id);
                }
                Ok(BufferIdLua(id))
            })?,
        )?;
        let reg = registry.clone();
        async_mod.set(
            "_job_id_at_byte",
            lua.create_function(move |_, (id, pos): (BufferIdLua, i64)| {
                let r = reg.borrow();
                let Ok(buf) = r.get(id.0) else {
                    return Ok(None);
                };
                let len = buf.len();
                let mut bytes = vec![0u8; len as usize];
                if !bytes.is_empty() {
                    buf.snapshot_rope().slice(0, len, &mut bytes);
                }
                let text = std::str::from_utf8(&bytes).unwrap_or("");
                let pos = usize::try_from(pos.max(0)).unwrap_or(0);
                Ok(workers_buffer::job_id_at_byte(text, pos))
            })?,
        )?;
    }

    pmacs.set("_async", async_mod)?;
    Ok(())
}

/// Format the runtime's [`crate::async_runtime::WorkersSnapshot`] as a
/// Lua table the builtin can iterate. Pulled out of `install_async`
/// so the per-row translation stays compact at the call site.
fn workers_snapshot_to_lua(lua: &Lua, runtime: &SharedAsyncRuntime) -> mlua::Result<mlua::Value> {
    let snap = runtime.workers_snapshot();
    let out = lua.create_table()?;
    let active = lua.create_table_with_capacity(snap.active.len(), 0)?;
    for (i, job) in snap.active.iter().enumerate() {
        let row = lua.create_table_with_capacity(0, 6)?;
        row.set("id", job.id)?;
        row.set("kind", job.kind.label())?;
        row.set("age_ms", job.age_ms)?;
        if let Some(key) = &job.supersede_key {
            row.set("supersede", key.as_str())?;
        }
        row.set("cancel_requested", job.cancel_requested)?;
        row.set("is_stream", job.is_stream)?;
        active.set(i + 1, row)?;
    }
    out.set("active", active)?;
    let completed = lua.create_table_with_capacity(snap.completed.len(), 0)?;
    for (i, job) in snap.completed.iter().enumerate() {
        let row = lua.create_table_with_capacity(0, 7)?;
        row.set("id", job.id)?;
        row.set("kind", job.kind.label())?;
        row.set("duration_ms", job.duration_ms)?;
        row.set("settled_age_ms", job.settled_age_ms)?;
        if let Some(key) = &job.supersede_key {
            row.set("supersede", key.as_str())?;
        }
        let (status, value): (&'static str, mlua::Value) = match &job.outcome {
            JobOutcome::Complete(JobResult::Unit) => ("ok", mlua::Value::Nil),
            JobOutcome::Complete(JobResult::Sum(v)) => (
                "ok",
                mlua::Value::Integer(i64::try_from(*v).unwrap_or(i64::MAX)),
            ),
            JobOutcome::Complete(JobResult::Parse { duration_ms }) => (
                "ok",
                mlua::Value::Integer(i64::try_from(*duration_ms).unwrap_or(i64::MAX)),
            ),
            JobOutcome::Complete(JobResult::ReadDir(entries)) => (
                "ok",
                mlua::Value::Integer(i64::try_from(entries.len()).unwrap_or(i64::MAX)),
            ),
            JobOutcome::Complete(JobResult::Stat(entry)) => {
                ("ok", mlua::Value::String(lua.create_string(&entry.name)?))
            }
            JobOutcome::Complete(JobResult::Json(_)) => {
                // M9.1 MCP responses surface in the workers buffer
                // by status only; the JSON payload itself is
                // displayed in the consumer's own buffer (e.g. an
                // M9.5 resource buffer).
                ("ok", mlua::Value::Nil)
            }
            JobOutcome::Cancelled => ("cancelled", mlua::Value::Nil),
            JobOutcome::Failed(msg) => ("failed", mlua::Value::String(lua.create_string(msg)?)),
        };
        row.set("status", status)?;
        row.set("value", value)?;
        completed.set(i + 1, row)?;
    }
    out.set("completed", completed)?;
    Ok(mlua::Value::Table(out))
}

/// Build a fresh [`AsyncRuntime`] sized to `available_parallelism - 1`,
/// install its raw helpers under `pmacs._async`, and return the
/// [`SharedAsyncRuntime`]. The editor's run loop holds the same `Rc`
/// to drive [`AsyncRuntime::tick`] each iteration.
pub fn make_async_runtime(
    lua: &Lua,
    registry: Option<&SharedRegistry>,
) -> mlua::Result<SharedAsyncRuntime> {
    let runtime = Rc::new(AsyncRuntime::with_default_pool());
    install_async(lua, &runtime, registry)?;
    Ok(runtime)
}

// ---------------------------------------------------------------------------
// pmacs.parse: tree-sitter Lua surface (T M4.1)
// ---------------------------------------------------------------------------

/// Lua-facing wrapper around an [`Arc<ParseTreeBundle>`]. Cheap to
/// clone (an `Arc` bump). All methods are read-only --- the bundle
/// is immutable once installed; a new parse produces a new bundle.
#[derive(Clone)]
pub struct ParseTreeLua(Arc<ParseTreeBundle>);

/// Lua-facing wrapper around a node within a [`ParseTreeLua`]. The
/// node is identified by a path of child indices from the tree's
/// root. Every method resolves the path and re-walks the tree, so
/// the userdata lifetime is decoupled from `tree_sitter::Node`'s
/// borrow of `Tree`. O(depth) per access, which is fine for the
/// shallow-traversal patterns Lua scripts actually use.
#[derive(Clone)]
pub struct ParseNodeLua {
    bundle: Arc<ParseTreeBundle>,
    path: Vec<u32>,
}

impl ParseNodeLua {
    fn resolve(&self) -> Option<tree_sitter::Node<'_>> {
        let mut node = self.bundle.root_tree().root_node();
        for &idx in &self.path {
            node = node.child(idx)?;
        }
        Some(node)
    }
}

fn point_to_lua(lua: &Lua, p: tree_sitter::Point) -> mlua::Result<mlua::Value> {
    let t = lua.create_table_with_capacity(0, 2)?;
    t.set("row", p.row)?;
    t.set("column", p.column)?;
    Ok(mlua::Value::Table(t))
}

impl UserData for ParseTreeLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("root", |_, this, ()| {
            Ok(ParseNodeLua {
                bundle: this.0.clone(),
                path: Vec::new(),
            })
        });
        methods.add_method("language", |_, this, ()| Ok(this.0.language_name.clone()));
        methods.add_method("parse_duration_ms", |_, this, ()| {
            Ok(i64::try_from(this.0.parse_duration.as_millis()).unwrap_or(i64::MAX))
        });
        methods.add_method("source_len", |_, this, ()| {
            Ok(i64::try_from(this.0.source.len()).unwrap_or(i64::MAX))
        });
        methods.add_method("text", |lua, this, ()| {
            lua.create_string(this.0.source.as_ref())
        });
        methods.add_method("sexp", |_, this, ()| {
            Ok(this.0.root_tree().root_node().to_sexp())
        });
    }
}

impl UserData for ParseNodeLua {
    #[allow(
        clippy::too_many_lines,
        reason = "linear list of read-only Node accessors; splitting into helpers fragments a coherent API surface"
    )]
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("type", |_, this, ()| {
            Ok(this.resolve().map(|n| n.kind().to_owned()))
        });
        methods.add_method("start_byte", |_, this, ()| {
            Ok(this
                .resolve()
                .map(|n| i64::try_from(n.start_byte()).unwrap_or(i64::MAX)))
        });
        methods.add_method("end_byte", |_, this, ()| {
            Ok(this
                .resolve()
                .map(|n| i64::try_from(n.end_byte()).unwrap_or(i64::MAX)))
        });
        methods.add_method("start_position", |lua, this, ()| {
            this.resolve()
                .map(|n| point_to_lua(lua, n.start_position()))
                .transpose()
        });
        methods.add_method("end_position", |lua, this, ()| {
            this.resolve()
                .map(|n| point_to_lua(lua, n.end_position()))
                .transpose()
        });
        methods.add_method("child_count", |_, this, ()| {
            Ok(this.resolve().map(|n| n.child_count()))
        });
        methods.add_method("named_child_count", |_, this, ()| {
            Ok(this.resolve().map(|n| n.named_child_count()))
        });
        methods.add_method("child", |_, this, idx: u32| {
            let Some(node) = this.resolve() else {
                return Ok(None);
            };
            if node.child(idx).is_none() {
                return Ok(None);
            }
            let mut path = this.path.clone();
            path.push(idx);
            Ok(Some(ParseNodeLua {
                bundle: this.bundle.clone(),
                path,
            }))
        });
        methods.add_method("children", |lua, this, ()| {
            let Some(node) = this.resolve() else {
                return Ok(mlua::Value::Nil);
            };
            let count = node.child_count() as u32;
            let t = lua.create_table_with_capacity(count as usize, 0)?;
            for i in 0..count {
                let mut path = this.path.clone();
                path.push(i);
                t.set(
                    i + 1,
                    ParseNodeLua {
                        bundle: this.bundle.clone(),
                        path,
                    },
                )?;
            }
            Ok(mlua::Value::Table(t))
        });
        methods.add_method("named_children", |lua, this, ()| {
            let Some(node) = this.resolve() else {
                return Ok(mlua::Value::Nil);
            };
            // Walk through children, keep only the named ones, but
            // record the *child* index (not the named-only index) so
            // re-resolution from the path works.
            let count = node.child_count() as u32;
            let t = lua.create_table()?;
            let mut out_idx = 0;
            for i in 0..count {
                let Some(child) = node.child(i) else { continue };
                if !child.is_named() {
                    continue;
                }
                let mut path = this.path.clone();
                path.push(i);
                out_idx += 1;
                t.set(
                    out_idx,
                    ParseNodeLua {
                        bundle: this.bundle.clone(),
                        path,
                    },
                )?;
            }
            Ok(mlua::Value::Table(t))
        });
        methods.add_method("parent", |_, this, ()| {
            if this.path.is_empty() {
                return Ok(None);
            }
            let mut path = this.path.clone();
            path.pop();
            Ok(Some(ParseNodeLua {
                bundle: this.bundle.clone(),
                path,
            }))
        });
        methods.add_method("is_named", |_, this, ()| {
            Ok(this.resolve().map(|n| n.is_named()))
        });
        methods.add_method("is_missing", |_, this, ()| {
            Ok(this.resolve().map(|n| n.is_missing()))
        });
        methods.add_method("has_error", |_, this, ()| {
            Ok(this.resolve().map(|n| n.has_error()))
        });
        methods.add_method("text", |lua, this, ()| {
            let Some(node) = this.resolve() else {
                return Ok(None);
            };
            let start = node.start_byte();
            let end = node.end_byte().min(this.bundle.source.len());
            let bytes = &this.bundle.source[start.min(end)..end];
            Ok(Some(lua.create_string(bytes)?))
        });
        methods.add_method("sexp", |_, this, ()| {
            Ok(this.resolve().map(|n| n.to_sexp()))
        });
    }
}

/// Resolve `buf_id` to a [`ParseViewHandle`], creating and attaching
/// a fresh [`ParseView`] if the buffer doesn't have one yet. Errors
/// if the language is unknown or the buffer id is stale.
fn get_or_create_parse_view(
    syntax: &SharedSyntaxRegistry,
    registry: &SharedRegistry,
    buf_id: BufferId,
    lang_name: &str,
) -> mlua::Result<ParseViewHandle> {
    if let Some(handle) = syntax.view(buf_id) {
        // If the language doesn't match, we could re-create. M4.1
        // doesn't support per-buffer language changes; the first
        // language wins. Future M4.x can switch grammars by
        // detaching and reattaching.
        if handle.language_name() == lang_name {
            return Ok(handle);
        }
    }
    let language = syntax
        .language(lang_name)
        .ok_or_else(|| mlua::Error::external(format!("unknown language: {lang_name}")))?;
    let mut reg = registry.borrow_mut();
    let buf = reg
        .get_mut(buf_id)
        .map_err(|_| mlua::Error::external(BindingError::StaleId { id: buf_id }))?;
    let view = ParseView::new(buf, language, lang_name.to_owned());
    let handle = view.handle();
    buf.attach_view(Box::new(view));
    syntax.attach_view(buf_id, handle.clone());
    Ok(handle)
}

/// Install the tree-sitter Lua surface onto `pmacs.parse`. Must run
/// after [`install`] so the `pmacs` table already exists, and after
/// [`install_async`] so a runtime is available for `_dispatch`.
///
/// Languages are *not* registered through Lua --- a
/// [`tree_sitter::Language`] is a Rust-side value. The host (or
/// tests) populate `syntax` before invoking this function. T M4.1
/// ships only the integration plumbing; T M4.2 wires the actual
/// `tree-sitter-rust` and `tree-sitter-lua` registrations.
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw Lua bindings; splitting fragments a coherent surface"
)]
pub fn install_parse(
    lua: &Lua,
    runtime: &SharedAsyncRuntime,
    syntax: &SharedSyntaxRegistry,
    registry: &SharedRegistry,
) -> mlua::Result<()> {
    lua.set_app_data(syntax.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let parse_mod = lua.create_table()?;

    {
        let s = syntax.clone();
        parse_mod.set(
            "_has_language",
            lua.create_function(move |_, name: String| Ok(s.has_language(&name)))?,
        )?;
    }

    {
        let s = syntax.clone();
        parse_mod.set(
            "language_for_path",
            lua.create_function(move |_, path: String| Ok(s.language_name_for_path(&path)))?,
        )?;
    }

    {
        let s = syntax.clone();
        parse_mod.set(
            "_register_extension",
            lua.create_function(move |_, (ext, name): (String, String)| {
                if !s.has_language(&name) {
                    return Err(mlua::Error::external(format!(
                        "register_extension: unknown language {name}"
                    )));
                }
                s.register_extension(ext, name);
                Ok(())
            })?,
        )?;
    }

    // Injection alias write-through (framing Q#IJ4). `syntax.lua` wraps
    // this in a `pmacs.parse.injection_aliases` proxy table so users write
    // `pmacs.parse.injection_aliases.mylang = "rust"`. The registry holds
    // the merged map (defaults + overrides); each dispatch snapshots it.
    {
        let s = syntax.clone();
        parse_mod.set(
            "_register_injection_alias",
            lua.create_function(move |_, (alias, lang): (String, String)| {
                s.register_injection_alias(alias, lang);
                Ok(())
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        parse_mod.set(
            "_has_view",
            lua.create_function(move |_, id: BufferIdLua| Ok(s.view(id.0).is_some()))?,
        )?;
    }

    {
        let s = syntax.clone();
        parse_mod.set(
            "_pending_edits",
            lua.create_function(move |_, id: BufferIdLua| {
                Ok(s.view(id.0).map(|h| h.pending_edit_count()))
            })?,
        )?;
    }

    // True if the buffer's installed bundle hit the injection layer backstop
    // (framing Q#IJ3). `syntax.lua` reads this after settle to surface the
    // cap once via `pmacs.error` rather than dropping regions silently.
    {
        let s = syntax.clone();
        parse_mod.set(
            "_injection_capped",
            lua.create_function(move |_, id: BufferIdLua| {
                Ok(s.view(id.0)
                    .and_then(|h| h.current())
                    .is_some_and(|b| b.injection_capped))
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        parse_mod.set(
            "tree",
            lua.create_function(move |_, id: BufferIdLua| {
                Ok(s.view(id.0).and_then(|h| h.current()).map(ParseTreeLua))
            })?,
        )?;
    }

    // Synchronous parse: convenience for tests and one-off scripts.
    // Builds the request, runs the parser inline on the main thread,
    // installs the bundle. Returns the freshly-installed
    // `ParseTreeLua`.
    {
        let s = syntax.clone();
        let reg = registry.clone();
        parse_mod.set(
            "_parse_now",
            lua.create_function(move |_, (id, lang): (BufferIdLua, String)| {
                let handle = get_or_create_parse_view(&s, &reg, id.0, &lang)?;
                let mut req = handle.make_request();
                // Snapshot the alias map on the sync path too (framing Q#IJ4)
                // — otherwise a `py` fence or a Lua-added alias would resolve
                // asynchronously but not through `_parse_now`.
                req.injection_aliases = s.injection_alias_snapshot();
                let bundle = syntax::run_parse(req).map_err(mlua::Error::external)?;
                // Resolve each layer's highlight query from the registry
                // cache before install so producers can style every layer
                // (framing Q#IJ2 stage 2).
                let arc = s.resolve_layer_queries(&bundle);
                handle.install(arc.clone());
                Ok(ParseTreeLua(arc))
            })?,
        )?;
    }

    // Async dispatch: builds the request, hands it to the runtime,
    // records the (job_id, buffer_id) mapping for the install path,
    // and returns the runtime job id.
    {
        let s = syntax.clone();
        let reg = registry.clone();
        let rt = runtime.clone();
        parse_mod.set(
            "_dispatch",
            lua.create_function(move |_, (id, lang): (BufferIdLua, String)| {
                let handle = get_or_create_parse_view(&s, &reg, id.0, &lang)?;
                let mut req = handle.make_request();
                // Snapshot the alias map into the request so the worker can
                // resolve dynamic fence names off the main thread (Q#IJ4).
                req.injection_aliases = s.injection_alias_snapshot();
                let job_id = rt.dispatch_parse(req, None);
                s.record_parse_job(job_id, id.0);
                Ok(job_id)
            })?,
        )?;
    }

    // Settle path: drains the parse-handoff bundle for `job_id`,
    // installs it into the view, and *also* drains the pending
    // entry from the runtime --- a fire-and-forget parse never
    // calls `take_result` itself, so without this step the pending
    // table would leak. Returns `true` if a bundle was installed,
    // `false` if the job is unknown, was cancelled or failed, or
    // has already been installed (idempotent).
    {
        let s = syntax.clone();
        let rt = runtime.clone();
        parse_mod.set(
            "_install_settled",
            lua.create_function(move |_, job_id: u64| {
                let buf_id = s.take_parse_job(job_id);
                let bundle = rt.take_parse_tree(job_id);
                // Drain the pending entry whether or not we have a
                // bundle. `take_result` returns None for an unknown
                // or still-running id; that's a benign no-op.
                let _ = rt.take_result(job_id);
                let (Some(buf_id), Some(bundle)) = (buf_id, bundle) else {
                    return Ok(false);
                };
                if let Some(handle) = s.view(buf_id) {
                    // Stage 2 (framing Q#IJ2): resolve each layer's highlight
                    // query on the main thread before install.
                    let resolved = s.resolve_layer_queries(&bundle);
                    handle.install(resolved);
                    Ok(true)
                } else {
                    Ok(false)
                }
            })?,
        )?;
    }

    // T M4.3 highlight overlay attach. Looks up the parse view +
    // bundled `highlights.scm` for the language; constructs a
    // `SyntaxHighlightView` over them and pushes it as an overlay on
    // the active window. Each call pushes a fresh overlay — there is
    // no dedup at this layer. Callers that may re-attach (the M4
    // after-load path, the M9.7 prompt-result path) gate against
    // double-push themselves: the M4 path tracks attached buffer ids
    // in `builtin/runtime/syntax.lua`'s `highlighted_buffers`; the
    // M9.7 path issues `pmacs.window.switch_buffer` immediately
    // before, which clears overlays.
    {
        let s = syntax.clone();
        parse_mod.set(
            "_attach_highlight",
            lua.create_function(move |lua, (id, lang): (BufferIdLua, String)| {
                let Some(handle) = s.view(id.0) else {
                    return Err(mlua::Error::external(format!(
                        "no parse view for buffer {:?}",
                        id.0
                    )));
                };
                if s.highlights_query(&lang).is_none() {
                    // Root language ships no highlights --- treat as a benign
                    // no-op so callers don't need to special-case grammars
                    // without highlights bundled. (Injected child layers still
                    // resolve their own queries at settle; a root-highlight-less
                    // injector is out of v1 scope.)
                    return Ok(false);
                }
                let theme = s.theme();
                let core = lua
                    .app_data_ref::<SharedCore>()
                    .ok_or_else(|| mlua::Error::external("editor core not yet installed"))?;
                let mut core_borrow = core.borrow_mut();
                let win = core_borrow.active_window_mut();
                if win.buffer_id != id.0 {
                    return Err(mlua::Error::external(format!(
                        "active window's buffer is not {:?}",
                        id.0
                    )));
                }
                let overlay = SyntaxHighlightView::new(handle, theme);
                win.push_overlay(Box::new(overlay));
                Ok(true)
            })?,
        )?;
    }

    pmacs.set("parse", parse_mod)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// pmacs.theme + pmacs.parse._attach_highlight (T M4.3)
// ---------------------------------------------------------------------------

/// Convert a Lua-side color value into a [`Color`].
///
/// Accepted shapes:
/// * `nil` or the string `"default"` → terminal default.
/// * An integer 0..=255 → 8-bit indexed color.
/// * A 3-element array of integers `{r, g, b}` (each 0..=255) →
///   truecolor RGB.
fn lua_to_color(value: &mlua::Value) -> mlua::Result<Color> {
    match value {
        mlua::Value::Nil => Ok(Color::Default),
        mlua::Value::String(s) => {
            let bytes = s.as_bytes();
            if bytes.as_ref() == b"default" {
                Ok(Color::Default)
            } else {
                Err(mlua::Error::external(format!(
                    "invalid color string: {:?}",
                    String::from_utf8_lossy(bytes.as_ref())
                )))
            }
        }
        mlua::Value::Integer(n) => u8::try_from(*n)
            .map(Color::Indexed)
            .map_err(|_| mlua::Error::external(format!("indexed color out of 0..=255: {n}"))),
        mlua::Value::Table(t) => {
            let r: u8 = t.get(1)?;
            let g: u8 = t.get(2)?;
            let b: u8 = t.get(3)?;
            Ok(Color::Rgb(r, g, b))
        }
        _ => Err(mlua::Error::external(format!(
            "color must be nil/string/integer/{{r,g,b}}, got {}",
            value.type_name()
        ))),
    }
}

fn color_to_lua(lua: &Lua, color: Color) -> mlua::Result<mlua::Value> {
    Ok(match color {
        Color::Default => mlua::Value::String(lua.create_string("default")?),
        Color::Indexed(n) => mlua::Value::Integer(i64::from(n)),
        Color::Rgb(red, green, blue) => {
            let table = lua.create_table_with_capacity(3, 0)?;
            table.set(1, red)?;
            table.set(2, green)?;
            table.set(3, blue)?;
            mlua::Value::Table(table)
        }
    })
}

fn lua_to_underline(value: &mlua::Value) -> mlua::Result<UnderlineStyle> {
    let mlua::Value::String(s) = value else {
        if matches!(value, mlua::Value::Nil) {
            return Ok(UnderlineStyle::None);
        }
        return Err(mlua::Error::external(format!(
            "underline must be string, got {}",
            value.type_name()
        )));
    };
    Ok(match s.as_bytes().as_ref() {
        b"none" => UnderlineStyle::None,
        b"single" => UnderlineStyle::Single,
        b"double" => UnderlineStyle::Double,
        b"curly" => UnderlineStyle::Curly,
        b"dotted" => UnderlineStyle::Dotted,
        b"dashed" => UnderlineStyle::Dashed,
        other => {
            return Err(mlua::Error::external(format!(
                "unknown underline style: {:?}",
                String::from_utf8_lossy(other)
            )));
        }
    })
}

fn underline_to_lua(style: UnderlineStyle) -> &'static str {
    match style {
        UnderlineStyle::None => "none",
        UnderlineStyle::Single => "single",
        UnderlineStyle::Double => "double",
        UnderlineStyle::Curly => "curly",
        UnderlineStyle::Dotted => "dotted",
        UnderlineStyle::Dashed => "dashed",
    }
}

/// Convert a Lua-side style table to a [`Style`]. Missing fields
/// default to the same values as `Style::default()` --- a Lua table
/// with `{ bold = true }` produces a style that is otherwise
/// terminal-default.
///
/// Every lookup PROPAGATES its error (PR #120 round 1 finding 2):
/// `Table::get` runs `__index`, so a raising metatable must fail the
/// enclosing transactional mutation (Q#TH6 all-or-nothing), not
/// silently parse as an all-default style and let sibling entries
/// commit. Color/underline fields validate strictly through their
/// converters; the boolean fields follow Lua truthiness (mlua's
/// `bool` conversion), so `reverse = 1` reads as `true` by design.
fn lua_to_style(t: &Table) -> mlua::Result<Style> {
    let fg: mlua::Value = t.get("fg")?;
    let bg: mlua::Value = t.get("bg")?;
    let underline: mlua::Value = t.get("underline")?;
    let underline_color: mlua::Value = t.get("underline_color")?;
    Ok(Style {
        fg: lua_to_color(&fg)?,
        bg: lua_to_color(&bg)?,
        bold: t.get::<Option<bool>>("bold")?.unwrap_or(false),
        italic: t.get::<Option<bool>>("italic")?.unwrap_or(false),
        underline: lua_to_underline(&underline)?,
        reverse: t.get::<Option<bool>>("reverse")?.unwrap_or(false),
        underline_color: lua_to_color(&underline_color)?,
    })
}

fn style_to_lua(lua: &Lua, style: Style) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 7)?;
    t.set("fg", color_to_lua(lua, style.fg)?)?;
    t.set("bg", color_to_lua(lua, style.bg)?)?;
    t.set("bold", style.bold)?;
    t.set("italic", style.italic)?;
    t.set("underline", underline_to_lua(style.underline))?;
    t.set("reverse", style.reverse)?;
    t.set("underline_color", color_to_lua(lua, style.underline_color)?)?;
    Ok(t)
}

/// Install `pmacs.theme.*`. The module reads and writes the shared
/// [`Theme`] held by the [`crate::syntax::SyntaxRegistry`]; every
/// attached [`SyntaxHighlightView`] sees the change on its next
/// render. T M4.3 acceptance: "theming via Lua-defined color
/// schemes."
/// Themes arc Q#TH6: how [`commit_theme_entries`] applies a parsed
/// entry set to the theme's capture map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ThemeCommit {
    /// `pmacs.theme.set`: the parsed entries become the whole map
    /// (faces wiped with captures, Q#TH10); `default_style` is
    /// untouched.
    Replace,
    /// `pmacs.theme.merge`: insert/overwrite the parsed entries.
    Merge,
}

/// Themes arc Q#TH6: the transactional mutation helper behind
/// `pmacs.theme.set` / `pmacs.theme.merge`. Collects the WHOLE entry
/// stream before touching the theme lock, so a malformed entry
/// anywhere in the input returns its error with the theme untouched
/// and the mutation counters unbumped — the pre-fix `merge` inserted
/// while iterating, letting early entries land before a later one
/// failed. After a successful commit the counters bump from their
/// prior values (never reset — [`crate::highlight::Theme`]'s
/// increment-only invariant): `Replace` touches both namespaces
/// wholesale so it bumps both; `Merge` classifies every committed
/// key through [`crate::highlight::is_face_name`] (bare `ui`
/// included) and bumps `syntax_epoch` iff any non-face key
/// committed, `face_epoch` iff any face key did.
fn commit_theme_entries(
    theme: &crate::highlight::ThemeHandle,
    mode: ThemeCommit,
    entries: impl Iterator<Item = mlua::Result<(String, Style)>>,
) -> mlua::Result<()> {
    let entries: Vec<(String, Style)> = entries.collect::<mlua::Result<_>>()?;
    let mut th = theme.lock().expect("theme mutex poisoned");
    match mode {
        ThemeCommit::Replace => {
            // Replace the FIELD, never the `Theme` value: a fresh
            // Theme's zeroed counters would let consecutive `set`
            // calls share an epoch and stay invisible to every gate.
            th.by_capture = entries.into_iter().collect();
            th.syntax_epoch += 1;
            th.face_epoch += 1;
        }
        ThemeCommit::Merge => {
            let any_face = entries
                .iter()
                .any(|(n, _)| crate::highlight::is_face_name(n));
            let any_syntax = entries
                .iter()
                .any(|(n, _)| !crate::highlight::is_face_name(n));
            for (name, style) in entries {
                th.by_capture.insert(name, style);
            }
            if any_syntax {
                th.syntax_epoch += 1;
            }
            if any_face {
                th.face_epoch += 1;
            }
        }
    }
    Ok(())
}

/// Adapt a Lua theme table to [`commit_theme_entries`]'s ordered
/// result stream: each raw `(name, style_table)` pair maps through
/// [`lua_to_style`], and any iteration or conversion error rides the
/// stream so the helper can fail before locking.
fn lua_theme_entries(table: &Table) -> impl Iterator<Item = mlua::Result<(String, Style)>> + use<> {
    table
        .pairs::<String, Table>()
        .map(|pair| {
            let (name, style) = pair?;
            Ok((name, lua_to_style(&style)?))
        })
        .collect::<Vec<_>>()
        .into_iter()
}

fn install_theme(lua: &Lua, syntax: &SharedSyntaxRegistry) -> mlua::Result<Table> {
    let theme_mod = lua.create_table()?;

    {
        let s = syntax.clone();
        theme_mod.set(
            "set",
            lua.create_function(move |_, table: Table| {
                commit_theme_entries(&s.theme(), ThemeCommit::Replace, lua_theme_entries(&table))
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "merge",
            lua.create_function(move |_, table: Table| {
                commit_theme_entries(&s.theme(), ThemeCommit::Merge, lua_theme_entries(&table))
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "get",
            lua.create_function(move |lua, name: String| {
                let theme = s.theme();
                let th = theme.lock().expect("theme mutex poisoned");
                style_to_lua(lua, th.lookup(&name))
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "clear",
            lua.create_function(move |_, ()| {
                let theme = s.theme();
                let mut th = theme.lock().expect("theme mutex poisoned");
                th.clear();
                // Q#TH6: clear empties both namespaces — bump both.
                th.syntax_epoch += 1;
                th.face_epoch += 1;
                Ok(())
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "default",
            lua.create_function(move |_, style: Table| {
                // Q#TH6: parse before locking; default_style is a
                // syntax-namespace fallback, so bump syntax only.
                let parsed = lua_to_style(&style)?;
                let theme = s.theme();
                let mut th = theme.lock().expect("theme mutex poisoned");
                th.default_style = parsed;
                th.syntax_epoch += 1;
                Ok(())
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "current",
            lua.create_function(move |lua, ()| {
                let theme = s.theme();
                let th = theme.lock().expect("theme mutex poisoned");
                let out = lua.create_table_with_capacity(0, th.by_capture.len())?;
                for (name, style) in &th.by_capture {
                    out.set(name.as_str(), style_to_lua(lua, *style)?)?;
                }
                let dt = lua.create_table_with_capacity(0, 1)?;
                dt.set("default", style_to_lua(lua, th.default_style)?)?;
                out.set("_meta", dt)?;
                Ok(out)
            })?,
        )?;
    }

    Ok(theme_mod)
}

/// Build a fresh [`crate::syntax::SyntaxRegistry`] and install
/// `pmacs.parse` over it. Returns the [`SharedSyntaxRegistry`] so
/// the host (or tests) can register languages.
pub fn make_syntax_registry(
    lua: &Lua,
    runtime: &SharedAsyncRuntime,
    registry: &SharedRegistry,
) -> mlua::Result<SharedSyntaxRegistry> {
    let syntax = Rc::new(crate::syntax::SyntaxRegistry::new());
    install_parse(lua, runtime, &syntax, registry)?;
    let theme_mod = install_theme(lua, &syntax)?;
    let pmacs: Table = lua.globals().get("pmacs")?;
    pmacs.set("theme", theme_mod)?;
    Ok(syntax)
}

// ---------------------------------------------------------------------------
// pmacs.process: process supervisor surface (T M4.4)
// ---------------------------------------------------------------------------

use crate::process::{
    ProcessEvent, ProcessEventKind, ProcessId, ProcessMode, ProcessSpec, ProcessState,
    ProcessSupervisor, RestartPolicy, Termination,
};

/// Shared, single-threaded handle to the editor's process
/// supervisor. Same `Rc<RefCell<...>>` rationale as the other
/// shared registries: main-thread-only, interior-mutable for
/// closure capture.
pub type SharedProcessSupervisor = Rc<RefCell<ProcessSupervisor>>;

/// Lua-facing wrapper around [`ProcessId`]. Same Copy/userdata
/// pattern as [`BufferIdLua`].
#[derive(Copy, Clone)]
pub struct ProcessIdLua(pub ProcessId);

impl ProcessIdLua {
    /// The wrapped [`ProcessId`].
    #[must_use]
    pub fn id(self) -> ProcessId {
        self.0
    }
}

impl FromLua for ProcessIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "ProcessIdLua".to_string(),
                message: Some("expected a process handle".to_string()),
            }),
        }
    }
}

impl UserData for ProcessIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}", this.0))
        });
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: ProcessIdLua| {
            Ok(this.0 == other.0)
        });
        methods.add_method("raw", |_, this, ()| Ok(this.0.raw()));
    }
}

fn parse_signal(name: &str) -> mlua::Result<nix::sys::signal::Signal> {
    use nix::sys::signal::Signal;
    Ok(match name {
        "SIGTERM" | "TERM" | "term" => Signal::SIGTERM,
        "SIGKILL" | "KILL" | "kill" => Signal::SIGKILL,
        "SIGINT" | "INT" | "int" => Signal::SIGINT,
        "SIGHUP" | "HUP" | "hup" => Signal::SIGHUP,
        "SIGUSR1" | "USR1" | "usr1" => Signal::SIGUSR1,
        "SIGUSR2" | "USR2" | "usr2" => Signal::SIGUSR2,
        "SIGQUIT" | "QUIT" | "quit" => Signal::SIGQUIT,
        other => {
            return Err(mlua::Error::external(format!(
                "unknown signal name: {other:?}"
            )));
        }
    })
}

fn parse_restart(name: &str) -> mlua::Result<RestartPolicy> {
    Ok(match name {
        "never" | "Never" => RestartPolicy::Never,
        "on_crash" | "OnCrash" | "on-crash" => RestartPolicy::OnCrash,
        "always" | "Always" => RestartPolicy::Always,
        other => {
            return Err(mlua::Error::external(format!(
                "unknown restart policy: {other:?} (expected never|on_crash|always)"
            )));
        }
    })
}

fn lua_to_spec(table: &Table) -> mlua::Result<ProcessSpec> {
    let label: String = table.get("label").unwrap_or_else(|_| "unnamed".to_owned());
    let command: String = table.get("command")?;
    let args: Vec<String> = table.get("args").unwrap_or_default();
    let cwd: Option<String> = table.get("cwd").ok().flatten();
    let env_table: Option<Table> = table.get("env").ok().flatten();
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(env_table) = env_table {
        env_table.for_each(|k: String, v: String| {
            env.push((k, v));
            Ok(())
        })?;
    }
    let mode = if let Ok(Some(pty)) = table.get::<Option<Table>>("pty") {
        let rows: u16 = pty.get("rows").unwrap_or(24);
        let cols: u16 = pty.get("cols").unwrap_or(80);
        let term_mode = match pty.get::<Option<String>>("mode").ok().flatten().as_deref() {
            None | Some("raw") => crate::process::TerminalMode::Raw,
            Some("canonical") => crate::process::TerminalMode::Canonical,
            Some(other) => {
                return Err(mlua::Error::external(format!(
                    "pty.mode must be \"raw\" or \"canonical\"; got {other:?}"
                )));
            }
        };
        ProcessMode::Pty {
            rows,
            cols,
            mode: term_mode,
        }
    } else {
        ProcessMode::Pipes
    };
    let restart = match table.get::<Option<String>>("restart").ok().flatten() {
        Some(s) => parse_restart(&s)?,
        None => RestartPolicy::Never,
    };
    let ansi_events = table
        .get::<Option<bool>>("ansi")
        .ok()
        .flatten()
        .unwrap_or(false);
    // Compile-mode process shape (Q#CM3). Both options are
    // pipe-mode-only; the supervisor rejects them at spawn under PTY
    // so misconfiguration surfaces as a spawn error, not silence.
    // Type errors are HARD errors, not silent coercions: `stdin =
    // true` would quietly keep a piped stdin (hang), and a mistyped
    // `group` would coerce through Lua truthiness to whichever
    // boolean its truthiness happens to be — either way the caller's
    // intent is unverifiable, and these fields carry process-hygiene
    // guarantees (PR #113 round-1 finding 6; wording corrected in
    // round 2 finding 5). RAW reads (round-3 finding 3): a spec
    // table is plain data — metatable-provided fields are
    // deliberately not honored (the compile.lua rawget posture), and
    // a raising __index must not be silently absorbed into "false"
    // and quietly disable process-group isolation.
    let stdin_raw: Option<String> = table.raw_get("stdin").map_err(|_| {
        mlua::Error::external("stdin must be the string \"piped\" or \"null\"".to_owned())
    })?;
    let stdin = match stdin_raw.as_deref() {
        None | Some("piped") => crate::process::StdinMode::Piped,
        Some("null") => crate::process::StdinMode::Null,
        Some(other) => {
            return Err(mlua::Error::external(format!(
                "stdin must be \"piped\" or \"null\"; got {other:?}"
            )));
        }
    };
    // Read as a raw Value: mlua's `bool` conversion applies Lua
    // truthiness, so `group = "true"` would silently coerce instead
    // of erroring. A raw Value read cannot raise; any residual error
    // is still a hard error, never a silent default.
    let group = match table.raw_get::<mlua::Value>("group") {
        Ok(mlua::Value::Nil) => false,
        Ok(mlua::Value::Boolean(b)) => b,
        Ok(other) => {
            return Err(mlua::Error::external(format!(
                "group must be a boolean; got {}",
                other.type_name()
            )));
        }
        Err(e) => return Err(e),
    };
    Ok(ProcessSpec {
        label,
        command,
        args,
        cwd: cwd.map(std::path::PathBuf::from),
        env,
        mode,
        restart,
        ansi_events,
        ansi_profile: crate::ansi::AnsiParserProfile::LineOriented,
        stdin,
        group,
    })
}

fn state_to_lua(lua: &Lua, state: &ProcessState) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 4)?;
    match state {
        ProcessState::Starting => {
            t.set("kind", "starting")?;
        }
        ProcessState::Running { pid, .. } => {
            t.set("kind", "running")?;
            t.set("pid", *pid)?;
        }
        ProcessState::Exiting { pid, .. } => {
            t.set("kind", "exiting")?;
            t.set("pid", *pid)?;
        }
        ProcessState::Terminated(term) => {
            t.set("kind", "terminated")?;
            match term {
                Termination::Exited { code, .. } => {
                    t.set("outcome", "exited")?;
                    t.set("code", *code)?;
                }
                Termination::Signaled { signal, .. } => {
                    t.set("outcome", "signaled")?;
                    t.set("signal", signal.as_str())?;
                }
                Termination::Crashed { error, .. } => {
                    t.set("outcome", "crashed")?;
                    t.set("error", error.as_str())?;
                }
            }
        }
    }
    Ok(t)
}

fn event_to_lua(lua: &Lua, ev: &ProcessEvent) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 4)?;
    t.set("id", ProcessIdLua(ev.id))?;
    match &ev.kind {
        ProcessEventKind::Started { pid } => {
            t.set("kind", "started")?;
            t.set("pid", *pid)?;
        }
        ProcessEventKind::Stdout(bytes) => {
            t.set("kind", "stdout")?;
            t.set("bytes", lua.create_string(bytes)?)?;
        }
        ProcessEventKind::Stderr(bytes) => {
            t.set("kind", "stderr")?;
            t.set("bytes", lua.create_string(bytes)?)?;
        }
        ProcessEventKind::Ansi(events) => {
            t.set("kind", "ansi")?;
            let out = lua.create_table_with_capacity(events.len(), 0)?;
            for (i, event) in events.iter().enumerate() {
                out.set(i + 1, event_to_lua_table(lua, event)?)?;
            }
            t.set("events", out)?;
        }
        ProcessEventKind::Exited { code } => {
            t.set("kind", "exited")?;
            t.set("code", *code)?;
        }
        ProcessEventKind::Signaled { signal } => {
            t.set("kind", "signaled")?;
            t.set("signal", signal.as_str())?;
        }
        ProcessEventKind::Crashed { error } => {
            t.set("kind", "crashed")?;
            t.set("error", error.as_str())?;
        }
        ProcessEventKind::Restarting { attempt } => {
            t.set("kind", "restarting")?;
            t.set("attempt", *attempt)?;
        }
    }
    Ok(t)
}

/// Install `pmacs.process.*` (T M4.4). The supervisor itself is
/// constructed by [`make_process_supervisor`] (which calls this
/// function); the editor's tick loop then drives
/// [`ProcessSupervisor::tick`] each iteration so reader threads,
/// exit polling, and restart accounting all run on a frame cadence.
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings; splitting fragments a coherent surface"
)]
pub fn install_process(lua: &Lua, supervisor: &SharedProcessSupervisor) -> mlua::Result<()> {
    lua.set_app_data(supervisor.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let proc_mod = lua.create_table()?;

    {
        let s = supervisor.clone();
        proc_mod.set(
            "spawn",
            lua.create_function(move |_, spec: Table| {
                let parsed = lua_to_spec(&spec)?;
                let id = s
                    .borrow_mut()
                    .spawn(parsed)
                    .map_err(mlua::Error::external)?;
                Ok(ProcessIdLua(id))
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "signal",
            lua.create_function(move |_, (id, sig): (ProcessIdLua, String)| {
                let signal = parse_signal(&sig)?;
                s.borrow_mut()
                    .signal(id.0, signal)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "terminate",
            lua.create_function(move |_, id: ProcessIdLua| {
                s.borrow_mut()
                    .terminate(id.0)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "write_stdin",
            lua.create_function(move |_, (id, bytes): (ProcessIdLua, mlua::String)| {
                let payload = bytes.as_bytes();
                s.borrow_mut()
                    .write_stdin(id.0, &payload)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "resize_pty",
            lua.create_function(move |_, (id, rows, cols): (ProcessIdLua, u16, u16)| {
                s.borrow_mut()
                    .resize_pty(id.0, rows, cols)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "status",
            lua.create_function(move |lua, id: ProcessIdLua| {
                let sup = s.borrow();
                match sup.state(id.0) {
                    Some(state) => {
                        let t = state_to_lua(lua, state)?;
                        Ok(mlua::Value::Table(t))
                    }
                    None => Ok(mlua::Value::Nil),
                }
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "events_take",
            lua.create_function(move |lua, id: ProcessIdLua| {
                let evs = s.borrow_mut().take_events(id.0);
                let out = lua.create_table_with_capacity(evs.len(), 0)?;
                for (i, ev) in evs.iter().enumerate() {
                    out.set(i + 1, event_to_lua(lua, ev)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "list",
            lua.create_function(move |lua, ()| {
                let sup = s.borrow();
                let ids: Vec<ProcessId> = sup
                    .ids()
                    .filter(|id| {
                        sup.spec(*id).is_none_or(|spec| {
                            spec.ansi_profile == crate::ansi::AnsiParserProfile::LineOriented
                        })
                    })
                    .collect();
                let out = lua.create_table_with_capacity(ids.len(), 0)?;
                for (i, id) in ids.iter().enumerate() {
                    let row = lua.create_table_with_capacity(0, 3)?;
                    row.set("id", ProcessIdLua(*id))?;
                    if let Some(spec) = sup.spec(*id) {
                        row.set("label", spec.label.as_str())?;
                        row.set("command", spec.command.as_str())?;
                    }
                    if let Some(state) = sup.state(*id) {
                        row.set("state", state_to_lua(lua, state)?)?;
                    }
                    out.set(i + 1, row)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "forget",
            lua.create_function(move |_, id: ProcessIdLua| {
                s.borrow_mut().forget(id.0).map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let s = supervisor.clone();
        proc_mod.set(
            "_tick",
            lua.create_function(move |_, ()| {
                s.borrow_mut().tick();
                Ok(())
            })?,
        )?;
    }

    pmacs.set("process", proc_mod)?;
    Ok(())
}

/// `pmacs.gpu.*` — GPU frontend preferences (Arc 4 stage 2, framing
/// Q#F2). Installs the module and returns the shared preference
/// handle the `semantic_render` producer reads. Called from
/// `EditorState::new` BEFORE `load_user_config` runs: font selection
/// is primarily configuration, and an init.lua `set_font` must land
/// in the same state the first attachment's producer reads.
///
/// `set_font` follows the live `pmacs.theme.set` pattern — no
/// `require_init_phase` gate; mid-session calls re-ship on the next
/// frame. The kwargs table is STRICT PLAIN DATA: `raw_get` reads,
/// unknown raw keys are rejected by name, and metatables are never
/// consulted (`for_each` iterates raw pairs) — a hostile `__index`
/// cannot inject values, and the whole table is parsed, validated,
/// and quantized before the lock is taken (all-or-nothing, Q#TH6).
pub fn make_font_pref(lua: &Lua) -> mlua::Result<crate::font_pref::FontPrefHandle> {
    let handle = crate::font_pref::new_handle();
    let gpu = lua.create_table()?;
    {
        let h = handle.clone();
        gpu.set(
            "set_font",
            lua.create_function(move |_, spec: Table| -> mlua::Result<()> {
                // Reject unknown keys first, naming the offender —
                // raw iteration, so metatable trickery is invisible.
                let mut unknown: Option<String> = None;
                spec.clone().for_each(|k: Value, _: Value| {
                    let name = match &k {
                        Value::String(s) => s.to_str()?.to_owned(),
                        other => format!("{other:?}"),
                    };
                    if name != "family" && name != "size" && unknown.is_none() {
                        unknown = Some(name);
                    }
                    Ok(())
                })?;
                if let Some(key) = unknown {
                    return Err(mlua::Error::external(format!(
                        "pmacs.gpu.set_font: unknown field `{key}` (expected `family` and/or `size`)"
                    )));
                }
                // Parse + validate the complete table BEFORE locking.
                let family = match spec.raw_get::<Value>("family")? {
                    Value::Nil => None,
                    Value::String(s) => {
                        let f = s.to_str()?.to_owned();
                        if f.is_empty() {
                            return Err(mlua::Error::external(
                                "pmacs.gpu.set_font: `family` must be a non-empty string",
                            ));
                        }
                        Some(f)
                    }
                    other => {
                        return Err(mlua::Error::external(format!(
                            "pmacs.gpu.set_font: `family` must be a string, got {}",
                            other.type_name()
                        )));
                    }
                };
                let size_centi_px = match spec.raw_get::<Value>("size")? {
                    Value::Nil => None,
                    Value::Integer(i) => Some(validate_font_size(i as f64)?),
                    Value::Number(n) => Some(validate_font_size(n)?),
                    other => {
                        return Err(mlua::Error::external(format!(
                            "pmacs.gpu.set_font: `size` must be a number, got {}",
                            other.type_name()
                        )));
                    }
                };
                let mut pref = h.lock().expect("font pref mutex poisoned");
                pref.family = family;
                pref.size_centi_px = size_centi_px;
                pref.epoch += 1;
                Ok(())
            })?,
        )?;
    }
    {
        let h = handle.clone();
        gpu.set(
            "font",
            lua.create_function(move |lua, ()| -> mlua::Result<Table> {
                // A FRESH plain table each call — a getter, never the
                // stored table or a mutable handle (Q#F2).
                let t = lua.create_table()?;
                let pref = h.lock().expect("font pref mutex poisoned");
                if let Some(f) = &pref.family {
                    t.set("family", f.clone())?;
                }
                if let Some(c) = pref.size_centi_px {
                    t.set("size", f64::from(c) / 100.0)?;
                }
                Ok(t)
            })?,
        )?;
    }
    let pmacs: Table = lua.globals().get("pmacs")?;
    pmacs.set("gpu", gpu)?;
    Ok(handle)
}

/// Range-check the ORIGINAL value first — `5.999` must error, not
/// round into range — then quantize to the nearest hundredth of a
/// logical pixel (framing Q#F2, round 2 finding 5).
fn validate_font_size(size: f64) -> mlua::Result<u32> {
    if !size.is_finite() || !(6.0..=72.0).contains(&size) {
        return Err(mlua::Error::external(format!(
            "pmacs.gpu.set_font: `size` must be a finite number in [6.0, 72.0] logical px, got {size}"
        )));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((size * 100.0).round() as u32)
}

/// Build a fresh [`ProcessSupervisor`] and install
/// `pmacs.process.*` over it. Mirrors [`make_async_runtime`] /
/// [`make_syntax_registry`] in shape.
pub fn make_process_supervisor(lua: &Lua) -> mlua::Result<SharedProcessSupervisor> {
    let supervisor = Rc::new(RefCell::new(ProcessSupervisor::new()));
    install_process(lua, &supervisor)?;
    Ok(supervisor)
}

// ---------------------------------------------------------------------------
// pmacs.terminal: owned terminal session surface (Arc 5 Stage 2)
// ---------------------------------------------------------------------------

/// Build the shared terminal registry and install strict raw Lua primitives.
pub fn make_terminal_manager(
    lua: &Lua,
    supervisor: &SharedProcessSupervisor,
) -> mlua::Result<crate::terminal::SharedTerminalManager> {
    let manager = Rc::new(RefCell::new(crate::terminal::TerminalManager::new()));
    install_terminal(lua, &manager, supervisor)?;
    Ok(manager)
}

fn terminal_shared_core(lua: &Lua, operation: &str) -> mlua::Result<SharedCore> {
    lua.app_data_ref::<SharedCore>()
        .map(|core| core.clone())
        .ok_or_else(|| {
            mlua::Error::external(format!(
                "pmacs.terminal.{operation}: editor core unavailable"
            ))
        })
}

fn terminal_command_frontend(lua: &Lua, core: &SharedCore) -> crate::protocol::FrontendId {
    lua.app_data_ref::<InteractiveCommandOrigin>()
        .and_then(|origin| origin.current())
        .unwrap_or_else(|| core.borrow().active_frontend)
}

fn active_terminal_view_key(
    lua: &Lua,
    core: &SharedCore,
    manager: &crate::terminal::SharedTerminalManager,
    operation: &str,
) -> mlua::Result<crate::terminal::TerminalViewKey> {
    let frontend_id = lua
        .app_data_ref::<InteractiveCommandOrigin>()
        .and_then(|origin| origin.current())
        .ok_or_else(|| {
            mlua::Error::external(format!(
                "pmacs.terminal.{operation}: requires an interactive frontend context"
            ))
        })?;
    let core = core.borrow();
    let window = core.active_window_for(frontend_id).ok_or_else(|| {
        mlua::Error::external(format!(
            "pmacs.terminal.{operation}: invoking frontend has no active window"
        ))
    })?;
    if !manager.borrow().is_terminal(window.buffer_id) {
        return Err(mlua::Error::external(format!(
            "pmacs.terminal.{operation}: invoking frontend's active window is not a terminal"
        )));
    }
    Ok(crate::terminal::TerminalViewKey::new(
        frontend_id,
        core.views
            .get(&frontend_id)
            .expect("active window implies registered frontend view")
            .active,
        window.buffer_id,
    ))
}

fn terminal_context_integer(context: &Table, field: &str) -> mlua::Result<u64> {
    match context.raw_get::<Value>(field)? {
        Value::Integer(value) => u64::try_from(value).map_err(|_| {
            mlua::Error::external(format!(
                "pmacs.terminal.view_state: `{field}` must be nonnegative"
            ))
        }),
        Value::Nil => Err(mlua::Error::external(format!(
            "pmacs.terminal.view_state: missing field `{field}`"
        ))),
        other => Err(mlua::Error::external(format!(
            "pmacs.terminal.view_state: `{field}` must be an integer, got {}",
            other.type_name()
        ))),
    }
}

fn terminal_context_buffer(context: &Table) -> mlua::Result<crate::buffer::BufferId> {
    match context.raw_get::<Value>("buffer")? {
        Value::UserData(buffer) => buffer
            .borrow::<BufferIdLua>()
            .map(|buffer| buffer.0)
            .map_err(|_| {
                mlua::Error::external("pmacs.terminal.view_state: `buffer` must be a buffer id")
            }),
        Value::Nil => Err(mlua::Error::external(
            "pmacs.terminal.view_state: missing field `buffer`",
        )),
        other => Err(mlua::Error::external(format!(
            "pmacs.terminal.view_state: `buffer` must be a buffer id, got {}",
            other.type_name()
        ))),
    }
}

fn terminal_view_key_from_context(
    core: &SharedCore,
    context: &Table,
) -> mlua::Result<Option<crate::terminal::TerminalViewKey>> {
    const FIELDS: &[&str] = &["frontend", "window", "buffer", "active"];
    let mut unknown = None;
    context.clone().for_each(|key: Value, _: Value| {
        if unknown.is_none() {
            match key {
                Value::String(value) => {
                    let value = value.to_str()?;
                    if !FIELDS.contains(&value.as_ref()) {
                        unknown = Some(value.to_owned());
                    }
                }
                other => unknown = Some(format!("{other:?}")),
            }
        }
        Ok(())
    })?;
    if let Some(field) = unknown {
        return Err(mlua::Error::external(format!(
            "pmacs.terminal.view_state: unknown field `{field}`"
        )));
    }
    let frontend_id = crate::protocol::FrontendId(terminal_context_integer(context, "frontend")?);
    let window_raw = terminal_context_integer(context, "window")?;
    let buffer_id = terminal_context_buffer(context)?;

    let core = core.borrow();
    let Some(view) = core.views.get(&frontend_id) else {
        return Ok(None);
    };
    let Some(window_id) = view
        .layout
        .iter_ids()
        .into_iter()
        .find(|window_id| window_id.raw() == window_raw)
    else {
        return Ok(None);
    };
    if core
        .windows
        .get(&window_id)
        .is_none_or(|window| window.buffer_id != buffer_id)
    {
        return Ok(None);
    }
    Ok(Some(crate::terminal::TerminalViewKey::new(
        frontend_id,
        window_id,
        buffer_id,
    )))
}

#[allow(
    clippy::too_many_lines,
    reason = "single strict Lua module installation"
)]
fn install_terminal(
    lua: &Lua,
    manager: &crate::terminal::SharedTerminalManager,
    supervisor: &SharedProcessSupervisor,
) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let terminal = lua.create_table()?;

    {
        let manager = manager.clone();
        let supervisor = supervisor.clone();
        terminal.set(
            "_open",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<BufferIdLua> {
                let spec = parse_terminal_spec(&spec)?;
                let core = lua
                    .app_data_ref::<SharedCore>()
                    .map(|core| core.clone())
                    .ok_or_else(|| {
                        mlua::Error::external("pmacs.terminal.open: editor core unavailable")
                    })?;
                let frontend_id = terminal_command_frontend(lua, &core);
                if core.borrow().active_window_for(frontend_id).is_none() {
                    return Err(mlua::Error::external(
                        "pmacs.terminal.open: target frontend has no active window",
                    ));
                }
                let buffer_id = {
                    let mut manager = manager.borrow_mut();
                    manager
                        .open(spec, &mut core.borrow_mut(), &mut supervisor.borrow_mut())
                        .map_err(mlua::Error::external)?
                };
                let key = {
                    let mut core = core.borrow_mut();
                    if let Err(error) = core.switch_active_buffer_for(frontend_id, buffer_id) {
                        let _ = core.registry.borrow_mut().remove(buffer_id);
                        manager
                            .borrow_mut()
                            .prune(&mut core, &mut supervisor.borrow_mut());
                        return Err(mlua::Error::external(format!(
                            "pmacs.terminal.open: active-window switch failed: {error}"
                        )));
                    }
                    crate::terminal::TerminalViewKey::new(
                        frontend_id,
                        core.views
                            .get(&frontend_id)
                            .expect("checked frontend has active view")
                            .active,
                        buffer_id,
                    )
                };
                let claimed = {
                    let mut manager = manager.borrow_mut();
                    manager.register_view(key) && manager.claim_controller(key)
                };
                if !claimed {
                    let mut core = core.borrow_mut();
                    let _ = core.registry.borrow_mut().remove(buffer_id);
                    manager
                        .borrow_mut()
                        .prune(&mut core, &mut supervisor.borrow_mut());
                    return Err(mlua::Error::external(
                        "pmacs.terminal.open: failed to claim the new terminal view",
                    ));
                }
                run_hook_if_defined(lua, "buffer.after-switch", mlua::MultiValue::new());
                Ok(BufferIdLua(buffer_id))
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "is_terminal",
            lua.create_function(move |_, buffer: BufferIdLua| {
                Ok(manager.borrow().is_terminal(buffer.0))
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "state",
            lua.create_function(move |lua, buffer: BufferIdLua| {
                let snapshot = manager.borrow().snapshot(buffer.0).ok_or_else(|| {
                    mlua::Error::external(format!(
                        "pmacs.terminal.state: buffer {:?} is not a terminal",
                        buffer.0
                    ))
                })?;
                terminal_state_table(lua, snapshot)
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        let supervisor = supervisor.clone();
        terminal.set(
            "send",
            lua.create_function(move |_, (buffer, bytes): (BufferIdLua, mlua::String)| {
                manager
                    .borrow()
                    .send(
                        buffer.0,
                        bytes.as_bytes().as_ref(),
                        &mut supervisor.borrow_mut(),
                    )
                    .map_err(mlua::Error::external)
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        let supervisor = supervisor.clone();
        terminal.set(
            "terminate",
            lua.create_function(move |_, buffer: BufferIdLua| {
                manager
                    .borrow_mut()
                    .terminate(buffer.0, &mut supervisor.borrow_mut())
                    .map_err(mlua::Error::external)
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "view_state",
            lua.create_function(move |lua, context: Table| -> mlua::Result<Option<Table>> {
                let core = terminal_shared_core(lua, "view_state")?;
                let Some(key) = terminal_view_key_from_context(&core, &context)? else {
                    return Ok(None);
                };
                let Some(status) = manager.borrow_mut().view_status(key) else {
                    return Ok(None);
                };
                let table = lua.create_table()?;
                table.set("at_bottom", status.at_bottom)?;
                table.set("scroll_offset", status.scroll_offset)?;
                table.set("selection", status.selection)?;
                Ok(Some(table))
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "scroll",
            lua.create_function(move |lua, lines: i64| {
                let lines = i32::try_from(lines).unwrap_or_else(|_| {
                    if lines.is_negative() {
                        i32::MIN
                    } else {
                        i32::MAX
                    }
                });
                let core = terminal_shared_core(lua, "scroll")?;
                let key = active_terminal_view_key(lua, &core, &manager, "scroll")?;
                Ok(manager.borrow_mut().scroll_lines(key, lines))
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "_scroll_page",
            lua.create_function(move |lua, direction: i64| {
                let direction = i32::try_from(direction).map_err(|_| {
                    mlua::Error::external(
                        "pmacs.terminal._scroll_page: `direction` exceeds i32 range",
                    )
                })?;
                let core = terminal_shared_core(lua, "_scroll_page")?;
                let key = active_terminal_view_key(lua, &core, &manager, "_scroll_page")?;
                Ok(manager.borrow_mut().scroll_page(key, direction))
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "scroll_to_bottom",
            lua.create_function(move |lua, ()| {
                let core = terminal_shared_core(lua, "scroll_to_bottom")?;
                let key = active_terminal_view_key(lua, &core, &manager, "scroll_to_bottom")?;
                Ok(manager.borrow_mut().scroll_to_bottom(key))
            })?,
        )?;
    }

    {
        let manager = manager.clone();
        terminal.set(
            "copy_selection",
            lua.create_function(move |lua, ()| {
                let core = terminal_shared_core(lua, "copy_selection")?;
                let key = active_terminal_view_key(lua, &core, &manager, "copy_selection")?;
                let Some(bytes) = manager.borrow_mut().copy_selection(key) else {
                    return Ok(false);
                };
                core.borrow_mut().clipboard_set_for(key.frontend_id, bytes);
                Ok(true)
            })?,
        )?;
    }

    pmacs.set("terminal", terminal)
}

fn parse_terminal_spec(table: &Table) -> mlua::Result<crate::terminal::TerminalSpec> {
    const FIELDS: &[&str] = &[
        "command",
        "args",
        "cwd",
        "env",
        "name",
        "rows",
        "cols",
        "scrollback_rows",
    ];
    let mut unknown = None;
    table.clone().for_each(|key: Value, _: Value| {
        let key = match key {
            Value::String(key) => key.to_str()?.to_owned(),
            other => {
                unknown = Some(format!("<{} key>", other.type_name()));
                return Ok(());
            }
        };
        if !FIELDS.contains(&key.as_str()) {
            unknown = Some(key);
        }
        Ok(())
    })?;
    if let Some(field) = unknown {
        return Err(mlua::Error::external(format!(
            "pmacs.terminal.open: unknown field `{field}`"
        )));
    }

    let command = strict_terminal_string(table.raw_get("command")?, "command", false)?
        .ok_or_else(|| mlua::Error::external("pmacs.terminal.open: missing field `command`"))?;
    let args = strict_terminal_args(table.raw_get("args")?)?;
    let cwd =
        strict_terminal_string(table.raw_get("cwd")?, "cwd", true)?.map(std::path::PathBuf::from);
    let env = strict_terminal_env(table.raw_get("env")?)?;
    let name = strict_terminal_string(table.raw_get("name")?, "name", true)?;
    let rows = strict_terminal_u16(table.raw_get("rows")?, "rows", 24)?;
    let cols = strict_terminal_u16(table.raw_get("cols")?, "cols", 80)?;
    let scrollback_rows = strict_terminal_usize(
        table.raw_get("scrollback_rows")?,
        "scrollback_rows",
        crate::terminal::DEFAULT_TERMINAL_SCROLLBACK_ROWS,
    )?;

    Ok(crate::terminal::TerminalSpec {
        command,
        args,
        cwd,
        env,
        name,
        rows,
        cols,
        scrollback_rows,
    })
}

fn strict_terminal_string(
    value: Value,
    field: &'static str,
    optional: bool,
) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil if optional => Ok(None),
        Value::String(value) => Ok(Some(value.to_str()?.to_owned())),
        Value::Nil => Err(mlua::Error::external(format!(
            "pmacs.terminal.open: missing field `{field}`"
        ))),
        other => Err(mlua::Error::external(format!(
            "pmacs.terminal.open: `{field}` must be a string, got {}",
            other.type_name()
        ))),
    }
}

fn strict_terminal_args(value: Value) -> mlua::Result<Vec<String>> {
    let Value::Table(table) = value else {
        return match value {
            Value::Nil => Ok(Vec::new()),
            other => Err(mlua::Error::external(format!(
                "pmacs.terminal.open: `args` must be a dense string array, got {}",
                other.type_name()
            ))),
        };
    };
    let mut entries = std::collections::BTreeMap::new();
    table.for_each(|key: Value, value: Value| {
        let Value::Integer(index) = key else {
            return Err(mlua::Error::external(
                "pmacs.terminal.open: `args` keys must be positive integers",
            ));
        };
        let index = usize::try_from(index).map_err(|_| {
            mlua::Error::external("pmacs.terminal.open: `args` keys must be positive integers")
        })?;
        if index == 0 {
            return Err(mlua::Error::external(
                "pmacs.terminal.open: `args` keys must be positive integers",
            ));
        }
        let Value::String(value) = value else {
            return Err(mlua::Error::external(format!(
                "pmacs.terminal.open: `args[{index}]` must be a string"
            )));
        };
        entries.insert(index, value.to_str()?.to_owned());
        Ok(())
    })?;
    let mut args = Vec::with_capacity(entries.len());
    for expected in 1..=entries.len() {
        let value = entries.remove(&expected).ok_or_else(|| {
            mlua::Error::external(format!(
                "pmacs.terminal.open: `args` has a hole at index {expected}"
            ))
        })?;
        args.push(value);
    }
    if let Some((&index, _)) = entries.first_key_value() {
        return Err(mlua::Error::external(format!(
            "pmacs.terminal.open: `args` has a hole before index {index}"
        )));
    }
    Ok(args)
}

fn strict_terminal_env(value: Value) -> mlua::Result<Vec<(String, String)>> {
    let Value::Table(table) = value else {
        return match value {
            Value::Nil => Ok(Vec::new()),
            other => Err(mlua::Error::external(format!(
                "pmacs.terminal.open: `env` must be a string-to-string table, got {}",
                other.type_name()
            ))),
        };
    };
    let mut env = Vec::new();
    table.for_each(|key: Value, value: Value| {
        let Value::String(key) = key else {
            return Err(mlua::Error::external(
                "pmacs.terminal.open: `env` keys must be strings",
            ));
        };
        let key = key.to_str()?.to_owned();
        let Value::String(value) = value else {
            return Err(mlua::Error::external(format!(
                "pmacs.terminal.open: `env[{key}]` must be a string"
            )));
        };
        env.push((key, value.to_str()?.to_owned()));
        Ok(())
    })?;
    env.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(env)
}

fn strict_terminal_u16(value: Value, field: &'static str, default: u16) -> mlua::Result<u16> {
    match value {
        Value::Nil => Ok(default),
        Value::Integer(value) => u16::try_from(value).map_err(|_| {
            mlua::Error::external(format!(
                "pmacs.terminal.open: `{field}` must be an integer in 0..={}",
                u16::MAX
            ))
        }),
        other => Err(mlua::Error::external(format!(
            "pmacs.terminal.open: `{field}` must be an integer, got {}",
            other.type_name()
        ))),
    }
}

fn strict_terminal_usize(value: Value, field: &'static str, default: usize) -> mlua::Result<usize> {
    match value {
        Value::Nil => Ok(default),
        Value::Integer(value) => usize::try_from(value).map_err(|_| {
            mlua::Error::external(format!(
                "pmacs.terminal.open: `{field}` must be a non-negative integer"
            ))
        }),
        other => Err(mlua::Error::external(format!(
            "pmacs.terminal.open: `{field}` must be an integer, got {}",
            other.type_name()
        ))),
    }
}

fn terminal_state_table(
    lua: &Lua,
    snapshot: crate::terminal::TerminalSnapshot,
) -> mlua::Result<Table> {
    let state = lua.create_table()?;
    state.set("buffer", BufferIdLua(snapshot.buffer_id))?;
    state.set("pid", i64::from(snapshot.pid))?;
    state.set("rows", i64::from(snapshot.size.rows))?;
    state.set("cols", i64::from(snapshot.size.cols))?;
    if let Some(title) = snapshot.title {
        state.set("title", title)?;
    }
    state.set(
        "screen_generation",
        i64::try_from(snapshot.screen_generation).unwrap_or(i64::MAX),
    )?;
    let process = lua.create_table()?;
    match snapshot.process {
        crate::terminal::TerminalProcessState::Running => process.set("kind", "running")?,
        crate::terminal::TerminalProcessState::Exited(code) => {
            process.set("kind", "exited")?;
            process.set("code", code)?;
        }
        crate::terminal::TerminalProcessState::Signaled(signal) => {
            process.set("kind", "signaled")?;
            process.set("signal", signal)?;
        }
        crate::terminal::TerminalProcessState::Crashed(message) => {
            process.set("kind", "crashed")?;
            process.set("message", message)?;
        }
    }
    state.set("process", process)?;
    Ok(state)
}

// ---------------------------------------------------------------------------
// pmacs.lsp: LSP client surface (T M4.5)
// ---------------------------------------------------------------------------

use crate::lsp::{
    LspClientState, LspError, LspEvent, LspEventKind, LspManager, LspRestartPolicy, LspServerId,
    LspServerSpec, SharedLspManager, state_label_for,
};

/// Lua-facing wrapper around [`LspServerId`]. Mirrors
/// [`ProcessIdLua`].
#[derive(Copy, Clone)]
pub struct LspServerIdLua(pub LspServerId);

impl LspServerIdLua {
    /// The wrapped [`LspServerId`].
    #[must_use]
    pub fn id(self) -> LspServerId {
        self.0
    }
}

impl FromLua for LspServerIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "LspServerIdLua".to_string(),
                message: Some("expected an LSP server handle".to_string()),
            }),
        }
    }
}

impl UserData for LspServerIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}", this.0))
        });
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: LspServerIdLua| {
            Ok(this.0 == other.0)
        });
        methods.add_method("raw", |_, this, ()| Ok(this.0.raw()));
    }
}

fn parse_lsp_restart(name: &str) -> mlua::Result<LspRestartPolicy> {
    Ok(match name {
        "never" | "Never" => LspRestartPolicy::Never,
        "on_crash" | "OnCrash" | "on-crash" => LspRestartPolicy::OnCrash,
        "always" | "Always" => LspRestartPolicy::Always,
        other => {
            return Err(mlua::Error::external(format!(
                "unknown LSP restart policy: {other:?} (expected never|on_crash|always)"
            )));
        }
    })
}

/// JSON ↔ Lua marshalling. Lua-side users specify request params as
/// nested tables; we translate to `serde_json::Value` for the wire
/// and back for inbound messages. Number-keyed tables become arrays
/// (LSP rarely needs a "table that's both an array and a map"); a
/// table that isn't a contiguous array starting at index 1 is
/// treated as an object.
fn lua_to_json(value: Value) -> mlua::Result<serde_json::Value> {
    use serde_json::Value as J;
    Ok(match value {
        Value::Nil => J::Null,
        Value::Boolean(b) => J::Bool(b),
        Value::Integer(i) => serde_json::Number::from(i).into(),
        Value::Number(n) => serde_json::Number::from_f64(n).map_or(J::Null, J::Number),
        Value::String(s) => J::String(s.to_str()?.to_owned()),
        Value::Table(t) => lua_table_to_json(&t)?,
        other => {
            return Err(mlua::Error::external(format!(
                "cannot marshall {} to JSON",
                other.type_name()
            )));
        }
    })
}

fn lua_table_to_json(t: &Table) -> mlua::Result<serde_json::Value> {
    use serde_json::Value as J;
    // Heuristic: contiguous integer keys 1..=n make an array.
    let len = t.len()? as usize;
    let mut count = 0usize;
    let mut all_int = true;
    t.clone().for_each(|k: Value, _: Value| {
        match k {
            Value::Integer(_) => {}
            _ => all_int = false,
        }
        count += 1;
        Ok(())
    })?;
    if all_int && count == len && len > 0 {
        let mut arr = Vec::with_capacity(len);
        for i in 1..=len {
            let v: Value = t.get(i)?;
            arr.push(lua_to_json(v)?);
        }
        return Ok(J::Array(arr));
    }
    // Otherwise: object. Empty tables also land here as `{}`.
    let mut map = serde_json::Map::new();
    t.clone().for_each(|k: Value, v: Value| {
        let key = match k {
            Value::String(s) => s.to_str()?.to_owned(),
            Value::Integer(i) => i.to_string(),
            other => {
                return Err(mlua::Error::external(format!(
                    "JSON object keys must be strings or integers, got {}",
                    other.type_name()
                )));
            }
        };
        map.insert(key, lua_to_json(v)?);
        Ok(())
    })?;
    Ok(J::Object(map))
}

fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    use serde_json::Value as J;
    Ok(match value {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else {
                Value::Number(n.as_f64().unwrap_or(0.0))
            }
        }
        J::String(s) => Value::String(lua.create_string(s)?),
        J::Array(items) => {
            let t = lua.create_table_with_capacity(items.len(), 0)?;
            for (i, v) in items.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
        J::Object(map) => {
            let t = lua.create_table_with_capacity(0, map.len())?;
            for (k, v) in map {
                t.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

fn lua_to_lsp_spec(t: &Table) -> mlua::Result<LspServerSpec> {
    let label: String = t.get("label").unwrap_or_else(|_| "unnamed".to_owned());
    let language_id: String = t
        .get("language_id")
        .unwrap_or_else(|_| "plaintext".to_owned());
    let command: String = t.get("command")?;
    let args: Vec<String> = t.get("args").unwrap_or_default();
    let cwd: Option<String> = t.get("cwd").ok().flatten();
    let root_uri: Option<String> = t.get("root_uri").ok().flatten();
    let env_t: Option<Table> = t.get("env").ok().flatten();
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(env_t) = env_t {
        env_t.for_each(|k: String, v: String| {
            env.push((k, v));
            Ok(())
        })?;
    }
    let init_options: Option<serde_json::Value> = match t.get::<Option<Value>>("init_options")? {
        Some(Value::Nil) | None => None,
        Some(other) => Some(lua_to_json(other)?),
    };
    let settings: Option<serde_json::Value> = match t.get::<Option<Value>>("settings")? {
        Some(Value::Nil) | None => None,
        Some(other) => Some(lua_to_json(other)?),
    };
    let capabilities: Option<serde_json::Value> = match t.get::<Option<Value>>("capabilities")? {
        Some(Value::Nil) | None => None,
        Some(other) => Some(lua_to_json(other)?),
    };
    let restart = match t.get::<Option<String>>("restart").ok().flatten() {
        Some(s) => parse_lsp_restart(&s)?,
        None => LspRestartPolicy::OnCrash,
    };
    Ok(LspServerSpec {
        label,
        language_id,
        command,
        args,
        cwd: cwd.map(std::path::PathBuf::from),
        root_uri,
        env,
        init_options,
        settings,
        capabilities,
        restart,
    })
}

fn lsp_state_to_lua(lua: &Lua, state: &LspClientState) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 4)?;
    t.set("kind", state_label_for(state))?;
    match state {
        LspClientState::Starting | LspClientState::Stopped { .. } => {}
        LspClientState::Initializing {
            init_request_id, ..
        } => {
            t.set("init_request_id", *init_request_id)?;
        }
        LspClientState::Initialized {
            capabilities,
            server_info,
            ..
        } => {
            t.set("capabilities", json_to_lua(lua, capabilities)?)?;
            if let Some(info) = server_info {
                t.set("server_info", json_to_lua(lua, info)?)?;
            }
        }
        LspClientState::ShuttingDown {
            shutdown_request_id,
        } => {
            if let Some(id) = shutdown_request_id {
                t.set("shutdown_request_id", *id)?;
            }
        }
        LspClientState::Crashed { reason, .. } => {
            t.set("reason", reason.as_str())?;
        }
    }
    Ok(t)
}

fn lsp_event_to_lua(lua: &Lua, ev: &LspEvent) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 5)?;
    t.set("server", LspServerIdLua(ev.server))?;
    match &ev.kind {
        LspEventKind::Started { pid } => {
            t.set("kind", "started")?;
            t.set("pid", *pid)?;
        }
        LspEventKind::Initialized { capabilities } => {
            t.set("kind", "initialized")?;
            t.set("capabilities", json_to_lua(lua, capabilities)?)?;
        }
        LspEventKind::Notification { method, params } => {
            t.set("kind", "notification")?;
            t.set("method", method.as_str())?;
            t.set("params", json_to_lua(lua, params)?)?;
        }
        LspEventKind::Request { id, method, params } => {
            t.set("kind", "request")?;
            t.set("request_id", json_to_lua(lua, id)?)?;
            t.set("method", method.as_str())?;
            t.set("params", json_to_lua(lua, params)?)?;
        }
        LspEventKind::Response {
            id,
            result,
            error,
            method,
        } => {
            t.set("kind", "response")?;
            t.set("request_id", *id)?;
            t.set("method", method.as_str())?;
            t.set("result", json_to_lua(lua, result)?)?;
            if let Some(e) = error {
                let err_t = lua.create_table_with_capacity(0, 3)?;
                err_t.set("code", e.code)?;
                err_t.set("message", e.message.as_str())?;
                if let Some(d) = &e.data {
                    err_t.set("data", json_to_lua(lua, d)?)?;
                }
                t.set("error", err_t)?;
            }
        }
        LspEventKind::ShuttingDown => {
            t.set("kind", "shutting_down")?;
        }
        LspEventKind::Stopped => {
            t.set("kind", "stopped")?;
        }
        LspEventKind::Crashed { reason } => {
            t.set("kind", "crashed")?;
            t.set("reason", reason.as_str())?;
        }
        LspEventKind::Restarting { attempt } => {
            t.set("kind", "restarting")?;
            t.set("attempt", *attempt)?;
        }
        LspEventKind::Stderr(bytes) => {
            t.set("kind", "stderr")?;
            t.set("bytes", lua.create_string(bytes)?)?;
        }
        LspEventKind::ProtocolError { message } => {
            t.set("kind", "protocol_error")?;
            t.set("message", message.as_str())?;
        }
    }
    Ok(t)
}

/// Lua → [`LspError`]. Either a plain string (becomes a generic
/// `code = -32603` internal error) or a table with `{code, message,
/// data?}`.
fn lua_to_lsp_error(value: Value) -> mlua::Result<LspError> {
    match value {
        Value::String(s) => Ok(LspError {
            code: -32603,
            message: s.to_str()?.to_owned(),
            data: None,
        }),
        Value::Table(t) => {
            let code: i64 = t.get("code").unwrap_or(-32603);
            let message: String = t.get("message").unwrap_or_else(|_| "error".to_owned());
            let data = match t.get::<Option<Value>>("data")? {
                Some(Value::Nil) | None => None,
                Some(other) => Some(lua_to_json(other)?),
            };
            Ok(LspError {
                code,
                message,
                data,
            })
        }
        other => Err(mlua::Error::external(format!(
            "lsp error must be a string or table, got {}",
            other.type_name()
        ))),
    }
}

/// Install `pmacs.lsp.*` (T M4.5).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings; splitting fragments a coherent surface"
)]
pub fn install_lsp(
    lua: &Lua,
    manager: &SharedLspManager,
    syntax: &SharedSyntaxRegistry,
) -> mlua::Result<()> {
    lua.set_app_data(manager.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let lsp_mod = lua.create_table()?;

    {
        // T M4.5 L1: decode a server-returned `file://` URI to a
        // filesystem path so cross-file navigation can open it.
        lsp_mod.set(
            "path_for_uri",
            lua.create_function(|_, uri: String| {
                Ok(crate::project_index::uri_to_path(&uri).map(|p| p.display().to_string()))
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "spawn",
            lua.create_function(move |_, spec: Table| {
                let parsed = lua_to_lsp_spec(&spec)?;
                let id = m
                    .borrow_mut()
                    .spawn(parsed)
                    .map_err(mlua::Error::external)?;
                Ok(LspServerIdLua(id))
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "stop",
            lua.create_function(move |_, id: LspServerIdLua| {
                m.borrow_mut().stop(id.0).map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "send_request",
            lua.create_function(
                move |_, (id, method, params): (LspServerIdLua, String, Option<Value>)| {
                    let json_params = match params {
                        Some(Value::Nil) | None => serde_json::Value::Null,
                        Some(other) => lua_to_json(other)?,
                    };
                    let req_id = m
                        .borrow_mut()
                        .send_request(id.0, method, json_params)
                        .map_err(mlua::Error::external)?;
                    Ok(req_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "send_notification",
            lua.create_function(
                move |_, (id, method, params): (LspServerIdLua, String, Option<Value>)| {
                    let json_params = match params {
                        Some(Value::Nil) | None => serde_json::Value::Null,
                        Some(other) => lua_to_json(other)?,
                    };
                    m.borrow_mut()
                        .send_notification(id.0, method, json_params)
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "send_response",
            lua.create_function(
                move |_,
                      (id, request_id, result, err): (
                    LspServerIdLua,
                    Value,
                    Value,
                    Option<Value>,
                )| {
                    let request_id = lua_to_json(request_id)?;
                    let outcome = match err {
                        Some(Value::Nil) | None => Ok(lua_to_json(result)?),
                        Some(e) => Err(lua_to_lsp_error(e)?),
                    };
                    m.borrow_mut()
                        .send_response(id.0, request_id, outcome)
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "did_open",
            lua.create_function(
                move |_, (id, uri, version, text): (LspServerIdLua, String, i64, String)| {
                    m.borrow_mut()
                        .did_open(id.0, uri, version, text)
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "did_change",
            lua.create_function(
                move |_, (id, uri, version, text): (LspServerIdLua, String, i64, String)| {
                    m.borrow_mut()
                        .did_change_full(id.0, uri, version, text)
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        // Mark `uri`'s cached LSP render families (diagnostics,
        // semantic tokens, inlay hints) stale without sending
        // anything. The didChange-debounce glue in
        // `builtin/runtime/lsp.lua` calls this per edit so stale
        // suppression stays keystroke-accurate while the O(file)
        // full-document notification is coalesced.
        let m = manager.clone();
        lsp_mod.set(
            "_mark_document_stale",
            lua.create_function(move |_, uri: String| {
                m.borrow().mark_document_stale(&uri);
                Ok(())
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "did_close",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                m.borrow_mut()
                    .did_close(id.0, uri)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        // T M4.5 file watching. `changes` is the Lua-built FileEvent
        // array `{ { uri = , type = 1|2|3 }, … }`; converted to JSON
        // and sent as `workspace/didChangeWatchedFiles`.
        let m = manager.clone();
        lsp_mod.set(
            "did_change_watched_files",
            lua.create_function(move |_, (id, changes): (LspServerIdLua, Value)| {
                let changes = lua_to_json(changes)?;
                m.borrow_mut()
                    .did_change_watched_files(id.0, &changes)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        // T M4.5 task #8: hard per-request timeout, tunable from
        // init.lua (e.g. raise it for a slow language server on a
        // cold cache, or lower it in tests).
        let m = manager.clone();
        lsp_mod.set(
            "set_request_timeout_ms",
            lua.create_function(move |_, ms: u64| {
                m.borrow_mut()
                    .set_request_timeout(std::time::Duration::from_millis(ms));
                Ok(())
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_completion_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_completion(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_hover_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_hover(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_signature_help_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_signature_help(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_definition_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_definition(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_references_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_references(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_declaration_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_declaration(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_type_definition_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_type_definition(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_implementation_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_implementation(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_document_symbol_raw",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let job_id = m
                    .borrow_mut()
                    .request_document_symbol(id.0, uri)
                    .map_err(mlua::Error::external)?;
                Ok(job_id)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_workspace_symbol_raw",
            lua.create_function(move |_, (id, query): (LspServerIdLua, String)| {
                let job_id = m
                    .borrow_mut()
                    .request_workspace_symbol(id.0, query)
                    .map_err(mlua::Error::external)?;
                Ok(job_id)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_document_highlight_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_document_highlight(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_formatting_raw",
            lua.create_function(
                move |_,
                      (id, uri, tab_size, insert_spaces): (
                    LspServerIdLua,
                    String,
                    u32,
                    Option<bool>,
                )| {
                    let job_id = m
                        .borrow_mut()
                        .request_formatting(id.0, uri, tab_size, insert_spaces.unwrap_or(true))
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_rename_raw",
            lua.create_function(
                move |_,
                      (id, uri, line, col, new_name): (
                    LspServerIdLua,
                    String,
                    u32,
                    u32,
                    String,
                )| {
                    let job_id = m
                        .borrow_mut()
                        .request_rename(id.0, uri, line, col, new_name)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_prepare_rename_raw",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let job_id = m
                        .borrow_mut()
                        .request_prepare_rename(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_code_action_raw",
            lua.create_function(
                move |_,
                      (id, uri, sl, sc, el, ec): (
                    LspServerIdLua,
                    String,
                    u32,
                    u32,
                    u32,
                    u32,
                )| {
                    // v1 sends an empty diagnostics context (point/
                    // range actions). Diagnostic-driven quick-fixes
                    // are a later refinement of this same call.
                    let job_id = m
                        .borrow_mut()
                        .request_code_action(id.0, uri, sl, sc, el, ec, &[])
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_inlay_hint_raw",
            lua.create_function(
                move |_,
                      (id, uri, sl, sc, el, ec): (
                    LspServerIdLua,
                    String,
                    u32,
                    u32,
                    u32,
                    u32,
                )| {
                    let job_id = m
                        .borrow_mut()
                        .request_inlay_hint(id.0, uri, sl, sc, el, ec)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_semantic_tokens_raw",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let job_id = m
                    .borrow_mut()
                    .request_semantic_tokens(id.0, uri)
                    .map_err(mlua::Error::external)?;
                Ok(job_id)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_semantic_tokens_range_raw",
            lua.create_function(
                move |_,
                      (id, uri, sl, sc, el, ec): (
                    LspServerIdLua,
                    String,
                    u32,
                    u32,
                    u32,
                    u32,
                )| {
                    let job_id = m
                        .borrow_mut()
                        .request_semantic_tokens_range(id.0, uri, sl, sc, el, ec)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_semantic_tokens_delta_raw",
            lua.create_function(
                move |_, (id, uri, prev): (LspServerIdLua, String, String)| {
                    let job_id = m
                        .borrow_mut()
                        .request_semantic_tokens_delta(id.0, uri, prev)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_request_execute_command_raw",
            lua.create_function(
                move |_, (id, command, args): (LspServerIdLua, String, Option<Value>)| {
                    let arguments = match args {
                        Some(v) => lua_to_json(v)?.as_array().cloned().unwrap_or_default(),
                        None => Vec::new(),
                    };
                    let job_id = m
                        .borrow_mut()
                        .request_execute_command(id.0, command, &arguments)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        // Normalise an arbitrary LSP `WorkspaceEdit` JSON value (e.g.
        // a server→client `workspace/applyEdit` param) into the same
        // `{ ops = { … } }` ordered-op shape the rename/code-action
        // surfaces hand back — so the Lua applier has exactly one
        // input format regardless of origin.
        lsp_mod.set(
            "_parse_workspace_edit",
            lua.create_function(move |lua, edit: Value| {
                let json = lua_to_json(edit)?;
                let parsed = WorkspaceEditResponse::from_lsp_value(&json);
                let out = lua.create_table_with_capacity(0, 1)?;
                out.set("ops", workspace_ops_to_lua(lua, &parsed)?)?;
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "status",
            lua.create_function(move |lua, id: LspServerIdLua| {
                let mgr = m.borrow();
                match mgr.state(id.0) {
                    Some(state) => Ok(Value::Table(lsp_state_to_lua(lua, state)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "capabilities",
            lua.create_function(move |lua, id: LspServerIdLua| {
                let mgr = m.borrow();
                match mgr.capabilities(id.0) {
                    Some(caps) => Ok(json_to_lua(lua, caps)?),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "events_take",
            lua.create_function(move |lua, id: LspServerIdLua| {
                let evs = m.borrow_mut().take_events(id.0);
                let out = lua.create_table_with_capacity(evs.len(), 0)?;
                for (i, ev) in evs.iter().enumerate() {
                    out.set(i + 1, lsp_event_to_lua(lua, ev)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "list",
            lua.create_function(move |lua, ()| {
                let mgr = m.borrow();
                let ids: Vec<LspServerId> = mgr.ids().collect();
                let out = lua.create_table_with_capacity(ids.len(), 0)?;
                for (i, id) in ids.iter().enumerate() {
                    let row = lua.create_table_with_capacity(0, 5)?;
                    row.set("id", LspServerIdLua(*id))?;
                    if let Some(spec) = mgr.spec(*id) {
                        row.set("label", spec.label.as_str())?;
                        row.set("language_id", spec.language_id.as_str())?;
                        row.set("command", spec.command.as_str())?;
                    }
                    if let Some(state) = mgr.state(*id) {
                        row.set("state", lsp_state_to_lua(lua, state)?)?;
                    }
                    if let Some(attempt) = mgr.attempt(*id) {
                        row.set("attempt", attempt)?;
                    }
                    out.set(i + 1, row)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "forget",
            lua.create_function(move |_, id: LspServerIdLua| {
                m.borrow_mut().forget(id.0).map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "_tick",
            lua.create_function(move |_, ()| {
                m.borrow_mut().tick();
                Ok(())
            })?,
        )?;
    }

    {
        // T M4.8: short modeline label, e.g. "ready" / "idx" /
        // "crashed". Stable string set.
        let m = manager.clone();
        lsp_mod.set(
            "modeline_label",
            lua.create_function(move |_, id: LspServerIdLua| {
                Ok(m.borrow().modeline_label(id.0).to_owned())
            })?,
        )?;
    }

    {
        // T M4.8: most recent error for `id`, or nil if none.
        let m = manager.clone();
        lsp_mod.set(
            "last_error",
            lua.create_function(move |lua, id: LspServerIdLua| {
                let mgr_ref = m.borrow();
                let Some(err) = mgr_ref.last_error(id.0) else {
                    return Ok(Value::Nil);
                };
                let t = lua.create_table_with_capacity(0, 3)?;
                t.set("source", err.source)?;
                t.set("message", err.message.as_str())?;
                if let Some(c) = err.code {
                    t.set("code", c)?;
                }
                Ok(Value::Table(t))
            })?,
        )?;
    }

    {
        // T M4.8: ring of recent log lines for `id`. Optional `n`
        // returns only the last `n` entries.
        let m = manager.clone();
        lsp_mod.set(
            "recent_messages",
            lua.create_function(move |lua, (id, n): (LspServerIdLua, Option<usize>)| {
                let mgr_ref = m.borrow();
                let Some(st) = mgr_ref.status_for(id.0) else {
                    return lua.create_table();
                };
                let take = n.unwrap_or(st.recent_messages.len());
                let start = st.recent_messages.len().saturating_sub(take);
                let slice = &st.recent_messages[start..];
                let out = lua.create_table_with_capacity(slice.len(), 0)?;
                for (i, m) in slice.iter().enumerate() {
                    let t = lua.create_table_with_capacity(0, 3)?;
                    t.set("channel", m.channel)?;
                    t.set("summary", m.summary.as_str())?;
                    if let Some(d) = m.detail.as_deref() {
                        t.set("detail", d)?;
                    }
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // T M4.8: a snapshot of the entire status surface for `id`,
        // for callers that want one Lua call per render frame.
        let m = manager.clone();
        lsp_mod.set(
            "status_summary",
            lua.create_function(move |lua, id: LspServerIdLua| {
                let mgr_ref = m.borrow();
                let Some(st) = mgr_ref.status_for(id.0) else {
                    return Ok(Value::Nil);
                };
                let t = lua.create_table_with_capacity(0, 6)?;
                t.set("kind", st.kind.tag())?;
                t.set("label", st.kind.label())?;
                t.set("restarts", st.restarts)?;
                if let crate::lsp_status::LspStatusKind::Indexing { title, percentage } = &st.kind {
                    t.set("indexing_title", title.as_str())?;
                    if let Some(p) = percentage {
                        t.set("indexing_percentage", *p)?;
                    }
                }
                if let crate::lsp_status::LspStatusKind::Degraded { reason }
                | crate::lsp_status::LspStatusKind::Crashed { reason } = &st.kind
                {
                    t.set("reason", reason.as_str())?;
                }
                if let Some(info) = &st.server_info {
                    let s = lua.create_table_with_capacity(0, 2)?;
                    s.set("name", info.name.as_str())?;
                    if let Some(v) = info.version.as_deref() {
                        s.set("version", v)?;
                    }
                    t.set("server_info", s)?;
                }
                if let Some(err) = &st.last_error {
                    let e = lua.create_table_with_capacity(0, 3)?;
                    e.set("source", err.source)?;
                    e.set("message", err.message.as_str())?;
                    if let Some(c) = err.code {
                        e.set("code", c)?;
                    }
                    t.set("last_error", e)?;
                }
                Ok(Value::Table(t))
            })?,
        )?;
    }

    {
        // T M4.8: rendered text for the `*lsp*` status buffer.
        let m = manager.clone();
        lsp_mod.set(
            "status_buffer_text",
            lua.create_function(move |_, ()| Ok(m.borrow().status_buffer_text()))?,
        )?;
    }

    // M_B1: TUI-side LSP styling. Sibling of
    // `pmacs.parse._attach_highlight`: pushes an `LspStyleView` overlay
    // on the active window so the grid renderer paints LSP semantic
    // tokens as cell styles for buffers with no bundled tree-sitter
    // grammar. Lua callers gate against the grammar-backed case (policy
    // A: one styling authority per buffer); double-attach pushes a
    // fresh overlay each call, so the Lua side is also responsible for
    // dedup, matching `_attach_highlight`'s contract.
    {
        let m = manager.clone();
        let s = syntax.clone();
        lsp_mod.set(
            "_attach_style",
            lua.create_function(move |lua, id: BufferIdLua| {
                let theme = s.theme();
                let core = lua
                    .app_data_ref::<SharedCore>()
                    .ok_or_else(|| mlua::Error::external("editor core not yet installed"))?;
                let mut core_borrow = core.borrow_mut();
                let win = core_borrow.active_window_mut();
                if win.buffer_id != id.0 {
                    return Err(mlua::Error::external(format!(
                        "active window's buffer is not {:?}",
                        id.0
                    )));
                }
                let overlay = crate::highlight::LspStyleView::new(m.clone(), theme);
                win.push_overlay(Box::new(overlay));
                Ok(true)
            })?,
        )?;
    }

    pmacs.set("lsp", lsp_mod)?;
    Ok(())
}

/// Build a fresh [`LspManager`] over `supervisor` and install
/// `pmacs.lsp.*` over it.
pub fn make_lsp_manager(
    lua: &Lua,
    supervisor: SharedProcessSupervisor,
    runtime: crate::async_runtime::SharedAsyncRuntime,
    syntax: &SharedSyntaxRegistry,
) -> mlua::Result<SharedLspManager> {
    let manager = Rc::new(RefCell::new(LspManager::new(supervisor, runtime)));
    install_lsp(lua, &manager, syntax)?;
    diag::install_diag(lua, &manager, &syntax.theme())?;
    install_completion(lua, &manager)?;
    install_hover(lua, &manager)?;
    install_signature(lua, &manager)?;
    install_definition(lua, &manager)?;
    install_locations(lua, &manager)?;
    install_symbol(lua, &manager)?;
    install_document_highlight(lua, &manager)?;
    install_formatting(lua, &manager)?;
    install_rename(lua, &manager)?;
    install_prepare_rename(lua, &manager)?;
    install_code_action(lua, &manager)?;
    install_inlay_hint(lua, &manager)?;
    install_semantic_tokens(lua, &manager)?;
    Ok(manager)
}

// ---------------------------------------------------------------------------
// pmacs.completion / pmacs.hover / pmacs.signature: T M4.7 surfaces
// ---------------------------------------------------------------------------

use crate::code_action::{CodeActionItem, CodeActionKey};
use crate::completion::{CompletionItem, CompletionItemKind, CompletionKey, CompletionTriggers};
use crate::definition::{DefinitionKey, DefinitionLocation, DefinitionResponse};
use crate::document_highlight::{DocumentHighlightKey, Highlight};
use crate::formatting::{FormattingKey, FormattingResponse, TextEdit};
use crate::hover::{Hover, HoverKey};
use crate::inlay_hint::{InlayHint as LspInlayHint, InlayHintKey};
use crate::locations::{LocationKind, LocationsKey};
use crate::prepare_rename::PrepareRenameKey;
use crate::rename::{RenameKey, WorkspaceEditResponse, WorkspaceOp};
use crate::semantic_tokens::{
    SemanticToken as LspSemanticToken, SemanticTokenKey, SemanticTokensLegend,
};
use crate::signature::{Signature, SignatureHelp, SignatureKey, SignatureParameter};
use crate::symbol::{Symbol as LspSymbol, SymbolKey};

fn completion_item_to_lua(lua: &Lua, item: &CompletionItem) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 7)?;
    t.set("label", item.label.as_str())?;
    t.set("kind_code", item.kind as i64)?;
    t.set("kind_glyph", item.kind.glyph().to_string())?;
    if let Some(d) = item.detail.as_deref() {
        t.set("detail", d)?;
    }
    if let Some(d) = item.documentation.as_deref() {
        t.set("documentation", d)?;
    }
    if let Some(d) = item.insert_text.as_deref() {
        t.set("insert_text", d)?;
    }
    if let Some(d) = item.sort_text.as_deref() {
        t.set("sort_text", d)?;
    }
    if let Some(d) = item.filter_text.as_deref() {
        t.set("filter_text", d)?;
    }
    Ok(t)
}

/// Install `pmacs.completion.*` (T M4.7).
#[allow(clippy::too_many_lines, reason = "linear list of raw bindings")]
pub fn install_completion(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    // Merge into an existing `pmacs.completion` (the popup surface
    // installs at editor-attach time, before the LSP manager exists)
    // instead of clobbering it --- the same idiom as
    // `install_completion_framework`.
    let m: Table = match pmacs.get::<Option<Table>>("completion")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        let mgr = manager.clone();
        m.set(
            "items",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().completion_store();
                let guard = store_handle
                    .lock()
                    .expect("completion store mutex poisoned");
                let key = CompletionKey::new(id.0.raw().to_string(), uri);
                let items = guard.items(&key);
                let out = lua.create_table_with_capacity(items.len(), 0)?;
                for (i, it) in items.iter().enumerate() {
                    out.set(i + 1, completion_item_to_lua(lua, it)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "is_incomplete",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().completion_store();
                let guard = store_handle
                    .lock()
                    .expect("completion store mutex poisoned");
                let key = CompletionKey::new(id.0.raw().to_string(), uri);
                Ok(guard.is_incomplete(&key))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "selected",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().completion_store();
                let guard = store_handle
                    .lock()
                    .expect("completion store mutex poisoned");
                let key = CompletionKey::new(id.0.raw().to_string(), uri);
                Ok(guard.selected(&key))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "select",
            lua.create_function(move |_, (id, uri, idx): (LspServerIdLua, String, usize)| {
                let store_handle = mgr.borrow().completion_store();
                let mut guard = store_handle
                    .lock()
                    .expect("completion store mutex poisoned");
                let key = CompletionKey::new(id.0.raw().to_string(), uri);
                guard.select(&key, idx);
                Ok(())
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "move_selection",
            lua.create_function(move |_, (id, uri, delta): (LspServerIdLua, String, i64)| {
                let store_handle = mgr.borrow().completion_store();
                let mut guard = store_handle
                    .lock()
                    .expect("completion store mutex poisoned");
                let key = CompletionKey::new(id.0.raw().to_string(), uri);
                guard.move_selection(&key, delta as isize);
                Ok(())
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().completion_store();
                let mut guard = store_handle
                    .lock()
                    .expect("completion store mutex poisoned");
                let key = CompletionKey::new(id.0.raw().to_string(), uri);
                guard.clear(&key);
                Ok(())
            })?,
        )?;
    }

    {
        // Inspect server capabilities to compute trigger characters.
        let mgr = manager.clone();
        m.set(
            "trigger_characters",
            lua.create_function(move |lua, id: LspServerIdLua| {
                let mgr_ref = mgr.borrow();
                let triggers = mgr_ref.capabilities(id.0).map_or_else(
                    CompletionTriggers::empty,
                    CompletionTriggers::from_capabilities,
                );
                let chars = triggers.chars();
                let out = lua.create_table_with_capacity(chars.len(), 0)?;
                for (i, ch) in chars.iter().enumerate() {
                    out.set(i + 1, ch.to_string())?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // Test whether `ch` is a trigger character given the server's
        // negotiated capabilities. Convenience around
        // `trigger_characters`.
        let mgr = manager.clone();
        m.set(
            "should_fire",
            lua.create_function(move |_, (id, ch_str): (LspServerIdLua, String)| {
                let Some(ch) = ch_str.chars().next() else {
                    return Ok(false);
                };
                let mgr_ref = mgr.borrow();
                let triggers = mgr_ref.capabilities(id.0).map_or_else(
                    CompletionTriggers::empty,
                    CompletionTriggers::from_capabilities,
                );
                Ok(triggers.should_fire(ch))
            })?,
        )?;
    }

    {
        // Static lookup: kind code → glyph. Useful for previewing
        // popup rendering from Lua tests.
        m.set(
            "kind_glyph",
            lua.create_function(move |_, code: i64| {
                #[allow(
                    clippy::match_same_arms,
                    reason = "explicit code 1 arm documents the LSP mapping; Text is also the wildcard fallback"
                )]
                let kind = match code {
                    1 => CompletionItemKind::Text,
                    2 => CompletionItemKind::Method,
                    3 => CompletionItemKind::Function,
                    4 => CompletionItemKind::Constructor,
                    5 => CompletionItemKind::Field,
                    6 => CompletionItemKind::Variable,
                    7 => CompletionItemKind::Class,
                    8 => CompletionItemKind::Interface,
                    9 => CompletionItemKind::Module,
                    10 => CompletionItemKind::Property,
                    11 => CompletionItemKind::Unit,
                    12 => CompletionItemKind::Value,
                    13 => CompletionItemKind::Enum,
                    14 => CompletionItemKind::Keyword,
                    15 => CompletionItemKind::Snippet,
                    16 => CompletionItemKind::Color,
                    17 => CompletionItemKind::File,
                    18 => CompletionItemKind::Reference,
                    19 => CompletionItemKind::Folder,
                    20 => CompletionItemKind::EnumMember,
                    21 => CompletionItemKind::Constant,
                    22 => CompletionItemKind::Struct,
                    23 => CompletionItemKind::Event,
                    24 => CompletionItemKind::Operator,
                    25 => CompletionItemKind::TypeParameter,
                    _ => CompletionItemKind::Text,
                };
                Ok(kind.glyph().to_string())
            })?,
        )?;
    }

    pmacs.set("completion", m)?;
    Ok(())
}

fn hover_to_lua(lua: &Lua, h: &Hover) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 4)?;
    t.set("contents", h.contents.as_str())?;
    if let Some(r) = h.range {
        let range = lua.create_table_with_capacity(0, 4)?;
        range.set("start_line", r.start_line)?;
        range.set("start_col", r.start_col)?;
        range.set("end_line", r.end_line)?;
        range.set("end_col", r.end_col)?;
        t.set("range", range)?;
    }
    Ok(t)
}

/// Install `pmacs.hover.*` (T M4.7).
pub fn install_hover(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "current",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().hover_store();
                let guard = store_handle.lock().expect("hover store mutex poisoned");
                let key = HoverKey::new(id.0.raw().to_string(), uri);
                match guard.get(&key) {
                    Some(h) => Ok(Value::Table(hover_to_lua(lua, h)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().hover_store();
                let mut guard = store_handle.lock().expect("hover store mutex poisoned");
                let key = HoverKey::new(id.0.raw().to_string(), uri);
                guard.clear(&key);
                Ok(())
            })?,
        )?;
    }

    pmacs.set("hover", m)?;
    Ok(())
}

fn signature_parameter_to_lua(lua: &Lua, p: &SignatureParameter) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 3)?;
    t.set("label", p.label.as_str())?;
    if let Some((s, e)) = p.span {
        let span = lua.create_table_with_capacity(2, 0)?;
        span.set(1, s)?;
        span.set(2, e)?;
        t.set("span", span)?;
    }
    if let Some(d) = p.documentation.as_deref() {
        t.set("documentation", d)?;
    }
    Ok(t)
}

fn signature_to_lua(lua: &Lua, s: &Signature) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 4)?;
    t.set("label", s.label.as_str())?;
    if let Some(d) = s.documentation.as_deref() {
        t.set("documentation", d)?;
    }
    let params = lua.create_table_with_capacity(s.parameters.len(), 0)?;
    for (i, p) in s.parameters.iter().enumerate() {
        params.set(i + 1, signature_parameter_to_lua(lua, p)?)?;
    }
    t.set("parameters", params)?;
    if let Some(ap) = s.active_parameter {
        t.set("active_parameter", ap)?;
    }
    Ok(t)
}

fn signature_help_to_lua(lua: &Lua, h: &SignatureHelp) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 3)?;
    let sigs = lua.create_table_with_capacity(h.signatures.len(), 0)?;
    for (i, s) in h.signatures.iter().enumerate() {
        sigs.set(i + 1, signature_to_lua(lua, s)?)?;
    }
    t.set("signatures", sigs)?;
    t.set("active_signature", h.active_signature)?;
    if let Some(ap) = h.active_parameter {
        t.set("active_parameter", ap)?;
    }
    if let Some(idx) = h.active_parameter_index() {
        t.set("active_parameter_index", idx)?;
    }
    Ok(t)
}

/// Install `pmacs.signature.*` (T M4.7).
pub fn install_signature(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "current",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().signature_store();
                let guard = store_handle.lock().expect("signature store mutex poisoned");
                let key = SignatureKey::new(id.0.raw().to_string(), uri);
                match guard.get(&key) {
                    Some(h) => Ok(Value::Table(signature_help_to_lua(lua, h)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().signature_store();
                let mut guard = store_handle.lock().expect("signature store mutex poisoned");
                let key = SignatureKey::new(id.0.raw().to_string(), uri);
                guard.clear(&key);
                Ok(())
            })?,
        )?;
    }

    pmacs.set("signature", m)?;
    Ok(())
}

fn definition_location_to_lua(lua: &Lua, loc: &DefinitionLocation) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 3)?;
    t.set("uri", loc.uri.as_str())?;
    t.set("line", loc.line)?;
    t.set("col", loc.col)?;
    Ok(t)
}

fn definition_response_to_lua(lua: &Lua, r: &DefinitionResponse) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(r.locations.len(), 0)?;
    for (i, loc) in r.locations.iter().enumerate() {
        t.set(i + 1, definition_location_to_lua(lua, loc)?)?;
    }
    Ok(t)
}

/// Install `pmacs.definition.*` (T M4.12).
pub fn install_definition(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "locations",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().definition_store();
                let guard = store_handle
                    .lock()
                    .expect("definition store mutex poisoned");
                let key = DefinitionKey::new(id.0.raw().to_string(), uri);
                if let Some(r) = guard.get(&key) {
                    Ok(Value::Table(definition_response_to_lua(lua, r)?))
                } else {
                    let empty = lua.create_table_with_capacity(0, 0)?;
                    Ok(Value::Table(empty))
                }
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().definition_store();
                let mut guard = store_handle
                    .lock()
                    .expect("definition store mutex poisoned");
                let key = DefinitionKey::new(id.0.raw().to_string(), uri);
                guard.clear(&key);
                Ok(())
            })?,
        )?;
    }

    pmacs.set("definition", m)?;
    Ok(())
}

/// Install `pmacs.references` / `.declaration` / `.type_definition`
/// / `.implementation`, each `{ locations(sid,uri), clear(sid,uri) }`,
/// mirroring `pmacs.definition` (same Location-list shape, hence the
/// reused `definition_response_to_lua`). T M4.5.
pub fn install_locations(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    for kind in [
        LocationKind::References,
        LocationKind::Declaration,
        LocationKind::TypeDefinition,
        LocationKind::Implementation,
    ] {
        let m = lua.create_table()?;
        {
            let mgr = manager.clone();
            m.set(
                "locations",
                lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                    let store_handle = mgr.borrow().locations_store();
                    let guard = store_handle.lock().expect("locations store mutex poisoned");
                    let key = LocationsKey::new(id.0.raw().to_string(), uri, kind);
                    if let Some(r) = guard.get(&key) {
                        Ok(Value::Table(definition_response_to_lua(lua, r)?))
                    } else {
                        Ok(Value::Table(lua.create_table_with_capacity(0, 0)?))
                    }
                })?,
            )?;
        }
        {
            let mgr = manager.clone();
            m.set(
                "clear",
                lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                    let store_handle = mgr.borrow().locations_store();
                    let mut guard = store_handle.lock().expect("locations store mutex poisoned");
                    guard.clear(&LocationsKey::new(id.0.raw().to_string(), uri, kind));
                    Ok(())
                })?,
            )?;
        }
        pmacs.set(kind.label(), m)?;
    }
    Ok(())
}

fn symbol_to_lua(lua: &Lua, s: &LspSymbol) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 7)?;
    t.set("name", s.name.as_str())?;
    t.set("kind", s.kind)?;
    t.set("uri", s.uri.as_str())?;
    t.set("line", s.line)?;
    t.set("col", s.col)?;
    t.set("depth", s.depth)?;
    if let Some(c) = &s.container {
        t.set("container", c.as_str())?;
    }
    Ok(t)
}

fn highlight_to_lua(lua: &Lua, h: &Highlight) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 5)?;
    t.set("start_line", h.start_line)?;
    t.set("start_col", h.start_col)?;
    t.set("end_line", h.end_line)?;
    t.set("end_col", h.end_col)?;
    t.set("kind", h.kind)?;
    Ok(t)
}

/// Install `pmacs.document_symbol` (`symbols(sid,uri)` / `clear`) and
/// `pmacs.workspace_symbol` (`symbols(sid,query)` / `clear`) over the
/// scope-keyed symbol store. T M4.5.
pub fn install_symbol(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;

    let doc = lua.create_table()?;
    {
        let mgr = manager.clone();
        doc.set(
            "symbols",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store = mgr.borrow().symbol_store();
                let guard = store.lock().expect("symbol store mutex poisoned");
                let key = SymbolKey::document(id.0.raw().to_string(), uri);
                let out = lua.create_table()?;
                if let Some(r) = guard.get(&key) {
                    for (i, s) in r.symbols.iter().enumerate() {
                        out.set(i + 1, symbol_to_lua(lua, s)?)?;
                    }
                }
                Ok(Value::Table(out))
            })?,
        )?;
    }
    {
        let mgr = manager.clone();
        doc.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store = mgr.borrow().symbol_store();
                let mut guard = store.lock().expect("symbol store mutex poisoned");
                guard.clear(&SymbolKey::document(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }
    pmacs.set("document_symbol", doc)?;

    let ws = lua.create_table()?;
    {
        let mgr = manager.clone();
        ws.set(
            "symbols",
            lua.create_function(move |lua, (id, query): (LspServerIdLua, String)| {
                let store = mgr.borrow().symbol_store();
                let guard = store.lock().expect("symbol store mutex poisoned");
                let key = SymbolKey::workspace(id.0.raw().to_string(), query);
                let out = lua.create_table()?;
                if let Some(r) = guard.get(&key) {
                    for (i, s) in r.symbols.iter().enumerate() {
                        out.set(i + 1, symbol_to_lua(lua, s)?)?;
                    }
                }
                Ok(Value::Table(out))
            })?,
        )?;
    }
    {
        let mgr = manager.clone();
        ws.set(
            "clear",
            lua.create_function(move |_, (id, query): (LspServerIdLua, String)| {
                let store = mgr.borrow().symbol_store();
                let mut guard = store.lock().expect("symbol store mutex poisoned");
                guard.clear(&SymbolKey::workspace(id.0.raw().to_string(), query));
                Ok(())
            })?,
        )?;
    }
    pmacs.set("workspace_symbol", ws)?;
    Ok(())
}

/// Install `pmacs.document_highlight` (`highlights(sid,uri)` /
/// `clear`). T M4.5.
pub fn install_document_highlight(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;
    {
        let mgr = manager.clone();
        m.set(
            "highlights",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store = mgr.borrow().document_highlight_store();
                let guard = store
                    .lock()
                    .expect("document highlight store mutex poisoned");
                let key = DocumentHighlightKey::new(id.0.raw().to_string(), uri);
                let out = lua.create_table()?;
                if let Some(r) = guard.get(&key) {
                    for (i, h) in r.highlights.iter().enumerate() {
                        out.set(i + 1, highlight_to_lua(lua, h)?)?;
                    }
                }
                Ok(Value::Table(out))
            })?,
        )?;
    }
    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store = mgr.borrow().document_highlight_store();
                let mut guard = store
                    .lock()
                    .expect("document highlight store mutex poisoned");
                guard.clear(&DocumentHighlightKey::new(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }
    pmacs.set("document_highlight", m)?;
    Ok(())
}

fn text_edit_to_lua(lua: &Lua, edit: &TextEdit) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 5)?;
    t.set("start_line", edit.start_line)?;
    t.set("start_col", edit.start_col)?;
    t.set("end_line", edit.end_line)?;
    t.set("end_col", edit.end_col)?;
    t.set("new_text", edit.new_text.as_str())?;
    Ok(t)
}

fn formatting_response_to_lua(lua: &Lua, r: &FormattingResponse) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(r.edits.len(), 0)?;
    for (i, e) in r.edits.iter().enumerate() {
        t.set(i + 1, text_edit_to_lua(lua, e)?)?;
    }
    Ok(t)
}

/// Install `pmacs.formatting.*` (T M4.12).
pub fn install_formatting(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "edits",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().formatting_store();
                let guard = store_handle
                    .lock()
                    .expect("formatting store mutex poisoned");
                let key = FormattingKey::new(id.0.raw().to_string(), uri);
                if let Some(r) = guard.get(&key) {
                    Ok(Value::Table(formatting_response_to_lua(lua, r)?))
                } else {
                    let empty = lua.create_table_with_capacity(0, 0)?;
                    Ok(Value::Table(empty))
                }
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().formatting_store();
                let mut guard = store_handle
                    .lock()
                    .expect("formatting store mutex poisoned");
                let key = FormattingKey::new(id.0.raw().to_string(), uri);
                guard.clear(&key);
                Ok(())
            })?,
        )?;
    }

    pmacs.set("formatting", m)?;
    Ok(())
}

/// The edit ops only, as `{ { uri =, edits = { … } }, … }` — the
/// back-compat per-file view (`pmacs.rename.file_edits`).
fn file_edits_to_lua(lua: &Lua, r: &WorkspaceEditResponse) -> mlua::Result<Table> {
    let files = r.files();
    let out = lua.create_table_with_capacity(files.len(), 0)?;
    for (i, f) in files.iter().enumerate() {
        let entry = lua.create_table_with_capacity(0, 2)?;
        entry.set("uri", f.uri.as_str())?;
        let edits = lua.create_table_with_capacity(f.edits.len(), 0)?;
        for (j, e) in f.edits.iter().enumerate() {
            edits.set(j + 1, text_edit_to_lua(lua, e)?)?;
        }
        entry.set("edits", edits)?;
        out.set(i + 1, entry)?;
    }
    Ok(out)
}

/// The full `WorkspaceEdit` as an ordered op list. Each element is a
/// table tagged by `op`: `"edit"` (`uri`, `edits`), `"create"`
/// (`uri`, `overwrite`, `ignore_if_exists`), `"rename"` (`old_uri`,
/// `new_uri`, `overwrite`, `ignore_if_exists`), or `"delete"` (`uri`,
/// `recursive`, `ignore_if_not_exists`). Order is the server's.
fn workspace_ops_to_lua(lua: &Lua, r: &WorkspaceEditResponse) -> mlua::Result<Table> {
    let out = lua.create_table_with_capacity(r.ops.len(), 0)?;
    for (i, op) in r.ops.iter().enumerate() {
        let t = lua.create_table()?;
        match op {
            WorkspaceOp::Edit(f) => {
                t.set("op", "edit")?;
                t.set("uri", f.uri.as_str())?;
                let edits = lua.create_table_with_capacity(f.edits.len(), 0)?;
                for (j, e) in f.edits.iter().enumerate() {
                    edits.set(j + 1, text_edit_to_lua(lua, e)?)?;
                }
                t.set("edits", edits)?;
            }
            WorkspaceOp::Create {
                uri,
                overwrite,
                ignore_if_exists,
            } => {
                t.set("op", "create")?;
                t.set("uri", uri.as_str())?;
                t.set("overwrite", *overwrite)?;
                t.set("ignore_if_exists", *ignore_if_exists)?;
            }
            WorkspaceOp::Rename {
                old_uri,
                new_uri,
                overwrite,
                ignore_if_exists,
            } => {
                t.set("op", "rename")?;
                t.set("old_uri", old_uri.as_str())?;
                t.set("new_uri", new_uri.as_str())?;
                t.set("overwrite", *overwrite)?;
                t.set("ignore_if_exists", *ignore_if_exists)?;
            }
            WorkspaceOp::Delete {
                uri,
                recursive,
                ignore_if_not_exists,
            } => {
                t.set("op", "delete")?;
                t.set("uri", uri.as_str())?;
                t.set("recursive", *recursive)?;
                t.set("ignore_if_not_exists", *ignore_if_not_exists)?;
            }
        }
        out.set(i + 1, t)?;
    }
    Ok(out)
}

/// Install `pmacs.rename.*`. `ops(sid, uri)` returns the parsed
/// `WorkspaceEdit` as an ordered op list (T M4.5 L4 — edits and
/// resource ops interleaved exactly as the server sent them);
/// `file_edits(sid, uri)` is the edit-only back-compat view (`{ {
/// uri =, edits = { … } }, … }`); `clear(sid, uri)` drops the entry.
pub fn install_rename(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "ops",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().rename_store();
                let guard = store_handle.lock().expect("rename store mutex poisoned");
                let key = RenameKey::new(id.0.raw().to_string(), uri);
                if let Some(r) = guard.get(&key) {
                    Ok(Value::Table(workspace_ops_to_lua(lua, r)?))
                } else {
                    Ok(Value::Table(lua.create_table_with_capacity(0, 0)?))
                }
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "file_edits",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().rename_store();
                let guard = store_handle.lock().expect("rename store mutex poisoned");
                let key = RenameKey::new(id.0.raw().to_string(), uri);
                if let Some(r) = guard.get(&key) {
                    Ok(Value::Table(file_edits_to_lua(lua, r)?))
                } else {
                    Ok(Value::Table(lua.create_table_with_capacity(0, 0)?))
                }
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().rename_store();
                let mut guard = store_handle.lock().expect("rename store mutex poisoned");
                guard.clear(&RenameKey::new(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }

    pmacs.set("rename", m)?;
    Ok(())
}

/// Install `pmacs.prepare_rename.*` (T M4.5). `result(sid, uri)`
/// returns `{ allowed, placeholder?, start_line?, start_col?,
/// end_line?, end_col? }` for the last `textDocument/prepareRename`,
/// or nil if none landed; `clear(sid, uri)` drops the entry. The
/// rename flow reads `allowed` to gate the prompt and `placeholder`
/// to pre-fill it.
pub fn install_prepare_rename(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "result",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().prepare_rename_store();
                let guard = store_handle
                    .lock()
                    .expect("prepare rename store mutex poisoned");
                let key = PrepareRenameKey::new(id.0.raw().to_string(), uri);
                let Some(r) = guard.get(&key) else {
                    return Ok(Value::Nil);
                };
                let t = lua.create_table_with_capacity(0, 6)?;
                t.set("allowed", r.allowed)?;
                if let Some(p) = r.placeholder.as_deref() {
                    t.set("placeholder", p)?;
                }
                if let Some((sl, sc, el, ec)) = r.range {
                    t.set("start_line", sl)?;
                    t.set("start_col", sc)?;
                    t.set("end_line", el)?;
                    t.set("end_col", ec)?;
                }
                Ok(Value::Table(t))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().prepare_rename_store();
                let mut guard = store_handle
                    .lock()
                    .expect("prepare rename store mutex poisoned");
                guard.clear(&PrepareRenameKey::new(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }

    pmacs.set("prepare_rename", m)?;
    Ok(())
}

fn code_action_item_to_lua(lua: &Lua, a: &CodeActionItem) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 4)?;
    t.set("title", a.title.as_str())?;
    if let Some(k) = a.kind.as_deref() {
        t.set("kind", k)?;
    }
    t.set("has_edit", a.has_edit())?;
    // Always present (possibly empty) so Lua can `#item.edit`. The
    // ordered-op shape, identical to `pmacs.rename.ops`.
    t.set("edit", workspace_ops_to_lua(lua, &a.edit)?)?;
    if let Some(c) = a.command.as_ref() {
        let ct = lua.create_table_with_capacity(0, 3)?;
        ct.set("command", c.command.as_str())?;
        ct.set("title", c.title.as_str())?;
        let args = lua.create_table_with_capacity(c.arguments.len(), 0)?;
        for (i, v) in c.arguments.iter().enumerate() {
            args.set(i + 1, json_to_lua(lua, v)?)?;
        }
        ct.set("arguments", args)?;
        t.set("command", ct)?;
    }
    Ok(t)
}

/// Install `pmacs.code_action.*` (T M4.5 L3). `actions(sid, uri)`
/// returns `{ { title, kind?, has_edit, edit = { … }, command? }, … }`
/// in server order; `clear(sid, uri)` drops the entry.
pub fn install_code_action(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "actions",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().code_action_store();
                let guard = store_handle
                    .lock()
                    .expect("code action store mutex poisoned");
                let key = CodeActionKey::new(id.0.raw().to_string(), uri);
                let out = lua.create_table()?;
                if let Some(r) = guard.get(&key) {
                    for (i, a) in r.actions.iter().enumerate() {
                        out.set(i + 1, code_action_item_to_lua(lua, a)?)?;
                    }
                }
                Ok(Value::Table(out))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().code_action_store();
                let mut guard = store_handle
                    .lock()
                    .expect("code action store mutex poisoned");
                guard.clear(&CodeActionKey::new(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }

    pmacs.set("code_action", m)?;
    Ok(())
}

fn inlay_hint_to_lua(lua: &Lua, h: &LspInlayHint) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 7)?;
    t.set("line", h.line)?;
    t.set("col", h.col)?;
    t.set("label", h.label.as_str())?;
    if let Some(k) = h.kind {
        t.set("kind", k.as_str())?;
    }
    t.set("padding_left", h.padding_left)?;
    t.set("padding_right", h.padding_right)?;
    if let Some(tt) = h.tooltip.as_deref() {
        t.set("tooltip", tt)?;
    }
    Ok(t)
}

/// Install `pmacs.inlay_hint.*` (T M4.5). `hints(sid, uri)` returns
/// `{ { line, col, label, kind?, padding_left, padding_right,
/// tooltip? }, … }` in server order; `clear(sid, uri)` drops the
/// entry. No renderer here — that is a separate milestone; this is
/// the data surface a render layer (or a list view) reads.
pub fn install_inlay_hint(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "hints",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().inlay_hint_store();
                let guard = store_handle
                    .lock()
                    .expect("inlay hint store mutex poisoned");
                let key = InlayHintKey::new(id.0.raw().to_string(), uri);
                let out = lua.create_table()?;
                if let Some(r) = guard.get(&key) {
                    for (i, h) in r.hints.iter().enumerate() {
                        out.set(i + 1, inlay_hint_to_lua(lua, h)?)?;
                    }
                }
                Ok(Value::Table(out))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().inlay_hint_store();
                let mut guard = store_handle
                    .lock()
                    .expect("inlay hint store mutex poisoned");
                guard.clear(&InlayHintKey::new(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }

    pmacs.set("inlay_hint", m)?;
    Ok(())
}

fn semantic_token_to_lua(lua: &Lua, t: &LspSemanticToken) -> mlua::Result<Table> {
    let out = lua.create_table_with_capacity(0, 5)?;
    out.set("line", t.line)?;
    out.set("start", t.start)?;
    out.set("length", t.length)?;
    out.set("token_type", t.token_type)?;
    out.set("token_modifiers", t.token_modifiers)?;
    Ok(out)
}

/// Install `pmacs.semantic_tokens.*` (T M4.5). `tokens(sid, uri)`
/// returns the decoded absolute tokens `{ { line, start, length,
/// token_type, token_modifiers }, … }`; `legend(sid)` returns
/// `{ token_types = {…}, token_modifiers = {…} }` from the server's
/// advertised `semanticTokensProvider.legend` (or nil), so callers
/// resolve the `token_type` index / `token_modifiers` bitset;
/// `clear(sid, uri)` drops the entry. No renderer here — wiring LSP
/// tokens into styling is a separate rendering milestone; this is the
/// data surface that work reads.
pub fn install_semantic_tokens(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m = lua.create_table()?;

    {
        let mgr = manager.clone();
        m.set(
            "tokens",
            lua.create_function(move |lua, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().semantic_token_store();
                let guard = store_handle
                    .lock()
                    .expect("semantic token store mutex poisoned");
                let key = SemanticTokenKey::new(id.0.raw().to_string(), uri);
                let out = lua.create_table()?;
                if let Some(r) = guard.get(&key) {
                    for (i, t) in r.tokens.iter().enumerate() {
                        out.set(i + 1, semantic_token_to_lua(lua, t)?)?;
                    }
                }
                Ok(Value::Table(out))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "legend",
            lua.create_function(move |lua, id: LspServerIdLua| {
                // Parse out an owned legend inside the borrow, then
                // build the Lua table once the manager borrow is
                // dropped.
                let parsed = {
                    let guard = mgr.borrow();
                    guard
                        .capabilities(id.0)
                        .and_then(SemanticTokensLegend::from_capabilities)
                };
                let Some(legend) = parsed else {
                    return Ok(Value::Nil);
                };
                let to_arr = |names: &[String]| -> mlua::Result<Table> {
                    let t = lua.create_table_with_capacity(names.len(), 0)?;
                    for (i, n) in names.iter().enumerate() {
                        t.set(i + 1, n.as_str())?;
                    }
                    Ok(t)
                };
                let out = lua.create_table_with_capacity(0, 2)?;
                out.set("token_types", to_arr(&legend.token_types)?)?;
                out.set("token_modifiers", to_arr(&legend.token_modifiers)?)?;
                Ok(Value::Table(out))
            })?,
        )?;
    }

    {
        // The opaque server cursor from the last full/delta response,
        // or nil. Pass it as the `previousResultId` of the next
        // `/full/delta` request.
        let mgr = manager.clone();
        m.set(
            "result_id",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().semantic_token_store();
                let guard = store_handle
                    .lock()
                    .expect("semantic token store mutex poisoned");
                let key = SemanticTokenKey::new(id.0.raw().to_string(), uri);
                Ok(guard.get(&key).and_then(|r| r.result_id.clone()))
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        m.set(
            "clear",
            lua.create_function(move |_, (id, uri): (LspServerIdLua, String)| {
                let store_handle = mgr.borrow().semantic_token_store();
                let mut guard = store_handle
                    .lock()
                    .expect("semantic token store mutex poisoned");
                guard.clear(&SemanticTokenKey::new(id.0.raw().to_string(), uri));
                Ok(())
            })?,
        )?;
    }

    pmacs.set("semantic_tokens", m)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// pmacs.project: project model (T M4.9)
// ---------------------------------------------------------------------------

use crate::project::{Project, ProjectId, ProjectKind, Workspace};

/// Cheaply-cloneable shared workspace.
pub type SharedWorkspace = Rc<RefCell<Workspace>>;

/// Userdata wrapper around [`ProjectId`] so Lua callers can pass
/// project ids around opaquely. Mirrors `LspServerIdLua`.
#[derive(Copy, Clone)]
pub struct ProjectIdLua(pub ProjectId);

impl FromLua for ProjectIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "ProjectIdLua".to_string(),
                message: Some("expected a project handle".to_string()),
            }),
        }
    }
}

impl UserData for ProjectIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("raw", |_, this, ()| Ok(this.0.raw()));
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}", this.0))
        });
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: ProjectIdLua| {
            Ok(this.0 == other.0)
        });
    }
}

fn project_kind_from_tag(tag: &str) -> ProjectKind {
    match tag {
        "rust" => ProjectKind::Rust,
        "lua" => ProjectKind::Lua,
        "node" => ProjectKind::Node,
        "python" => ProjectKind::Python,
        "go" => ProjectKind::Go,
        "deno" => ProjectKind::Deno,
        "git" => ProjectKind::Git,
        other => ProjectKind::Custom(other.to_owned()),
    }
}

fn project_to_lua(lua: &Lua, p: &Project) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 5)?;
    t.set("id", ProjectIdLua(p.id))?;
    t.set("root", p.root.display().to_string())?;
    t.set("kind", p.kind.tag())?;
    t.set("name", p.name.as_str())?;
    t.set("language_id", p.kind.default_language_id())?;
    Ok(t)
}

/// Install `pmacs.project.*` (T M4.9).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of project bindings; splitting fragments a coherent surface"
)]
pub fn install_project(
    lua: &Lua,
    workspace: &SharedWorkspace,
    lsp_manager: &SharedLspManager,
) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    // Preserve keys (e.g. `pmacs.project.search` from default.lua)
    // installed by builtin Lua chunks before us. We add to the
    // existing table when present, otherwise create a fresh one.
    let m: Table = match pmacs.get::<Option<Table>>("project")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        let ws = workspace.clone();
        m.set(
            "detect",
            lua.create_function(move |lua, file_path: String| {
                let ws_ref = ws.borrow();
                let Some((root, kind)) = ws_ref.detect(std::path::Path::new(&file_path)) else {
                    return Ok(Value::Nil);
                };
                let t = lua.create_table_with_capacity(0, 3)?;
                t.set("root", root.display().to_string())?;
                t.set("kind", kind.tag())?;
                t.set("language_id", kind.default_language_id())?;
                Ok(Value::Table(t))
            })?,
        )?;
    }

    {
        // pmacs.project.set_search_boundary(path | nil)
        //
        // Clamp the upward marker walk performed by `detect`. With
        // `nil` (the default), detection walks ancestors all the way
        // to the filesystem root, matching `git rev-parse
        // --show-toplevel` semantics. Setting a path clamps the walk
        // so ancestors above that path are not consulted, which lets
        // a user with a stray marker high in the tree (e.g.,
        // `/tmp/.git`, an orphaned `.git` in `~`) bound detection to
        // their project home (e.g., `~/code`).
        //
        // Function-call form (not assignable property) so the
        // side-effect of changing detection behavior is visible at
        // the call site, matching the convention of `pmacs.attach`,
        // `pmacs.packages.install`, etc.
        let ws = workspace.clone();
        m.set(
            "set_search_boundary",
            lua.create_function(move |_, path: Option<String>| {
                let p = path.map(std::path::PathBuf::from);
                ws.borrow_mut().set_search_boundary(p);
                Ok(())
            })?,
        )?;
    }

    {
        // pmacs.project.search_boundary() → string | nil
        //
        // Read back the current boundary as a path string, or `nil`
        // when no boundary is set. Returns the canonicalized form
        // (the boundary is canonicalized at set time).
        let ws = workspace.clone();
        m.set(
            "search_boundary",
            lua.create_function(move |_, ()| -> mlua::Result<Option<String>> {
                Ok(ws
                    .borrow()
                    .search_boundary()
                    .map(|p| p.display().to_string()))
            })?,
        )?;
    }

    {
        // open(root [, kind, name]) → ProjectId. If `kind` is omitted,
        // detection runs on `root` itself; if no marker matches, a
        // Custom("generic") kind is used so the call still succeeds.
        let ws = workspace.clone();
        m.set(
            "open",
            lua.create_function(
                move |_, (root, kind, name): (String, Option<String>, Option<String>)| {
                    let path = std::path::PathBuf::from(&root);
                    let mut ws_ref = ws.borrow_mut();
                    let resolved_kind = match kind {
                        Some(tag) => project_kind_from_tag(&tag),
                        None => match crate::project::detect_project(&path, ws_ref.markers()) {
                            Some((_, k)) => k,
                            None => ProjectKind::Custom("generic".into()),
                        },
                    };
                    let id = ws_ref.open(path, resolved_kind, name);
                    Ok(ProjectIdLua(id))
                },
            )?,
        )?;
    }

    {
        // open_for_file(file_path) → ProjectId | nil.
        let ws = workspace.clone();
        m.set(
            "open_for_file",
            lua.create_function(move |lua, file_path: String| {
                let mut ws_ref = ws.borrow_mut();
                match ws_ref.open_for_file(std::path::Path::new(&file_path)) {
                    Some(id) => Ok(Value::UserData(lua.create_userdata(ProjectIdLua(id))?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let ws = workspace.clone();
        m.set(
            "switch",
            lua.create_function(move |_, id: ProjectIdLua| {
                ws.borrow_mut()
                    .set_active(id.0)
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let ws = workspace.clone();
        m.set(
            "close",
            lua.create_function(move |_, id: ProjectIdLua| {
                ws.borrow_mut().close(id.0);
                Ok(())
            })?,
        )?;
    }

    {
        let ws = workspace.clone();
        m.set(
            "active",
            lua.create_function(move |lua, ()| {
                let ws_ref = ws.borrow();
                match ws_ref.active() {
                    Some(p) => Ok(Value::Table(project_to_lua(lua, p)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        let ws = workspace.clone();
        m.set(
            "list",
            lua.create_function(move |lua, ()| {
                let ws_ref = ws.borrow();
                let projects: Vec<Project> = ws_ref.projects().cloned().collect();
                let out = lua.create_table_with_capacity(projects.len(), 0)?;
                for (i, p) in projects.iter().enumerate() {
                    out.set(i + 1, project_to_lua(lua, p)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let ws = workspace.clone();
        m.set(
            "get",
            lua.create_function(move |lua, id: ProjectIdLua| {
                let ws_ref = ws.borrow();
                match ws_ref.get(id.0) {
                    Some(p) => Ok(Value::Table(project_to_lua(lua, p)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        // Get-or-spawn the LSP server scoped to (project_id,
        // language_id). Caller passes a spec table whose `cwd` /
        // `root_uri` will be defaulted from the project root if
        // they're omitted. Returns an LspServerIdLua.
        let ws = workspace.clone();
        let mgr = lsp_manager.clone();
        m.set(
            "lsp_for",
            lua.create_function(
                move |_, (id, language_id, spec_t): (ProjectIdLua, Option<String>, Table)| {
                    let ws_ref = ws.borrow();
                    let project = ws_ref
                        .get(id.0)
                        .ok_or_else(|| mlua::Error::external(format!("unknown project: {}", id.0)))?
                        .clone();
                    drop(ws_ref);
                    let language_id = language_id
                        .unwrap_or_else(|| project.kind.default_language_id().to_owned());
                    let template = lua_to_lsp_spec(&spec_t)?;
                    let sid = mgr
                        .borrow_mut()
                        .ensure_server_for_project(project.root, language_id, template)
                        .map_err(mlua::Error::external)?;
                    Ok(LspServerIdLua(sid))
                },
            )?,
        )?;
    }

    pmacs.set("project", m)?;
    Ok(())
}

/// Build a fresh [`Workspace`] and install `pmacs.project.*` over it.
/// Mirrors [`make_lsp_manager`] in shape.
pub fn make_workspace(lua: &Lua, lsp_manager: &SharedLspManager) -> mlua::Result<SharedWorkspace> {
    let ws: SharedWorkspace = Rc::new(RefCell::new(Workspace::new()));
    install_project(lua, &ws, lsp_manager)?;
    Ok(ws)
}

// ---------------------------------------------------------------------------
// pmacs.completion: unified completion framework (T M4.11)
// ---------------------------------------------------------------------------

use crate::completion_framework::{
    CompletionCandidate, CompletionContext, CompletionRegistry, CompletionTrigger, ProviderFn,
    ProviderId, SharedCompletionRegistry, SharedSnippetRegistry, Snippet, SnippetRegistry,
    dabbrev_provider, lsp_completion_provider, project_symbols_provider, snippet_provider,
};

fn completion_kind_tag(kind: crate::completion::CompletionItemKind) -> &'static str {
    use crate::completion::CompletionItemKind as K;
    match kind {
        K::Text => "text",
        K::Method => "method",
        K::Function => "function",
        K::Constructor => "constructor",
        K::Field => "field",
        K::Variable => "variable",
        K::Class => "class",
        K::Interface => "interface",
        K::Module => "module",
        K::Property => "property",
        K::Unit => "unit",
        K::Value => "value",
        K::Enum => "enum",
        K::Keyword => "keyword",
        K::Snippet => "snippet",
        K::Color => "color",
        K::File => "file",
        K::Reference => "reference",
        K::Folder => "folder",
        K::EnumMember => "enum_member",
        K::Constant => "constant",
        K::Struct => "struct",
        K::Event => "event",
        K::Operator => "operator",
        K::TypeParameter => "type_parameter",
    }
}

fn completion_kind_from_tag(tag: &str) -> crate::completion::CompletionItemKind {
    use crate::completion::CompletionItemKind as K;
    match tag {
        "method" => K::Method,
        "function" => K::Function,
        "constructor" => K::Constructor,
        "field" => K::Field,
        "variable" => K::Variable,
        "class" => K::Class,
        "interface" | "trait" => K::Interface,
        "module" | "namespace" => K::Module,
        "property" => K::Property,
        "unit" => K::Unit,
        "value" => K::Value,
        "enum" => K::Enum,
        "keyword" => K::Keyword,
        "snippet" => K::Snippet,
        "color" => K::Color,
        "file" => K::File,
        "reference" => K::Reference,
        "folder" => K::Folder,
        "enum_member" | "enummember" => K::EnumMember,
        "constant" => K::Constant,
        "struct" => K::Struct,
        "event" => K::Event,
        "operator" => K::Operator,
        "type_parameter" | "typeparameter" => K::TypeParameter,
        _ => K::Text,
    }
}

fn lua_table_to_completion_item(t: &Table) -> mlua::Result<crate::completion::CompletionItem> {
    let label: String = t.get("label")?;
    let kind_tag: Option<String> = t.get("kind").ok().flatten();
    let kind = kind_tag.as_deref().map_or(
        crate::completion::CompletionItemKind::Text,
        completion_kind_from_tag,
    );
    let detail: Option<String> = t.get("detail").ok().flatten();
    let documentation: Option<String> = t.get("documentation").ok().flatten();
    let insert_text: Option<String> = t.get("insert_text").ok().flatten();
    let sort_text: Option<String> = t.get("sort_text").ok().flatten();
    let filter_text: Option<String> = t.get("filter_text").ok().flatten();
    Ok(crate::completion::CompletionItem {
        label,
        kind,
        detail,
        documentation,
        insert_text,
        sort_text,
        filter_text,
    })
}

fn ctx_to_lua(lua: &Lua, ctx: &CompletionContext) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 8)?;
    t.set("prefix", ctx.prefix.as_str())?;
    t.set("line", ctx.line)?;
    t.set("col", ctx.col)?;
    t.set("buffer_text", ctx.buffer_text.as_ref())?;
    if let Some(l) = &ctx.language {
        t.set("language", l.as_str())?;
    }
    if let Some(p) = &ctx.project_root {
        t.set("project_root", p.display().to_string())?;
    }
    if let Some(u) = &ctx.uri {
        t.set("uri", u.as_str())?;
    }
    let (trigger_tag, trigger_char): (&'static str, Option<String>) = match ctx.trigger {
        CompletionTrigger::Invoked => ("invoked", None),
        CompletionTrigger::Char(c) => ("char", Some(c.to_string())),
        CompletionTrigger::Incomplete => ("incomplete", None),
    };
    t.set("trigger", trigger_tag)?;
    if let Some(c) = trigger_char {
        t.set("trigger_char", c)?;
    }
    Ok(t)
}

fn lua_table_to_ctx(t: &Table) -> CompletionContext {
    let prefix: String = t.get("prefix").unwrap_or_default();
    let line: u32 = t.get("line").unwrap_or(0);
    let col: u32 = t.get("col").unwrap_or(0);
    let buffer_text: String = t.get("buffer_text").unwrap_or_default();
    let language: Option<String> = t.get::<Option<String>>("language").ok().flatten();
    let project_root: Option<String> = t.get::<Option<String>>("project_root").ok().flatten();
    let trigger_tag: Option<String> = t.get::<Option<String>>("trigger").ok().flatten();
    let trigger_char: Option<String> = t.get::<Option<String>>("trigger_char").ok().flatten();
    let trigger = match trigger_tag.as_deref() {
        Some("char") => trigger_char
            .and_then(|s| s.chars().next())
            .map_or(CompletionTrigger::Invoked, CompletionTrigger::Char),
        Some("incomplete") => CompletionTrigger::Incomplete,
        _ => CompletionTrigger::Invoked,
    };
    let uri: Option<String> = t.get::<Option<String>>("uri").ok().flatten();
    CompletionContext {
        prefix,
        line,
        col,
        buffer_text: Rc::from(buffer_text),
        language,
        project_root: project_root.map(std::path::PathBuf::from),
        trigger,
        uri,
    }
}

/// Userdata wrapper around [`ProviderId`] so Lua callers can pass
/// completion-provider handles around opaquely. Mirrors
/// [`ProjectIdLua`] / [`LspServerIdLua`].
#[derive(Copy, Clone)]
pub struct ProviderIdLua(pub ProviderId);

impl FromLua for ProviderIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "ProviderIdLua".to_string(),
                message: Some("expected a provider handle".to_string()),
            }),
        }
    }
}

impl UserData for ProviderIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("raw", |_, this, ()| Ok(this.0.raw()));
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}", this.0))
        });
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: ProviderIdLua| {
            Ok(this.0 == other.0)
        });
    }
}

fn candidate_to_lua(lua: &Lua, c: &CompletionCandidate) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 9)?;
    t.set("label", c.item.label.as_str())?;
    t.set("kind", completion_kind_tag(c.item.kind))?;
    if let Some(d) = &c.item.detail {
        t.set("detail", d.as_str())?;
    }
    if let Some(d) = &c.item.documentation {
        t.set("documentation", d.as_str())?;
    }
    t.set("insert_text", c.insert_text())?;
    if let Some(s) = &c.item.sort_text {
        t.set("sort_text", s.as_str())?;
    }
    if let Some(f) = &c.item.filter_text {
        t.set("filter_text", f.as_str())?;
    }
    t.set("source", c.source.as_str())?;
    t.set("priority", c.priority)?;
    t.set("score", c.score)?;
    Ok(t)
}

fn install_completion_snippets(
    pmacs_completion: &Table,
    lua: &Lua,
    snippets: &SharedSnippetRegistry,
) -> mlua::Result<()> {
    let m: Table = match pmacs_completion.get::<Option<Table>>("snippets")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        let s = snippets.clone();
        m.set(
            "add",
            lua.create_function(move |_, spec: Table| {
                let name: String = spec.get("name")?;
                let prefix: String = spec.get("prefix").unwrap_or_else(|_| name.clone());
                let body: String = spec.get("body")?;
                let description: Option<String> = spec.get("description").ok().flatten();
                let scope: Option<String> = spec.get("scope").ok().flatten();
                s.borrow_mut().add(Snippet {
                    name,
                    prefix,
                    body,
                    description,
                    scope,
                });
                Ok(())
            })?,
        )?;
    }

    {
        let s = snippets.clone();
        m.set(
            "remove",
            lua.create_function(move |_, name: String| Ok(s.borrow_mut().remove(&name)))?,
        )?;
    }

    {
        let s = snippets.clone();
        m.set(
            "list",
            lua.create_function(move |lua, ()| {
                let snips = s.borrow();
                let listed = snips.list();
                let out = lua.create_table_with_capacity(listed.len(), 0)?;
                for (i, sn) in listed.iter().enumerate() {
                    let t = lua.create_table_with_capacity(0, 5)?;
                    t.set("name", sn.name.as_str())?;
                    t.set("prefix", sn.prefix.as_str())?;
                    t.set("body", sn.body.as_str())?;
                    if let Some(d) = &sn.description {
                        t.set("description", d.as_str())?;
                    }
                    if let Some(sc) = &sn.scope {
                        t.set("scope", sc.as_str())?;
                    }
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let s = snippets.clone();
        m.set(
            "find",
            lua.create_function(move |lua, (prefix, language): (String, Option<String>)| {
                let snips = s.borrow();
                let found = snips.find(&prefix, language.as_deref());
                let out = lua.create_table_with_capacity(found.len(), 0)?;
                for (i, sn) in found.iter().enumerate() {
                    let t = lua.create_table_with_capacity(0, 5)?;
                    t.set("name", sn.name.as_str())?;
                    t.set("prefix", sn.prefix.as_str())?;
                    t.set("body", sn.body.as_str())?;
                    if let Some(d) = &sn.description {
                        t.set("description", d.as_str())?;
                    }
                    if let Some(sc) = &sn.scope {
                        t.set("scope", sc.as_str())?;
                    }
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
    }

    pmacs_completion.set("snippets", m)?;
    Ok(())
}

/// Install `pmacs.completion.*` (T M4.11). Preserves any pre-
/// existing keys (`pmacs.completion.search`-style helpers shipped
/// by `default.lua` chunks).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of completion-framework bindings; splitting fragments a coherent surface"
)]
pub fn install_completion_framework(
    lua: &Lua,
    registry: &SharedCompletionRegistry,
    snippets: &SharedSnippetRegistry,
) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m: Table = match pmacs.get::<Option<Table>>("completion")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        // register({ name, priority?, fn }) -> ProviderIdLua
        let reg = registry.clone();
        m.set(
            "register",
            lua.create_function(move |_, spec: Table| {
                let name: String = spec.get("name")?;
                let priority: i32 = spec.get("priority").unwrap_or(0);
                let lua_fn: Function = spec.get("fn")?;
                let provider: ProviderFn = Box::new(move |ctx: &CompletionContext| {
                    let result: mlua::Result<Vec<Table>> = lua_fn.call(lua_compat_ctx_args(ctx));
                    match result {
                        Ok(tables) => tables
                            .iter()
                            .filter_map(|t| lua_table_to_completion_item(t).ok())
                            .collect(),
                        Err(_) => Vec::new(),
                    }
                });
                let id = reg.borrow_mut().register(name, priority, provider);
                Ok(ProviderIdLua(id))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        m.set(
            "unregister",
            lua.create_function(move |_, id: ProviderIdLua| Ok(reg.borrow_mut().unregister(id.0)))?,
        )?;
    }

    {
        let reg = registry.clone();
        m.set(
            "set_priority",
            lua.create_function(move |_, (id, priority): (ProviderIdLua, i32)| {
                Ok(reg.borrow_mut().set_priority(id.0, priority))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        m.set(
            "set_enabled",
            lua.create_function(move |_, (id, enabled): (ProviderIdLua, bool)| {
                Ok(reg.borrow_mut().set_enabled(id.0, enabled))
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        m.set(
            "providers",
            lua.create_function(move |lua, ()| {
                let r = reg.borrow();
                let out = lua.create_table_with_capacity(r.len(), 0)?;
                for (i, p) in r.providers().iter().enumerate() {
                    let t = lua.create_table_with_capacity(0, 4)?;
                    t.set("id", ProviderIdLua(p.id))?;
                    t.set("name", p.name.as_str())?;
                    t.set("priority", p.priority)?;
                    t.set("enabled", p.enabled)?;
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        m.set(
            "collect",
            lua.create_function(move |lua, ctx_t: Table| {
                let ctx = lua_table_to_ctx(&ctx_t);
                let cands = reg.borrow().collect(&ctx);
                let out = lua.create_table_with_capacity(cands.len(), 0)?;
                for (i, c) in cands.iter().enumerate() {
                    out.set(i + 1, candidate_to_lua(lua, c)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // ctx_to_lua exposed as `pmacs.completion.context_for(...)`
        // for callers that need to construct a context table from
        // primitives. Convenience only --- callers can build their
        // own. Trailing optionals: a "char" trigger needs its
        // `trigger_char` (Q#C1 nit --- the helper previously could
        // not express `CompletionTrigger::Char` at all), and `uri`
        // scopes URI-keyed providers (Q#C8).
        m.set(
            "context_for",
            lua.create_function(move |lua, args: ContextForArgs| {
                let (prefix, line, col, buffer_text, language, project_root, trigger, ch, uri) =
                    args;
                let trigger = match trigger.as_deref() {
                    Some("incomplete") => CompletionTrigger::Incomplete,
                    Some("char") => ch
                        .and_then(|s| s.chars().next())
                        .map_or(CompletionTrigger::Invoked, CompletionTrigger::Char),
                    _ => CompletionTrigger::Invoked,
                };
                let ctx = CompletionContext {
                    prefix,
                    line: line.unwrap_or(0),
                    col: col.unwrap_or(0),
                    buffer_text: Rc::from(buffer_text.unwrap_or_default()),
                    language,
                    project_root: project_root.map(std::path::PathBuf::from),
                    trigger,
                    uri,
                };
                ctx_to_lua(lua, &ctx)
            })?,
        )?;
    }

    install_completion_snippets(&m, lua, snippets)?;

    pmacs.set("completion", m)?;
    Ok(())
}

/// Build a fresh registry + snippet registry, install
/// `pmacs.completion.*`, register the four built-in providers
/// (dabbrev, snippets, project symbols, LSP) at sensible default
/// priorities, and return the shared handles.
pub fn make_completion_framework(
    lua: &Lua,
    lsp_manager: &SharedLspManager,
    indexer: &SharedProjectIndexer,
) -> mlua::Result<(SharedCompletionRegistry, SharedSnippetRegistry)> {
    let registry: SharedCompletionRegistry = Rc::new(RefCell::new(CompletionRegistry::new()));
    let snippets: SharedSnippetRegistry = Rc::new(RefCell::new(SnippetRegistry::new()));
    install_completion_framework(lua, &registry, &snippets)?;
    {
        let mut r = registry.borrow_mut();
        // Default priorities: LSP highest (most precise), then
        // snippets (user-curated), then project symbols (cheap),
        // then dabbrev (last-resort fallback).
        r.register("lsp", 100, lsp_completion_provider(lsp_manager.clone()));
        r.register("snippets", 80, snippet_provider(snippets.clone()));
        r.register(
            "project_symbols",
            60,
            project_symbols_provider(indexer.clone()),
        );
        r.register("dabbrev", 20, dabbrev_provider());
    }
    Ok((registry, snippets))
}

/// Install the in-buffer completion popup surface (Arc 1a, Q#C2) into
/// `pmacs.completion`: `popup_show{...}` publishes a session into the
/// core's shared popup (the Lua driver's write path), `popup_hide()`
/// closes it, `popup_visible()` peeks. Separate from
/// [`install_completion_framework`] because these need the
/// [`SharedCore`], which only exists once the editor attaches.
pub fn install_completion_popup(lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m: Table = match pmacs.get::<Option<Table>>("completion")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        // popup_show{ buffer, anchor, prefix?, total?, candidates = {
        //   { label, kind?, detail?, insert_text? }, ... } } -> bool
        //
        // Returns false (popup left closed) for an empty candidate
        // list. `kind` uses the same string tags as
        // `pmacs.completion.collect` rows, so driver code can pass
        // collect() output straight through.
        let cc = core.clone();
        m.set(
            "popup_show",
            lua.create_function(move |_, spec: Table| {
                let buffer: BufferIdLua = spec.get("buffer")?;
                let anchor: u64 = spec.get("anchor")?;
                let prefix: String = spec
                    .get::<Option<String>>("prefix")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let rows: Table = spec.get("candidates")?;
                let mut candidates = Vec::new();
                for row in rows.sequence_values::<Table>() {
                    let row = row?;
                    let label: String = row.get("label")?;
                    let kind_tag: Option<String> = row.get::<Option<String>>("kind").ok().flatten();
                    let kind = kind_tag.as_deref().map_or(
                        crate::completion::CompletionItemKind::Text,
                        completion_kind_from_tag,
                    );
                    let detail: Option<String> = row.get::<Option<String>>("detail").ok().flatten();
                    let insert_text: Option<String> =
                        row.get::<Option<String>>("insert_text").ok().flatten();
                    candidates.push(crate::completion::PopupCandidate {
                        insert_text: insert_text.unwrap_or_else(|| label.clone()),
                        label,
                        kind,
                        detail,
                    });
                }
                let total: usize = spec
                    .get::<Option<usize>>("total")
                    .ok()
                    .flatten()
                    .unwrap_or(candidates.len());
                let Some(state) = crate::completion::CompletionPopupState::new(
                    buffer.0, anchor, prefix, candidates, total,
                ) else {
                    return Ok(false);
                };
                cc.borrow_mut().completion_popup_open(state);
                Ok(true)
            })?,
        )?;
    }

    {
        let cc = core.clone();
        m.set(
            "popup_hide",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().completion_popup_close();
                Ok(())
            })?,
        )?;
    }

    {
        let cc = core.clone();
        m.set(
            "popup_visible",
            lua.create_function(move |_, ()| Ok(cc.borrow().completion_popup_is_open()))?,
        )?;
    }

    pmacs.set("completion", m)?;
    Ok(())
}

/// Argument tuple passed to a Lua-registered completion provider.
/// Positional rather than table-based because the provider closure
/// has no `&Lua` to build a table with at call time.
type LuaProviderArgs = (
    String,
    u32,
    u32,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

/// Argument tuple for `pmacs.completion.context_for`: positional
/// primitives that mlua can deserialize without a custom `IntoLua`.
type ContextForArgs = (
    String,
    Option<u32>,
    Option<u32>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// mlua doesn't accept arbitrary Rust types as call arguments
/// without `IntoLua`, so the registered Lua provider closure
/// passes a positional tuple of primitives. The Lua callable
/// receives them as: `(prefix, line, col, buffer_text, language,
/// project_root, trigger, trigger_char)`.
fn lua_compat_ctx_args(ctx: &CompletionContext) -> LuaProviderArgs {
    let (trigger_tag, trigger_char): (&'static str, Option<String>) = match ctx.trigger {
        CompletionTrigger::Invoked => ("invoked", None),
        CompletionTrigger::Char(c) => ("char", Some(c.to_string())),
        CompletionTrigger::Incomplete => ("incomplete", None),
    };
    (
        ctx.prefix.clone(),
        ctx.line,
        ctx.col,
        ctx.buffer_text.to_string(),
        ctx.language.clone(),
        ctx.project_root.as_ref().map(|p| p.display().to_string()),
        trigger_tag.to_owned(),
        trigger_char,
        // Trailing addition (Q#C8): existing Lua providers that
        // ignore the ninth positional arg are unaffected.
        ctx.uri.clone(),
    )
}

// ---------------------------------------------------------------------------
// pmacs.window: split tree + focus management (T M2.8).
// ---------------------------------------------------------------------------

#[allow(
    clippy::too_many_lines,
    reason = "ten window-module bindings each follow the same Rc-borrow pattern; splitting them into helpers adds ceremony without clarity"
)]
fn install_window_module(lua: &Lua, core: &SharedCore) -> mlua::Result<Table> {
    let win = lua.create_table()?;

    {
        let cc = core.clone();
        win.set(
            "split_horizontal",
            lua.create_function(move |_, ()| {
                let new_id = cc
                    .borrow_mut()
                    .split_active(crate::window::Orientation::Horizontal, true);
                Ok(new_id.raw())
            })?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "split_vertical",
            lua.create_function(move |_, ()| {
                let new_id = cc
                    .borrow_mut()
                    .split_active(crate::window::Orientation::Vertical, true);
                Ok(new_id.raw())
            })?,
        )?;
    }

    {
        // UX gutter: set the active window's line-number mode
        // ("off" | "absolute"). Per-window (Q#UX5); a friendly toggle
        // command wraps this in `builtin/`.
        let cc = core.clone();
        win.set(
            "set_line_numbers",
            lua.create_function(move |_, mode: String| {
                let m = match mode.as_str() {
                    "off" | "none" => crate::window::LineNumberMode::Off,
                    "absolute" | "abs" | "on" => crate::window::LineNumberMode::Absolute,
                    "relative" | "rel" => crate::window::LineNumberMode::Relative,
                    "hybrid" => crate::window::LineNumberMode::Hybrid,
                    other => {
                        return Err(mlua::Error::external(format!(
                            "unknown line-number mode {other:?} \
                             (expected off|absolute|relative|hybrid)"
                        )));
                    }
                };
                cc.borrow_mut().active_window_mut().line_numbers = m;
                Ok(())
            })?,
        )?;
    }

    {
        // Read the active window's line-number mode as a string.
        let cc = core.clone();
        win.set(
            "line_numbers",
            lua.create_function(move |_, ()| {
                let mode = match cc.borrow().active_window().line_numbers {
                    crate::window::LineNumberMode::Off => "off",
                    crate::window::LineNumberMode::Absolute => "absolute",
                    crate::window::LineNumberMode::Relative => "relative",
                    crate::window::LineNumberMode::Hybrid => "hybrid",
                };
                Ok(mode)
            })?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "focus_next",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().focus_next();
                Ok(())
            })?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "focus_prev",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().focus_prev();
                Ok(())
            })?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "close",
            lua.create_function(move |_, ()| Ok(cc.borrow_mut().close_active()))?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "close_others",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().close_others();
                Ok(())
            })?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "list",
            lua.create_function(move |lua, ()| {
                let c = cc.borrow();
                let t = lua.create_table()?;
                for (i, id) in c.active_layout().iter_ids().iter().enumerate() {
                    t.set(i + 1, id.raw())?;
                }
                Ok(t)
            })?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "current",
            lua.create_function(move |_, ()| Ok(cc.borrow().active_window_id().raw()))?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "buffer",
            lua.create_function(move |_, ()| Ok(BufferIdLua(cc.borrow().active_buffer_id())))?,
        )?;
    }

    {
        let cc = core.clone();
        win.set(
            "switch_buffer",
            lua.create_function(move |lua, id: BufferIdLua| -> mlua::Result<()> {
                cc.borrow_mut()
                    .switch_active_buffer(id.0)
                    .map_err(mlua::Error::external)?;
                // Arc 1b: switching clears the window's overlays;
                // subscribers (syntax highlight, LSP style/diag views)
                // re-attach theirs here — without this, C-x b / panel
                // navigation permanently stripped styling from the
                // session.
                run_hook_if_defined(lua, "buffer.after-switch", mlua::MultiValue::new());
                Ok(())
            })?,
        )?;
    }

    // Test seam: list of overlay-`kind` strings on the active window,
    // in push order. Used by acceptance tests to verify that an
    // overlay-attaching wire-up step (e.g. T M9.7's
    // `pmacs.parse._attach_highlight` call for code/markdown prompt
    // results) actually landed an overlay of the expected kind.
    // Leading-underscore prefix marks this as test/internal surface,
    // not stable user-facing API — feature packages shouldn't depend
    // on it.
    {
        let cc = core.clone();
        win.set(
            "_overlay_kinds",
            lua.create_function(move |lua, ()| {
                let c = cc.borrow();
                let kinds = c.active_window().overlay_kinds();
                let t = lua.create_table_with_capacity(kinds.len(), 0)?;
                for (i, k) in kinds.into_iter().enumerate() {
                    t.set(i + 1, k)?;
                }
                Ok(t)
            })?,
        )?;
    }

    // Test seams for view-top introspection / poking. Used by T M9.7's
    // re-invoke test to verify the buffer's switch-on-paint resets the
    // viewport to the top. Leading-underscore — test surface only.
    {
        let cc = core.clone();
        win.set(
            "_view_top",
            lua.create_function(move |_, ()| {
                Ok(i64::try_from(cc.borrow().active_window().view_top).unwrap_or(i64::MAX))
            })?,
        )?;
    }
    {
        let cc = core.clone();
        win.set(
            "_set_view_top",
            lua.create_function(move |_, n: i64| {
                let n = usize::try_from(n).map_err(mlua::Error::external)?;
                cc.borrow_mut().active_window_mut().view_top = n;
                Ok(())
            })?,
        )?;
    }

    Ok(win)
}

fn install_motion(editor: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    register(editor, lua, core, "move_left", EditorCore::move_left)?;
    register(editor, lua, core, "move_right", EditorCore::move_right)?;
    register(editor, lua, core, "move_up", EditorCore::move_up)?;
    register(editor, lua, core, "move_down", EditorCore::move_down)?;
    register(
        editor,
        lua,
        core,
        "move_line_start",
        EditorCore::move_line_start,
    )?;
    register(
        editor,
        lua,
        core,
        "move_line_end",
        EditorCore::move_line_end,
    )?;
    register(
        editor,
        lua,
        core,
        "move_word_left",
        EditorCore::move_word_left,
    )?;
    register(
        editor,
        lua,
        core,
        "move_word_right",
        EditorCore::move_word_right,
    )?;
    register(editor, lua, core, "move_page_up", EditorCore::move_page_up)?;
    register(
        editor,
        lua,
        core,
        "move_page_down",
        EditorCore::move_page_down,
    )?;
    register(
        editor,
        lua,
        core,
        "move_paragraph_up",
        EditorCore::move_paragraph_up,
    )?;
    register(
        editor,
        lua,
        core,
        "move_paragraph_down",
        EditorCore::move_paragraph_down,
    )?;
    {
        let cc = core.clone();
        editor.set(
            "move_to_line",
            lua.create_function(move |_, line: i64| {
                let line = usize::try_from(line).map_err(mlua::Error::external)?;
                cc.borrow_mut().move_to_line(line);
                Ok(())
            })?,
        )?;
    }
    // T M4.5 L1 — jump ring. Cross-file navigation records its
    // origin via `push_jump`; `jump_back` (M-,) unwinds it.
    {
        let cc = core.clone();
        editor.set(
            "push_jump",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().push_jump();
                Ok(())
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "jump_back",
            lua.create_function(move |lua, ()| {
                let (jumped, buffer_changed) = {
                    let mut core = cc.borrow_mut();
                    let before = core.active_buffer_id();
                    let jumped = core.jump_back();
                    (jumped, core.active_buffer_id() != before)
                };
                // Parity with `pmacs.window.switch_buffer` (compile-mode
                // additions #3): a jump that lands in another buffer
                // clears the destination window's overlays exactly like
                // any other switch, so overlay subscribers need the same
                // re-attach signal. Without this, RET → M-, permanently
                // stripped a generated buffer's styling. Same-buffer
                // jumps stay hook-silent.
                if buffer_changed {
                    run_hook_if_defined(lua, "buffer.after-switch", mlua::MultiValue::new());
                }
                Ok(jumped)
            })?,
        )?;
    }
    Ok(())
}

fn install_editing(editor: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    register(editor, lua, core, "backspace", EditorCore::backspace)?;
    register(
        editor,
        lua,
        core,
        "delete_forward",
        EditorCore::delete_forward,
    )?;
    register(
        editor,
        lua,
        core,
        "delete_word_backward",
        EditorCore::delete_word_backward,
    )?;
    register(
        editor,
        lua,
        core,
        "delete_word_forward",
        EditorCore::delete_word_forward,
    )?;
    {
        let cc = core.clone();
        editor.set(
            "insert_char",
            lua.create_function(move |_, codepoint: i64| {
                let ch = char_from_lua_codepoint(codepoint)?;
                cc.borrow_mut().insert_char(ch);
                Ok(())
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "insert_char_over_region",
            lua.create_function(move |_, codepoint: i64| {
                let ch = char_from_lua_codepoint(codepoint)?;
                cc.borrow_mut().insert_char_over_region(ch);
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

fn install_history(editor: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    register(editor, lua, core, "undo", EditorCore::undo)?;
    register(editor, lua, core, "redo", EditorCore::redo)?;
    Ok(())
}

/// Install the `pmacs.editor.search_*` primitives that drive
/// incremental search (Q#SR5). The live-typing keys are intercepted in
/// Rust (`EditorState::dispatch_search_key`); these bindings exist so
/// the *entry* commands (`search.forward` / `search.backward`) and any
/// post-accept navigation commands can begin / step a search from Lua.
fn install_search(editor: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    {
        // search_start(forward, regex): begin an isearch in the given
        // direction, anchored at the active buffer + cursor. `regex`
        // selects regex vs literal substring matching (Q#RX3).
        let cc = core.clone();
        editor.set(
            "search_start",
            lua.create_function(move |_, (forward, regex): (bool, bool)| {
                cc.borrow_mut().search_begin(forward, regex);
                Ok(())
            })?,
        )?;
    }
    {
        // search_step(forward): move the active buffer's match focus
        // (works during a live search and after accept, for navigation
        // commands). No-op when the buffer has no matches.
        let cc = core.clone();
        editor.set(
            "search_step",
            lua.create_function(move |_, forward: bool| {
                cc.borrow_mut().search_step(forward);
                Ok(())
            })?,
        )?;
    }
    {
        // search_active(): true while an isearch session is running.
        let cc = core.clone();
        editor.set(
            "search_active",
            lua.create_function(move |_, ()| Ok(cc.borrow().search_active()))?,
        )?;
    }
    {
        // query_replace_start(from, to, regex): begin an interactive
        // query-replace from the cursor forward (Arc 2). The Lua
        // `query-replace` command collects `from`/`to` via chained
        // minibuffer prompts, then calls this; the interactive y/n/!/./q
        // phase is a core dispatcher shadow from here on.
        let cc = core.clone();
        editor.set(
            "query_replace_start",
            lua.create_function(move |_, (from, to, regex): (String, String, bool)| {
                cc.borrow_mut().query_replace_begin(from, to, regex);
                Ok(())
            })?,
        )?;
    }
    {
        // query_replace_active(): true during the interactive phase.
        let cc = core.clone();
        editor.set(
            "query_replace_active",
            lua.create_function(move |_, ()| Ok(cc.borrow().query_replace_active()))?,
        )?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "linear list of session bindings; the surface is coherent and split would fragment review"
)]
fn install_session(editor: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    {
        // save() returns true on success so the buffer.save command can
        // gate `buffer.after-save` firing on actual writes.
        let cc = core.clone();
        editor.set(
            "save",
            lua.create_function(move |_, ()| Ok(cc.borrow_mut().save()))?,
        )?;
    }
    {
        // Overwrite even though the file changed on disk since this buffer
        // read it. `save()` refuses that case rather than silently
        // clobbering another writer; this is the deliberate override.
        let cc = core.clone();
        editor.set(
            "save_ignoring_disk_changes",
            lua.create_function(move |_, ()| Ok(cc.borrow_mut().save_ignoring_disk_changes()))?,
        )?;
    }
    register(editor, lua, core, "quit", |c| c.quit = true)?;
    register(editor, lua, core, "cancel", |c| {
        c.status = "Quit".into();
    })?;
    {
        let cc = core.clone();
        editor.set(
            "set_status",
            lua.create_function(move |_, msg: String| {
                cc.borrow_mut().status = msg;
                Ok(())
            })?,
        )?;
    }
    {
        // Milliseconds on a process-local monotonic clock. Only
        // differences are meaningful (the epoch is the first call).
        // Exists for Lua-side debounce/throttle logic — notably the
        // LSP didChange coalescing in `builtin/runtime/lsp.lua` —
        // which needs wall-clock-independent elapsed time; `os.clock`
        // is CPU time and `os.time` is second-granular.
        editor.set(
            "monotonic_ms",
            lua.create_function(|_, ()| {
                static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
                let epoch = *EPOCH.get_or_init(std::time::Instant::now);
                Ok(i64::try_from(epoch.elapsed().as_millis()).unwrap_or(i64::MAX))
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "cursor",
            lua.create_function(move |_, ()| {
                let c = cc.borrow();
                i64::try_from(c.cursor()).map_err(mlua::Error::external)
            })?,
        )?;
    }
    {
        // goto_byte(pos): set the active cursor to a byte offset
        // (clamped). The byte-exact restore saveplace/desktop need
        // (Arc 3) — `move_to_line` is line-based, and switch zeroes the
        // cursor, so restore sets it here after opening.
        let cc = core.clone();
        editor.set(
            "goto_byte",
            lua.create_function(move |_, pos: i64| {
                let byte = u64::try_from(pos).map_err(mlua::Error::external)?;
                cc.borrow_mut().set_cursor_byte(byte);
                Ok(())
            })?,
        )?;
    }
    {
        // last_command(): the active frontend's previous interactive
        // command — Emacs's `last-command` as seen from inside the
        // running command (kill ring Q#KR2). `nil` after a non-command
        // input (optimistic edit, pointer gesture, paste, unbound key)
        // broke the chain.
        let cc = core.clone();
        editor.set(
            "last_command",
            lua.create_function(move |_, ()| Ok(cc.borrow().last_command().map(str::to_owned)))?,
        )?;
    }
    {
        // this_command(): the command currently executing for the
        // active frontend — the input-origin signal. Inside
        // `buffer.after-edit`, "buffer.self-insert" means the edit was
        // a typed character; nil means a non-command input (paste,
        // pointer gesture, optimistic delete/undo). Per-frontend.
        let cc = core.clone();
        editor.set(
            "this_command",
            lua.create_function(move |_, ()| Ok(cc.borrow().this_command().map(str::to_owned)))?,
        )?;
    }
    {
        // take_typed_edit(): auto-pairing Q#AP9 — the one-shot exact
        // provenance record of the self-insert that produced the
        // current `buffer.after-edit` fan-out, or nil. Where
        // `this_command()` names only the input class, this record
        // carries the typed codepoint and the requested vs effective
        // (post-intercept) edit, so a consumer can fail closed on a
        // transformed, relocated, or context-switched source edit.
        // Consuming clears the slot: later callbacks and nested manual
        // hook runs see nil, and the producer clears any untaken
        // record when the fan-out returns. Per-frontend — one
        // frontend can never take another's record. `char` is the
        // codepoint as a UTF-8 string (LuaJIT has no `utf8` library
        // to convert `codepoint` Lua-side).
        let cc = core.clone();
        editor.set(
            "take_typed_edit",
            lua.create_function(move |lua, ()| {
                let Some(rec) = cc.borrow_mut().take_typed_edit() else {
                    return Ok(mlua::Value::Nil);
                };
                let cvt = |v: u64| i64::try_from(v).map_err(mlua::Error::external);
                let t = lua.create_table()?;
                t.set("buffer", BufferIdLua(rec.buffer))?;
                t.set("window", cvt(rec.window.raw())?)?;
                t.set("codepoint", i64::from(u32::from(rec.codepoint)))?;
                t.set("char", rec.codepoint.to_string())?;
                t.set("requested_start", cvt(rec.requested_start)?)?;
                t.set("requested_end", cvt(rec.requested_end)?)?;
                t.set("effective_start", cvt(rec.effective_start)?)?;
                t.set("effective_end", cvt(rec.effective_end)?)?;
                t.set("inserted_len", cvt(rec.inserted_len)?)?;
                t.set("post_cursor", cvt(rec.post_cursor)?)?;
                t.set("clean", rec.clean)?;
                Ok(mlua::Value::Table(t))
            })?,
        )?;
    }
    {
        // view_top(): the active window's first visible source line.
        // The saveplace getter (Arc 3) — pairs with set_view_top so a
        // reopen restores the viewport, not just the cursor.
        let cc = core.clone();
        editor.set(
            "view_top",
            lua.create_function(move |_, ()| {
                i64::try_from(cc.borrow().view_top()).map_err(mlua::Error::external)
            })?,
        )?;
    }
    {
        // set_view_top(line): set the first visible source line
        // (clamped to the buffer's line count) — desktop restore.
        let cc = core.clone();
        editor.set(
            "set_view_top",
            lua.create_function(move |_, top: i64| {
                let top = usize::try_from(top).map_err(mlua::Error::external)?;
                cc.borrow_mut().set_view_top(top);
                Ok(())
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "cursor_line",
            lua.create_function(move |_, ()| {
                let c = cc.borrow();
                i64::try_from(c.cursor_line()).map_err(mlua::Error::external)
            })?,
        )?;
    }
    {
        // 0-based byte column of the active window's cursor within its
        // current line. LSP-friendly: positions the manager hands to
        // `pmacs.lsp.request_*` are `(cursor_line, cursor_col)`. ASCII
        // and UTF-8 inputs match LSP's "UTF-16 code units" enough for
        // v0.1 — the multibyte conversion lands with the v0.2 LSP
        // hardening pass.
        let cc = core.clone();
        editor.set(
            "cursor_col",
            lua.create_function(move |_, ()| {
                let c = cc.borrow();
                let aw = c.active_window();
                let line = c.cursor_line();
                let line_start = aw.text_view.line_offset(line).unwrap_or(aw.cursor);
                i64::try_from(aw.cursor.saturating_sub(line_start)).map_err(mlua::Error::external)
            })?,
        )?;
    }
    {
        // The identifier under the cursor, or nil (Q#CM3 `symbol`
        // context). The context menu uses it to decide whether to show
        // symbol-oriented LSP items.
        let cc = core.clone();
        editor.set(
            "word_at_cursor",
            lua.create_function(move |_, ()| Ok(cc.borrow().word_at_cursor()))?,
        )?;
    }
    {
        // Active buffer's backing file path, or `nil` if none. Used by
        // the LSP runtime to compute file:// URIs and locate the
        // enclosing project root.
        let cc = core.clone();
        editor.set(
            "file_path",
            lua.create_function(move |_, ()| {
                let c = cc.borrow();
                Ok(c.active_buffer_path().map(|p| p.display().to_string()))
            })?,
        )?;
    }
    // T M2.12: region introspection and the canonical region-aware
    // delete. Lua callers do `local lo, hi = pmacs.editor.region()`;
    // the function returns nil when no region is active or empty.
    {
        let cc = core.clone();
        editor.set(
            "region",
            lua.create_function(move |lua, ()| {
                let c = cc.borrow();
                match c.active_region() {
                    Some((lo, hi)) => {
                        let t = lua.create_table()?;
                        t.set("start", i64::try_from(lo).map_err(mlua::Error::external)?)?;
                        t.set("end", i64::try_from(hi).map_err(mlua::Error::external)?)?;
                        Ok(mlua::Value::Table(t))
                    }
                    None => Ok(mlua::Value::Nil),
                }
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "delete_region",
            lua.create_function(move |_, ()| -> mlua::Result<bool> {
                let mut c = cc.borrow_mut();
                if c.active_region().is_none() {
                    return Ok(false);
                }
                c.delete_region().map_err(mlua::Error::external)?;
                Ok(true)
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "clear_selection",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().clear_selection();
                Ok(())
            })?,
        )?;
    }
    {
        // Anchor a selection at the given byte offset. Subsequent
        // motion implicitly extends the selected region from the
        // anchor to the new cursor position. Used by the
        // `cursor.select-*` commands wired in the default keymap to
        // implement CUA-style Shift+motion selection.
        let cc = core.clone();
        editor.set(
            "begin_selection",
            lua.create_function(move |_, anchor: i64| {
                let anchor = u64::try_from(anchor).map_err(mlua::Error::external)?;
                cc.borrow_mut().begin_selection(anchor);
                Ok(())
            })?,
        )?;
    }
    // Q#CM6: clipboard primitives. Copy/cut publish the region to the
    // OS clipboard (the daemon drains the queued publish and sends
    // `InstanceSignal::Clipboard` to the originating frontend); paste
    // inserts the in-core slot. Each returns whether it acted, so the
    // `edit.*` commands can report status / fall through.
    {
        let cc = core.clone();
        editor.set(
            "clipboard_copy",
            lua.create_function(move |_, ()| Ok(cc.borrow_mut().clipboard_copy()))?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "clipboard_cut",
            lua.create_function(move |_, ()| -> mlua::Result<bool> {
                cc.borrow_mut()
                    .clipboard_cut()
                    .map_err(mlua::Error::external)
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "clipboard_paste",
            lua.create_function(move |_, ()| -> mlua::Result<bool> {
                cc.borrow_mut()
                    .clipboard_paste()
                    .map_err(mlua::Error::external)
            })?,
        )?;
    }
    {
        // clipboard_set(bytes): set the slot + queue the OS publish to
        // the acting frontend (kill ring Q#KR1). The ring's kills have
        // no region for clipboard_copy to read.
        let cc = core.clone();
        editor.set(
            "clipboard_set",
            lua.create_function(move |_, bytes: mlua::String| {
                cc.borrow_mut().clipboard_set(bytes.as_bytes().to_vec());
                Ok(())
            })?,
        )?;
    }
    {
        // clipboard_get() -> string?: the slot's bytes (kill ring
        // Q#KR6 — yank's "did external content arrive via a paste
        // since our last kill" check). nil when empty.
        let cc = core.clone();
        editor.set(
            "clipboard_get",
            lua.create_function(move |lua, ()| {
                let cc = cc.borrow();
                match cc.clipboard_get() {
                    Some(bytes) => Ok(Some(lua.create_string(bytes)?)),
                    None => Ok(None),
                }
            })?,
        )?;
    }
    {
        let cc = core.clone();
        editor.set(
            "select_all",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().select_all();
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// pmacs.minibuffer: prompt sessions, completion, history (T M2.7).
// ---------------------------------------------------------------------------

/// Install `pmacs.minibuffer.*` --- the prompt API. Borrows the
/// editor core through the captured `SharedCore`; the command and
/// buffer registries (used by builtin completion sources) are
/// resolved lazily through the Lua state's app data.
fn install_minibuffer_module(lua: &Lua, core: &SharedCore) -> mlua::Result<Table> {
    let mb = lua.create_table()?;
    install_minibuffer_read(&mb, lua, core)?;
    install_minibuffer_query(&mb, lua, core)?;
    install_minibuffer_motion(&mb, lua, core)?;
    install_minibuffer_lifecycle(&mb, lua, core)?;
    Ok(mb)
}

fn install_minibuffer_read(mb: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    let cc = core.clone();
    mb.set(
        "read",
        lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
            for pair in spec.clone().pairs::<Value, Value>() {
                let (k, _) = pair?;
                let key = require_string_key(k)?;
                if !matches!(
                    key.as_str(),
                    "prompt"
                        | "initial"
                        | "history"
                        | "source"
                        | "source_root"
                        | "on_accept"
                        | "on_cancel"
                ) {
                    return Err(mlua::Error::external(
                        crate::command::CommandError::UnknownField { field: key },
                    ));
                }
            }
            let prompt: Option<String> = spec.get("prompt")?;
            let initial: Option<String> = spec.get("initial")?;
            let history: Option<String> = spec.get("history")?;
            let on_accept: Function = spec.get("on_accept").map_err(|_| {
                mlua::Error::external(BindingError::SpecFieldType {
                    field: "on_accept",
                    expected: "function",
                })
            })?;
            let on_cancel: Option<Function> = spec.get("on_cancel")?;
            let source = parse_completion_source(&spec)?;
            let session = crate::minibuffer::MinibufferSession {
                prompt: prompt.unwrap_or_default(),
                initial: initial.unwrap_or_default(),
                history_bucket: history.unwrap_or_default(),
                source,
                on_accept,
                on_cancel,
                candidates: Vec::new(),
                selected: None,
                history_index: None,
                typed_before_history_nav: None,
            };
            cc.borrow_mut().minibuffer.begin(session);
            // Compute initial candidate list against the live registries.
            let cmds_app = lua
                .app_data_ref::<SharedCommandRegistry>()
                .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
            let reg_app = lua
                .app_data_ref::<SharedRegistry>()
                .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
            let cmds = cmds_app.borrow();
            let reg = reg_app.borrow();
            cc.borrow_mut()
                .minibuffer
                .recompute_candidates(&cmds, &reg)?;
            Ok(())
        })?,
    )
}

fn install_minibuffer_query(mb: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    {
        let cc = core.clone();
        mb.set(
            "is_active",
            lua.create_function(move |_, ()| Ok(cc.borrow().minibuffer.is_active()))?,
        )?;
    }
    {
        let cc = core.clone();
        mb.set(
            "contents",
            lua.create_function(move |_, ()| Ok(cc.borrow().minibuffer.contents()))?,
        )?;
    }
    {
        let cc = core.clone();
        mb.set(
            "candidates",
            lua.create_function(move |lua, ()| {
                let c = cc.borrow();
                let t = lua.create_table()?;
                if let Some(s) = c.minibuffer.session.as_ref() {
                    for (i, c) in s.candidates.iter().enumerate() {
                        t.set(i + 1, c.clone())?;
                    }
                }
                Ok(t)
            })?,
        )?;
    }
    let cc = core.clone();
    mb.set(
        "selected",
        lua.create_function(move |_, ()| {
            let c = cc.borrow();
            Ok(c.minibuffer
                .session
                .as_ref()
                .and_then(|s| s.selected.and_then(|i| s.candidates.get(i).cloned())))
        })?,
    )
}

fn install_minibuffer_motion(mb: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    {
        // `set_contents` mirrors the keyboard-driven typing path
        // (`MinibufferAction::SelfInsert` runs `replace_contents` *and*
        // `recompute_candidates` together). Without the recompute, a
        // Lua-driven `set_contents("foo")` followed by `accept()`
        // resolves against a stale candidate list — selected was
        // computed against the *previous* needle, so accept may pick
        // an unrelated candidate. M9.6's `editor.describe-command`
        // acceptance test is the first user that hit this; the keyboard
        // path was always coherent so it never came up before.
        let cc = core.clone();
        let lua_for_app = lua.clone();
        mb.set(
            "set_contents",
            lua.create_function(move |_, s: String| {
                let cmds_app = lua_for_app
                    .app_data_ref::<SharedCommandRegistry>()
                    .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
                let reg_app = lua_for_app
                    .app_data_ref::<SharedRegistry>()
                    .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
                let cmds = cmds_app.borrow();
                let reg = reg_app.borrow();
                let mut core = cc.borrow_mut();
                core.minibuffer.replace_contents(&s);
                core.minibuffer.recompute_candidates(&cmds, &reg)?;
                Ok(())
            })?,
        )?;
    }
    {
        let cc = core.clone();
        mb.set(
            "scroll",
            lua.create_function(move |_, delta: i64| {
                let delta =
                    i32::try_from(delta).unwrap_or(if delta < 0 { i32::MIN } else { i32::MAX });
                cc.borrow_mut().minibuffer.scroll_candidate(delta);
                Ok(())
            })?,
        )?;
    }
    {
        let cc = core.clone();
        mb.set(
            "complete",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().minibuffer.complete();
                Ok(())
            })?,
        )?;
    }
    {
        let cc = core.clone();
        mb.set(
            "history_prev",
            lua.create_function(move |_, ()| {
                cc.borrow_mut().minibuffer.history_prev();
                Ok(())
            })?,
        )?;
    }
    let cc = core.clone();
    mb.set(
        "history_next",
        lua.create_function(move |_, ()| {
            cc.borrow_mut().minibuffer.history_next();
            Ok(())
        })?,
    )
}

fn install_minibuffer_lifecycle(mb: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    {
        let cc = core.clone();
        mb.set(
            "accept",
            lua.create_function(move |lua, ()| -> mlua::Result<()> {
                let outcome = cc.borrow_mut().minibuffer.accept();
                if let Some((cb, contents)) = outcome {
                    let s = lua.create_string(&contents)?;
                    cb.call::<mlua::MultiValue>(mlua::MultiValue::from_vec(vec![Value::String(
                        s,
                    )]))?;
                }
                Ok(())
            })?,
        )?;
    }
    let cc = core.clone();
    mb.set(
        "cancel",
        lua.create_function(move |_, ()| -> mlua::Result<()> {
            let cb = cc.borrow_mut().minibuffer.cancel();
            if let Some(cb) = cb {
                cb.call::<mlua::MultiValue>(mlua::MultiValue::new())?;
            }
            Ok(())
        })?,
    )
}

fn parse_completion_source(spec: &Table) -> mlua::Result<crate::minibuffer::CompletionSource> {
    let v: Value = spec.get("source")?;
    match v {
        Value::Nil => Ok(crate::minibuffer::CompletionSource::None),
        Value::String(s) => match s.to_str()?.as_ref() {
            "none" => Ok(crate::minibuffer::CompletionSource::None),
            "commands" => Ok(crate::minibuffer::CompletionSource::Commands),
            "buffers" => Ok(crate::minibuffer::CompletionSource::Buffers),
            "files" => {
                let root: Option<String> = spec.get("source_root")?;
                let root = root.unwrap_or_else(|| ".".to_string());
                Ok(crate::minibuffer::CompletionSource::Files {
                    root: std::path::PathBuf::from(root),
                })
            }
            other => Err(mlua::Error::external(
                BindingError::UnknownCompletionSource {
                    got: other.to_owned(),
                },
            )),
        },
        Value::Function(f) => Ok(crate::minibuffer::CompletionSource::Custom(f)),
        _ => Err(mlua::Error::external(BindingError::SpecFieldType {
            field: "source",
            expected: "string or function",
        })),
    }
}

/// Register a no-arg primitive on the `pmacs.editor` table whose body
/// borrows the [`SharedCore`] mutably and runs `f`.
fn register<F>(editor: &Table, lua: &Lua, core: &SharedCore, name: &str, f: F) -> mlua::Result<()>
where
    F: Fn(&mut EditorCore) + 'static,
{
    let cc = core.clone();
    editor.set(
        name,
        lua.create_function(move |_, ()| {
            f(&mut cc.borrow_mut());
            Ok(())
        })?,
    )?;
    Ok(())
}

fn char_from_lua_codepoint(value: i64) -> mlua::Result<char> {
    let cp = u32::try_from(value)
        .map_err(|_| mlua::Error::external(BindingError::InvalidCodepoint { value }))?;
    char::from_u32(cp)
        .ok_or_else(|| mlua::Error::external(BindingError::InvalidCodepoint { value }))
}

// ---- bind/unbind spec parsing -------------------------------------------

struct BindArgs {
    sequence: Vec<crate::key::Chord>,
    command: String,
    scope: ScopeArg,
    source: SourceLocation,
}

enum ScopeArg {
    Global,
    Buffer(BufferId),
    Mode(String),
}

impl BindArgs {
    fn apply(self, stack: &mut KeymapStack) -> mlua::Result<()> {
        match self.scope {
            ScopeArg::Global => stack.bind_global(&self.sequence, self.command, self.source),
            ScopeArg::Buffer(id) => {
                stack.bind_buffer(id, &self.sequence, self.command, self.source)
            }
            ScopeArg::Mode(name) => {
                stack.bind_mode(&name, &self.sequence, self.command, self.source)
            }
        }
        .map_err(mlua::Error::external)
    }
}

struct UnbindArgs {
    sequence: Vec<crate::key::Chord>,
    scope: ScopeArg,
}

impl UnbindArgs {
    fn apply(self, stack: &mut KeymapStack) -> mlua::Result<()> {
        let result = match self.scope {
            ScopeArg::Global => stack.unbind_global(&self.sequence),
            ScopeArg::Buffer(id) => stack.unbind_buffer(id, &self.sequence),
            ScopeArg::Mode(name) => stack.unbind_mode(&name, &self.sequence),
        };
        result.map(|_| ()).map_err(mlua::Error::external)
    }
}

fn parse_bind_spec(lua: &Lua, spec: &Table) -> mlua::Result<BindArgs> {
    // R50: reject unknown keys.
    for pair in spec.clone().pairs::<Value, Value>() {
        let (k, _) = pair?;
        let key = require_string_key(k)?;
        if !matches!(
            key.as_str(),
            "scope" | "sequence" | "command" | "buffer" | "mode"
        ) {
            return Err(mlua::Error::external(CommandError::UnknownField {
                field: key,
            }));
        }
    }
    let sequence_str: String = spec.get("sequence")?;
    let command: String = spec.get("command")?;
    let sequence = parse_sequence(&sequence_str).map_err(mlua::Error::external)?;
    let scope = parse_scope_arg(spec)?;
    Ok(BindArgs {
        sequence,
        command,
        scope,
        source: caller_source(lua, 2),
    })
}

fn parse_unbind_spec(spec: &Table) -> mlua::Result<UnbindArgs> {
    for pair in spec.clone().pairs::<Value, Value>() {
        let (k, _) = pair?;
        let key = require_string_key(k)?;
        if !matches!(key.as_str(), "scope" | "sequence" | "buffer" | "mode") {
            return Err(mlua::Error::external(CommandError::UnknownField {
                field: key,
            }));
        }
    }
    let sequence_str: String = spec.get("sequence")?;
    let sequence = parse_sequence(&sequence_str).map_err(mlua::Error::external)?;
    let scope = parse_scope_arg(spec)?;
    Ok(UnbindArgs { sequence, scope })
}

fn parse_scope_arg(spec: &Table) -> mlua::Result<ScopeArg> {
    let scope: String = spec.get("scope")?;
    match scope.as_str() {
        "global" => Ok(ScopeArg::Global),
        "buffer" => {
            let buf: Option<BufferIdLua> = spec.get("buffer")?;
            let buf = buf.ok_or_else(|| {
                mlua::Error::external(BindingError::MissingScopeField {
                    scope: "buffer",
                    field: "buffer",
                })
            })?;
            Ok(ScopeArg::Buffer(buf.0))
        }
        "mode" => {
            let mode: Option<String> = spec.get("mode")?;
            let mode = mode.ok_or_else(|| {
                mlua::Error::external(BindingError::MissingScopeField {
                    scope: "mode",
                    field: "mode",
                })
            })?;
            Ok(ScopeArg::Mode(mode))
        }
        other => Err(mlua::Error::external(BindingError::UnknownScope {
            got: other.to_owned(),
        })),
    }
}

fn require_string_key(v: Value) -> mlua::Result<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        other => Err(mlua::Error::external(BindingError::NonStringSpecKey {
            got: other.type_name().to_string(),
        })),
    }
}

fn build_command_from_spec(lua: &Lua, spec: &Table) -> mlua::Result<Command> {
    // R50: reject unknown keys before reading anything we expect.
    for pair in spec.clone().pairs::<Value, Value>() {
        let (k, _) = pair?;
        let key = match k {
            Value::String(s) => s.to_str()?.to_string(),
            other => {
                return Err(mlua::Error::external(BindingError::NonStringSpecKey {
                    got: other.type_name().to_string(),
                }));
            }
        };
        if !matches!(key.as_str(), "name" | "description" | "fn" | "predicate") {
            return Err(mlua::Error::external(CommandError::UnknownField {
                field: key,
            }));
        }
    }

    let name: String = spec.get("name").map_err(|_| {
        mlua::Error::external(BindingError::SpecFieldType {
            field: "name",
            expected: "string",
        })
    })?;
    let description: Option<String> = spec.get("description")?;
    let body: Function = spec
        .get("fn")
        .map_err(|_| mlua::Error::external(CommandError::MissingFn { name: name.clone() }))?;
    let predicate: Option<Function> = spec.get("predicate")?;

    let description = description
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| {
            mlua::Error::external(CommandError::MissingDescription { name: name.clone() })
        })?;

    Ok(Command {
        name,
        description,
        source: caller_source(lua, 2),
        body,
        predicate,
    })
}

/// Build a [`MenuItem`] from a `pmacs.menu.item` spec table.
///
/// Mirrors [`build_command_from_spec`]: rejects unknown keys (R50
/// typo-detection) before reading, then pulls the fields. `label` and
/// `command` are required strings; `id`, `context`, `predicate`,
/// `group`, and `order` are optional. The registry validates the
/// `context` vocabulary and non-empty invariants.
fn build_menu_item_from_spec(lua: &Lua, spec: &Table) -> mlua::Result<MenuItem> {
    for pair in spec.clone().pairs::<Value, Value>() {
        let (k, _) = pair?;
        let key = match k {
            Value::String(s) => s.to_str()?.to_string(),
            other => {
                return Err(mlua::Error::external(BindingError::NonStringSpecKey {
                    got: other.type_name().to_string(),
                }));
            }
        };
        if !matches!(
            key.as_str(),
            "id" | "label" | "command" | "context" | "predicate" | "group" | "order"
        ) {
            return Err(mlua::Error::external(
                crate::menu::MenuError::UnknownField { field: key },
            ));
        }
    }

    let label: String = spec.get("label").map_err(|_| {
        mlua::Error::external(BindingError::SpecFieldType {
            field: "label",
            expected: "string",
        })
    })?;
    let command: String = spec.get("command").map_err(|_| {
        mlua::Error::external(BindingError::SpecFieldType {
            field: "command",
            expected: "string",
        })
    })?;
    let id: Option<String> = spec.get("id")?;
    let context: Option<String> = spec.get("context")?;
    let predicate: Option<Function> = spec.get("predicate")?;
    let group: String = spec.get::<Option<String>>("group")?.unwrap_or_default();
    let order: i64 = spec.get::<Option<i64>>("order")?.unwrap_or(0);

    Ok(MenuItem {
        id,
        label,
        command,
        context,
        predicate,
        group,
        order,
        source: caller_source(lua, 2),
    })
}

/// Inspect the Lua call stack at `level` frames above the C boundary
/// and return the caller's source location, or a default if debug info
/// is unavailable.
///
/// `level = 1` is the function that invoked our Rust closure; `level
/// = 2` is its caller --- which is what `pmacs.command.define` wants
/// (the user's `init.lua`, not the helper that wrapped `define`).
fn caller_source(lua: &Lua, level: usize) -> SourceLocation {
    // Walk up to `level` first; if the user's stack is shallower (e.g.
    // a chunk loaded with `load(...)` and called with no enclosing Lua
    // frame), fall back to the deepest frame we can reach.
    for try_level in (1..=level).rev() {
        if let Some(dbg) = lua.inspect_stack(try_level) {
            let src = dbg.source();
            return SourceLocation {
                file: src.short_src.as_deref().unwrap_or("[unknown]").to_string(),
                line: dbg.curr_line(),
            };
        }
    }
    SourceLocation::default()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn with_registry<R>(
    lua: &Lua,
    f: impl FnOnce(&BufferRegistry) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let app = lua
        .app_data_ref::<SharedRegistry>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
    // try_borrow rather than borrow: a `with_registry_mut` higher in
    // the call stack (typically a buffer-mutation Lua method whose
    // intercept chain re-entered into Lua) would otherwise panic on
    // recursive borrow. Surface a typed error instead.
    let r = app
        .try_borrow()
        .map_err(|_| mlua::Error::external(BindingError::Reentrant))?;
    f(&r)
}

fn with_registry_mut<R>(
    lua: &Lua,
    f: impl FnOnce(&mut BufferRegistry) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let app = lua
        .app_data_ref::<SharedRegistry>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoRegistry))?;
    // try_borrow_mut rather than borrow_mut: see with_registry for
    // the intercept-reentry rationale. A re-entrant call returns a
    // typed `BindingError::Reentrant` rather than panicking.
    let mut r = app
        .try_borrow_mut()
        .map_err(|_| mlua::Error::external(BindingError::Reentrant))?;
    f(&mut r)
}

fn resolve(r: &BufferRegistry, id: BufferId) -> mlua::Result<&crate::buffer::Buffer> {
    r.get(id)
        .map_err(|_| mlua::Error::external(BindingError::StaleId { id }))
}

fn resolve_mut(r: &mut BufferRegistry, id: BufferId) -> mlua::Result<&mut crate::buffer::Buffer> {
    r.get_mut(id)
        .map_err(|_| mlua::Error::external(BindingError::StaleId { id }))
}

fn u64_from_lua(n: i64) -> mlua::Result<u64> {
    if n < 0 {
        return Err(mlua::Error::external(BindingError::NegativePosition {
            got: n,
        }));
    }
    Ok(n as u64)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Value;

    fn fresh() -> (
        Lua,
        SharedRegistry,
        SharedCommandRegistry,
        SharedKeymapStack,
        SharedHookRegistry,
    ) {
        let lua = Lua::new();
        let reg: SharedRegistry = Rc::new(RefCell::new(BufferRegistry::new()));
        let cmds: SharedCommandRegistry = Rc::new(RefCell::new(CommandRegistry::new()));
        let kms: SharedKeymapStack = Rc::new(RefCell::new(KeymapStack::new()));
        // The menu registry isn't returned --- install clones it into
        // app data, which keeps it alive for the VM's lifetime, so tests
        // that don't exercise menus needn't carry the handle.
        let mns: SharedMenuRegistry = Rc::new(RefCell::new(MenuRegistry::new()));
        let hks: SharedHookRegistry = Rc::new(RefCell::new(HookRegistry::new()));
        install(&lua, &reg, &cmds, &kms, &mns, &hks).expect("install");
        (lua, reg, cmds, kms, hks)
    }

    fn attach_test_editor(lua: &Lua, registry: &SharedRegistry) -> SharedCore {
        let core = Rc::new(RefCell::new(EditorCore::new(registry.clone())));
        install_editor(lua, &core).expect("install editor");
        core
    }

    #[test]
    fn buffer_major_mode_is_strict_and_rejects_stale_ids() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (wrong_mode, wrong_id, stale_get, stale_set): (String, String, String, String) = lua
            .load(
                r#"
                local id = pmacs.buffer.create("mode-test")
                assert(pmacs.buffer.major_mode(id) == nil)
                pmacs.buffer.set_major_mode(id, "rust")
                assert(pmacs.buffer.major_mode(id) == "rust")
                pmacs.buffer.set_major_mode(id, nil)
                assert(pmacs.buffer.major_mode(id) == nil)

                local ok_type, err_type =
                    pcall(pmacs.buffer.set_major_mode, id, 42)
                assert(not ok_type)
                local ok_id, err_id = pcall(pmacs.buffer.major_mode, 1)
                assert(not ok_id)
                pmacs.buffer.remove(id)
                local ok_get, err_get = pcall(pmacs.buffer.major_mode, id)
                local ok_set, err_set =
                    pcall(pmacs.buffer.set_major_mode, id, "rust")
                assert(not ok_get and not ok_set)
                return tostring(err_type), tostring(err_id),
                    tostring(err_get), tostring(err_set)
                "#,
            )
            .eval()
            .unwrap();
        assert!(wrong_mode.contains("string"), "{wrong_mode}");
        assert!(wrong_id.contains("buffer handle"), "{wrong_id}");
        assert!(stale_get.contains("stale buffer handle"), "{stale_get}");
        assert!(stale_set.contains("stale buffer handle"), "{stale_set}");
    }

    #[test]
    fn editor_active_modes_tracks_the_active_buffers_major_mode() {
        let (lua, reg, _cmds, _kms, _hks) = fresh();
        let core = attach_test_editor(&lua, &reg);
        let active = core.borrow().active_buffer_id();
        lua.globals()
            .set("active_buffer", BufferIdLua(active))
            .unwrap();

        lua.load(
            r#"
            local modes = pmacs.editor.active_modes()
            assert(type(modes) == "table" and #modes == 0)
            pmacs.buffer.set_major_mode(active_buffer, "rust")
            modes = pmacs.editor.active_modes()
            assert(#modes == 1 and modes[1] == "rust")
            pmacs.buffer.set_major_mode(active_buffer, nil)
            assert(#pmacs.editor.active_modes() == 0)
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn describe_key_uses_the_active_buffers_major_mode() {
        let (lua, reg, _cmds, kms, _hks) = fresh();
        let core = attach_test_editor(&lua, &reg);
        let active = core.borrow().active_buffer_id();
        reg.borrow_mut()
            .get_mut(active)
            .unwrap()
            .set_major_mode(Some("rust".to_owned()));
        {
            let mut keymaps = kms.borrow_mut();
            keymaps
                .bind_global(
                    &parse_sequence("C-s").unwrap(),
                    "global.save",
                    SourceLocation::default(),
                )
                .unwrap();
            keymaps
                .bind_mode(
                    "rust",
                    &parse_sequence("C-s").unwrap(),
                    "rust.save",
                    SourceLocation::default(),
                )
                .unwrap();
        }

        let (command, scope): (String, String) = lua
            .load(
                r#"
                local info = pmacs.describe.key("C-s")
                return info.command, info.scope
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(command, "rust.save");
        assert_eq!(scope, "mode:rust");
    }

    /// A theme handle with one syntax entry and nonzero counters, for
    /// pinning that failed commits change nothing and successful ones
    /// bump from the PRIOR values (themes arc Q#TH6).
    fn seeded_theme() -> crate::highlight::ThemeHandle {
        let mut th = crate::highlight::Theme::empty();
        th.insert(
            "keyword",
            Style {
                bold: true,
                ..Style::default()
            },
        );
        th.syntax_epoch = 3;
        th.face_epoch = 5;
        std::sync::Arc::new(std::sync::Mutex::new(th))
    }

    #[test]
    fn theme_commit_is_all_or_nothing_with_untouched_counters() {
        // Q#TH6 / acceptance 11 (the deterministic bite): an ordered
        // entry stream whose TAIL is malformed must error with zero
        // theme mutation — the helper collects the whole stream
        // before taking the lock. The pre-fix merge inserted while
        // iterating, so the leading Ok entry landed before the Err.
        let theme = seeded_theme();
        let entries: Vec<mlua::Result<(String, Style)>> = vec![
            Ok((
                "string".to_owned(),
                Style {
                    italic: true,
                    ..Style::default()
                },
            )),
            Err(mlua::Error::RuntimeError("malformed style".into())),
        ];
        let res = commit_theme_entries(&theme, ThemeCommit::Merge, entries.into_iter());
        assert!(res.is_err(), "a malformed tail entry must error");
        let th = theme.lock().expect("lock");
        assert!(
            !th.by_capture.contains_key("string"),
            "the leading Ok entry must NOT have landed"
        );
        assert!(
            th.by_capture.contains_key("keyword"),
            "pre-existing entries survive"
        );
        assert_eq!(th.syntax_epoch, 3, "failed commit bumps nothing");
        assert_eq!(th.face_epoch, 5, "failed commit bumps nothing");
    }

    #[test]
    fn theme_commit_replace_advances_counters_from_prior_values() {
        // Q#TH6 / acceptance 10: consecutive wholesale replacements
        // must each advance the counters — replacing the FIELD, not
        // the Theme value, or two `set`s share an epoch. Replace also
        // leaves default_style alone (the historical `set` contract).
        let theme = seeded_theme();
        theme.lock().expect("lock").default_style = Style {
            reverse: true,
            ..Style::default()
        };
        for expected in [(4, 6), (5, 7)] {
            let entries: Vec<mlua::Result<(String, Style)>> =
                vec![Ok(("type".to_owned(), Style::default()))];
            commit_theme_entries(&theme, ThemeCommit::Replace, entries.into_iter())
                .expect("commit");
            let th = theme.lock().expect("lock");
            assert_eq!((th.syntax_epoch, th.face_epoch), expected);
            assert!(!th.by_capture.contains_key("keyword"), "replaced wholesale");
            assert!(th.default_style.reverse, "default_style preserved");
        }
    }

    #[test]
    fn theme_commit_merge_classifies_face_and_syntax_keys() {
        // Q#TH6: merge bumps syntax_epoch iff any non-face key
        // committed and face_epoch iff any face key did — bare `ui`
        // classifies as a face (Q#TH2, round 2 finding 3).
        let theme = seeded_theme();
        // One lock per observation: two `lock()` calls inside one
        // tuple expression self-deadlock (the first guard outlives
        // the second call).
        let epochs = |theme: &crate::highlight::ThemeHandle| {
            let th = theme.lock().expect("lock");
            (th.syntax_epoch, th.face_epoch)
        };

        let face_only: Vec<mlua::Result<(String, Style)>> =
            vec![Ok(("ui.modeline".to_owned(), Style::default()))];
        commit_theme_entries(&theme, ThemeCommit::Merge, face_only.into_iter()).expect("commit");
        assert_eq!(
            epochs(&theme),
            (3, 6),
            "face-only merge bumps face_epoch only"
        );

        let bare_ui: Vec<mlua::Result<(String, Style)>> =
            vec![Ok(("ui".to_owned(), Style::default()))];
        commit_theme_entries(&theme, ThemeCommit::Merge, bare_ui.into_iter()).expect("commit");
        assert_eq!(epochs(&theme), (3, 7), "bare ui is a face key");

        let mixed: Vec<mlua::Result<(String, Style)>> = vec![
            Ok(("comment".to_owned(), Style::default())),
            Ok(("ui.gutter".to_owned(), Style::default())),
        ];
        commit_theme_entries(&theme, ThemeCommit::Merge, mixed.into_iter()).expect("commit");
        assert_eq!(epochs(&theme), (4, 8), "mixed merge bumps both");

        let empty: Vec<mlua::Result<(String, Style)>> = Vec::new();
        commit_theme_entries(&theme, ThemeCommit::Merge, empty.into_iter()).expect("commit");
        assert_eq!(
            epochs(&theme),
            (4, 8),
            "an empty merge commits nothing and bumps nothing"
        );
    }

    #[test]
    fn create_query_insert_observe_round_trip() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let v: Value = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                assert(id:len() == 0, "len initially zero")
                id:insert(0, "hello, world")
                assert(id:len() == 12, "len after insert")
                return id:slice(0, 5)
                "#,
            )
            .eval()
            .unwrap();
        match v {
            Value::String(s) => assert_eq!(s.to_str().unwrap(), "hello"),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn from_bytes_seeds_buffer() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let len: i64 = lua
            .load("return pmacs.buffer.from_bytes('seed', 'abcde'):len()")
            .eval()
            .unwrap();
        assert_eq!(len, 5);
    }

    #[test]
    fn menu_item_registers_and_lists_with_defaults() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (label, command, group, order, has_pred): (String, String, String, i64, bool) = lua
            .load(
                r#"
                pmacs.menu.item { label = "Copy", command = "edit.copy" }
                local items = pmacs.menu.list()
                assert(#items == 1, "one item")
                local it = items[1]
                return it.label, it.command, it.group, it.order, it.has_predicate
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(label, "Copy");
        assert_eq!(command, "edit.copy");
        assert_eq!(group, ""); // group defaults to empty
        assert_eq!(order, 0); // order defaults to 0
        assert!(!has_pred); // no predicate given
    }

    #[test]
    fn menu_item_carries_context_and_predicate_through() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (context, has_pred): (String, bool) = lua
            .load(
                r#"
                pmacs.menu.item {
                  label = "Paste", command = "edit.paste",
                  context = "selection",
                  predicate = function(cx) return true end,
                  group = "edit", order = 30,
                }
                local it = pmacs.menu.list()[1]
                return it.context, it.has_predicate
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(context, "selection");
        assert!(has_pred);
    }

    #[test]
    fn menu_remove_and_clear_work() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (removed, removed_again, after_clear): (bool, bool, i64) = lua
            .load(
                r#"
                pmacs.menu.item { id = "a", label = "A", command = "cmd.a" }
                pmacs.menu.item { label = "B", command = "cmd.b" }
                local r1 = pmacs.menu.remove("a")
                local r2 = pmacs.menu.remove("a")
                pmacs.menu.clear()
                return r1, r2, #pmacs.menu.list()
                "#,
            )
            .eval()
            .unwrap();
        assert!(removed);
        assert!(!removed_again);
        assert_eq!(after_clear, 0);
    }

    #[test]
    fn menu_item_rejects_unknown_field() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let err = lua
            .load(r#"pmacs.menu.item { label = "X", command = "x", colour = "red" }"#)
            .exec()
            .unwrap_err();
        assert!(
            err.to_string().contains("unknown field `colour`"),
            "got: {err}"
        );
    }

    #[test]
    fn menu_item_rejects_unknown_context() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let err = lua
            .load(r#"pmacs.menu.item { label = "X", command = "x", context = "selecton" }"#)
            .exec()
            .unwrap_err();
        assert!(
            err.to_string().contains("unknown context `selecton`"),
            "got: {err}"
        );
    }

    #[test]
    fn from_file_loads_existing_file_as_clean_buffer() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("seed.txt");
        std::fs::write(&path, b"abcde").expect("write seed file");
        let loaded: BufferIdLua = lua
            .load("return pmacs.buffer.from_file(...)")
            .call(path.display().to_string())
            .unwrap();
        let content: String = lua
            .load("local id = ...; return id:slice(0, id:len())")
            .call(loaded)
            .unwrap();
        let modified: bool = lua
            .load("local id = ...; return id:is_modified()")
            .call(loaded)
            .unwrap();
        assert_eq!(content, "abcde");
        assert!(!modified, "loaded buffers should start clean");
    }

    #[test]
    fn delete_replace_undo_redo() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            local id = pmacs.buffer.from_bytes("t", "abcdef")
            id:delete(1, 3)            -- "adef"
            assert(id:len() == 4, "len after delete")
            assert(id:slice(0, 4) == "adef", "content after delete")
            id:replace(1, 3, "BC")     -- "aBCf"
            assert(id:slice(0, 4) == "aBCf", "content after replace")
            assert(id:undo() == true,  "undo replace")
            assert(id:slice(0, 4) == "adef", "content after undo")
            assert(id:redo() == true,  "redo replace")
            assert(id:slice(0, 4) == "aBCf", "content after redo")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn out_of_bounds_insert_surfaces_rope_error() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        // Insert past end-of-buffer: rope reports OutOfBounds. The error
        // travels back to Lua as an external error; we reach it via pcall
        // and assert the structured fields survive into the message.
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("t")
                local ok, err = pcall(function() id:insert(100, "x") end)
                assert(not ok, "insert past end should fail")
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(msg.contains("100"), "expected pos 100 in error: {msg}");
        assert!(msg.contains("len = 0"), "expected len = 0 in error: {msg}");
    }

    #[test]
    fn stale_id_after_remove_fails_typed() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("doomed")
                pmacs.buffer.remove(id)
                local ok, err = pcall(function() return id:len() end)
                assert(not ok, "stale handle must fail")
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("stale buffer handle"),
            "expected stale-handle error: {msg}"
        );
    }

    #[test]
    fn is_valid_reports_truthfully() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            local id = pmacs.buffer.create("v")
            assert(id:is_valid() == true)
            pmacs.buffer.remove(id)
            assert(id:is_valid() == false)
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn list_returns_handles_in_insertion_order() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let names: Vec<String> = lua
            .load(
                r#"
                pmacs.buffer.create("a")
                pmacs.buffer.create("b")
                pmacs.buffer.create("c")
                local out = {}
                for i, id in ipairs(pmacs.buffer.list()) do
                    out[i] = id:name()
                end
                return out
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn negative_position_is_rejected_with_typed_error() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("n")
                local ok, err = pcall(function() id:insert(-1, "x") end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("non-negative"),
            "expected non-negative error: {msg}"
        );
        assert!(msg.contains("-1"), "expected -1 in error: {msg}");
    }

    #[test]
    fn handles_compare_equal_when_pointing_at_same_buffer() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            local a = pmacs.buffer.create("x")
            local b = pmacs.buffer.list()[1]
            assert(a == b, "two handles to same buffer must compare equal")
            local c = pmacs.buffer.create("y")
            assert(a ~= c, "handles to different buffers must not")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn name_and_is_modified_round_trip() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            local id = pmacs.buffer.create("notes")
            assert(id:name() == "notes")
            assert(id:is_modified() == false)
            id:insert(0, "x")
            assert(id:is_modified() == true)
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn rust_can_observe_lua_edits_through_registry() {
        // Acceptance: "Lua can create a buffer, query its size, apply an
        // edit, and observe the result." The Rust side observes too.
        let (lua, reg, _cmds, _kms, _hks) = fresh();
        let id_lua: BufferIdLua = lua
            .load(
                r#"
                local id = pmacs.buffer.create("shared")
                id:insert(0, "rust sees this")
                return id
                "#,
            )
            .eval()
            .unwrap();
        let r = reg.borrow();
        let buf = r.get(id_lua.0).expect("registered");
        assert_eq!(buf.len(), b"rust sees this".len() as u64);
    }

    #[test]
    fn no_registry_app_data_yields_typed_error() {
        // If a caller installs UserData methods on a Lua state without
        // calling install(), method calls report a typed BindingError.
        let lua = Lua::new();
        // Don't call install --- we hand a BufferIdLua in directly.
        let stale = BufferIdLua(BufferId::next());
        lua.globals().set("id", stale).unwrap();
        let msg: String = lua
            .load("local ok, err = pcall(function() return id:len() end); return tostring(err)")
            .eval()
            .unwrap();
        assert!(
            msg.contains("BufferRegistry was not installed"),
            "expected NoRegistry error: {msg}"
        );
    }

    // -----------------------------------------------------------------
    // T M6.4: chained buffer intercepts
    // -----------------------------------------------------------------

    #[test]
    fn m6_4_intercept_pass_through_returning_nil() {
        // The most common case: an intercept that observes but neither
        // transforms nor rejects returns nil and the edit proceeds
        // verbatim.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let len: i64 = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                pmacs.buffer.add_intercept(id, function(_op) return nil end)
                id:insert(0, "abc")
                return id:len()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(len, 3);
    }

    #[test]
    fn m8_3_intercept_input_carries_inserted_bytes() {
        // M8.3 enhancement: the intercept input table carries the
        // proposed insert/replace bytes verbatim as a Lua string.
        // The wdired layer relies on this to validate permission-
        // column edits against the rwx alphabet without waiting for
        // the chmod syscall.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let observed: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                local seen = nil
                pmacs.buffer.add_intercept(id, function(op)
                    if op.kind == "insert" then seen = op.bytes end
                    return nil
                end)
                id:insert(0, "hello")
                return seen
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(observed, "hello");
    }

    #[test]
    fn m8_3_intercept_input_bytes_round_trip_non_utf8() {
        // Lua strings are byte-clean; surfacing bytes as a Lua string
        // must preserve arbitrary 8-bit content, not coerce to UTF-8.
        // The dired-class package handles non-UTF-8 filename bytes
        // (POSIX permits any byte except `/` and NUL).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        // Insert two bytes: 0xC3 0x28, which is *not* a valid UTF-8
        // sequence (0xC3 starts a 2-byte form; 0x28 is below the
        // 0x80..0xBF continuation range).
        let observed_len: i64 = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                local seen_len = nil
                pmacs.buffer.add_intercept(id, function(op)
                    if op.kind == "insert" then seen_len = #op.bytes end
                    return nil
                end)
                id:insert(0, string.char(0xC3, 0x28))
                return seen_len
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(observed_len, 2, "non-UTF-8 bytes must round-trip 1:1");
    }

    #[test]
    fn m8_3_intercept_input_bytes_for_replace() {
        // Replace ops also carry the *incoming* bytes (not the bytes
        // being overwritten).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let observed: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                id:insert(0, "AAA")
                local seen = nil
                pmacs.buffer.add_intercept(id, function(op)
                    if op.kind == "replace" then seen = op.bytes end
                    return nil
                end)
                id:replace(0, 3, "ZZ")
                return seen
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(observed, "ZZ");
    }

    #[test]
    fn m6_4_intercept_reject_via_error_propagates_to_lua() {
        // An intercept that raises an error stops the edit; the Rust
        // side returns BufferError::Intercepted; the Lua-side
        // `id:insert` re-raises the message.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                pmacs.buffer.add_intercept(id, function(op)
                    error("forbidden: " .. op.kind .. " at " .. (op.pos or op.start))
                end)
                local ok, err = pcall(function() id:insert(0, "x") end)
                assert(not ok, "rejected insert must surface as error")
                assert(id:len() == 0, "buffer must be unchanged after rejection")
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("forbidden: insert at 0"),
            "expected verbatim Lua message in error: {msg}"
        );
    }

    #[test]
    fn intercept_can_reenter_other_buffer_for_read() {
        // M7.4 acceptance: an intercept on buffer A that calls
        // `B:slice(...)` (a different buffer) succeeds. Pre-M7.4 this
        // panicked or returned BindingError::Reentrant.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let observed: String = lua
            .load(
                r#"
                local a = pmacs.buffer.from_bytes("a", "AAA")
                local b = pmacs.buffer.from_bytes("b", "BBBBB")
                local seen = ""
                pmacs.buffer.add_intercept(a, function(_op)
                    -- Read-only re-entry on a different buffer.
                    seen = b:slice(0, b:len())
                    return nil
                end)
                a:insert(0, "x")
                return seen
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(observed, "BBBBB");
    }

    #[test]
    fn intercept_can_reenter_other_buffer_for_write() {
        // M7.4 acceptance: an intercept on buffer A that calls
        // `B:insert(...)` succeeds; the edit on B applies before the
        // original edit on A completes (this test reads B back from
        // outside the intercept after `a:insert` returns).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: String = lua
            .load(
                r#"
                local a = pmacs.buffer.create("a")
                local b = pmacs.buffer.create("b")
                pmacs.buffer.add_intercept(a, function(_op)
                    b:insert(0, "from-a-intercept")
                    return nil
                end)
                a:insert(0, "x")
                -- B should reflect the write the intercept performed,
                -- and A should reflect its own (unintercepted) insert.
                return b:slice(0, b:len()) .. "|" .. a:slice(0, a:len())
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, "from-a-intercept|x");
    }

    #[test]
    fn intercept_same_buffer_reentry_returns_concurrent_edit() {
        // M7.4 acceptance: an intercept on buffer A that calls
        // `A:insert(...)` (the same buffer) returns
        // BufferError::ConcurrentEdit, not a panic, not silent
        // corruption.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                pmacs.buffer.add_intercept(id, function(_op)
                    -- Same-buffer re-entry --- gated by editing_in_progress.
                    id:insert(0, "should not work")
                    return nil
                end)
                local ok, err = pcall(function() id:insert(0, "x") end)
                assert(not ok, "same-buffer re-entry must surface as error")
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("already being edited")
                || msg.contains("ConcurrentEdit")
                || msg.contains("re-entrant"),
            "expected ConcurrentEdit error message; got: {msg}"
        );
    }

    #[test]
    fn intercept_same_buffer_remove_returns_concurrent_edit() {
        // Reviewer-flagged sibling of the M7.4 same-buffer re-entry
        // case: `pmacs.buffer.remove(A)` from inside an intercept on
        // `A` would otherwise drop the buffer mid-edit. The registry
        // gates removal on `editing_in_progress` and returns a typed
        // concurrent-edit error, leaving the buffer in place.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("doomed")
                pmacs.buffer.add_intercept(id, function(_op)
                    -- Removing the buffer that is currently mid-edit
                    -- must surface as a typed error, not drop it.
                    pmacs.buffer.remove(id)
                    return nil
                end)
                local ok, err = pcall(function() id:insert(0, "x") end)
                assert(not ok, "remove during own intercept must error")
                -- Buffer must still resolve afterwards: the gate left
                -- it in place.
                assert(id:len() == 0, "buffer must still be live")
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("already being edited") || msg.contains("ConcurrentEdit"),
            "expected ConcurrentEdit-style error; got: {msg}"
        );
    }

    #[test]
    fn bypass_intercept_option_skips_intercept_chain() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let content: String = lua
            .load(
                r#"
                local id = pmacs.buffer.from_bytes("scratch", "abcdef")
                pmacs.buffer.add_intercept(id, function()
                    error("blocked")
                end)
                local ok = pcall(function() id:replace(0, 1, "X") end)
                assert(not ok, "plain replace must still run intercepts")
                id:replace(0, 1, "Y", { bypass_intercept = true })
                return id:slice(0, id:len())
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(content, "Ybcdef");
    }

    #[test]
    fn bypass_intercept_keeps_same_buffer_reentry_gate() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.from_bytes("scratch", "abc")
                pmacs.buffer.add_intercept(id, function()
                    id:replace(0, 1, "X", { bypass_intercept = true })
                    return nil
                end)
                local ok, err = pcall(function() id:replace(0, 1, "Y") end)
                assert(not ok, "same-buffer re-entry must still be gated")
                assert(id:slice(0, id:len()) == "abc")
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("already being edited") || msg.contains("ConcurrentEdit"),
            "expected ConcurrentEdit-style error; got: {msg}"
        );
    }

    #[test]
    fn cross_buffer_remove_from_intercept_succeeds() {
        // Cross-buffer remove from inside an intercept is allowed:
        // the gate is per-buffer, not global. A's intercept removing
        // B is the legitimate path for "tear down an auxiliary
        // scratch buffer when the primary edit completes".
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let still_present_after: bool = lua
            .load(
                r#"
                local a = pmacs.buffer.create("a")
                local b = pmacs.buffer.create("b")
                pmacs.buffer.add_intercept(a, function(_op)
                    pmacs.buffer.remove(b)
                    return nil
                end)
                a:insert(0, "x")
                -- Look for `b` in the live list: must be gone.
                for _, id in ipairs(pmacs.buffer.list()) do
                    if id == b then return true end
                end
                return false
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            !still_present_after,
            "cross-buffer remove from intercept should drop the target buffer"
        );
    }

    #[test]
    fn on_removed_callback_fires_once_and_can_be_removed() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let called: bool = lua
            .load(
                r#"
                local first = pmacs.buffer.create("first")
                local second = pmacs.buffer.create("second")
                local calls = 0
                pmacs.buffer.on_removed(first, function(dead)
                    assert(dead == first, "removed callback receives the removed buffer")
                    calls = calls + 1
                end)
                local handle = pmacs.buffer.on_removed(second, function()
                    calls = calls + 100
                end)
                assert(handle:remove() == true)
                assert(handle:remove() == false)
                pmacs.buffer.remove(first)
                pmacs.buffer.remove(second)
                return calls == 1
                "#,
            )
            .eval()
            .unwrap();
        assert!(called);
    }

    #[test]
    fn buffer_revision_bumps_on_edit_undo_and_redo() {
        // Compile-mode's external-edit guard (Q#CM2) leans on all
        // three bump sources: a same-length replace changes content
        // without changing length, and undo/redo are exactly the
        // mutations the guard exists to catch.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let ok: bool = lua
            .load(
                r#"
                local b = pmacs.buffer.from_bytes("rev", "abcd")
                local r0 = b:revision()
                b:insert(4, "e")
                local r1 = b:revision()
                b:replace(0, 1, "X") -- same-length replace still bumps
                local r2 = b:revision()
                assert(b:undo(), "undo applies")
                local r3 = b:revision()
                assert(b:redo(), "redo applies")
                local r4 = b:revision()
                return r1 > r0 and r2 > r1 and r3 > r2 and r4 > r3
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            ok,
            "revision must be strictly monotonic across edit/undo/redo"
        );
    }

    #[test]
    fn buffer_remove_prunes_buffer_local_keymaps() {
        let (lua, _reg, _cmds, kms, _hks) = fresh();
        let id: BufferIdLua = lua
            .load(
                r#"
                local id = pmacs.buffer.create("keyed")
                pmacs.keymap.bind {
                    scope = "buffer",
                    buffer = id,
                    sequence = "C-c x",
                    command = "buffer.save",
                }
                return id
                "#,
            )
            .eval()
            .unwrap();
        assert!(kms.borrow().buffers.contains_key(&id.0));
        lua.load("pmacs.buffer.remove(...)").call::<()>(id).unwrap();
        assert!(
            !kms.borrow().buffers.contains_key(&id.0),
            "buffer-local keymaps should be pruned on removal"
        );
    }

    #[test]
    fn m6_4_intercept_transform_overrides_position() {
        // An intercept may transform an insert by returning a table
        // with a different `pos`. The bytes pass through unchanged
        // (M6.4 limit). Used by the REPL to truncate edits to the
        // input region.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let content: String = lua
            .load(
                r#"
                local id = pmacs.buffer.from_bytes("scratch", "0123456789")
                -- Force every insert to land at position 5, regardless
                -- of where Lua asked.
                pmacs.buffer.add_intercept(id, function(op)
                    return { kind = "insert", pos = 5, bytes_len = op.bytes_len }
                end)
                id:insert(0, "XX")  -- requested at 0, transformed to pos 5
                return id:slice(0, id:len())
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(content, "01234XX56789");
    }

    #[test]
    fn m6_4_intercept_kind_change_is_typed_error() {
        // M6.4 forbids kind-changing transforms; the lifetime
        // contract on EditOp only holds when bytes are immutable
        // through the chain. The error message names the workaround.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.from_bytes("scratch", "abcdef")
                pmacs.buffer.add_intercept(id, function(_op)
                    return { kind = "delete", start = 0, ["end"] = 2 }
                end)
                local ok, err = pcall(function() id:insert(0, "x") end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("kind from `insert` to `delete`"),
            "expected kind-change error: {msg}"
        );
        assert!(
            msg.contains("kind=\"insert\""),
            "expected workaround hint: {msg}"
        );
    }

    #[test]
    fn m6_4_intercept_chain_threads_position_through() {
        // Same idea, simpler: verify the second intercept sees the
        // first's output, by running insert at a position where the
        // chained transform lands inside the buffer.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let content: String = lua
            .load(
                r#"
                local id = pmacs.buffer.from_bytes("scratch", "0123456789")
                pmacs.buffer.add_intercept(id, function(op)
                    return { kind = op.kind, pos = op.pos + 2, bytes_len = op.bytes_len }
                end)
                pmacs.buffer.add_intercept(id, function(op)
                    -- This intercept asserts it sees the first's output.
                    assert(op.pos == 2, "second intercept saw pos " .. op.pos)
                    return { kind = op.kind, pos = op.pos + 1, bytes_len = op.bytes_len }
                end)
                id:insert(0, "Y")  -- requested 0 → 2 → 3
                return id:slice(0, id:len())
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(content, "012Y3456789");
    }

    #[test]
    fn m6_4_intercept_remove_handle_idempotent() {
        // Removing an intercept stops further calls. Removing twice
        // is idempotent (returns false the second time).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: bool = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                local n = 0
                local h = pmacs.buffer.add_intercept(id, function(_op)
                    n = n + 1
                    return nil
                end)
                id:insert(0, "a")
                id:insert(1, "b")
                assert(n == 2, "intercept fired twice")
                local removed = pmacs.buffer.remove_intercept(h)
                assert(removed, "first remove must succeed")
                id:insert(2, "c")
                assert(n == 2, "intercept must not fire after removal")
                local removed2 = pmacs.buffer.remove_intercept(h)
                assert(not removed2, "second remove must report false")
                return id:slice(0, 3) == "abc"
                "#,
            )
            .eval()
            .unwrap();
        assert!(result);
    }

    #[test]
    fn m6_4_intercept_remove_stale_buffer_returns_false() {
        // Removing an intercept whose buffer was already removed
        // returns false rather than raising. (The intercept handle is
        // a (BufferId, ViewId) pair; if the buffer is gone, there's
        // nothing to detach.)
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: bool = lua
            .load(
                r#"
                local id = pmacs.buffer.create("doomed")
                local h = pmacs.buffer.add_intercept(id, function() return nil end)
                pmacs.buffer.remove(id)
                return pmacs.buffer.remove_intercept(h)
                "#,
            )
            .eval()
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn m6_4_intercept_returning_bad_type_is_typed_error() {
        // Intercept must return nil or a table. Anything else
        // (number, string, boolean) is a typed error from the Rust
        // side.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local id = pmacs.buffer.create("scratch")
                pmacs.buffer.add_intercept(id, function() return 42 end)
                local ok, err = pcall(function() id:insert(0, "x") end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("must return nil or a table") && msg.contains("integer"),
            "expected return-type error, got: {msg}"
        );
    }

    #[test]
    fn m6_4_intercept_op_table_carries_correct_fields_for_each_kind() {
        // The op table delivered to Lua carries kind-appropriate
        // fields (insert: pos+bytes_len; delete: start+end;
        // replace: start+end+bytes_len). This is the contract the
        // REPL package consumes; lock it in.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            local id = pmacs.buffer.from_bytes("scratch", "0123456789")
            local seen = {}
            pmacs.buffer.add_intercept(id, function(op)
                table.insert(seen, op)
                return nil
            end)
            id:insert(0, "X")
            id:delete(0, 1)
            id:replace(0, 2, "YZ")

            assert(#seen == 3, "three intercepts fired")
            assert(seen[1].kind == "insert" and seen[1].pos == 0 and seen[1].bytes_len == 1,
                "insert shape")
            assert(seen[2].kind == "delete" and seen[2].start == 0 and seen[2]["end"] == 1,
                "delete shape")
            assert(seen[3].kind == "replace" and seen[3].start == 0 and seen[3]["end"] == 2
                and seen[3].bytes_len == 2,
                "replace shape")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn m6_4_buffer_marks_follow_lua_edits_with_gravity() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            local id = pmacs.buffer.from_bytes("scratch", "abcd")
            local left = pmacs.buffer.mark_create(id, 2, { gravity = "left" })
            local right = pmacs.buffer.mark_create(id, 2, { gravity = "right" })
            id:insert(2, "XX")
            assert(left:get() == 2, "left mark moved to " .. left:get())
            assert(right:get() == 4, "right mark moved to " .. right:get())
            right:set(1)
            assert(right:pos() == 1, "set/pos roundtrip")
            assert(right:remove(), "first remove succeeds")
            assert(not right:remove(), "second remove reports false")
            "#,
        )
        .exec()
        .unwrap();
    }

    // -----------------------------------------------------------------
    // T M6.4: pmacs.ansi parser exposure
    // -----------------------------------------------------------------

    #[test]
    fn m6_4_ansi_parser_emits_text_event_for_plain_bytes() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: String = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("hello world")
                assert(#events == 1, "one event for plain text")
                assert(events[1].kind == "text", "event kind is text")
                return events[1].text
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn m6_4_ansi_parser_emits_set_style_with_color_table() {
        // SGR 31 (red foreground) → SetStyle event with fg = 1
        // (8-color palette index). The style table reuses the theme
        // surface's color encoding (string "default" / integer /
        // {r,g,b} array).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: i64 = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("\27[31mhello\27[0m")
                -- Expected stream: SetStyle(red), Text("hello"),
                -- SetStyle(default).
                local styled = nil
                for _, ev in ipairs(events) do
                    if ev.kind == "set_style" and type(ev.style.fg) == "number" then
                        styled = ev.style.fg
                        break
                    end
                end
                assert(styled ~= nil, "expected at least one SetStyle with palette fg")
                return styled
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, 1);
    }

    #[test]
    fn m6_4_ansi_parser_emits_truecolor_rgb_as_array() {
        // SGR 38;2;200;100;50 → fg = {200, 100, 50}
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (r, g, b): (i64, i64, i64) = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("\27[38;2;200;100;50mZ")
                local rgb = nil
                for _, ev in ipairs(events) do
                    if ev.kind == "set_style" and type(ev.style.fg) == "table" then
                        rgb = ev.style.fg
                        break
                    end
                end
                assert(rgb ~= nil, "expected truecolor SetStyle")
                return rgb[1], rgb[2], rgb[3]
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!((r, g, b), (200, 100, 50));
    }

    #[test]
    fn m6_4_ansi_parser_emits_carriage_return_and_erase_to_eol() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let kinds: Vec<String> = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("a\r\27[Kb")
                local out = {}
                for _, ev in ipairs(events) do
                    table.insert(out, ev.kind)
                end
                return out
                "#,
            )
            .eval()
            .unwrap();
        assert!(kinds.contains(&"carriage_return".to_string()));
        assert!(kinds.contains(&"erase_to_eol".to_string()));
    }

    #[test]
    fn m6_4_ansi_parser_emits_set_title_for_osc_0() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let title: String = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("\27]0;hello\7")
                for _, ev in ipairs(events) do
                    if ev.kind == "set_title" then return ev.title end
                end
                return ""
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(title, "hello");
    }

    #[test]
    fn m6_4_ansi_parser_emits_prompt_marker_events_for_osc_133() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: String = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("\27]133;A\7$ \27]133;B\7")
                assert(#events == 3, "expected prompt_start/text/prompt_end")
                return events[1].kind .. "|" .. events[2].kind .. "|" .. events[3].kind
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, "prompt_start|text|prompt_end");
    }

    #[test]
    fn m6_4_ansi_parser_emits_alt_screen_markers() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let kinds: Vec<String> = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local events = p:feed("\27[?1049hX\27[?1049l")
                local out = {}
                for _, ev in ipairs(events) do
                    table.insert(out, ev.kind)
                end
                return out
                "#,
            )
            .eval()
            .unwrap();
        assert!(kinds.contains(&"alt_screen_enter".to_string()));
        assert!(kinds.contains(&"alt_screen_exit".to_string()));
        // The "X" between the markers must NOT appear as a text event
        // (alt-screen suppression).
        assert!(!kinds.contains(&"text".to_string()));
    }

    #[test]
    fn m6_4_ansi_parser_state_persists_across_feed_calls() {
        // A CSI started in one feed continues in the next. M6.2's
        // coalescing makes per-tick chunk boundaries common; this
        // property is the same one the M6.3 truncated_csi test
        // covers, restated at the Lua surface to lock in the
        // contract from the consumer's side.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let text: String = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                local _ = p:feed("\27[")    -- partial CSI; no events expected
                local _ = p:feed("31m")     -- complete the CSI
                local events = p:feed("hi") -- now produces text
                for _, ev in ipairs(events) do
                    if ev.kind == "text" then return ev.text end
                end
                return ""
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(text, "hi");
    }

    #[test]
    fn m6_4_ansi_parser_reset_returns_to_ground() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: String = lua
            .load(
                r#"
                local p = pmacs.ansi.parser()
                p:feed("\27[")              -- enter CsiEntry mid-sequence
                p:reset()                   -- back to Ground
                local events = p:feed("a")  -- emits text "a"
                for _, ev in ipairs(events) do
                    if ev.kind == "text" then return ev.text end
                end
                return ""
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, "a");
    }

    // -----------------------------------------------------------------
    // Command system (T M2.3)
    // -----------------------------------------------------------------

    #[test]
    fn command_define_then_invoke_round_trip() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: i64 = lua
            .load(
                r#"
                pmacs.command.define {
                    name = "math.add",
                    description = "Sum two numbers.",
                    fn = function(a, b) return a + b end,
                }
                return pmacs.command.invoke("math.add", 2, 3)
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, 5);
    }

    #[test]
    fn command_define_missing_description_is_registration_error_r42() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local ok, err = pcall(function()
                    pmacs.command.define {
                        name = "no.desc",
                        fn = function() end,
                    }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("requires a non-empty description"),
            "expected R42 error: {msg}"
        );
        assert!(msg.contains("R42"), "expected R42 reference: {msg}");
    }

    #[test]
    fn command_define_unknown_field_is_rejected_r50() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local ok, err = pcall(function()
                    pmacs.command.define {
                        name = "x",
                        description = "y",
                        fn = function() end,
                        unknown_typo_field = 42,
                    }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("unknown field `unknown_typo_field`"),
            "expected unknown-field error: {msg}"
        );
    }

    #[test]
    fn command_define_duplicate_is_rejected() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                pmacs.command.define { name = "x", description = "y", fn = function() end }
                local ok, err = pcall(function()
                    pmacs.command.define { name = "x", description = "y", fn = function() end }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("already defined"),
            "expected duplicate-name error: {msg}"
        );
    }

    #[test]
    fn command_invoke_unknown_command_errors() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local ok, err = pcall(function() pmacs.command.invoke("nope") end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("\"nope\" not found"),
            "expected NotFound: {msg}"
        );
    }

    #[test]
    fn command_unregister_then_redefine_round_trips() {
        // Regression for the M8.2 reproducibility/reload finding:
        // packages that define commands at top level need an inverse
        // of `define` so re-running their chunk doesn't hit
        // DuplicateName.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: String = lua
            .load(
                r#"
                pmacs.command.define { name = "x", description = "v1", fn = function() return "v1" end }
                local removed = pmacs.command.unregister("x")
                assert(removed == true, "first unregister must report removed")
                pmacs.command.define { name = "x", description = "v2", fn = function() return "v2" end }
                return pmacs.command.invoke("x")
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(result, "v2");
    }

    #[test]
    fn command_unregister_unknown_returns_false() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let result: bool = lua
            .load(r#"return pmacs.command.unregister("nope")"#)
            .eval()
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn command_unregister_works_post_init_phase() {
        // unregister is registry CRUD, symmetric with define (which
        // also runs post-init). Reload itself isn't init-gated, so a
        // gate here would break the dev-loop for any package that
        // defines commands and tries to clean them up from
        // pmacs.packages.on_unload running post-startup.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.command.define { name = "x", description = "y", fn = function() end }"#)
            .exec()
            .unwrap();
        // Flip init-complete the way EditorState::new does in production.
        lua.app_data_ref::<InitCompleteFlag>()
            .expect("init flag installed by fresh()")
            .set_complete();
        let removed: bool = lua
            .load(r#"return pmacs.command.unregister("x")"#)
            .eval()
            .unwrap();
        assert!(
            removed,
            "unregister must succeed after init-complete (parity with define)"
        );
    }

    #[test]
    fn command_list_returns_insertion_order() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let names: Vec<String> = lua
            .load(
                r#"
                pmacs.command.define { name = "alpha",   description = "1", fn = function() end }
                pmacs.command.define { name = "beta",    description = "2", fn = function() end }
                pmacs.command.define { name = "charlie", description = "3", fn = function() end }
                return pmacs.command.list()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(names, vec!["alpha", "beta", "charlie"]);
    }

    #[test]
    fn describe_command_returns_full_metadata() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let info: Table = lua
            .load(
                r#"
                pmacs.command.define {
                    name = "buffer.save",
                    description = "Save the current buffer.",
                    fn = function() end,
                }
                return pmacs.describe.command("buffer.save")
                "#,
            )
            .eval()
            .unwrap();
        let name: String = info.get("name").unwrap();
        let description: String = info.get("description").unwrap();
        let source: String = info.get("source").unwrap();
        let bindings: Table = info.get("key_bindings").unwrap();
        assert_eq!(name, "buffer.save");
        assert_eq!(description, "Save the current buffer.");
        // Source captured at the chunk's line; the chunk has no name so
        // short_src is something like `[string "..."]`. Just assert it's
        // non-empty and includes a line number.
        assert!(!source.is_empty(), "source: {source}");
        assert!(source.contains(':'), "source must carry line: {source}");
        // key_bindings is empty in M2.3 but the table exists for palette code.
        let len: usize = bindings.len().unwrap() as usize;
        assert_eq!(len, 0);
    }

    #[test]
    fn describe_command_for_unknown_returns_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let v: Value = lua
            .load(r#"return pmacs.describe.command("does.not.exist")"#)
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil), "expected nil, got {v:?}");
    }

    #[test]
    fn predicate_is_preserved_and_callable() {
        let (lua, _reg, cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            pmacs.command.define {
                name = "with.pred",
                description = "Has a predicate.",
                fn = function() return 7 end,
                predicate = function() return true end,
            }
            "#,
        )
        .exec()
        .unwrap();
        let r = cmds.borrow();
        let cmd = r.get("with.pred").expect("registered");
        let pred = cmd.predicate.as_ref().expect("predicate present");
        let ok: bool = pred.call::<bool>(()).unwrap();
        assert!(ok);
    }

    #[test]
    fn rust_can_invoke_command_from_lua_host() {
        // Acceptance: commands defined in Lua are callable by name from
        // both Lua *and* the (Rust-side) command palette wiring.
        let mut host = crate::lua::LuaHost::new().unwrap();
        host.eval(
            None,
            r#"
            pmacs.command.define {
                name = "math.mul",
                description = "Multiply two numbers.",
                fn = function(a, b) return a * b end,
            }
            "#,
        )
        .unwrap();
        let args = mlua::MultiValue::from_iter([mlua::Value::Integer(6), mlua::Value::Integer(7)]);
        let result = host.invoke_command("math.mul", args).unwrap();
        let v = result
            .into_iter()
            .next()
            .expect("at least one return value");
        match v {
            mlua::Value::Integer(n) => assert_eq!(n, 42),
            other => panic!("expected 42, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Keymap system (T M2.4)
    // -----------------------------------------------------------------

    #[test]
    fn keymap_bind_global_then_lookup() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            pmacs.command.define { name = "buffer.save", description = "Save.", fn = function() end }
            pmacs.keymap.bind { scope = "global", sequence = "C-x C-s", command = "buffer.save" }
            "#,
        )
        .exec()
        .unwrap();
        let info: Table = lua
            .load(r#"return pmacs.keymap.lookup("C-x C-s")"#)
            .eval()
            .unwrap();
        let cmd: String = info.get("command").unwrap();
        let scope: String = info.get("scope").unwrap();
        let seq: String = info.get("sequence").unwrap();
        assert_eq!(cmd, "buffer.save");
        assert_eq!(scope, "global");
        assert_eq!(seq, "C-x C-s");
    }

    #[test]
    fn keymap_lookup_unbound_returns_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let v: Value = lua
            .load(r#"return pmacs.keymap.lookup("C-q")"#)
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil));
    }

    #[test]
    fn keymap_bind_conflict_surfaces_at_bind_time() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            pmacs.command.define { name = "a", description = "A.", fn = function() end }
            pmacs.command.define { name = "b", description = "B.", fn = function() end }
            pmacs.keymap.bind { scope = "global", sequence = "C-x C-s", command = "a" }
            "#,
        )
        .exec()
        .unwrap();
        let msg: String = lua
            .load(
                r#"
                local ok, err = pcall(function()
                    pmacs.keymap.bind { scope = "global", sequence = "C-x", command = "b" }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            msg.contains("would shadow"),
            "expected shadow-submap error: {msg}"
        );
    }

    #[test]
    fn keymap_describe_key_returns_full_metadata() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let info: Table = lua
            .load(
                r#"
                pmacs.command.define {
                    name = "buffer.save",
                    description = "Save the current buffer.",
                    fn = function() end,
                }
                pmacs.keymap.bind { scope = "global", sequence = "C-x C-s", command = "buffer.save" }
                return pmacs.describe.key("C-x C-s")
                "#,
            )
            .eval()
            .unwrap();
        let scope: String = info.get("scope").unwrap();
        let cmd: String = info.get("command").unwrap();
        let desc: String = info.get("description").unwrap();
        let source: String = info.get("source").unwrap();
        assert_eq!(scope, "global");
        assert_eq!(cmd, "buffer.save");
        assert_eq!(desc, "Save the current buffer.");
        assert!(!source.is_empty());
    }

    #[test]
    fn describe_command_cross_references_key_bindings() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let info: Table = lua
            .load(
                r#"
                pmacs.command.define {
                    name = "buffer.save",
                    description = "Save.",
                    fn = function() end,
                }
                pmacs.keymap.bind { scope = "global", sequence = "C-x C-s", command = "buffer.save" }
                pmacs.keymap.bind { scope = "global", sequence = "C-s",     command = "buffer.save" }
                return pmacs.describe.command("buffer.save")
                "#,
            )
            .eval()
            .unwrap();
        let bindings: Table = info.get("key_bindings").unwrap();
        let len: usize = bindings.len().unwrap() as usize;
        assert_eq!(len, 2, "expected two cross-referenced bindings");
        let mut sequences: Vec<String> = (1..=len)
            .map(|i| {
                let entry: Table = bindings.get(i).unwrap();
                entry.get::<String>("sequence").unwrap()
            })
            .collect();
        sequences.sort();
        assert_eq!(sequences, vec!["C-s", "C-x C-s"]);
    }

    #[test]
    fn keymap_buffer_local_overrides_global() {
        let (lua, _reg, _cmds, kms, _hks) = fresh();
        lua.load(
            r#"
            pmacs.command.define { name = "g", description = "G.", fn = function() end }
            pmacs.command.define { name = "b", description = "B.", fn = function() end }
            local id = pmacs.buffer.create("scratch")
            pmacs.keymap.bind { scope = "global", sequence = "C-s", command = "g" }
            pmacs.keymap.bind { scope = "buffer", buffer = id, sequence = "C-s", command = "b" }
            -- We assert the resolution from Rust below; lookup() (no
            -- editor wiring yet) only consults global.
            "#,
        )
        .exec()
        .unwrap();
        // Resolve directly against the stack with the buffer scope.
        let km = kms.borrow();
        let buffer_id = km
            .buffers
            .keys()
            .next()
            .copied()
            .expect("buffer-local map exists");
        let chords = parse_sequence("C-s").unwrap();
        match km.resolve(&chords, Some(buffer_id), &[]) {
            crate::keymap_stack::StackResolution::Bound(rb) => {
                assert_eq!(rb.binding.command, "b");
                assert_eq!(rb.scope, crate::keymap_stack::Scope::Buffer(buffer_id));
            }
            other => panic!("expected buffer-local Bound, got {other:?}"),
        }
    }

    #[test]
    fn keymap_unbind_global_removes_binding() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"
            pmacs.command.define { name = "x", description = "X.", fn = function() end }
            pmacs.keymap.bind { scope = "global", sequence = "C-z", command = "x" }
            pmacs.keymap.unbind { scope = "global", sequence = "C-z" }
            "#,
        )
        .exec()
        .unwrap();
        let v: Value = lua
            .load(r#"return pmacs.keymap.lookup("C-z")"#)
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil));
    }

    #[test]
    fn keymap_unknown_scope_is_typed_error() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                pmacs.command.define { name = "x", description = "X.", fn = function() end }
                local ok, err = pcall(function()
                    pmacs.keymap.bind { scope = "bogus", sequence = "C-z", command = "x" }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(msg.contains("unknown keymap scope"), "msg: {msg}");
    }

    #[test]
    fn keymap_buffer_scope_requires_buffer_field() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                pmacs.command.define { name = "x", description = "X.", fn = function() end }
                local ok, err = pcall(function()
                    pmacs.keymap.bind { scope = "buffer", sequence = "C-z", command = "x" }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(msg.contains("requires field `buffer`"), "msg: {msg}");
    }

    #[test]
    fn keymap_list_returns_all_bindings_with_scope() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let entries: Table = lua
            .load(
                r#"
                pmacs.command.define { name = "g", description = "G.", fn = function() end }
                pmacs.command.define { name = "m", description = "M.", fn = function() end }
                pmacs.keymap.bind { scope = "global", sequence = "C-q", command = "g" }
                pmacs.keymap.bind { scope = "mode", mode = "normal", sequence = "C-s", command = "m" }
                return pmacs.keymap.list()
                "#,
            )
            .eval()
            .unwrap();
        let len: usize = entries.len().unwrap() as usize;
        let mut lines: Vec<String> = (1..=len)
            .map(|i| {
                let entry: Table = entries.get(i).unwrap();
                let s: String = entry.get("scope").unwrap();
                let seq: String = entry.get("sequence").unwrap();
                let c: String = entry.get("command").unwrap();
                format!("{s}:{seq}:{c}")
            })
            .collect();
        lines.sort();
        assert_eq!(lines, vec!["global:C-q:g", "mode:normal:C-s:m"]);
    }

    // ---- describe-* introspection (T M2.11) -------------------------------

    #[test]
    fn describe_buffer_returns_metadata() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let t: Table = lua
            .load(
                r#"
                local id = pmacs.buffer.from_bytes("scratch", "hello")
                id:insert(5, "!")
                return pmacs.describe.buffer(id)
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(t.get::<String>("name").unwrap(), "scratch");
        assert_eq!(t.get::<i64>("length").unwrap(), 6);
        assert!(t.get::<bool>("modified").unwrap(), "expected modified");
        assert_eq!(t.get::<i64>("view_count").unwrap(), 0);
    }

    #[test]
    fn describe_buffer_unknown_id_yields_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        // Make a real id, remove it, then ask describe.buffer about it.
        let v: Value = lua
            .load(
                r#"
                local id = pmacs.buffer.create("doomed")
                pmacs.buffer.remove(id)
                return pmacs.describe.buffer(id)
                "#,
            )
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil));
    }

    #[test]
    fn describe_mode_lists_bindings() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let t: Table = lua
            .load(
                r#"
                pmacs.command.define { name = "x", description = "X.", fn = function() end }
                pmacs.command.define { name = "y", description = "Y.", fn = function() end }
                pmacs.keymap.bind { scope = "mode", mode = "demo", sequence = "C-x", command = "x" }
                pmacs.keymap.bind { scope = "mode", mode = "demo", sequence = "C-y", command = "y" }
                return pmacs.describe.mode("demo")
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(t.get::<String>("name").unwrap(), "demo");
        let bindings: Table = t.get("bindings").unwrap();
        let n = bindings.len().unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn describe_mode_unknown_yields_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let v: Value = lua
            .load("return pmacs.describe.mode('nope')")
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil));
    }

    #[test]
    fn describe_hook_lists_callbacks_in_registration_order() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let sources: Vec<String> = {
            let t: Table = lua
                .load(
                    r#"
                    pmacs.hook.define { name = "demo", description = "Demo hook." }
                    pmacs.hook.add("demo", function() end)
                    pmacs.hook.add("demo", function() end)
                    pmacs.hook.add("demo", function() end)
                    return pmacs.describe.hook("demo")
                    "#,
                )
                .eval()
                .unwrap();
            assert_eq!(t.get::<String>("name").unwrap(), "demo");
            assert_eq!(t.get::<String>("description").unwrap(), "Demo hook.");
            let cbs: Table = t.get("callbacks").unwrap();
            let n = cbs.len().unwrap() as usize;
            (1..=n)
                .map(|i| {
                    let e: Table = cbs.get(i).unwrap();
                    e.get::<String>("source").unwrap()
                })
                .collect()
        };
        // Three callbacks, all with non-empty source --- the line numbers
        // appear in the order the calls happened.
        assert_eq!(sources.len(), 3);
        for s in &sources {
            assert!(!s.is_empty(), "source: {s}");
        }
    }

    #[test]
    fn describe_hook_unknown_yields_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let v: Value = lua
            .load("return pmacs.describe.hook('not-a-hook')")
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil));
    }

    #[test]
    fn describe_view_reports_view_count() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let t: Table = lua
            .load(
                r#"
                local id = pmacs.buffer.create("v")
                return pmacs.describe.view(id)
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(t.get::<String>("buffer_name").unwrap(), "v");
        assert_eq!(t.get::<i64>("view_count").unwrap(), 0);
        let ids: Table = t.get("view_ids").unwrap();
        assert_eq!(ids.len().unwrap(), 0);
    }

    #[test]
    fn pmacs_hook_define_rejects_missing_description() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local ok, err = pcall(function()
                    pmacs.hook.define { name = "h" }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(msg.contains("non-empty description"), "msg: {msg}");
    }

    #[test]
    fn pmacs_hook_define_rejects_unknown_field() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let msg: String = lua
            .load(
                r#"
                local ok, err = pcall(function()
                    pmacs.hook.define { name = "h", description = "ok", run = "all" }
                end)
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .unwrap();
        assert!(msg.contains("unknown field"), "msg: {msg}");
    }

    // ----- M5.6c: InitCompleteFlag + require_init_phase -----

    #[test]
    fn init_complete_flag_default_is_false() {
        let f = InitCompleteFlag::new();
        assert!(!f.is_complete());
    }

    #[test]
    fn init_complete_flag_set_complete_flips_to_true() {
        let f = InitCompleteFlag::new();
        f.set_complete();
        assert!(f.is_complete());
    }

    #[test]
    fn init_complete_flag_is_idempotent() {
        // Calling set_complete twice is fine — no panic, stays true.
        // Matters because EditorState::new could in theory be re-entered
        // through some test path; the flag should not care.
        let f = InitCompleteFlag::new();
        f.set_complete();
        f.set_complete();
        assert!(f.is_complete());
    }

    #[test]
    fn init_complete_flag_clones_share_state() {
        // The Rc<Cell<bool>> is the whole point: the flag stored in
        // app_data and any clone held by a binding closure observe the
        // same toggle.
        let f = InitCompleteFlag::new();
        let cloned = f.clone();
        assert!(!cloned.is_complete());
        f.set_complete();
        assert!(cloned.is_complete());
    }

    #[test]
    fn install_registers_init_flag_in_app_data() {
        // After install(), bindings can find the flag via app_data_ref.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let flag = lua
            .app_data_ref::<InitCompleteFlag>()
            .expect("flag should be registered by install()");
        assert!(!flag.is_complete(), "init phase begins as not-complete");
    }

    #[test]
    fn require_init_phase_succeeds_before_set_complete() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        require_init_phase(&lua, "pmacs.attach").expect("init phase: gate should permit the call");
    }

    #[test]
    fn require_init_phase_errors_after_set_complete() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<InitCompleteFlag>()
            .expect("flag installed")
            .set_complete();

        let err = require_init_phase(&lua, "pmacs.attach")
            .expect_err("post-init: gate should refuse the call");
        let msg = err.to_string();
        assert!(msg.contains("pmacs.attach"), "{msg}");
        assert!(
            msg.contains("init.lua"),
            "error should name the right call site: {msg}"
        );
        assert!(
            msg.contains("CLI flag"),
            "error should point at the workaround: {msg}"
        );
    }

    #[test]
    fn require_init_phase_with_no_flag_yields_typed_error() {
        // A bare Lua state (no install() called) is a programming
        // error; the gate reports it via NoInitFlag, distinct from
        // InitOnlyApi so callers can tell setup-mistakes from
        // user-mistakes apart.
        let lua = Lua::new();
        let err = require_init_phase(&lua, "pmacs.attach").expect_err("no flag → typed error");
        let msg = err.to_string();
        assert!(msg.contains("InitCompleteFlag"), "{msg}");
        assert!(msg.contains("not installed"), "{msg}");
    }

    #[test]
    fn flag_stays_false_through_install_and_builtin_loads() {
        // The full install sequence (install + install_editor + a few
        // builtin chunk evals) must not flip the flag — the flag's
        // contract is "user init.lua has finished," not "any Lua has
        // run." Simulate the early lifecycle without going through
        // EditorState::new (which is `cfg(not(test))` for config).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        // Run a chunk to confirm Lua execution does not flip the flag.
        let _: Value = lua.load("return 1 + 1").eval().unwrap();
        let flag = lua
            .app_data_ref::<InitCompleteFlag>()
            .expect("flag installed");
        assert!(
            !flag.is_complete(),
            "ordinary Lua eval must not flip the init flag"
        );
    }

    // ----- M5.6d: pmacs.attach{...} init-time binding -----

    fn requested(lua: &Lua) -> RequestedAttach {
        lua.app_data_ref::<RequestedAttach>()
            .expect("RequestedAttach installed")
            .clone()
    }

    fn assert_eval_err_contains(lua: &Lua, chunk: &str, needle: &str) {
        let err = lua.load(chunk).exec().expect_err("chunk should raise");
        let msg = err.to_string();
        assert!(
            msg.contains(needle),
            "expected error to contain {needle:?}; got {msg}"
        );
    }

    #[test]
    fn attach_target_string_local_records_request() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ target = "local:/run/pmacs/default.sock" }"#)
            .exec()
            .expect("local target accepted");
        let req = requested(&lua).get().expect("request recorded");
        match req {
            AttachTarget::LocalSocket(p) => {
                assert_eq!(p, std::path::PathBuf::from("/run/pmacs/default.sock"));
            }
            other => panic!("expected LocalSocket, got {other:?}"),
        }
    }

    #[test]
    fn attach_target_string_ssh_records_request_in_v01() {
        // Stub posture: SSH parses, validates, and gets stored. The
        // activation-time error fires later in the dispatcher, not
        // here. This is what lets a user write the line in init.lua
        // today and have it just work in v0.2.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ target = "ssh:lev@mac-studio/research" }"#)
            .exec()
            .expect("ssh target accepted at parse/store time");
        let req = requested(&lua).get().expect("request recorded");
        match req {
            AttachTarget::Ssh {
                host,
                user,
                instance_name,
            } => {
                assert_eq!(host, "mac-studio");
                assert_eq!(user.as_deref(), Some("lev"));
                assert_eq!(instance_name.as_deref(), Some("research"));
            }
            other => panic!("expected Ssh, got {other:?}"),
        }
    }

    #[test]
    fn attach_kwargs_local_socket() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ kind = "local", socket = "/tmp/x.sock" }"#)
            .exec()
            .expect("kwargs local accepted");
        let req = requested(&lua).get().expect("request recorded");
        assert_eq!(req.kind_name(), "local");
    }

    #[test]
    fn attach_kwargs_ssh_with_user_and_instance() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ kind = "ssh", host = "h", user = "u", instance = "i" }"#)
            .exec()
            .expect("kwargs ssh accepted");
        let req = requested(&lua).get().expect("recorded");
        match req {
            AttachTarget::Ssh {
                host,
                user,
                instance_name,
            } => {
                assert_eq!(host, "h");
                assert_eq!(user.as_deref(), Some("u"));
                assert_eq!(instance_name.as_deref(), Some("i"));
            }
            other => panic!("expected Ssh, got {other:?}"),
        }
    }

    #[test]
    fn attach_kwargs_tls_records_for_v02() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ kind = "tls", endpoint = "h:9", cert = "/etc/c" }"#)
            .exec()
            .expect("kwargs tls accepted (stored, error deferred to activation)");
        assert_eq!(requested(&lua).get().unwrap().kind_name(), "tls");
    }

    #[test]
    fn attach_kwargs_custom_command_table() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ kind = "custom", command = { "docker", "exec" } }"#)
            .exec()
            .expect("kwargs custom accepted");
        match requested(&lua).get().unwrap() {
            AttachTarget::Custom { command } => {
                assert_eq!(command, vec!["docker".to_string(), "exec".to_string()]);
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn attach_after_init_complete_errors_with_workaround_pointer() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<InitCompleteFlag>()
            .unwrap()
            .set_complete();
        let err = lua
            .load(r#"pmacs.attach{ target = "local:/x.sock" }"#)
            .exec()
            .expect_err("post-init: gate should fire");
        let msg = err.to_string();
        assert!(msg.contains("pmacs.attach"), "{msg}");
        assert!(msg.contains("init.lua"), "{msg}");
        assert!(msg.contains("CLI flag"), "{msg}");
        // No request should have been recorded.
        assert!(requested(&lua).get().is_none());
    }

    #[test]
    fn attach_called_twice_errors_and_preserves_first_request() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ target = "local:/first.sock" }"#)
            .exec()
            .unwrap();
        let err = lua
            .load(r#"pmacs.attach{ target = "local:/second.sock" }"#)
            .exec()
            .expect_err("second call should error");
        let msg = err.to_string();
        assert!(msg.contains("already been called"), "{msg}");
        assert!(
            msg.contains("local:/first.sock"),
            "error names the prior target: {msg}"
        );
        // First request is preserved.
        match requested(&lua).get().unwrap() {
            AttachTarget::LocalSocket(p) => {
                assert_eq!(p, std::path::PathBuf::from("/first.sock"));
            }
            other => panic!("expected first request preserved, got {other:?}"),
        }
    }

    #[test]
    fn attach_missing_kind_and_target_errors() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        assert_eval_err_contains(&lua, "pmacs.attach{}", "either `target`");
    }

    #[test]
    fn attach_unknown_kind_errors_with_menu() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let err = lua
            .load(r#"pmacs.attach{ kind = "smtp" }"#)
            .exec()
            .expect_err("unknown kind");
        let msg = err.to_string();
        assert!(msg.contains("smtp"), "{msg}");
        for k in ["local", "ssh", "tls", "custom"] {
            assert!(msg.contains(k), "menu missing {k}: {msg}");
        }
    }

    #[test]
    fn attach_kwargs_local_missing_socket_errors() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        assert_eval_err_contains(&lua, r#"pmacs.attach{ kind = "local" }"#, "field `socket`");
    }

    #[test]
    fn attach_kwargs_ssh_missing_host_errors() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        assert_eval_err_contains(&lua, r#"pmacs.attach{ kind = "ssh" }"#, "field `host`");
    }

    #[test]
    fn attach_target_string_invalid_surfaces_parse_error() {
        // Validation runs as the final step of parse, so embedded
        // nulls or syntax errors come through with the protocol-side
        // error messages (we don't translate them in the binding).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let err = lua
            .load(r#"pmacs.attach{ target = "smtp:host" }"#)
            .exec()
            .expect_err("bad scheme rejected");
        let msg = err.to_string();
        assert!(msg.contains("unknown attach target kind"), "{msg}");
    }

    #[test]
    fn attach_target_string_validates_after_parse() {
        // Path with embedded null: parse accepts split, validate rejects.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let err = lua
            .load(r#"pmacs.attach{ target = "local:/foo\0bar" }"#)
            .exec()
            .expect_err("null byte rejected");
        let msg = err.to_string();
        assert!(msg.contains("null byte"), "{msg}");
    }

    #[test]
    fn attach_kwargs_with_invalid_field_validates() {
        // Ssh with an empty host is valid syntax (no parse failure)
        // but invalid semantically. Validation must catch it from
        // the kwargs path, not just the target-string path.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let err = lua
            .load(r#"pmacs.attach{ kind = "ssh", host = "" }"#)
            .exec()
            .expect_err("empty host rejected");
        let msg = err.to_string();
        assert!(msg.contains("host must not be empty"), "{msg}");
    }

    #[test]
    fn attach_kwargs_custom_coerces_lua_string_compatible_values() {
        // Lua's automatic number→string coercion applies through mlua's
        // `sequence_values::<String>` iterator. `42` becomes `"42"`.
        // Pin this so behaviour change in either direction is visible.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ kind = "custom", command = { "ok", 42 } }"#)
            .exec()
            .expect("integer coerces to string per Lua semantics");
        match requested(&lua).get().unwrap() {
            AttachTarget::Custom { command } => {
                assert_eq!(command, vec!["ok".to_string(), "42".to_string()]);
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn attach_kwargs_custom_rejects_genuinely_non_coercible_element() {
        // A function value cannot coerce to string. This is the real
        // "non-string" case the schema error is for.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        assert_eval_err_contains(
            &lua,
            r#"pmacs.attach{ kind = "custom", command = { "ok", function() end } }"#,
            "table of strings",
        );
    }

    #[test]
    fn attach_target_string_takes_priority_over_kind() {
        // If both `target` and `kind` are present, `target` wins.
        // That matches Lua kwarg ergonomics: a user with a string they
        // already built shouldn't have to also clear `kind`.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(
            r#"pmacs.attach{ target = "local:/from-string", kind = "ssh", host = "ignored" }"#,
        )
        .exec()
        .unwrap();
        match requested(&lua).get().unwrap() {
            AttachTarget::LocalSocket(p) => {
                assert_eq!(p, std::path::PathBuf::from("/from-string"));
            }
            other => panic!("target string did not win, got {other:?}"),
        }
    }

    #[test]
    fn requested_attach_take_consumes_then_yields_none() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"pmacs.attach{ target = "local:/x.sock" }"#)
            .exec()
            .unwrap();
        let r = requested(&lua);
        assert!(r.take().is_some(), "first take returns the request");
        assert!(r.take().is_none(), "second take yields None");
    }

    #[test]
    fn requested_attach_try_set_rejects_overwrite() {
        let r = RequestedAttach::new();
        let first = AttachTarget::LocalSocket(std::path::PathBuf::from("/a"));
        let second = AttachTarget::LocalSocket(std::path::PathBuf::from("/b"));
        r.try_set(first.clone()).expect("first set ok");
        let returned = r
            .try_set(second.clone())
            .expect_err("second set should error");
        assert_eq!(returned, first, "returned target is the prior one");
        assert_eq!(r.get().unwrap(), first, "slot still holds the prior");
    }

    #[test]
    fn requested_attach_clones_share_state() {
        // A binding closure clones the slot; both clones must observe
        // the same set/take, mirroring the InitCompleteFlag contract.
        let r = RequestedAttach::new();
        let cloned = r.clone();
        let target = AttachTarget::LocalSocket(std::path::PathBuf::from("/s"));
        r.try_set(target.clone()).unwrap();
        assert_eq!(cloned.get().unwrap(), target);
        assert_eq!(cloned.take(), Some(target));
        assert!(r.get().is_none(), "take through one clone empties all");
    }

    // ----- M5.6e: pmacs.current_attachment() -----

    fn sample_handle_local() -> AttachmentHandle {
        AttachmentHandle::new(
            crate::protocol::FrontendId(7),
            InstanceIdentity {
                pmacs_version: "0.1.0".into(),
                build_hash: Some("a3f9c21".into()),
                instance_name: Some("research".into()),
                uptime_secs: 2_847,
                working_directory: "/home/researcher/project".into(),
            },
            AttachTarget::LocalSocket(std::path::PathBuf::from("/run/p.sock")),
        )
    }

    #[test]
    fn current_attachment_returns_nil_by_default() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let v: Value = lua
            .load("return pmacs.current_attachment()")
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Nil), "expected nil; got {v:?}");
    }

    #[test]
    fn current_attachment_returns_table_when_slot_populated() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<CurrentAttachmentSlot>()
            .unwrap()
            .set(sample_handle_local());
        let v: Value = lua
            .load("return pmacs.current_attachment()")
            .eval()
            .unwrap();
        assert!(matches!(v, Value::Table(_)), "expected table; got {v:?}");
    }

    #[test]
    fn current_attachment_table_top_level_fields() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<CurrentAttachmentSlot>()
            .unwrap()
            .set(sample_handle_local());
        let (fid, has_identity, has_target): (i64, bool, bool) = lua
            .load(
                r#"
                local h = pmacs.current_attachment()
                return h.frontend_id, type(h.identity) == "table", type(h.target) == "table"
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(fid, 7);
        assert!(has_identity);
        assert!(has_target);
    }

    #[test]
    fn current_attachment_identity_fields_present_when_set() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<CurrentAttachmentSlot>()
            .unwrap()
            .set(sample_handle_local());
        let (ver, hash, name, uptime, cwd): (String, String, String, i64, String) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.identity.pmacs_version, h.identity.build_hash,
                       h.identity.instance_name, h.identity.uptime_secs,
                       h.identity.working_directory
                ",
            )
            .eval()
            .unwrap();
        assert_eq!(ver, "0.1.0");
        assert_eq!(hash, "a3f9c21");
        assert_eq!(name, "research");
        assert_eq!(uptime, 2_847);
        assert_eq!(cwd, "/home/researcher/project");
    }

    #[test]
    fn current_attachment_identity_optional_fields_absent_as_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let h = AttachmentHandle::new(
            crate::protocol::FrontendId(2),
            InstanceIdentity {
                pmacs_version: "0.1.0".into(),
                build_hash: None,
                instance_name: None,
                uptime_secs: 1,
                working_directory: "/x".into(),
            },
            AttachTarget::LocalSocket(std::path::PathBuf::from("/s")),
        );
        lua.app_data_ref::<CurrentAttachmentSlot>().unwrap().set(h);
        let (hash_is_nil, name_is_nil): (bool, bool) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.identity.build_hash == nil, h.identity.instance_name == nil
                ",
            )
            .eval()
            .unwrap();
        assert!(hash_is_nil);
        assert!(name_is_nil);
    }

    #[test]
    fn current_attachment_target_local_shape() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<CurrentAttachmentSlot>()
            .unwrap()
            .set(sample_handle_local());
        let (kind, path, display): (String, String, String) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.target.kind, h.target.path, h.target.display
                ",
            )
            .eval()
            .unwrap();
        assert_eq!(kind, "local");
        assert_eq!(path, "/run/p.sock");
        assert_eq!(display, "local:/run/p.sock");
    }

    #[test]
    fn current_attachment_target_ssh_with_user_and_instance_shape() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let h = AttachmentHandle::new(
            crate::protocol::FrontendId(2),
            sample_handle_local().identity,
            AttachTarget::Ssh {
                host: "mac-studio".into(),
                user: Some("lev".into()),
                instance_name: Some("research".into()),
            },
        );
        lua.app_data_ref::<CurrentAttachmentSlot>().unwrap().set(h);
        let (kind, host, user, inst, display): (String, String, String, String, String) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.target.kind, h.target.host, h.target.user,
                       h.target.instance, h.target.display
                ",
            )
            .eval()
            .unwrap();
        assert_eq!(kind, "ssh");
        assert_eq!(host, "mac-studio");
        assert_eq!(user, "lev");
        assert_eq!(inst, "research");
        assert_eq!(display, "ssh:lev@mac-studio/research");
    }

    #[test]
    fn current_attachment_target_ssh_minimal_omits_optional_keys() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let h = AttachmentHandle::new(
            crate::protocol::FrontendId(2),
            sample_handle_local().identity,
            AttachTarget::Ssh {
                host: "h".into(),
                user: None,
                instance_name: None,
            },
        );
        lua.app_data_ref::<CurrentAttachmentSlot>().unwrap().set(h);
        let (user_nil, inst_nil): (bool, bool) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.target.user == nil, h.target.instance == nil
                ",
            )
            .eval()
            .unwrap();
        assert!(user_nil);
        assert!(inst_nil);
    }

    #[test]
    fn current_attachment_target_tls_shape() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let h = AttachmentHandle::new(
            crate::protocol::FrontendId(2),
            sample_handle_local().identity,
            AttachTarget::Tls {
                endpoint: "host:9999".into(),
                cert: std::path::PathBuf::from("/etc/pmacs.crt"),
            },
        );
        lua.app_data_ref::<CurrentAttachmentSlot>().unwrap().set(h);
        let (kind, endpoint, cert): (String, String, String) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.target.kind, h.target.endpoint, h.target.cert
                ",
            )
            .eval()
            .unwrap();
        assert_eq!(kind, "tls");
        assert_eq!(endpoint, "host:9999");
        assert_eq!(cert, "/etc/pmacs.crt");
    }

    #[test]
    fn current_attachment_target_custom_shape() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let h = AttachmentHandle::new(
            crate::protocol::FrontendId(2),
            sample_handle_local().identity,
            AttachTarget::Custom {
                command: vec!["docker".into(), "exec".into(), "container".into()],
            },
        );
        lua.app_data_ref::<CurrentAttachmentSlot>().unwrap().set(h);
        let (kind, len, second): (String, i64, String) = lua
            .load(
                r"
                local h = pmacs.current_attachment()
                return h.target.kind, #h.target.command, h.target.command[2]
                ",
            )
            .eval()
            .unwrap();
        assert_eq!(kind, "custom");
        assert_eq!(len, 3);
        assert_eq!(second, "exec");
    }

    #[test]
    fn current_attachment_target_display_round_trips_through_attach_target_parse() {
        // The `display` field on the table is the canonical string the
        // user could feed back into `pmacs.attach{ target = ... }`.
        // Parsing it must reproduce the original target.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let original = AttachTarget::Ssh {
            host: "h".into(),
            user: Some("u".into()),
            instance_name: None,
        };
        let h = AttachmentHandle::new(
            crate::protocol::FrontendId(2),
            sample_handle_local().identity,
            original.clone(),
        );
        lua.app_data_ref::<CurrentAttachmentSlot>().unwrap().set(h);
        let display: String = lua
            .load("return pmacs.current_attachment().target.display")
            .eval()
            .unwrap();
        assert_eq!(AttachTarget::parse(&display).unwrap(), original);
    }

    #[test]
    fn current_attachment_table_is_freshly_built_per_call() {
        // Stability disclaimer: each call returns a new table.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<CurrentAttachmentSlot>()
            .unwrap()
            .set(sample_handle_local());
        let same: bool = lua
            .load(
                r"
                local a = pmacs.current_attachment()
                local b = pmacs.current_attachment()
                return rawequal(a, b)
                ",
            )
            .eval()
            .unwrap();
        assert!(
            !same,
            "two calls should return distinct tables (snapshot semantics)"
        );
    }

    #[test]
    fn current_attachment_no_slot_yields_typed_error() {
        // A bare Lua state without install() is a programming error;
        // the binding reports it via NoCurrentAttachmentSlot.
        let lua = Lua::new();
        // Manually register only the binding without its slot.
        let f = install_current_attachment_binding(&lua).unwrap();
        let err = f.call::<Value>(()).expect_err("no slot → error");
        let msg = err.to_string();
        assert!(msg.contains("CurrentAttachmentSlot"), "{msg}");
        assert!(msg.contains("not installed"), "{msg}");
    }

    #[test]
    fn current_attachment_slot_set_and_clear() {
        let s = CurrentAttachmentSlot::new();
        assert!(s.get().is_none());
        s.set(sample_handle_local());
        assert!(s.get().is_some());
        s.clear();
        assert!(s.get().is_none());
    }

    // -----------------------------------------------------------------------
    // T M5.6f --- pmacs.instance.{identity, echo_line, show}
    // -----------------------------------------------------------------------

    #[test]
    fn instance_identity_returns_table_with_self_fields() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (ver, cwd, has_uptime): (String, String, bool) = lua
            .load(
                r"
                local id = pmacs.instance.identity()
                return id.pmacs_version, id.working_directory,
                       type(id.uptime_secs) == 'number'
                ",
            )
            .eval()
            .unwrap();
        assert_eq!(ver, env!("CARGO_PKG_VERSION"));
        assert!(!cwd.is_empty(), "cwd must not be empty");
        assert!(has_uptime, "uptime must be a number");
    }

    #[test]
    fn instance_identity_instance_name_defaults_to_nil() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let name_is_nil: bool = lua
            .load("return pmacs.instance.identity().instance_name == nil")
            .eval()
            .unwrap();
        assert!(
            name_is_nil,
            "default instance_name must be nil before set_name override"
        );
    }

    #[test]
    fn instance_identity_reflects_set_name_override() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<LocalInstanceInfo>()
            .unwrap()
            .set_name(Some("work".into()));
        let name: String = lua
            .load("return pmacs.instance.identity().instance_name")
            .eval()
            .unwrap();
        assert_eq!(name, "work");
    }

    #[test]
    fn instance_identity_uptime_is_monotonic_nondecreasing() {
        // Two consecutive reads must report uptimes in non-decreasing
        // order, since the slot's `started` anchor is fixed at install.
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let (a, b): (i64, i64) = lua
            .load(
                r"
                local a = pmacs.instance.identity().uptime_secs
                local b = pmacs.instance.identity().uptime_secs
                return a, b
                ",
            )
            .eval()
            .unwrap();
        assert!(b >= a, "uptime decreased between calls: {a} → {b}");
    }

    #[test]
    fn instance_identity_returns_fresh_table_per_call() {
        // Snapshot semantics: two consecutive calls return distinct
        // tables (mirrors `pmacs.current_attachment` per M5.6e).
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let same: bool = lua
            .load(
                r"
                local a = pmacs.instance.identity()
                local b = pmacs.instance.identity()
                return rawequal(a, b)
                ",
            )
            .eval()
            .unwrap();
        assert!(!same, "two calls must return distinct tables");
    }

    #[test]
    fn instance_echo_line_describes_self_when_no_attachment() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let line: String = lua
            .load("return pmacs.instance.echo_line()")
            .eval()
            .unwrap();
        assert!(line.starts_with("pmacs "), "{line}");
        // Default name is None → "[local]" segment.
        assert!(line.contains("[local]"), "{line}");
        assert!(line.contains("uptime "), "{line}");
        assert!(!line.contains('\n'), "echo line must be single-line");
    }

    #[test]
    fn instance_echo_line_uses_set_name_when_set() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<LocalInstanceInfo>()
            .unwrap()
            .set_name(Some("work".into()));
        let line: String = lua
            .load("return pmacs.instance.echo_line()")
            .eval()
            .unwrap();
        assert!(line.contains("[work]"), "{line}");
    }

    #[test]
    fn instance_echo_line_describes_attachment_when_present() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        lua.app_data_ref::<CurrentAttachmentSlot>()
            .unwrap()
            .set(sample_handle_local());
        let line: String = lua
            .load("return pmacs.instance.echo_line()")
            .eval()
            .unwrap();
        assert!(
            line.contains("attached to research"),
            "expected remote name in line: {line}"
        );
        assert!(
            line.contains("local:/run/p.sock"),
            "expected target string in line: {line}"
        );
    }

    #[test]
    fn instance_show_returns_buffer_and_creates_named_buffer() {
        let (lua, reg, _cmds, _kms, _hks) = fresh();
        // `show()` returns a `BufferIdLua`; round-trip through the
        // registry confirms the name.
        let id_lua: BufferIdLua = lua.load("return pmacs.instance.show()").eval().unwrap();
        let r = reg.borrow();
        let buf = r.get(id_lua.0).expect("buffer exists");
        assert_eq!(buf.name(), crate::instance_buffer::INSTANCE_BUFFER_NAME);
    }

    #[test]
    fn instance_show_reuses_buffer_id_on_second_call() {
        let (lua, _reg, _cmds, _kms, _hks) = fresh();
        let same: bool = lua
            .load(
                r"
                local a = pmacs.instance.show()
                local b = pmacs.instance.show()
                return a == b
                ",
            )
            .eval()
            .unwrap();
        assert!(
            same,
            "consecutive show() calls must reuse the *pmacs-instance* buffer"
        );
    }

    #[test]
    fn instance_show_buffer_contains_self_section() {
        let (lua, reg, _cmds, _kms, _hks) = fresh();
        let id_lua: BufferIdLua = lua.load("return pmacs.instance.show()").eval().unwrap();
        let r = reg.borrow();
        let buf = r.get(id_lua.0).unwrap();
        let len = buf.len();
        let mut bytes = vec![0u8; usize::try_from(len).unwrap()];
        buf.snapshot_rope().slice(0, len, &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("This instance"), "{body}");
        assert!(body.contains("(no outbound attachment"), "{body}");
    }

    #[test]
    fn instance_identity_no_slot_yields_typed_error() {
        // A bare Lua state without install() is a programming error;
        // the binding reports it via NoLocalInstanceInfo.
        let lua = Lua::new();
        let f = install_instance_identity_binding(&lua).unwrap();
        let err = f.call::<Value>(()).expect_err("no slot → error");
        let msg = err.to_string();
        assert!(msg.contains("LocalInstanceInfo"), "{msg}");
        assert!(msg.contains("not installed"), "{msg}");
    }

    #[test]
    fn instance_echo_line_no_slot_yields_typed_error() {
        let lua = Lua::new();
        let f = install_instance_echo_line_binding(&lua).unwrap();
        let err = f.call::<Value>(()).expect_err("no slot → error");
        let msg = err.to_string();
        assert!(msg.contains("LocalInstanceInfo"), "{msg}");
    }

    #[test]
    fn local_instance_info_set_name_round_trip() {
        let info = LocalInstanceInfo::new();
        // Default: name is None.
        assert!(info.build_identity().instance_name.is_none());
        info.set_name(Some("work".into()));
        assert_eq!(info.build_identity().instance_name.as_deref(), Some("work"));
        info.set_name(None);
        assert!(info.build_identity().instance_name.is_none());
    }

    #[test]
    fn local_instance_info_set_started_changes_uptime_anchor() {
        // Setting an earlier `started` produces a larger uptime; the
        // exact value is jitter-prone, so we just check the relative
        // ordering against an unmodified default.
        let info = LocalInstanceInfo::new();
        let earlier = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(10_000))
            .expect("clock supports 10000s subtraction");
        info.set_started(earlier);
        let id = info.build_identity();
        assert!(
            id.uptime_secs >= 10_000,
            "expected uptime >= 10000 after rewinding `started`; got {}",
            id.uptime_secs
        );
    }

    #[test]
    fn style_overlay_dispose_detaches_translator_without_a_core() {
        // PR #113 round-7 finding 2: `fresh()` is the install-only /
        // headless host shape — the registry is registered as app
        // data, SharedCore is NOT. dispose() must detach the
        // buffer-attached translator through the registry alone;
        // pre-fix it returned success having done nothing, leaving
        // the translator attached (and paying per edit) for the
        // buffer's lifetime.
        let (lua, reg, _cmds, _kms, _hks) = fresh();
        lua.load(r#"_G.hbuf = pmacs.buffer.create("headless")"#)
            .exec()
            .unwrap();
        let id = reg
            .borrow()
            .find_by_name("headless")
            .expect("buffer exists");
        let baseline = reg.borrow().get(id).unwrap().view_count();
        lua.load(r"_G.hov = pmacs.buffer.add_style_overlay(_G.hbuf)")
            .exec()
            .unwrap();
        assert_eq!(
            reg.borrow().get(id).unwrap().view_count(),
            baseline + 1,
            "add_style_overlay attaches the translator"
        );
        lua.load("_G.hov:dispose()").exec().unwrap();
        assert_eq!(
            reg.borrow().get(id).unwrap().view_count(),
            baseline,
            "dispose must detach the translator with no core registered"
        );
        // Idempotent: a second dispose neither errors nor
        // over-detaches.
        lua.load("_G.hov:dispose()").exec().unwrap();
        assert_eq!(reg.borrow().get(id).unwrap().view_count(), baseline);
    }
}
