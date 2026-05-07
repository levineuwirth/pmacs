// lua.rs --- Lua VM owner. Single VM, single thread, !Send by construction.

//! The Lua boundary.
//!
//! Pmacs runs a single Lua VM on the main thread. The threading model
//! (spec §3 Checkpoint 5 and §6) requires the VM to never move across
//! threads: workers see snapshots of buffers, but the Lua state is
//! exclusively the main thread's. This module's [`LuaHost`] enforces
//! that contract at compile time.
//!
//! # Errors
//!
//! Lua chunks fail in a hundred ways. Pmacs's contract is that none of
//! those failures crash the editor or print to stderr. Every error
//! [`LuaHost::eval`] encounters is captured into the host's error log
//! and surfaced through the editor UI --- the status line for now; once
//! the `*errors*` buffer exists (T M2.8) it will be the canonical sink.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::SystemTime;

use mlua::{Lua, MultiValue, Value};

use crate::buffer::EditOp;
use crate::buffer_registry::BufferRegistry;
use crate::hook::{HookOutcome, HookRegistry};

/// Canonical name for the error log buffer. Surfacing here so callers
/// can look it up via [`crate::buffer_registry::BufferRegistry::find_by_name`].
pub const ERRORS_BUFFER_NAME: &str = "*errors*";
use crate::command::CommandRegistry;
use crate::keymap_stack::KeymapStack;
use crate::lua_bindings::{
    self, CurrentAttachmentSlot, InitCompleteFlag, LocalInstanceInfo, PackageInstallOverride,
    RequestedAttach, SharedCommandRegistry, SharedCore, SharedHookRegistry, SharedKeymapStack,
    SharedRegistry,
};
use crate::protocol::{AttachTarget, AttachmentHandle, InstanceIdentity};

// `Rc` and `RefCell` are pulled in for the registry-owning fields.

/// Owner of the Pmacs Lua VM.
///
/// `LuaHost` is `!Send` by construction: the inner `mlua::Lua` is `!Send`
/// without mlua's `send` feature, and a [`PhantomData<Rc<()>>`] field
/// reasserts the contract so it survives any future feature reshuffling
/// or upstream change. This is verified at compile time:
///
/// ```compile_fail
/// use pmacs::lua::LuaHost;
/// fn requires_send<T: Send>(_: T) {}
/// let host = LuaHost::new().unwrap();
/// requires_send(host); // error: `LuaHost` cannot be sent between threads safely
/// ```
///
/// # Threading
///
/// Construct on the main thread; never move off it. A `LuaHost` lives
/// inside [`crate::editor::EditorState`], which itself is owned by the
/// frontend's main loop. Workers (M3) cannot touch the VM directly;
/// they communicate with the main thread via typed messages.
pub struct LuaHost {
    lua: Lua,
    /// Buffer registry (owned by the host so its lifetime tracks the
    /// VM's). Bindings on the Lua side hold their own clones of the
    /// `Rc` and resolve handles through this on every call.
    registry: SharedRegistry,
    /// Command registry: every editor action is a named, introspectable
    /// command. Lua bindings (`pmacs.command.*`) and Rust call sites
    /// (`LuaHost::invoke_command`) share this through `Rc`.
    commands: SharedCommandRegistry,
    /// Keymap stack: per-scope key bindings keyed on chord sequences.
    /// Inputs from the editor's run loop will dispatch through this
    /// once T M2.5 lands; M2.4 builds the system without yet routing
    /// the live event stream through it.
    keymaps: SharedKeymapStack,
    /// Hook registry. Stub for T M2.11 introspection; T M2.6 will wire
    /// execution at the relevant call sites.
    hooks: SharedHookRegistry,
    /// Optional handle to the editor core. Set by [`Self::attach_editor`].
    /// Required to notify windows of edits that the host applies
    /// directly (e.g. [`Self::append_to_errors_buffer`] writes to the
    /// `*errors*` buffer without going through `editor_core`'s
    /// [`crate::editor_core::EditorCore::apply_active_edit`], so any
    /// window currently displaying that buffer would otherwise hold a
    /// stale [`crate::text_view::TextView`] line cache).
    core: Option<SharedCore>,
    errors: Vec<LuaErrorRecord>,
    /// T M7.8 cancel token. Owns the [`AtomicBool`] the count hook
    /// polls. Hosts hand out [`crate::lua_isolation::CancelHandle`]
    /// clones for cross-thread C-g delivery.
    cancel: crate::lua_isolation::CancelToken,
    _not_send: PhantomData<Rc<()>>,
}

/// A captured Lua error.
#[derive(Clone, Debug)]
pub struct LuaErrorRecord {
    /// Wall-clock time the error was captured.
    pub at: SystemTime,
    /// Source label (file path or chunk name) supplied by the caller, if any.
    pub source: Option<String>,
    /// The error's `Display` rendering. May be multi-line.
    pub message: String,
}

impl LuaHost {
    /// Build a fresh Lua VM with the standard library loaded.
    ///
    /// # Errors
    ///
    /// Returns a [`mlua::Error`] if mlua fails to initialize. In practice
    /// this never happens on a working build; an error here would
    /// indicate a broken Lua C library link.
    pub fn new() -> mlua::Result<Self> {
        Self::with_registry(Rc::new(RefCell::new(BufferRegistry::new())))
    }

    /// Build a host that shares its [`BufferRegistry`] with another
    /// owner (typically [`crate::editor_core::EditorCore`], which
    /// also holds its own clone of the `Rc` so its windows can
    /// resolve buffer ids without going through the host).
    ///
    /// # Errors
    ///
    /// Returns a [`mlua::Error`] if mlua fails to initialize or the
    /// boundary closures fail to register.
    pub fn with_registry(registry: SharedRegistry) -> mlua::Result<Self> {
        let lua = Lua::new();
        let cancel = crate::lua_isolation::CancelToken::new();
        // T M7.8: install the count hook before any chunk runs so even
        // the first eval is interruptible. The hook closure captures
        // an `Arc<AtomicBool>` clone of `cancel`'s flag; subsequent
        // `cancel.cancel()` / `cancel.handle().cancel()` calls are
        // observed within `DEFAULT_INSTRUCTION_BUDGET` instructions.
        crate::lua_isolation::install_cancel_hook(
            &lua,
            &cancel,
            crate::lua_isolation::DEFAULT_INSTRUCTION_BUDGET,
        );
        let commands: SharedCommandRegistry = Rc::new(RefCell::new(CommandRegistry::new()));
        let keymaps: SharedKeymapStack = Rc::new(RefCell::new(KeymapStack::new()));
        let hooks: SharedHookRegistry = Rc::new(RefCell::new(HookRegistry::new()));
        lua_bindings::install(&lua, &registry, &commands, &keymaps, &hooks)?;
        Ok(Self {
            lua,
            registry,
            commands,
            keymaps,
            hooks,
            core: None,
            errors: Vec::new(),
            cancel,
            _not_send: PhantomData,
        })
    }

    /// Reference the underlying [`mlua::Lua`]. Use sparingly; prefer the
    /// curated APIs that later milestones layer on top.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Cross-thread handle for flipping this VM's cancel flag.
    ///
    /// The returned [`crate::lua_isolation::CancelHandle`] is
    /// `Send + Sync` and may be moved or cloned to other threads
    /// (e.g. an input-watching thread that maps C-g to a cancel
    /// request). The next time the count hook runs in the VM (within
    /// [`crate::lua_isolation::DEFAULT_INSTRUCTION_BUDGET`]
    /// instructions on lua54; see the LuaJIT-trace caveat in
    /// [`crate::lua_isolation`]) the running chunk aborts with an
    /// [`crate::lua_isolation::IsolationError::Cancelled`].
    #[must_use]
    pub fn cancel_handle(&self) -> crate::lua_isolation::CancelHandle {
        self.cancel.handle()
    }

    /// Flip this VM's cancel flag in-process. Equivalent to
    /// `self.cancel_handle().cancel()` for callers that already hold
    /// `&self`.
    pub fn request_cancel(&self) {
        self.cancel.cancel();
    }

    /// Shared handle to the buffer registry. Both Rust callers (e.g. the
    /// editor's file-open path) and Lua bindings (via app data) read and
    /// mutate this through the same `Rc<RefCell<...>>`.
    pub fn registry(&self) -> &SharedRegistry {
        &self.registry
    }

    /// Shared handle to the command registry.
    pub fn commands(&self) -> &SharedCommandRegistry {
        &self.commands
    }

    /// Shared handle to the keymap stack. The run loop's
    /// [`crate::keymap_stack::KeyDispatcher`] resolves chord sequences
    /// against this stack and dispatches the resulting commands.
    pub fn keymaps(&self) -> &SharedKeymapStack {
        &self.keymaps
    }

    /// Shared handle to the hook registry. T M2.11 surfaces it via
    /// `pmacs.describe.hook`; T M2.6 will wire actual hook execution
    /// into the appropriate editor lifecycle points.
    pub fn hooks(&self) -> &SharedHookRegistry {
        &self.hooks
    }

    /// Install `pmacs.editor.*` and load the builtin command + keymap
    /// chunks.
    ///
    /// Called once by [`crate::editor::EditorState::new`] after
    /// the [`crate::editor_core::EditorCore`] is wrapped into a
    /// [`SharedCore`]. The chunks are embedded at compile time via
    /// [`include_str!`]; their failure is treated as a build defect
    /// rather than a user error and surfaces as `mlua::Error`.
    ///
    /// # Errors
    ///
    /// Propagates any mlua failure from registering the editor table
    /// or loading the builtin chunks.
    pub fn attach_editor(&mut self, core: &SharedCore) -> mlua::Result<()> {
        lua_bindings::install_editor(&self.lua, core)?;
        self.core = Some(core.clone());
        // Hooks first: command bodies in default.lua reference them.
        self.load_builtin(
            "@pmacs/builtin/hooks/default.lua",
            include_str!("../builtin/hooks/default.lua"),
        )?;
        self.load_builtin(
            "@pmacs/builtin/commands/default.lua",
            include_str!("../builtin/commands/default.lua"),
        )?;
        self.load_builtin(
            "@pmacs/builtin/keymaps/default.lua",
            include_str!("../builtin/keymaps/default.lua"),
        )?;
        Ok(())
    }

    fn load_builtin(&mut self, source: &str, chunk: &str) -> mlua::Result<()> {
        self.eval(Some(source), chunk).map(|_| ())
    }

    /// Fire the named hook from Rust. Snapshots the registry so the
    /// callbacks may freely re-enter Lua. Errors are appended to the
    /// `*errors*` buffer (same surface as [`Self::eval`]) and reported
    /// via [`HookOutcome::errors`].
    ///
    /// Returns [`None`] if no hook with that name is defined --- the
    /// caller decides whether that is fatal (typo) or no-op (optional
    /// observation point).
    pub fn run_hook(&mut self, name: &str, args: MultiValue) -> Option<HookOutcome> {
        let snapshot = self.hooks.borrow().snapshot(name);
        let (kind, callbacks) = snapshot?;
        let outcome = crate::hook::run_snapshot(kind, &callbacks, args);
        // T M7.8: if any callback observed a cancellation, the flag
        // is still set — reset before the next eval. (Callbacks
        // dispatched after the first cancel observed the still-set
        // flag and aborted as well; that matches the user-intent
        // semantics of C-g during a hook fan-out.)
        let mut saw_cancel = false;
        for err in &outcome.errors {
            if crate::lua_isolation::is_cancellation(&err.error) {
                saw_cancel = true;
            }
            let record = LuaErrorRecord {
                at: SystemTime::now(),
                source: Some(format!("hook:{name}")),
                message: format!("callback at {} raised: {}", err.source.render(), err.error),
            };
            self.append_to_errors_buffer(&record);
            self.errors.push(record);
        }
        if saw_cancel {
            self.cancel.reset();
        }
        Some(outcome)
    }

    /// Invoke a registered command by name. Used by the keymap dispatch
    /// (T M2.4) and any Rust-side call site that needs to fire a
    /// command (e.g. M-x once the minibuffer lands in T M2.7).
    ///
    /// Returns the command body's return values as `mlua::MultiValue`,
    /// or an error wrapping [`crate::command::CommandError::NotFound`]
    /// when the name doesn't resolve, or the body's own error.
    ///
    /// # Errors
    ///
    /// Returns a [`mlua::Error`] if the command is not found, or if the
    /// invocation itself raises.
    pub fn invoke_command(
        &self,
        name: &str,
        args: mlua::MultiValue,
    ) -> mlua::Result<mlua::MultiValue> {
        let body = {
            let r = self.commands.borrow();
            r.get(name)
                .ok_or_else(|| {
                    mlua::Error::external(crate::command::CommandError::NotFound {
                        name: name.to_owned(),
                    })
                })?
                .body
                .clone()
        };
        match body.call::<mlua::MultiValue>(args) {
            Ok(v) => Ok(v),
            Err(e) => {
                // T M7.8: consume the cancel signal once.
                if crate::lua_isolation::is_cancellation(&e) {
                    self.cancel.reset();
                }
                Err(e)
            }
        }
    }

    /// Evaluate a Lua chunk and return the resulting value.
    ///
    /// On failure the error is captured into the host's error log *and*
    /// returned to the caller. Callers who only need the side-effect log
    /// (e.g. running a user config file) can drop the result; callers
    /// who need the value (M-x commands, etc.) get a typed error to act
    /// on.
    ///
    /// `source` is an optional label (file path, chunk name) used in
    /// diagnostics; Lua reports it back in stack traces.
    pub fn eval(&mut self, source: Option<&str>, chunk: &str) -> mlua::Result<Value> {
        // Push the source label into the per-Lua app-data slot so
        // bindings that need the chunk's location (e.g.,
        // `pmacs.packages.install_project`'s relative-path
        // resolution) can read it back. This is the only way to
        // recover chunk source from a Rust callback in pmacs, since
        // the Lua state is built without the `debug` library
        // (`forbid(unsafe_code)` rules out `Lua::unsafe_new`).
        self.lua
            .set_app_data(crate::lua_bindings::CurrentEvalSource(
                source.map(str::to_owned),
            ));
        let mut loader = self.lua.load(chunk);
        if let Some(name) = source {
            loader = loader.set_name(name);
        }
        match loader.eval::<Value>() {
            Ok(v) => Ok(v),
            Err(e) => {
                // T M7.8: a cancellation is consumed exactly once.
                // Reset the flag here so the *next* eval starts
                // fresh. If we left it set, the very first hook tick
                // of the next chunk would abort it without anyone
                // having asked.
                if crate::lua_isolation::is_cancellation(&e) {
                    self.cancel.reset();
                }
                let record = LuaErrorRecord {
                    at: SystemTime::now(),
                    source: source.map(str::to_owned),
                    message: e.to_string(),
                };
                self.append_to_errors_buffer(&record);
                self.errors.push(record);
                Err(e)
            }
        }
    }

    /// Append `record` to the buffer named `*errors*`, creating the
    /// buffer on first use. Errors during the append are themselves
    /// dropped --- the error log is a best-effort surface, and a
    /// failure here would only happen if the buffer were itself
    /// concurrently held in a way the single-threaded contract
    /// rules out.
    fn append_to_errors_buffer(&self, record: &LuaErrorRecord) {
        let line = format!(
            "[{}] {}\n",
            record.source.as_deref().unwrap_or("[chunk]"),
            record.message
        );
        let (id, edit) = {
            let mut reg = self.registry.borrow_mut();
            let id = match reg.find_by_name(ERRORS_BUFFER_NAME) {
                Some(id) => id,
                None => reg.create(ERRORS_BUFFER_NAME),
            };
            let Ok(buf) = reg.get_mut(id) else {
                return;
            };
            let pos = buf.len();
            let Ok(edit) = buf.apply_edit(EditOp::Insert {
                pos,
                bytes: line.as_bytes(),
            }) else {
                return;
            };
            (id, edit)
        };
        // Window TextViews are not attached views on the buffer; they
        // sit on EditorCore and miss the broadcast that
        // `Buffer::apply_edit` performs on its own attached views. If a
        // window is currently displaying `*errors*` (e.g. the user
        // switched to it via C-x b), its line cache would otherwise go
        // stale on every appended error and cursor motion would stop
        // updating the screen.
        if let Some(core) = self.core.as_ref() {
            core.borrow_mut().notify_buffer_edit(id, &edit);
        }
    }

    /// `BufferId` of the `*errors*` buffer if it has been created.
    /// Returned by [`crate::editor::EditorState`] so the renderer can
    /// surface it once T M2.8 lands buffer-switching.
    #[must_use]
    pub fn errors_buffer_id(&self) -> Option<crate::buffer::BufferId> {
        self.registry.borrow().find_by_name(ERRORS_BUFFER_NAME)
    }

    /// Snapshot the full text of the `*errors*` buffer as a UTF-8
    /// string (lossy on non-UTF-8 bytes — error messages are routinely
    /// concatenations of arbitrary user data).
    ///
    /// Returns the empty string if the buffer hasn't been created
    /// yet. Used by tests and any introspection tool that needs the
    /// canonical error log without going through the buffer registry
    /// directly. Does not consume or clear the buffer.
    #[must_use]
    pub fn errors_buffer_text(&self) -> String {
        let Some(id) = self.errors_buffer_id() else {
            return String::new();
        };
        let reg = self.registry.borrow();
        let Ok(buf) = reg.get(id) else {
            return String::new();
        };
        let rope = buf.snapshot_rope();
        let len = rope.len();
        if len == 0 {
            return String::new();
        }
        let mut bytes = vec![0u8; usize::try_from(len).unwrap_or(usize::MAX)];
        rope.slice(0, len, &mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// All captured errors, in arrival order.
    pub fn errors(&self) -> &[LuaErrorRecord] {
        &self.errors
    }

    /// Most recently captured error, if any.
    pub fn last_error(&self) -> Option<&LuaErrorRecord> {
        self.errors.last()
    }

    /// Discard the captured error log.
    ///
    /// Useful once errors have been drained into the `*errors*` buffer
    /// (T M2.8) so the in-memory log doesn't grow without bound.
    pub fn clear_errors(&mut self) {
        self.errors.clear();
    }

    /// Mark the init phase complete.
    ///
    /// Called by [`crate::editor::EditorState::new`] after
    /// [`crate::config::load_user_config`] returns. Lifecycle-affecting
    /// Lua APIs (currently `pmacs.attach`; M5.6d+) gate on this flag
    /// via [`crate::lua_bindings::require_init_phase`] and refuse to
    /// run mid-session.
    ///
    /// Idempotent — calling twice is safe. If [`crate::lua_bindings::install`]
    /// was never called (the flag is missing from app data), this is a
    /// silent no-op rather than a panic; the missing-flag case is
    /// reported the next time a binding tries to read it.
    pub fn set_init_complete(&self) {
        if let Some(flag) = self.lua.app_data_ref::<InitCompleteFlag>() {
            flag.set_complete();
        }
    }

    /// Re-open the init phase (test/dev only). Counterpart to
    /// [`Self::set_init_complete`]: integration tests that exercise
    /// init-only Lua APIs against a fully-constructed
    /// [`crate::editor::EditorState`] use this to reset the flag
    /// the editor flips during startup. Marked `#[doc(hidden)]` to
    /// keep it out of the user-facing surface; production code
    /// never re-opens init phase after a single startup flip.
    #[doc(hidden)]
    pub fn reopen_init_phase_for_testing(&self) {
        if let Some(flag) = self.lua.app_data_ref::<InitCompleteFlag>() {
            flag.reopen_for_testing();
        }
    }

    /// Whether the init phase has finished. Mirrors the
    /// [`InitCompleteFlag`] state for callers that need to introspect
    /// without going through Lua app data themselves.
    #[must_use]
    pub fn is_init_complete(&self) -> bool {
        self.lua
            .app_data_ref::<InitCompleteFlag>()
            .is_some_and(|f| f.is_complete())
    }

    /// Consume the attach target requested by `pmacs.attach{...}` from
    /// init.lua, if any.
    ///
    /// Called by [`crate::editor::EditorState::new`] (or its eventual
    /// dispatcher) after [`crate::config::load_user_config`] returns,
    /// to decide whether to run the local editor or hand off to attach
    /// mode against a remote daemon.
    ///
    /// Returns `None` when init.lua did not call `pmacs.attach{...}`,
    /// or when the request has already been consumed.
    #[must_use]
    pub fn take_requested_attach(&self) -> Option<AttachTarget> {
        self.lua
            .app_data_ref::<RequestedAttach>()
            .and_then(|r| r.take())
    }

    /// Record the current outbound attachment so `pmacs.current_attachment()`
    /// returns a populated handle.
    ///
    /// In v0.1 there is no production-side caller — Local mode never
    /// completes a remote attach (the process hands off to attach mode,
    /// which has no `LuaHost`), and Daemon mode is the *target* of
    /// attachments. The setter exists for tests and for the future
    /// dispatcher (M5.6g+) to wire up.
    pub fn set_current_attachment(&self, handle: AttachmentHandle) {
        if let Some(slot) = self.lua.app_data_ref::<CurrentAttachmentSlot>() {
            slot.set(handle);
        }
    }

    /// Clear any recorded current attachment. No-op if already empty
    /// or if [`crate::lua_bindings::install`] was never called.
    pub fn clear_current_attachment(&self) {
        if let Some(slot) = self.lua.app_data_ref::<CurrentAttachmentSlot>() {
            slot.clear();
        }
    }

    /// Read the current attachment without consuming it. Mirrors what
    /// `pmacs.current_attachment()` exposes to Lua, but returns the
    /// Rust struct instead of a Lua table.
    #[must_use]
    pub fn current_attachment(&self) -> Option<AttachmentHandle> {
        self.lua
            .app_data_ref::<CurrentAttachmentSlot>()
            .and_then(|s| s.get())
    }

    /// Install a [`PackageInstallOverride`] so subsequent
    /// `pmacs.packages.install{...}` calls redirect their cache and
    /// install roots away from `$XDG_*` defaults.
    ///
    /// Production code does not call this. Tests use it because the
    /// project's `forbid(unsafe_code)` rules out `std::env::set_var`
    /// (which has been `unsafe` since Rust 2024).
    pub fn set_package_install_override(&self, override_: PackageInstallOverride) {
        self.lua.set_app_data(override_);
    }

    /// Override the instance name reported by `pmacs.instance.identity()`
    /// (M5.6f).
    ///
    /// Daemon mode calls this with the value the user passed via
    /// `--socket NAME`; Local mode never calls it (the slot's default
    /// `None` is correct for an in-process editor with no remote name).
    /// No-op if [`crate::lua_bindings::install`] was never called.
    pub fn set_instance_name(&self, name: Option<String>) {
        if let Some(info) = self.lua.app_data_ref::<LocalInstanceInfo>() {
            info.set_name(name);
        }
    }

    /// Override the `started` anchor used by `pmacs.instance.identity()`
    /// to compute uptime.
    ///
    /// Daemon mode calls this with its own `DaemonState::started` so
    /// the uptime reported via `pmacs.instance.*` matches what the
    /// daemon hands back over its `Hello`. Local mode leaves the
    /// install-time default in place.
    pub fn set_instance_started(&self, started: std::time::Instant) {
        if let Some(info) = self.lua.app_data_ref::<LocalInstanceInfo>() {
            info.set_started(started);
        }
    }

    /// Build an [`InstanceIdentity`] from the [`LocalInstanceInfo`]
    /// slot. Mirrors what `pmacs.instance.identity()` returns to Lua,
    /// for Rust-side callers (tests, the future M5.6g dispatcher).
    /// Returns `None` if [`crate::lua_bindings::install`] was never
    /// called on this state.
    #[must_use]
    pub fn local_instance_identity(&self) -> Option<InstanceIdentity> {
        self.lua
            .app_data_ref::<LocalInstanceInfo>()
            .map(|info| info.build_identity())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_script_runs() {
        let mut host = LuaHost::new().unwrap();
        let v = host.eval(None, "return 1 + 2").unwrap();
        match v {
            Value::Integer(n) => assert_eq!(n, 3),
            other => panic!("expected integer, got {other:?}"),
        }
        assert!(host.errors().is_empty());
    }

    #[test]
    fn syntax_error_is_captured_not_panicked() {
        let mut host = LuaHost::new().unwrap();
        let result = host.eval(Some("bad_chunk"), "this is not valid lua )");
        assert!(result.is_err());
        let last = host.last_error().expect("error captured");
        assert_eq!(last.source.as_deref(), Some("bad_chunk"));
        assert!(!last.message.is_empty());
    }

    #[test]
    fn runtime_error_is_captured() {
        let mut host = LuaHost::new().unwrap();
        let result = host.eval(None, "error('boom')");
        assert!(result.is_err());
        assert_eq!(host.errors().len(), 1);
        assert!(host.last_error().unwrap().message.contains("boom"));
    }

    #[test]
    fn multiple_errors_accumulate_in_order() {
        let mut host = LuaHost::new().unwrap();
        let _ = host.eval(None, "error('first')");
        let _ = host.eval(None, "error('second')");
        let errs = host.errors();
        assert_eq!(errs.len(), 2);
        assert!(errs[0].message.contains("first"));
        assert!(errs[1].message.contains("second"));
    }

    #[test]
    fn clear_errors_drains_the_log() {
        let mut host = LuaHost::new().unwrap();
        let _ = host.eval(None, "error('x')");
        assert_eq!(host.errors().len(), 1);
        host.clear_errors();
        assert!(host.errors().is_empty());
    }

    #[test]
    fn captured_error_lands_in_errors_buffer() {
        let mut host = LuaHost::new().unwrap();
        let _ = host.eval(Some("usercfg"), "error('boom')");
        let id = host.errors_buffer_id().expect("*errors* buffer created");
        let reg = host.registry().borrow();
        let buf = reg.get(id).unwrap();
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("[usercfg]"), "errors-buffer body: {body}");
        assert!(body.contains("boom"), "errors-buffer body: {body}");
        assert!(body.ends_with('\n'), "errors-buffer body: {body:?}");
    }

    #[test]
    fn errors_buffer_appends_each_error() {
        // Lua errors carry a multi-line traceback, so the buffer body
        // grows by more than one line per error. Verify by message
        // content and entry-prefix count rather than total line count.
        let mut host = LuaHost::new().unwrap();
        let _ = host.eval(None, "error('first')");
        let _ = host.eval(None, "error('second')");
        let id = host.errors_buffer_id().expect("buffer exists");
        let reg = host.registry().borrow();
        let buf = reg.get(id).unwrap();
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("first"), "missing first: {body}");
        assert!(body.contains("second"), "missing second: {body}");
        // Two entries: each begins with `[chunk]` (since `source` is None).
        let entries = body.matches("[chunk]").count();
        assert_eq!(entries, 2, "expected 2 entries; body: {body}");
    }

    #[test]
    fn standard_library_is_loaded() {
        // Confirms mlua opened the stdlib --- string.upper, math.pi, etc.
        // are accessible to user chunks.
        let mut host = LuaHost::new().unwrap();
        let v = host.eval(None, "return string.upper('hi')").unwrap();
        match v {
            Value::String(s) => assert_eq!(s.to_str().unwrap(), "HI"),
            other => panic!("expected string, got {other:?}"),
        }
    }

    // ----- M5.6c: init-complete plumbing on the host -----

    #[test]
    fn host_starts_in_init_phase() {
        // Fresh host = init phase in progress; lifecycle-gated bindings
        // should permit calls.
        let host = LuaHost::new().unwrap();
        assert!(!host.is_init_complete());
    }

    #[test]
    fn host_set_init_complete_flips_the_flag() {
        let host = LuaHost::new().unwrap();
        host.set_init_complete();
        assert!(host.is_init_complete());
    }

    #[test]
    fn host_set_init_complete_is_idempotent() {
        // EditorState::new could in principle be re-entered through a
        // test path; the flip must remain safe to call repeatedly.
        let host = LuaHost::new().unwrap();
        host.set_init_complete();
        host.set_init_complete();
        assert!(host.is_init_complete());
    }

    #[test]
    fn host_eval_does_not_implicitly_flip_init_complete() {
        // Builtin chunk loads (async / syntax / lsp / hooks / commands /
        // keymaps) all go through `eval` *before* user init.lua runs.
        // None of those should flip the gate; only `set_init_complete`
        // does.
        let mut host = LuaHost::new().unwrap();
        let _ = host.eval(None, "return 1");
        let _ = host.eval(Some("@builtin/example"), "x = 42");
        assert!(
            !host.is_init_complete(),
            "ordinary eval must not flip init-complete"
        );
    }

    #[test]
    fn host_load_user_config_does_not_flip_init_complete() {
        // The flip is the responsibility of EditorState::new (after
        // load_user_config_at returns), not of load_user_config_at
        // itself. Pin this so a future refactor that moves the flip
        // into load_user_config_at would force reconciliation with
        // tests' explicit `set_init_complete` calls.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("init.lua"), "USER_FLAG = 1").unwrap();
        let mut host = LuaHost::new().unwrap();
        crate::config::load_user_config_at(&mut host, dir.path());
        assert!(
            !host.is_init_complete(),
            "load_user_config_at alone must not flip init-complete"
        );
    }

    // ----- M5.6d: take_requested_attach plumbing -----

    #[test]
    fn take_requested_attach_returns_none_with_no_call() {
        let host = LuaHost::new().unwrap();
        assert!(host.take_requested_attach().is_none());
    }

    #[test]
    fn take_requested_attach_returns_target_set_from_init_lua() {
        // End-to-end: write an init.lua that calls pmacs.attach, run
        // it through load_user_config_at, then take the request from
        // the host.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("init.lua"),
            r#"pmacs.attach{ target = "local:/run/p.sock" }"#,
        )
        .unwrap();
        let mut host = LuaHost::new().unwrap();
        crate::config::load_user_config_at(&mut host, dir.path());
        let target = host.take_requested_attach().expect("request recorded");
        match target {
            AttachTarget::LocalSocket(p) => {
                assert_eq!(p, std::path::PathBuf::from("/run/p.sock"));
            }
            other => panic!("expected LocalSocket, got {other:?}"),
        }
        // Subsequent take is None.
        assert!(host.take_requested_attach().is_none());
    }

    #[test]
    fn init_lua_attach_error_does_not_record_request() {
        // A bad pmacs.attach call inside init.lua errors and is
        // captured into the error log, but no request is recorded
        // (the slot stays empty so the post-init dispatcher runs the
        // editor normally rather than handing off to a doomed attach).
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("init.lua"),
            r#"pmacs.attach{ kind = "smtp" }"#,
        )
        .unwrap();
        let mut host = LuaHost::new().unwrap();
        crate::config::load_user_config_at(&mut host, dir.path());
        assert!(host.last_error().is_some(), "error captured");
        assert!(
            host.take_requested_attach().is_none(),
            "no request should be recorded for a failed pmacs.attach call"
        );
    }

    // ----- M5.6e: current attachment plumbing on the host -----

    fn sample_handle() -> AttachmentHandle {
        AttachmentHandle::new(
            crate::protocol::FrontendId(7),
            crate::protocol::InstanceIdentity {
                pmacs_version: "0.1.0".into(),
                build_hash: None,
                instance_name: None,
                uptime_secs: 0,
                working_directory: "/tmp".into(),
            },
            AttachTarget::LocalSocket(std::path::PathBuf::from("/run/p.sock")),
        )
    }

    #[test]
    fn host_current_attachment_returns_none_by_default() {
        let host = LuaHost::new().unwrap();
        assert!(host.current_attachment().is_none());
    }

    #[test]
    fn host_set_current_attachment_round_trips() {
        let host = LuaHost::new().unwrap();
        let h = sample_handle();
        host.set_current_attachment(h.clone());
        assert_eq!(host.current_attachment().unwrap(), h);
    }

    #[test]
    fn host_clear_current_attachment_empties_slot() {
        let host = LuaHost::new().unwrap();
        host.set_current_attachment(sample_handle());
        host.clear_current_attachment();
        assert!(host.current_attachment().is_none());
    }

    #[test]
    fn host_set_current_attachment_overwrites_prior() {
        // Unlike RequestedAttach.try_set, the current-attachment slot
        // accepts overwrites — the dispatcher may reattach over time
        // (forward-compat for v0.2 reconnect flow). Pin this so we
        // notice if either contract drifts.
        let host = LuaHost::new().unwrap();
        let first = sample_handle();
        let second = AttachmentHandle::new(
            crate::protocol::FrontendId(99),
            first.identity.clone(),
            first.target.clone(),
        );
        host.set_current_attachment(first);
        host.set_current_attachment(second.clone());
        assert_eq!(host.current_attachment().unwrap(), second);
    }

    // -----------------------------------------------------------------------
    // T M5.6f --- LuaHost local instance accessors
    // -----------------------------------------------------------------------

    #[test]
    fn host_local_instance_identity_present_after_install() {
        let host = LuaHost::new().unwrap();
        let id = host
            .local_instance_identity()
            .expect("install populates the slot");
        assert_eq!(id.pmacs_version, env!("CARGO_PKG_VERSION"));
        assert!(id.instance_name.is_none(), "default name is None");
    }

    #[test]
    fn host_set_instance_name_round_trips_through_identity() {
        let host = LuaHost::new().unwrap();
        host.set_instance_name(Some("work".into()));
        let id = host.local_instance_identity().unwrap();
        assert_eq!(id.instance_name.as_deref(), Some("work"));
        host.set_instance_name(None);
        let id = host.local_instance_identity().unwrap();
        assert!(id.instance_name.is_none());
    }

    #[test]
    fn host_set_instance_started_changes_uptime() {
        let host = LuaHost::new().unwrap();
        let earlier = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(7_777))
            .expect("clock supports 7777s subtraction");
        host.set_instance_started(earlier);
        let id = host.local_instance_identity().unwrap();
        assert!(
            id.uptime_secs >= 7_777,
            "expected uptime >= 7777 after rewinding `started`; got {}",
            id.uptime_secs
        );
    }
}
