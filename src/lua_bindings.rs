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
use crate::editor_core::EditorCore;
use crate::highlight::{SyntaxHighlightView, Theme};
use crate::hook::{Hook, HookRegistry};
use crate::key::{display_sequence, parse_sequence};
use crate::keymap_stack::KeymapStack;
use crate::packages::{
    Address, Fetcher, InstallError, InstallScope, InstallSpec, InstalledPackage, Installer,
};
use crate::protocol::{AttachTarget, AttachmentHandle, InstanceIdentity};
use crate::rope::Range;
use crate::syntax::{self, ParseTreeBundle, ParseView, ParseViewHandle, SharedSyntaxRegistry};
use crate::workers_buffer;

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

/// Shared, single-threaded handle to the editor core --- the world
/// state mutated by `pmacs.editor.*` primitives invoked from inside
/// command bodies.
pub type SharedCore = Rc<RefCell<EditorCore>>;

/// Shared, single-threaded handle to the hook registry. Same
/// rationale as the other `Rc<RefCell<...>>` aliases.
pub type SharedHookRegistry = Rc<RefCell<HookRegistry>>;

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
/// Populated by `pmacs.packages.install{...}` and `install_project{...}`
/// (T M7.3). Read by `pmacs.packages.installed()` for introspection
/// and by the future M7.6 lockfile writer to enumerate the resolved
/// set. Single-threaded `Rc<RefCell<...>>` per the boundary's
/// main-thread invariant.
#[derive(Debug, Clone, Default)]
pub struct InstalledPackages(Rc<RefCell<Vec<InstalledPackage>>>);

impl InstalledPackages {
    /// Construct an empty roster.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful install. Order matches install order; that
    /// matters for diagnostics ("which install errored?") more than
    /// for resolution.
    pub fn record(&self, pkg: InstalledPackage) {
        self.0.borrow_mut().push(pkg);
    }

    /// Snapshot the current roster for read-only consumers.
    #[must_use]
    pub fn snapshot(&self) -> Vec<InstalledPackage> {
        self.0.borrow().clone()
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

    /// Stub for `pmacs.packages.update(...)` which is implemented in
    /// T M7.6. Per the project's "stub posture" convention, we accept
    /// the call shape (so v0.1 init.lua's that try it get a clean
    /// error) and fail with the milestone target named.
    #[error(
        "pmacs.packages.update is implemented in M7.6 (lockfile + \
         resolver). v0.1 / current builds: re-run `pmacs.packages.install` \
         with the new constraint to upgrade in place."
    )]
    PackagesUpdateUnsupported,

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
    methods.add_method("insert", |lua, this, (pos, bytes): (i64, mlua::String)| {
        let pos = u64_from_lua(pos)?;
        let payload = bytes.as_bytes();
        let edit = run_managed_edit(
            lua,
            this.0,
            EditOp::Insert {
                pos,
                bytes: &payload,
            },
        )?;
        notify_buffer_edit_to_windows(lua, this.0, &edit);
        Ok(())
    });

    methods.add_method("delete", |lua, this, (start, end): (i64, i64)| {
        let range = checked_range(start, end)?;
        let edit = run_managed_edit(lua, this.0, EditOp::Delete { range })?;
        notify_buffer_edit_to_windows(lua, this.0, &edit);
        Ok(())
    });

    methods.add_method(
        "replace",
        |lua, this, (start, end, bytes): (i64, i64, mlua::String)| {
            let range = checked_range(start, end)?;
            let payload = bytes.as_bytes();
            let edit = run_managed_edit(
                lua,
                this.0,
                EditOp::Replace {
                    range,
                    bytes: &payload,
                },
            )?;
            notify_buffer_edit_to_windows(lua, this.0, &edit);
            Ok(())
        },
    );
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
/// buffer was just edited via the Lua surface. Without this, a window
/// already displaying the edited buffer would keep a stale
/// [`crate::text_view::TextView`] line cache — cursor motions stop
/// updating the screen until the window switches buffers.
///
/// No-op when no [`SharedCore`] has been registered as Lua app data
/// (the shape used by the early-stage tests that exercise the
/// registry without an editor core).
fn notify_buffer_edit_to_windows(lua: &Lua, buffer_id: BufferId, edit: &crate::rope::Edit) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    core.borrow_mut().notify_buffer_edit(buffer_id, edit);
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
/// - `kind = "insert"`:  `pos: integer`, `bytes_len: integer`
/// - `kind = "delete"`:  `start: integer`, `end: integer`
/// - `kind = "replace"`: `start: integer`, `end: integer`, `bytes_len: integer`
///
/// `bytes_len` is informational. The bytes themselves are not
/// surfaced to Lua: [`crate::buffer::EditOp`] borrows them with a
/// lifetime tied to the caller's `apply_edit` frame, and a v0.1
/// byte-mutating intercept would require either copying the bytes
/// across the FFI boundary on every edit (expensive) or extending
/// [`crate::buffer::EditOp`] to use [`std::borrow::Cow`] (a wider
/// change than M6.4 needs). The byte stream is therefore immutable
/// through the chain in M6.4; M8's dired-class package will revisit
/// when it needs filename-edit-to-rename translation.
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
fn build_intercept_input(lua: &Lua, op: &EditOp<'_>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    match *op {
        EditOp::Insert { pos, bytes } => {
            t.set("kind", "insert")?;
            t.set("pos", i64_clamp(pos))?;
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
pub struct StyleOverlayHandleLua {
    /// Shared style spans rendered by every attached overlay view.
    spans: crate::overlay::SharedBufferStyleSpans,
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
    hooks: &SharedHookRegistry,
) -> mlua::Result<()> {
    lua.set_app_data(registry.clone());
    lua.set_app_data(commands.clone());
    lua.set_app_data(keymaps.clone());
    lua.set_app_data(hooks.clone());
    lua.set_app_data(InitCompleteFlag::new());
    lua.set_app_data(RequestedAttach::new());
    lua.set_app_data(CurrentAttachmentSlot::new());
    lua.set_app_data(LocalInstanceInfo::new());
    lua.set_app_data(InstalledPackages::new());

    let pmacs = lua.create_table()?;
    pmacs.set("buffer", install_buffer_module(lua, registry)?)?;
    pmacs.set("command", install_command_module(lua, commands)?)?;
    pmacs.set("keymap", install_keymap_module(lua, keymaps)?)?;
    pmacs.set("hook", install_hook_module(lua, hooks)?)?;
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
    lua.globals().set("pmacs", pmacs)?;
    Ok(())
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
        let id =
            crate::instance_buffer::render(&mut reg.borrow_mut(), &identity, attachment.as_ref());
        Ok(BufferIdLua(id))
    })
}

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
            "remove",
            lua.create_function(move |_, id: BufferIdLua| {
                reg.borrow_mut()
                    .remove(id.0)
                    .map(|_| ())
                    .map_err(mlua::Error::external)
            })?,
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
        buffer.set(
            "add_style_overlay",
            lua.create_function(
                move |lua, id: BufferIdLua| -> mlua::Result<StyleOverlayHandleLua> {
                    let spans = Arc::new(Mutex::new(Vec::new()));
                    let handle = StyleOverlayHandleLua {
                        spans: Arc::clone(&spans),
                    };
                    attach_style_overlay_to_visible_windows(lua, id.0, spans);
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
                    attach_style_overlay_to_visible_windows(lua, id.0, Arc::clone(&handle.spans));
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
    spans: crate::overlay::SharedBufferStyleSpans,
) {
    let Some(core) = lua.app_data_ref::<SharedCore>() else {
        return;
    };
    let mut core = core.borrow_mut();
    for win in core.windows.values_mut() {
        if win.buffer_id == buffer_id {
            win.push_overlay(Box::new(crate::overlay::BufferStyleOverlay::new(
                Arc::clone(&spans),
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// pmacs.ansi: M6.4-side exposure of the M6.3 parser
// ---------------------------------------------------------------------------

/// Lua-facing wrapper around [`crate::ansi::AnsiParser`].
///
/// Constructed via `pmacs.ansi.parser()`; methods `feed(bytes)` and
/// `reset()` mirror the Rust API. `feed` returns an array of event
/// tables --- see [`event_to_lua_table`] for the schema. The wrapper
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
/// - `pmacs.packages.update(...)` --- M7.6 stub; currently errors
///   pointing at the workaround (re-running install with a new
///   constraint).
///
/// Both install variants are init-time-only via [`require_init_phase`];
/// mid-session calls produce [`BindingError::InitOnlyApi`] naming
/// the equivalent CLI flag (none yet --- restart with an updated
/// init.lua). Each install is synchronous: errors raise back at the
/// call site so the offending init.lua line is named in the traceback.
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
        lua.create_function(|_, _args: Variadic<Value>| -> mlua::Result<()> {
            Err(mlua::Error::external(
                BindingError::PackagesUpdateUnsupported,
            ))
        })?,
    )?;

    register_package_searcher(lua)?;

    Ok(packages)
}

/// Register a custom searcher in `package.searchers` (Lua 5.4) /
/// `package.loaders` (Lua 5.1, LuaJIT) that consults the
/// [`InstalledPackages`] roster at require time.
///
/// # Why
///
/// `prepend_package_path` (the existing mechanism) only handles the
/// standard Lua layout: `<install_root>/<basename>.lua` or
/// `<install_root>/<basename>/init.lua`. A package whose manifest
/// declares e.g. `entry = "main.lua"` or `entry = "lib/foo.lua"`
/// has its entry file at a path the standard `?.lua;?/init.lua`
/// pattern does not match, and `require("<basename>")` would fail
/// even though the install completed. The custom searcher closes
/// that gap by mapping `require("<basename>")` directly to the
/// manifest's declared entry path.
///
/// # Precedence
///
/// The searcher is appended to the searchers/loaders list, after
/// the path-based searcher. Standard layouts (`init.lua` etc.)
/// continue to load via the path mechanism; the custom searcher
/// only kicks in when the path search misses. This keeps
/// drop-in-compatible packages on the well-trodden path and avoids
/// a behavior change for anyone using the conventional layout.
///
/// Within the searcher, the [`InstalledPackages`] roster is iterated
/// in *reverse* so the most recently installed package wins on a
/// basename collision. Combined with `init.lua`'s typical pattern
/// (user install first, then project install), this makes
/// project-scope installs override user-scope installs of the same
/// basename --- mirroring `prepend_package_path`'s "newer
/// installations prepend to package.path" semantics.
///
/// # 5.1 vs 5.4 names
///
/// Lua 5.1 / LuaJIT exposes the searcher list as `package.loaders`;
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

    let searcher = lua.create_function(
        |lua, name: String| -> mlua::Result<mlua::Value> {
            let Some(slot) = lua.app_data_ref::<InstalledPackages>() else {
                // Slot uninstalled (shouldn't happen under
                // production wiring, but a defensive nil keeps
                // require working under unusual test setups).
                return Ok(mlua::Value::Nil);
            };
            let snapshot = slot.snapshot();
            // Most-recent-first: a project-scope install of a
            // basename overrides a prior user-scope install.
            for pkg in snapshot.iter().rev() {
                if pkg.install_basename() != name {
                    continue;
                }
                let entry = pkg.entry_path();
                let bytes = match std::fs::read(&entry) {
                    Ok(b) => b,
                    Err(e) => {
                        // Searcher convention: a non-function return
                        // is treated as "not found, here's why" and
                        // appended to the require error message.
                        let s = lua.create_string(&format!(
                            "\n\tinstalled pmacs package '{name}' \
                             entry `{}` could not be read: {e}",
                            entry.display()
                        ))?;
                        return Ok(mlua::Value::String(s));
                    }
                };
                let chunk_name = format!("@{}", entry.display());
                let func = lua
                    .load(&bytes)
                    .set_name(&chunk_name)
                    .into_function()?;
                return Ok(mlua::Value::Function(func));
            }
            // No installed package matches. Return a string so Lua
            // appends our reason to the aggregate require error.
            let s = lua.create_string(&format!(
                "\n\tno installed pmacs package named '{name}'"
            ))?;
            Ok(mlua::Value::String(s))
        },
    )?;

    // Append to the searcher list. Lua tables are 1-indexed; the
    // new searcher runs after every existing searcher (preload,
    // path-based, etc.), so standard layouts are unaffected.
    let len = searchers.raw_len();
    searchers.set(len + 1, searcher)?;
    Ok(())
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
            let version_str: String = t
                .get::<String>("version")
                .unwrap_or_else(|_| "*".to_string());
            let address = Address::parse(&address_str)
                .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Address(e))))?;
            let version = semver::VersionReq::parse(&version_str).map_err(|e| {
                mlua::Error::external(BindingError::from(InstallError::InvalidVersionReq {
                    value: version_str,
                    cause: e.to_string(),
                }))
            })?;
            Ok(InstallSpec { address, version })
        }
        other => Err(mlua::Error::external(BindingError::InstallSpecWrongType {
            got: other.type_name().to_string(),
        })),
    }
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
/// `$XDG_CACHE_HOME/pmacs/git/`, run [`Installer::install`], extend
/// `package.path` so the entry module is requireable, and record the
/// result in the [`InstalledPackages`] roster.
///
/// A [`PackageInstallOverride`] in app data, if present, redirects the
/// fetcher's cache dir and the user-scope install root. Tests use this
/// instead of mutating `XDG_*` env vars (which would require `unsafe`).
fn do_install(lua: &Lua, spec: &InstallSpec, scope: &InstallScope) -> mlua::Result<Table> {
    let override_data = lua.app_data_ref::<PackageInstallOverride>();
    let cache_override = override_data.as_ref().and_then(|o| o.cache_dir.clone());
    let user_root_override = override_data
        .as_ref()
        .and_then(|o| o.user_install_root.clone());

    let fetcher = match cache_override {
        Some(dir) => Fetcher::with_cache_dir(dir),
        None => Fetcher::from_xdg()
            .map_err(|e| mlua::Error::external(BindingError::from(InstallError::Fetch(e))))?,
    };
    let mut installer = Installer::new(fetcher, scope.clone());
    if let (InstallScope::User, Some(root)) = (scope, user_root_override) {
        installer = installer.with_install_root_override(root);
    }
    let installed = installer
        .install(spec)
        .map_err(|e| mlua::Error::external(BindingError::from(e)))?;

    // Extend package.path so the package's entry module is requireable.
    if let Some(parent) = installed.install_path.parent() {
        prepend_package_path(lua, parent)?;
    }

    // Record in the in-memory roster.
    let slot = lua
        .app_data_ref::<InstalledPackages>()
        .ok_or_else(|| mlua::Error::external(BindingError::NoInstalledPackagesSlot))?;
    slot.record(installed.clone());

    installed_package_to_lua(lua, &installed)
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
        lua.create_function(move |_, id: BufferIdLua| -> mlua::Result<()> {
            cc.borrow_mut()
                .kill_buffer(id.0)
                .map_err(mlua::Error::external)
        })?,
    )?;
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
        let cmds = commands.clone();
        command.set(
            "exists",
            lua.create_function(move |_, name: String| Ok(cmds.borrow().contains(&name)))?,
        )?;
    }

    Ok(command)
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
                if let Some(id) = result {
                    rebuild_help_buffer_views(lua, id);
                }
                Ok(result.map(BufferIdLua))
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
                let result = {
                    let mut r = reg.borrow_mut();
                    let c = cmds.borrow();
                    let k = kms.borrow();
                    help::render_key(&mut r, &c, &k, &sequence)
                };
                if let Some(id) = result {
                    rebuild_help_buffer_views(lua, id);
                }
                Ok(result.map(BufferIdLua))
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
                if let Some(rid) = result {
                    rebuild_help_buffer_views(lua, rid);
                }
                Ok(result.map(BufferIdLua))
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
                if let Some(id) = result {
                    rebuild_help_buffer_views(lua, id);
                }
                Ok(result.map(BufferIdLua))
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
                if let Some(id) = result {
                    rebuild_help_buffer_views(lua, id);
                }
                Ok(result.map(BufferIdLua))
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
                if let Some(rid) = result {
                    rebuild_help_buffer_views(lua, rid);
                }
                Ok(result.map(BufferIdLua))
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
                if let Some(id) = result {
                    rebuild_help_buffer_views(lua, id);
                }
                Ok(result.map(BufferIdLua))
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
        let cmds = commands.clone();
        let kms = keymaps.clone();
        describe.set(
            "key",
            lua.create_function(move |lua, sequence: String| {
                let chords = parse_sequence(&sequence).map_err(mlua::Error::external)?;
                // Active scopes are an editor-runtime concept; describe.key
                // currently consults global only. Once the editor wires its
                // active buffer/modes through, we'll thread them in.
                let km = kms.borrow();
                let r = km.resolve(&chords, None, &[]);
                match r {
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

    install_motion(&editor, lua, core)?;
    install_editing(&editor, lua, core)?;
    install_history(&editor, lua, core)?;
    install_session(&editor, lua, core)?;

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
                            // Unit, Parse, and "no recorded outcome" all
                            // surface as a clean ok-with-nil to Lua;
                            // streams that close without an explicit
                            // outcome (the typical emit_n case) are
                            // indistinguishable from ones that
                            // returned `Unit`. Parse jobs aren't
                            // streams in M4.1 but the arm is here
                            // for exhaustiveness.
                            Some(JobOutcome::Complete(
                                JobResult::Unit | JobResult::Parse { .. },
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
            lua.create_function(move |_, ()| {
                let snap = rt.workers_snapshot();
                let id = workers_buffer::render(&mut reg.borrow_mut(), &snap);
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
        let mut node = self.bundle.tree.root_node();
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
        methods.add_method("sexp", |_, this, ()| Ok(this.0.tree.root_node().to_sexp()));
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
                let req = handle.make_request();
                let bundle = syntax::run_parse(req).map_err(mlua::Error::external)?;
                let arc = Arc::new(bundle);
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
                let req = handle.make_request();
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
                    handle.install(bundle);
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
    // the active window. Idempotent: if an overlay for the same
    // ParseViewHandle already lives on the active window, this is a
    // no-op (saves rebuilding query state for re-attach paths).
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
                let Some(query) = s.highlights_query(&lang) else {
                    // No highlights query for this language --- treat as
                    // a benign no-op so callers don't need to special-case
                    // grammars without highlights bundled.
                    return Ok(false);
                };
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
                let overlay = SyntaxHighlightView::new(handle, query, theme);
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
fn lua_to_style(t: &Table) -> mlua::Result<Style> {
    let fg: mlua::Value = t.get("fg").unwrap_or(mlua::Value::Nil);
    let bg: mlua::Value = t.get("bg").unwrap_or(mlua::Value::Nil);
    let underline: mlua::Value = t.get("underline").unwrap_or(mlua::Value::Nil);
    Ok(Style {
        fg: lua_to_color(&fg)?,
        bg: lua_to_color(&bg)?,
        bold: t.get("bold").unwrap_or(false),
        italic: t.get("italic").unwrap_or(false),
        underline: lua_to_underline(&underline)?,
        reverse: t.get("reverse").unwrap_or(false),
    })
}

fn style_to_lua(lua: &Lua, style: Style) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 6)?;
    t.set("fg", color_to_lua(lua, style.fg)?)?;
    t.set("bg", color_to_lua(lua, style.bg)?)?;
    t.set("bold", style.bold)?;
    t.set("italic", style.italic)?;
    t.set("underline", underline_to_lua(style.underline))?;
    t.set("reverse", style.reverse)?;
    Ok(t)
}

/// Install `pmacs.theme.*`. The module reads and writes the shared
/// [`Theme`] held by the [`crate::syntax::SyntaxRegistry`]; every
/// attached [`SyntaxHighlightView`] sees the change on its next
/// render. T M4.3 acceptance: "theming via Lua-defined color
/// schemes."
fn install_theme(lua: &Lua, syntax: &SharedSyntaxRegistry) -> mlua::Result<Table> {
    let theme_mod = lua.create_table()?;

    {
        let s = syntax.clone();
        theme_mod.set(
            "set",
            lua.create_function(move |_, table: Table| {
                let mut new_theme = Theme::empty();
                table.for_each(|name: String, style: Table| {
                    new_theme.insert(name, lua_to_style(&style)?);
                    Ok(())
                })?;
                let theme = s.theme();
                let mut th = theme.lock().expect("theme mutex poisoned");
                let prev_default = th.default_style;
                *th = new_theme;
                th.default_style = prev_default;
                Ok(())
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "merge",
            lua.create_function(move |_, table: Table| {
                let theme = s.theme();
                let mut th = theme.lock().expect("theme mutex poisoned");
                table.for_each(|name: String, style: Table| {
                    th.insert(name, lua_to_style(&style)?);
                    Ok(())
                })?;
                Ok(())
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
                theme.lock().expect("theme mutex poisoned").clear();
                Ok(())
            })?,
        )?;
    }

    {
        let s = syntax.clone();
        theme_mod.set(
            "default",
            lua.create_function(move |_, style: Table| {
                let theme = s.theme();
                theme.lock().expect("theme mutex poisoned").default_style = lua_to_style(&style)?;
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
    Ok(ProcessSpec {
        label,
        command,
        args,
        cwd: cwd.map(std::path::PathBuf::from),
        env,
        mode,
        restart,
        ansi_events,
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
                let ids: Vec<ProcessId> = sup.ids().collect();
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

/// Build a fresh [`ProcessSupervisor`] and install
/// `pmacs.process.*` over it. Mirrors [`make_async_runtime`] /
/// [`make_syntax_registry`] in shape.
pub fn make_process_supervisor(lua: &Lua) -> mlua::Result<SharedProcessSupervisor> {
    let supervisor = Rc::new(RefCell::new(ProcessSupervisor::new()));
    install_process(lua, &supervisor)?;
    Ok(supervisor)
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
pub fn install_lsp(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    lua.set_app_data(manager.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let lsp_mod = lua.create_table()?;

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
        let m = manager.clone();
        lsp_mod.set(
            "request_completion",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let req_id = m
                        .borrow_mut()
                        .request_completion(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(req_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "request_hover",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let req_id = m
                        .borrow_mut()
                        .request_hover(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(req_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "request_signature_help",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let req_id = m
                        .borrow_mut()
                        .request_signature_help(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(req_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "request_definition",
            lua.create_function(
                move |_, (id, uri, line, col): (LspServerIdLua, String, u32, u32)| {
                    let req_id = m
                        .borrow_mut()
                        .request_definition(id.0, uri, line, col)
                        .map_err(mlua::Error::external)?;
                    Ok(req_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        lsp_mod.set(
            "request_formatting",
            lua.create_function(
                move |_,
                      (id, uri, tab_size, insert_spaces): (
                    LspServerIdLua,
                    String,
                    u32,
                    Option<bool>,
                )| {
                    let req_id = m
                        .borrow_mut()
                        .request_formatting(id.0, uri, tab_size, insert_spaces.unwrap_or(true))
                        .map_err(mlua::Error::external)?;
                    Ok(req_id)
                },
            )?,
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

    pmacs.set("lsp", lsp_mod)?;
    Ok(())
}

/// Build a fresh [`LspManager`] over `supervisor` and install
/// `pmacs.lsp.*` over it.
pub fn make_lsp_manager(
    lua: &Lua,
    supervisor: SharedProcessSupervisor,
) -> mlua::Result<SharedLspManager> {
    let manager = Rc::new(RefCell::new(LspManager::new(supervisor)));
    install_lsp(lua, &manager)?;
    install_diag(lua, &manager)?;
    install_completion(lua, &manager)?;
    install_hover(lua, &manager)?;
    install_signature(lua, &manager)?;
    install_definition(lua, &manager)?;
    install_formatting(lua, &manager)?;
    Ok(manager)
}

// ---------------------------------------------------------------------------
// pmacs.diag: diagnostics surface (T M4.6)
// ---------------------------------------------------------------------------

use crate::diag::{Diagnostic, DiagnosticSeverity};

fn diagnostic_to_lua(lua: &Lua, d: &Diagnostic) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 8)?;
    t.set("severity", d.severity.label())?;
    t.set("severity_code", d.severity as i64)?;
    t.set("message", d.message.as_str())?;
    if let Some(s) = &d.source {
        t.set("source", s.as_str())?;
    }
    if let Some(c) = &d.code {
        t.set("code", c.as_str())?;
    }
    let range = lua.create_table_with_capacity(0, 4)?;
    let start = lua.create_table_with_capacity(0, 2)?;
    start.set("line", d.start_line)?;
    start.set("character", d.start_col)?;
    let end = lua.create_table_with_capacity(0, 2)?;
    end.set("line", d.end_line)?;
    end.set("character", d.end_col)?;
    range.set("start", start)?;
    range.set("end", end)?;
    t.set("range", range)?;
    t.set("start_line", d.start_line)?;
    t.set("start_col", d.start_col)?;
    t.set("end_line", d.end_line)?;
    t.set("end_col", d.end_col)?;
    Ok(t)
}

/// Install `pmacs.diag.*` (T M4.6).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings; splitting fragments a coherent surface"
)]
pub fn install_diag(lua: &Lua, manager: &SharedLspManager) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let diag_mod = lua.create_table()?;

    {
        let m = manager.clone();
        diag_mod.set(
            "list",
            lua.create_function(move |lua, uri: String| {
                let store_handle = m.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let diags = guard.for_uri(&uri);
                let out = lua.create_table_with_capacity(diags.len(), 0)?;
                for (i, d) in diags.iter().enumerate() {
                    out.set(i + 1, diagnostic_to_lua(lua, d)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "count",
            lua.create_function(move |_, uri: Option<String>| {
                let store_handle = m.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let n = match uri {
                    Some(u) => guard.count_for(&u),
                    None => {
                        guard.totals().0 + guard.totals().1 + guard.totals().2 + guard.totals().3
                    }
                };
                Ok(n)
            })?,
        )?;
    }

    {
        let mgr = manager.clone();
        diag_mod.set(
            "totals",
            lua.create_function(move |lua, ()| {
                let store_handle = mgr.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let (errs, warns, infos, hints) = guard.totals();
                let table = lua.create_table_with_capacity(0, 4)?;
                table.set("error", errs)?;
                table.set("warning", warns)?;
                table.set("info", infos)?;
                table.set("hint", hints)?;
                Ok(table)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "next",
            lua.create_function(
                move |lua, (uri, line, col, wrap): (String, u32, u32, Option<bool>)| {
                    let store_handle = m.borrow().diag_store();
                    let guard = store_handle.lock().expect("diag store mutex poisoned");
                    let found = guard.next_after(&uri, line, col).or_else(|| {
                        if wrap.unwrap_or(true) {
                            guard.first_for(&uri)
                        } else {
                            None
                        }
                    });
                    match found {
                        Some(d) => Ok(Value::Table(diagnostic_to_lua(lua, d)?)),
                        None => Ok(Value::Nil),
                    }
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "previous",
            lua.create_function(
                move |lua, (uri, line, col, wrap): (String, u32, u32, Option<bool>)| {
                    let store_handle = m.borrow().diag_store();
                    let guard = store_handle.lock().expect("diag store mutex poisoned");
                    let found = guard.previous_before(&uri, line, col).or_else(|| {
                        if wrap.unwrap_or(true) {
                            guard.last_for(&uri)
                        } else {
                            None
                        }
                    });
                    match found {
                        Some(d) => Ok(Value::Table(diagnostic_to_lua(lua, d)?)),
                        None => Ok(Value::Nil),
                    }
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        diag_mod.set(
            "uris",
            lua.create_function(move |lua, ()| {
                let store_handle = m.borrow().diag_store();
                let guard = store_handle.lock().expect("diag store mutex poisoned");
                let uris: Vec<String> = guard.uris().map(str::to_owned).collect();
                let out = lua.create_table_with_capacity(uris.len(), 0)?;
                for (i, u) in uris.iter().enumerate() {
                    out.set(i + 1, u.as_str())?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // Helper for tests / Lua-driven flows: clear the
        // diagnostics for a URI.
        let m = manager.clone();
        diag_mod.set(
            "clear",
            lua.create_function(move |_, uri: String| {
                let store_handle = m.borrow().diag_store();
                let mut guard = store_handle.lock().expect("diag store mutex poisoned");
                guard.clear(&uri);
                Ok(())
            })?,
        )?;
    }

    {
        // Look up the severity table-of-strings the rest of the
        // surface uses; returned as a constant table for callers
        // that prefer a tagged value.
        diag_mod.set("severity", {
            let t = lua.create_table_with_capacity(0, 4)?;
            t.set("error", DiagnosticSeverity::Error as i64)?;
            t.set("warning", DiagnosticSeverity::Warning as i64)?;
            t.set("info", DiagnosticSeverity::Information as i64)?;
            t.set("hint", DiagnosticSeverity::Hint as i64)?;
            t
        })?;
    }

    pmacs.set("diag", diag_mod)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// pmacs.completion / pmacs.hover / pmacs.signature: T M4.7 surfaces
// ---------------------------------------------------------------------------

use crate::completion::{CompletionItem, CompletionItemKind, CompletionKey, CompletionTriggers};
use crate::definition::{DefinitionKey, DefinitionLocation, DefinitionResponse};
use crate::formatting::{FormattingKey, FormattingResponse, TextEdit};
use crate::hover::{Hover, HoverKey};
use crate::signature::{Signature, SignatureHelp, SignatureKey, SignatureParameter};

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
    let m = lua.create_table()?;

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
// pmacs.index: project-scoped symbol index (T M4.10)
// ---------------------------------------------------------------------------

use crate::project_index::{
    FileEntry, ProjectIndexer, SearchHit, Symbol, SymbolKind, SymbolSource, extract_heuristic,
    fnv1a_64, ingest_lsp_symbols,
};

/// Cheaply-cloneable shared project index registry.
pub type SharedProjectIndexer = Rc<RefCell<ProjectIndexer>>;

fn symbol_kind_from_lua(tag: &str) -> SymbolKind {
    match tag {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "struct" => SymbolKind::Struct,
        "class" => SymbolKind::Class,
        "trait" | "interface" => SymbolKind::Trait,
        "enum" => SymbolKind::Enum,
        "variable" => SymbolKind::Variable,
        "constant" => SymbolKind::Constant,
        "field" | "property" => SymbolKind::Field,
        "module" | "namespace" => SymbolKind::Module,
        "macro" => SymbolKind::Macro,
        "type_alias" | "type" => SymbolKind::TypeAlias,
        other => SymbolKind::Other(other.to_owned()),
    }
}

fn lua_symbol_from_table(t: &Table) -> mlua::Result<Symbol> {
    let name: String = t.get("name")?;
    let kind_tag: Option<String> = t.get("kind").ok().flatten();
    let kind = kind_tag
        .as_deref()
        .map_or(SymbolKind::Other("unknown".into()), symbol_kind_from_lua);
    let line: u32 = t.get("line").unwrap_or(0);
    let col: u32 = t.get("col").unwrap_or(0);
    let source_tag: Option<String> = t.get("source").ok().flatten();
    let source = source_tag
        .as_deref()
        .and_then(SymbolSource::from_tag)
        .unwrap_or(SymbolSource::Lua);
    let container: Option<String> = t.get("container").ok().flatten();
    Ok(Symbol {
        name,
        kind,
        line,
        col,
        source,
        container,
    })
}

fn search_hit_to_lua(lua: &Lua, hit: &SearchHit) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 9)?;
    t.set("name", hit.name.as_str())?;
    t.set("kind", hit.kind.tag())?;
    t.set("source", hit.source.tag())?;
    t.set("path", hit.path.display().to_string())?;
    t.set("relative_path", hit.relative_path.display().to_string())?;
    t.set("line", hit.line)?;
    t.set("col", hit.col)?;
    t.set("score", hit.score)?;
    if let Some(c) = &hit.container {
        t.set("container", c.as_str())?;
    }
    if let Some(l) = &hit.language {
        t.set("language", l.as_str())?;
    }
    Ok(t)
}

/// Install `pmacs.index.*` (T M4.10). Preserves any existing
/// `pmacs.index` keys (e.g. user-supplied indexer extensions
/// installed by builtin Lua chunks).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of index bindings; splitting adds ceremony without clarity"
)]
pub fn install_project_index(lua: &Lua, indexer: &SharedProjectIndexer) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m: Table = match pmacs.get::<Option<Table>>("index")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        // open(root) -> root_string. Ensures an index exists for
        // `root`; idempotent. Returns the canonicalised root the
        // caller should pass back to subsequent calls.
        let ix = indexer.clone();
        m.set(
            "open",
            lua.create_function(move |_, root: String| {
                let mut ix_ref = ix.borrow_mut();
                let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                Ok(idx.root.display().to_string())
            })?,
        )?;
    }

    {
        // close(root): drop the in-memory index. Does not touch disk.
        let ix = indexer.clone();
        m.set(
            "close",
            lua.create_function(move |_, root: String| {
                Ok(ix.borrow_mut().forget(std::path::Path::new(&root)))
            })?,
        )?;
    }

    {
        // upsert_file(root, path, language, source) -> { added }.
        // Runs the heuristic extractor on `source`, hashes it, and
        // replaces the entry for `path`.
        let ix = indexer.clone();
        m.set(
            "upsert_file",
            lua.create_function(
                move |lua,
                      (root, path, language, source): (
                    String,
                    String,
                    Option<String>,
                    String,
                )| {
                    let mut ix_ref = ix.borrow_mut();
                    let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                    let lang = language.as_deref().unwrap_or("");
                    let symbols = if lang.is_empty() {
                        crate::project_index::extract_raw(&source)
                    } else {
                        extract_heuristic(lang, &source)
                    };
                    let added = symbols.len();
                    let entry = FileEntry {
                        path: std::path::PathBuf::from(&path),
                        mtime_secs: 0,
                        content_hash: fnv1a_64(source.as_bytes()),
                        language: language.clone(),
                        symbols,
                    };
                    idx.upsert_file(entry);
                    let t = lua.create_table_with_capacity(0, 1)?;
                    t.set("added", added)?;
                    Ok(t)
                },
            )?,
        )?;
    }

    {
        // upsert_symbols(root, path, language, symbol_array): push
        // pre-extracted symbols (e.g. from a Lua-side indexer) into
        // the index. Each entry is a table with name/kind/line/col/
        // source/container fields.
        let ix = indexer.clone();
        m.set(
            "upsert_symbols",
            lua.create_function(
                move |_,
                      (root, path, language, symbols): (
                    String,
                    String,
                    Option<String>,
                    Vec<Table>,
                )| {
                    let mut parsed = Vec::with_capacity(symbols.len());
                    for t in &symbols {
                        parsed.push(lua_symbol_from_table(t)?);
                    }
                    let added = parsed.len();
                    let mut ix_ref = ix.borrow_mut();
                    let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                    let entry = FileEntry {
                        path: std::path::PathBuf::from(&path),
                        mtime_secs: 0,
                        content_hash: 0,
                        language,
                        symbols: parsed,
                    };
                    idx.upsert_file(entry);
                    Ok(added)
                },
            )?,
        )?;
    }

    {
        // ingest_lsp(root, lsp_response): merge symbols from a
        // workspace/symbol or documentSymbol response. Groups
        // results by path and replaces each path's entry.
        let ix = indexer.clone();
        m.set(
            "ingest_lsp",
            lua.create_function(move |_, (root, value): (String, Value)| {
                let json = lua_to_json(value)?;
                let inbound = ingest_lsp_symbols(&json);
                let mut ix_ref = ix.borrow_mut();
                let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                let mut by_path: std::collections::HashMap<
                    std::path::PathBuf,
                    (Option<String>, Vec<Symbol>),
                > = std::collections::HashMap::new();
                for entry in inbound {
                    let bucket = by_path
                        .entry(entry.path)
                        .or_insert_with(|| (entry.language.clone(), Vec::new()));
                    bucket.1.push(entry.symbol);
                }
                let merged = by_path.len();
                for (path, (lang, symbols)) in by_path {
                    idx.upsert_file(FileEntry {
                        path,
                        mtime_secs: 0,
                        content_hash: 0,
                        language: lang,
                        symbols,
                    });
                }
                Ok(merged)
            })?,
        )?;
    }

    {
        // invalidate(root, path): drop one file's entry.
        let ix = indexer.clone();
        m.set(
            "invalidate",
            lua.create_function(move |_, (root, path): (String, String)| {
                let mut ix_ref = ix.borrow_mut();
                Ok(ix_ref
                    .get_mut(std::path::Path::new(&root))
                    .is_some_and(|idx| idx.forget_file(std::path::Path::new(&path))))
            })?,
        )?;
    }

    {
        // is_fresh(root, path, mtime_secs, content_hash) -> bool
        let ix = indexer.clone();
        m.set(
            "is_fresh",
            lua.create_function(
                move |_, (root, path, mtime_secs, content_hash): (String, String, u64, u64)| {
                    let ix_ref = ix.borrow();
                    Ok(ix_ref.get(std::path::Path::new(&root)).is_some_and(|idx| {
                        idx.is_fresh(std::path::Path::new(&path), mtime_secs, content_hash)
                    }))
                },
            )?,
        )?;
    }

    {
        // search(root, query [, limit]) -> array of hit tables.
        let ix = indexer.clone();
        m.set(
            "search",
            lua.create_function(
                move |lua, (root, query, limit): (String, String, Option<usize>)| {
                    let ix_ref = ix.borrow();
                    let Some(idx) = ix_ref.get(std::path::Path::new(&root)) else {
                        return lua.create_table();
                    };
                    let hits = idx.search(&query, limit.unwrap_or(50));
                    let out = lua.create_table_with_capacity(hits.len(), 0)?;
                    for (i, h) in hits.iter().enumerate() {
                        out.set(i + 1, search_hit_to_lua(lua, h)?)?;
                    }
                    Ok(out)
                },
            )?,
        )?;
    }

    {
        // save(root [, path]): persist the index. Path defaults to
        // <root>/.pmacs/index.json.
        let ix = indexer.clone();
        m.set(
            "save",
            lua.create_function(move |_, (root, path): (String, Option<String>)| {
                let ix_ref = ix.borrow();
                let idx = ix_ref
                    .get(std::path::Path::new(&root))
                    .ok_or_else(|| mlua::Error::external(format!("unknown index root: {root}")))?;
                let dest = path.map_or_else(|| idx.default_cache_path(), std::path::PathBuf::from);
                idx.save(&dest).map_err(mlua::Error::external)?;
                Ok(dest.display().to_string())
            })?,
        )?;
    }

    {
        // load(root [, path]): replace the in-memory index for
        // `root` with the on-disk cache. A missing cache file
        // results in an empty index (cold-start).
        let ix = indexer.clone();
        m.set(
            "load",
            lua.create_function(move |_, (root, path): (String, Option<String>)| {
                let root_path = std::path::PathBuf::from(&root);
                let cache_path = path.map_or_else(
                    || crate::project_index::ProjectIndex::cache_path_for(&root_path),
                    std::path::PathBuf::from,
                );
                let idx = crate::project_index::ProjectIndex::load(root_path.clone(), &cache_path)
                    .map_err(mlua::Error::external)?;
                let symbol_count = idx.symbol_count();
                let file_count = idx.file_count();
                let mut ix_ref = ix.borrow_mut();
                let key = idx.root.clone();
                ix_ref.forget(&key);
                let slot = ix_ref.ensure(key);
                *slot = idx;
                Ok((file_count, symbol_count))
            })?,
        )?;
    }

    {
        // stats(root) -> { files, symbols, generation } or nil.
        let ix = indexer.clone();
        m.set(
            "stats",
            lua.create_function(move |lua, root: String| {
                let ix_ref = ix.borrow();
                let Some(idx) = ix_ref.get(std::path::Path::new(&root)) else {
                    return Ok(Value::Nil);
                };
                let t = lua.create_table_with_capacity(0, 4)?;
                t.set("files", idx.file_count())?;
                t.set("symbols", idx.symbol_count())?;
                t.set("generation", idx.generation)?;
                t.set("root", idx.root.display().to_string())?;
                Ok(Value::Table(t))
            })?,
        )?;
    }

    {
        // roots() -> array of registered index roots.
        let ix = indexer.clone();
        m.set(
            "roots",
            lua.create_function(move |lua, ()| {
                let ix_ref = ix.borrow();
                let mut roots: Vec<String> =
                    ix_ref.roots().map(|p| p.display().to_string()).collect();
                roots.sort();
                let out = lua.create_table_with_capacity(roots.len(), 0)?;
                for (i, r) in roots.iter().enumerate() {
                    out.set(i + 1, r.as_str())?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // hash(text) -> u64. Exposes FNV-1a so Lua callers can
        // produce stable cache keys without a separate hash crate.
        m.set(
            "hash",
            lua.create_function(|_, text: String| Ok(fnv1a_64(text.as_bytes())))?,
        )?;
    }

    pmacs.set("index", m)?;
    Ok(())
}

/// Build a fresh [`ProjectIndexer`] and install `pmacs.index.*` over it.
pub fn make_project_indexer(lua: &Lua) -> mlua::Result<SharedProjectIndexer> {
    let ix: SharedProjectIndexer = Rc::new(RefCell::new(ProjectIndexer::new()));
    install_project_index(lua, &ix)?;
    Ok(ix)
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
    let t = lua.create_table_with_capacity(0, 7)?;
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
    CompletionContext {
        prefix,
        line,
        col,
        buffer_text: Rc::from(buffer_text),
        language,
        project_root: project_root.map(std::path::PathBuf::from),
        trigger,
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
        // own.
        m.set(
            "context_for",
            lua.create_function(move |lua, args: ContextForArgs| {
                let (prefix, line, col, buffer_text, language, project_root, trigger) = args;
                let ctx = CompletionContext {
                    prefix,
                    line: line.unwrap_or(0),
                    col: col.unwrap_or(0),
                    buffer_text: Rc::from(buffer_text.unwrap_or_default()),
                    language,
                    project_root: project_root.map(std::path::PathBuf::from),
                    trigger: if trigger.as_deref() == Some("incomplete") {
                        CompletionTrigger::Incomplete
                    } else {
                        CompletionTrigger::Invoked
                    },
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
                for (i, id) in c.layout.iter_ids().iter().enumerate() {
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
            lua.create_function(move |_, ()| Ok(cc.borrow().active.raw()))?,
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
            lua.create_function(move |_, id: BufferIdLua| -> mlua::Result<()> {
                cc.borrow_mut()
                    .switch_active_buffer(id.0)
                    .map_err(mlua::Error::external)
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
    Ok(())
}

fn install_history(editor: &Table, lua: &Lua, core: &SharedCore) -> mlua::Result<()> {
    register(editor, lua, core, "undo", EditorCore::undo)?;
    register(editor, lua, core, "redo", EditorCore::redo)?;
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
        // Active buffer's backing file path, or `nil` if none. Used by
        // the LSP runtime to compute file:// URIs and locate the
        // enclosing project root.
        let cc = core.clone();
        editor.set(
            "file_path",
            lua.create_function(move |_, ()| {
                let c = cc.borrow();
                Ok(c.file_path.as_ref().map(|p| p.display().to_string()))
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
        let cc = core.clone();
        mb.set(
            "set_contents",
            lua.create_function(move |_, s: String| {
                cc.borrow_mut().minibuffer.replace_contents(&s);
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
        let hks: SharedHookRegistry = Rc::new(RefCell::new(HookRegistry::new()));
        install(&lua, &reg, &cmds, &kms, &hks).expect("install");
        (lua, reg, cmds, kms, hks)
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
}
