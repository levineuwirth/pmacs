//! Terminal process/session registry.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use thiserror::Error;

use crate::ansi::AnsiParserProfile;
use crate::buffer::{Buffer, BufferId};
use crate::cell::{Cell, CellCoord, CellSize};
use crate::editor_core::EditorCore;
use crate::key::{Chord, parse_chord};
use crate::process::{
    ProcessEventKind, ProcessId, ProcessMode, ProcessSpec, ProcessState, ProcessSupervisor,
    RestartPolicy, StdinMode, TerminalMode,
};
use crate::terminal::screen::TerminalScreen;
use crate::terminal::view::{TerminalController, TerminalViewKey, TerminalViewState};
use crate::terminal::{
    MAX_TERMINAL_COLS, MAX_TERMINAL_HISTORY_CELLS, MAX_TERMINAL_METADATA_BYTES, MAX_TERMINAL_ROWS,
    MAX_TERMINAL_VISIBLE_CELLS, TerminalFrame, TerminalProcessState, TerminalSelectionSpan,
};

/// Shared single-owner terminal registry used by editor and future Lua bindings.
pub type SharedTerminalManager = Rc<RefCell<TerminalManager>>;

/// Complete owned description of a terminal child and its initial screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSpec {
    /// Executable path or name resolved through `PATH`.
    pub command: String,
    /// Child arguments, excluding argv[0].
    pub args: Vec<String>,
    /// Working directory, or the editor process directory when absent.
    pub cwd: Option<PathBuf>,
    /// Environment overrides inherited by the child. `TERM` defaults to
    /// `xterm-256color` when the caller does not provide it.
    pub env: Vec<(String, String)>,
    /// Identity-buffer name. Defaults to `*terminal:<command>*`.
    pub name: Option<String>,
    /// Initial terminal rows.
    pub rows: u16,
    /// Initial terminal columns.
    pub cols: u16,
    /// Retained main-screen scrollback row cap.
    pub scrollback_rows: usize,
}

impl TerminalSpec {
    /// Construct a conventional 24x80 terminal specification.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            name: None,
            rows: 24,
            cols: 80,
            scrollback_rows: crate::terminal::DEFAULT_TERMINAL_SCROLLBACK_ROWS,
        }
    }

    /// Validate every raw field before any buffer or process is created.
    pub fn validate(&self) -> Result<(), TerminalError> {
        if self.command.is_empty() {
            return Err(TerminalError::InvalidSpec(
                "command must not be empty".into(),
            ));
        }
        reject_nul("command", self.command.as_bytes())?;
        for arg in &self.args {
            reject_nul("argument", arg.as_bytes())?;
        }
        if let Some(cwd) = &self.cwd {
            if cwd.as_os_str().is_empty() {
                return Err(TerminalError::InvalidSpec(
                    "cwd must not be an empty path".into(),
                ));
            }
            reject_nul("cwd", cwd.as_os_str().as_encoded_bytes())?;
        }
        let mut env_names = HashSet::with_capacity(self.env.len());
        for (name, value) in &self.env {
            if name.is_empty() || name.contains('=') {
                return Err(TerminalError::InvalidSpec(format!(
                    "environment name {name:?} must be non-empty and contain no '='"
                )));
            }
            reject_nul("environment name", name.as_bytes())?;
            reject_nul("environment value", value.as_bytes())?;
            if !env_names.insert(name) {
                return Err(TerminalError::InvalidSpec(format!(
                    "duplicate environment name {name:?}"
                )));
            }
        }
        if let Some(name) = &self.name {
            if name.is_empty() {
                return Err(TerminalError::InvalidSpec(
                    "buffer name must not be empty".into(),
                ));
            }
            reject_nul("buffer name", name.as_bytes())?;
            if name.contains(['\r', '\n']) {
                return Err(TerminalError::InvalidSpec(
                    "buffer name must fit on one line".into(),
                ));
            }
        }
        validate_size(self.rows, self.cols)?;
        if self.scrollback_rows > MAX_TERMINAL_HISTORY_CELLS {
            return Err(TerminalError::InvalidSpec(format!(
                "scrollback row cap {} exceeds terminal history cell budget {}",
                self.scrollback_rows, MAX_TERMINAL_HISTORY_CELLS
            )));
        }
        Ok(())
    }

    fn buffer_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            let command = Path::new(&self.command)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(self.command.as_str());
            format!("*terminal:{command}*")
        })
    }
}

/// Owned, renderer-safe terminal state captured after a manager tick.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSnapshot {
    /// Identity buffer backing this terminal.
    pub buffer_id: BufferId,
    /// Visible grid dimensions.
    pub size: CellSize,
    /// Row-major visible cells.
    pub cells: Vec<Cell>,
    /// Visible child cursor, if enabled.
    pub cursor: Option<CellCoord>,
    /// Sanitized child title.
    pub title: Option<String>,
    /// Published screen generation.
    pub screen_generation: u64,
    /// Context selection. Empty in context-free Stage 1 snapshots.
    pub selection: Vec<TerminalSelectionSpan>,
    /// Context scrollback offset. Zero in Stage 1 snapshots.
    pub scroll_offset: u32,
    /// Whether this context follows the bottom. Always true in Stage 1.
    pub at_bottom: bool,
    /// Exact operating-system process id for this session generation.
    pub pid: u32,
    /// Latest observed process state.
    pub process: TerminalProcessState,
}

impl TerminalSnapshot {
    /// Convert an owned snapshot into its protocol-v19 wire form.
    ///
    /// The two shapes are deliberately distinct types even though their
    /// fields line up: the snapshot is core-owned state the TUI also
    /// consumes, while [`TerminalFrame`] is wire input a peer may forge.
    /// The conversion is total, but the CALLER must still
    /// [`TerminalFrame::validate`] before emitting — the aggregate glyph
    /// bound is a wire limit the screen does not enforce, so a child
    /// that builds a legal-but-huge internal snapshot must be caught
    /// here rather than sent truncated.
    #[must_use]
    pub fn into_terminal_frame(self) -> TerminalFrame {
        TerminalFrame {
            buffer_id: self.buffer_id,
            size: self.size,
            cells: self.cells,
            cursor: self.cursor,
            title: self.title,
            screen_generation: self.screen_generation,
            selection: self.selection,
            scroll_offset: self.scroll_offset,
            at_bottom: self.at_bottom,
            pid: self.pid,
            process: self.process,
        }
    }
}

/// Terminal session/registry failures.
#[derive(Debug, Error)]
pub enum TerminalError {
    /// Specification validation failed before creation began.
    #[error("invalid terminal specification: {0}")]
    InvalidSpec(String),
    /// The synchronous PTY spawn failed; no session is published.
    #[error("terminal spawn failed: {0}")]
    Spawn(String),
    /// Buffer registry work failed during transactional creation.
    #[error("terminal buffer operation failed: {0}")]
    Buffer(String),
    /// Screen construction or resize failed.
    #[error("terminal screen operation failed: {0}")]
    Screen(String),
    /// No session owns the requested identity buffer.
    #[error("buffer {0:?} is not a terminal")]
    NotTerminal(BufferId),
    /// Process I/O, resize, signal, or cleanup failed.
    #[error("terminal process operation failed: {0}")]
    Process(String),
}

pub(super) struct TerminalSession {
    pub(super) process_id: ProcessId,
    pub(super) pid: u32,
    pub(super) screen: TerminalScreen,
    pub(super) process: TerminalProcessState,
    pub(super) annotated: bool,
    /// Resolved `terminal.escape-key` for this terminal (Q#TC4c).
    ///
    /// The cache lives HERE, not in an editor-side map, because a
    /// session is created in [`TerminalManager::open`] and dropped on
    /// kill/prune — so its lifetime is exactly the cache's, with no
    /// purge hook to forget. An editor-side map would leak an entry per
    /// terminal; a single last-entry cache would reparse (and re-report
    /// an invalid value) every time focus alternates between two
    /// terminals.
    pub(super) escape: Option<EscapeCache>,
}

/// One terminal's parsed escape chord, valid for one config epoch.
pub(super) struct EscapeCache {
    /// The `ConfigRegistry::value_epoch` this was parsed at. The key is
    /// `(this session, epoch)`: the epoch alone is not enough, because
    /// it does not advance when focus moves between terminals with
    /// different buffer-local values.
    pub(super) epoch: u64,
    /// The effective chord — the parsed spelling, or the `C-c` fallback.
    pub(super) chord: Chord,
    /// The invalid spelling already reported for this terminal, if any.
    /// Reporting is once per terminal per effective invalid value: an
    /// unchanged bad value stays quiet, a *different* bad value reports
    /// again because it is a new mistake.
    pub(super) reported_invalid: Option<String>,
}

/// What each child was sent, in order, while the G5k tap is armed.
type ChildSendLog = Vec<(BufferId, Vec<u8>)>;

/// Owns the one-buffer/one-process/one-screen terminal registry.
#[derive(Default)]
pub struct TerminalManager {
    pub(super) sessions: HashMap<BufferId, TerminalSession>,
    /// An OPT-IN tap on child input, for parent 48 G5k's witnesses.
    ///
    /// The gesture-domain rows have to read what the child actually
    /// received --- a release delivered in the recorded encoding, and
    /// exactly one of it --- and no other seam exposes that. Off by
    /// default, so production pays one `is_some` check per send and
    /// never accumulates.
    send_tap: RefCell<Option<ChildSendLog>>,
    /// Total escape-key parses performed (Q#TC4c observability).
    escape_parses: u64,
    process_to_buffer: HashMap<ProcessId, BufferId>,
    /// Removed buffers whose children are still being reaped. Their events
    /// remain manager-owned so Lua/LSP/MCP consumers cannot steal a batch.
    closing: HashSet<ProcessId>,
    /// Per-frontend/window projections over the one session screen.
    pub(super) views: HashMap<TerminalViewKey, TerminalViewState>,
    /// At most one authenticated frontend/window controls each session PTY.
    pub(super) controllers: HashMap<BufferId, TerminalController>,
}

impl TerminalManager {
    /// Construct an empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of published terminal sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether no terminal session is currently published.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Transactionally create an internal terminal identity, PTY, and screen.
    pub fn open(
        &mut self,
        spec: TerminalSpec,
        core: &mut EditorCore,
        supervisor: &mut ProcessSupervisor,
    ) -> Result<BufferId, TerminalError> {
        spec.validate()?;
        let size = CellSize::new(u32::from(spec.rows), u32::from(spec.cols));
        let screen = TerminalScreen::new(size, spec.scrollback_rows)
            .map_err(|error| TerminalError::Screen(error.to_string()))?;

        let base_name = spec.buffer_name();
        let buffer_name = if spec.name.is_some() {
            base_name
        } else {
            unique_terminal_name(core, &base_name)
        };
        let buffer_id = BufferId::next();
        let mut buffer = Buffer::new(buffer_id, buffer_name.clone());
        buffer.set_read_only(true);
        core.registry.borrow_mut().insert(buffer);

        let purpose = format!("terminal running {}", spec.command);
        let mut process_spec = ProcessSpec::new(buffer_name, spec.command, purpose);
        process_spec.args = spec.args;
        process_spec.cwd = spec.cwd;
        process_spec.env = spec.env;
        if !process_spec.env.iter().any(|(name, _)| name == "TERM") {
            process_spec
                .env
                .push(("TERM".into(), "xterm-256color".into()));
        }
        process_spec.mode = ProcessMode::Pty {
            rows: spec.rows,
            cols: spec.cols,
            mode: TerminalMode::Raw,
        };
        process_spec.restart = RestartPolicy::Never;
        process_spec.ansi_events = true;
        process_spec.ansi_profile = AnsiParserProfile::FullScreen;
        process_spec.stdin = StdinMode::Piped;
        process_spec.group = false;

        let process_id = match supervisor.spawn_terminal(process_spec) {
            Ok(id) => id,
            Err(error) => {
                core.registry
                    .borrow_mut()
                    .remove(buffer_id)
                    .map_err(|rollback| {
                        TerminalError::Buffer(format!(
                            "spawn failed ({error}); buffer rollback failed: {rollback}"
                        ))
                    })?;
                return Err(TerminalError::Spawn(error));
            }
        };
        let pid =
            if let Some(ProcessState::Running { pid, .. } | ProcessState::Exiting { pid, .. }) =
                supervisor.state(process_id)
            {
                *pid
            } else {
                let _ = supervisor.terminate(process_id);
                let _ = core.registry.borrow_mut().remove(buffer_id);
                return Err(TerminalError::Spawn(
                    "supervisor published a PTY without a running pid".into(),
                ));
            };

        let previous = self.sessions.insert(
            buffer_id,
            TerminalSession {
                process_id,
                pid,
                screen,
                process: TerminalProcessState::Running,
                annotated: false,
                escape: None,
            },
        );
        debug_assert!(previous.is_none(), "fresh BufferId collided");
        self.process_to_buffer.insert(process_id, buffer_id);
        core.set_round_trip_input(buffer_id, true);
        Ok(buffer_id)
    }

    /// Whether `buffer_id` identifies a published terminal session.
    #[must_use]
    pub fn is_terminal(&self, buffer_id: BufferId) -> bool {
        self.sessions.contains_key(&buffer_id)
    }

    /// Owned process id for a terminal buffer. The OS pid stays in snapshots.
    #[must_use]
    pub fn process_id(&self, buffer_id: BufferId) -> Option<ProcessId> {
        self.sessions
            .get(&buffer_id)
            .map(|session| session.process_id)
    }

    /// Monotonic terminal BEL count used for per-frontend delivery baselines.
    #[must_use]
    pub fn bell_count(&self, buffer_id: BufferId) -> Option<u64> {
        self.sessions
            .get(&buffer_id)
            .map(|session| session.screen.bell_count())
    }

    /// Ensure an exact terminal view exists without changing its controller.
    ///
    /// Returns `false` when the key's buffer is not a published terminal.
    pub fn register_view(&mut self, key: TerminalViewKey) -> bool {
        if !self.sessions.contains_key(&key.buffer_id) {
            return false;
        }
        self.views.entry(key).or_default();
        true
    }

    /// Borrow fresh mutable state for an already registered exact view.
    pub fn view_state_mut(&mut self, key: TerminalViewKey) -> Option<&mut TerminalViewState> {
        self.views.get_mut(&key)
    }

    /// Borrow fresh state for an already registered exact view.
    #[must_use]
    pub fn view_state(&self, key: TerminalViewKey) -> Option<&TerminalViewState> {
        self.views.get(&key)
    }

    /// Retain only `live` views belonging to one authenticated frontend.
    pub fn retain_frontend_views(
        &mut self,
        frontend_id: crate::protocol::FrontendId,
        live: &HashSet<TerminalViewKey>,
    ) {
        self.views.retain(|key, _| {
            key.frontend_id != frontend_id
                || (live.contains(key) && self.sessions.contains_key(&key.buffer_id))
        });
        self.controllers.retain(|buffer_id, controller| {
            controller.frontend_id != frontend_id
                || live.contains(&TerminalViewKey::new(
                    frontend_id,
                    controller.window_id,
                    *buffer_id,
                ))
        });
    }

    /// Drop all view and controller state owned by a detached frontend.
    pub fn detach_frontend(&mut self, frontend_id: crate::protocol::FrontendId) {
        self.views.retain(|key, _| key.frontend_id != frontend_id);
        self.controllers
            .retain(|_, controller| controller.frontend_id != frontend_id);
    }

    /// Give an exact registered view durable PTY control for its session.
    ///
    /// A frontend controls at most one session. Claiming another registered
    /// view atomically releases that frontend's previous session first.
    pub fn claim_controller(&mut self, key: TerminalViewKey) -> bool {
        if !self.views.contains_key(&key) || !self.sessions.contains_key(&key.buffer_id) {
            return false;
        }
        self.controllers.retain(|buffer_id, controller| {
            controller.frontend_id != key.frontend_id || *buffer_id == key.buffer_id
        });
        self.controllers
            .insert(key.buffer_id, TerminalController::from_view(key));
        true
    }

    /// Release control only when `key` is the current controller.
    pub fn release_controller(&mut self, key: TerminalViewKey) -> bool {
        if self
            .controllers
            .get(&key.buffer_id)
            .is_some_and(|controller| controller.matches(key))
        {
            self.controllers.remove(&key.buffer_id);
            true
        } else {
            false
        }
    }

    /// Current durable controller for one terminal session.
    #[must_use]
    pub fn controller(&self, buffer_id: BufferId) -> Option<TerminalController> {
        self.controllers.get(&buffer_id).copied()
    }

    /// Capture context-free owned visible state after the latest tick.
    #[must_use]
    pub fn snapshot(&self, buffer_id: BufferId) -> Option<TerminalSnapshot> {
        let session = self.sessions.get(&buffer_id)?;
        let screen = session.screen.snapshot();
        Some(TerminalSnapshot {
            buffer_id,
            size: screen.size,
            cells: screen.cells,
            cursor: screen.cursor,
            title: screen.title.map(|title| sanitize_metadata(&title)),
            screen_generation: screen.generation,
            selection: Vec::new(),
            scroll_offset: 0,
            at_bottom: true,
            pid: session.pid,
            process: session.process.clone(),
        })
    }

    /// Drain only terminal-owned process IDs after the supervisor tick.
    pub fn tick(&mut self, supervisor: &mut ProcessSupervisor) {
        let process_ids: Vec<ProcessId> = self.process_to_buffer.keys().copied().collect();
        for process_id in process_ids {
            let Some(buffer_id) = self.process_to_buffer.get(&process_id).copied() else {
                continue;
            };
            let events = supervisor.take_events(process_id);
            let Some(session) = self.sessions.get_mut(&buffer_id) else {
                continue;
            };
            let mut outcome = None;
            for event in events {
                match event.kind {
                    ProcessEventKind::Started { pid } => session.pid = pid,
                    ProcessEventKind::Ansi(events) => {
                        for event in events {
                            if let Some(response) = session.screen.apply_event(event) {
                                let _ = supervisor.write_stdin(process_id, &response);
                            }
                        }
                    }
                    ProcessEventKind::Exited { code } => {
                        outcome = Some(TerminalProcessState::Exited(code));
                    }
                    ProcessEventKind::Signaled { signal } => {
                        outcome = Some(TerminalProcessState::Signaled(sanitize_metadata(&signal)));
                    }
                    ProcessEventKind::Crashed { error } => {
                        outcome = Some(TerminalProcessState::Crashed(sanitize_metadata(&error)));
                    }
                    ProcessEventKind::Stdout(_)
                    | ProcessEventKind::Stderr(_)
                    | ProcessEventKind::Restarting { .. } => {}
                }
            }
            let _ = session.screen.synchronized_watchdog_expired(Instant::now());
            if let Some(outcome) = outcome {
                finish_session(session, outcome);
            }
        }

        let closing: Vec<ProcessId> = self.closing.iter().copied().collect();
        for process_id in closing {
            // Continue owning and discarding every final batch until reaped.
            let _ = supervisor.take_events(process_id);
            if matches!(
                supervisor.state(process_id),
                Some(ProcessState::Terminated(_)) | None
            ) {
                let _ = supervisor.forget(process_id);
                self.closing.remove(&process_id);
            }
        }
    }

    /// Queue raw terminal input for a running child.
    pub fn send(
        &self,
        buffer_id: BufferId,
        bytes: &[u8],
        supervisor: &mut ProcessSupervisor,
    ) -> Result<(), TerminalError> {
        let session = self
            .sessions
            .get(&buffer_id)
            .ok_or(TerminalError::NotTerminal(buffer_id))?;
        if let Some(tap) = self.send_tap.borrow_mut().as_mut() {
            tap.push((buffer_id, bytes.to_vec()));
        }
        supervisor
            .write_stdin(session.process_id, bytes)
            .map_err(TerminalError::Process)
    }

    /// Begin recording child input for G5k's witnesses.
    #[doc(hidden)]
    pub fn start_send_tap_for_test(&self) {
        *self.send_tap.borrow_mut() = Some(Vec::new());
    }

    /// Take everything sent to children since the tap was started.
    ///
    /// Returns the sends in ORDER, because "one release, not two" and
    /// "the old gesture's release before the new gesture's press" are
    /// both ordering claims that a set cannot express.
    #[doc(hidden)]
    #[must_use]
    pub fn take_send_tap_for_test(&self) -> ChildSendLog {
        self.send_tap
            .borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Resolve this terminal's effective escape chord, parsing at most
    /// once per `(terminal, config epoch)` (Q#TC4c).
    ///
    /// `spelling` is the caller-resolved `terminal.escape-key` value and
    /// `epoch` the registry's `value_epoch()` it was read at. Returns the
    /// effective chord plus, at most once per terminal per effective
    /// invalid value, a message the caller should surface.
    ///
    /// An unparseable spelling falls back to `C-c` rather than leaving the
    /// terminal with no escape at all (Q#TC4a): without one, every key goes
    /// to the child and the user cannot reach the binding that would fix
    /// the setting that broke it.
    pub fn escape_chord(
        &mut self,
        buffer_id: BufferId,
        epoch: u64,
        spelling: &str,
    ) -> (Chord, Option<String>) {
        let fallback = default_escape_chord();
        if let Some(session) = self.sessions.get(&buffer_id)
            && let Some(cache) = session.escape.as_ref()
            && cache.epoch == epoch
        {
            return (cache.chord, None);
        }
        self.escape_parses = self.escape_parses.saturating_add(1);
        let Some(session) = self.sessions.get_mut(&buffer_id) else {
            return (fallback, None);
        };
        let previously_reported = session
            .escape
            .as_ref()
            .and_then(|cache| cache.reported_invalid.clone());
        let (chord, reported_invalid, report) = match parse_chord(spelling) {
            Ok(chord) => (chord, None, None),
            Err(error) => {
                let already = previously_reported.as_deref() == Some(spelling);
                let message = (!already).then(|| {
                    format!(
                        "terminal.escape-key {spelling:?} is not a valid chord ({error}); using C-c"
                    )
                });
                (fallback, Some(spelling.to_owned()), message)
            }
        };
        session.escape = Some(EscapeCache {
            epoch,
            chord,
            reported_invalid,
        });
        (chord, report)
    }

    /// How many escape-key spellings this manager has parsed.
    ///
    /// An observability seam for Q#TC4c's cache contract, which is
    /// otherwise unpinnable for a VALID setting: a correct per-session
    /// cache and a single last-entry cache produce identical behavior
    /// there and differ only in how often they parse. Counting reports
    /// covers the invalid case; this covers the valid one.
    #[must_use]
    pub fn escape_parses(&self) -> u64 {
        self.escape_parses
    }

    /// How many terminals currently hold a cached escape chord.
    ///
    /// The LIFETIME half of Q#TC4c's cache contract, which `escape_parses`
    /// cannot cover: parse counting says a valid setting is read once, but
    /// says nothing about whether the cache is ever released. Because the
    /// cache lives on [`TerminalSession`], this count falls with the
    /// session set by construction — which is exactly the property worth
    /// pinning, since the rejected alternative (an editor-side
    /// `HashMap<BufferId, EscapeCache>`) has no purge hook and would hold
    /// this at its high-water mark while sessions drained.
    #[must_use]
    pub fn escape_caches(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.escape.is_some())
            .count()
    }

    /// Resize a terminal screen and its PTY after validating shared limits.
    pub fn resize(
        &mut self,
        buffer_id: BufferId,
        rows: u16,
        cols: u16,
        supervisor: &mut ProcessSupervisor,
    ) -> Result<(), TerminalError> {
        validate_size(rows, cols)?;
        let session = self
            .sessions
            .get_mut(&buffer_id)
            .ok_or(TerminalError::NotTerminal(buffer_id))?;
        if matches!(session.process, TerminalProcessState::Running) {
            supervisor
                .resize_pty(session.process_id, rows, cols)
                .map_err(TerminalError::Process)?;
        }
        session
            .screen
            .resize(CellSize::new(u32::from(rows), u32::from(cols)))
            .map_err(|error| TerminalError::Screen(error.to_string()))
    }

    /// Request SIGTERM. Snapshot state stays `Running` until the outcome event.
    pub fn terminate(
        &mut self,
        buffer_id: BufferId,
        supervisor: &mut ProcessSupervisor,
    ) -> Result<(), TerminalError> {
        let session = self
            .sessions
            .get(&buffer_id)
            .ok_or(TerminalError::NotTerminal(buffer_id))?;
        if matches!(session.process, TerminalProcessState::Running) {
            supervisor
                .terminate(session.process_id)
                .map_err(TerminalError::Process)?;
        }
        Ok(())
    }

    /// Tear down sessions whose identity buffers were removed by any path.
    pub fn prune(&mut self, core: &mut EditorCore, supervisor: &mut ProcessSupervisor) {
        let removed: Vec<BufferId> = {
            let registry = core.registry.borrow();
            self.sessions
                .keys()
                .copied()
                .filter(|buffer_id| !registry.contains(*buffer_id))
                .collect()
        };
        for buffer_id in removed {
            core.set_round_trip_input(buffer_id, false);
            let Some(session) = self.sessions.remove(&buffer_id) else {
                continue;
            };
            self.process_to_buffer.remove(&session.process_id);
            self.views.retain(|key, _| key.buffer_id != buffer_id);
            self.controllers.remove(&buffer_id);
            match supervisor.state(session.process_id) {
                Some(
                    ProcessState::Starting
                    | ProcessState::Running { .. }
                    | ProcessState::Exiting { .. },
                ) => {
                    let _ = supervisor.terminate(session.process_id);
                    self.closing.insert(session.process_id);
                }
                Some(ProcessState::Terminated(_)) => {
                    let _ = supervisor.take_events(session.process_id);
                    let _ = supervisor.forget(session.process_id);
                }
                None => {}
            }
        }
    }

    /// Terminate every terminal child and unpublish all sessions.
    ///
    /// The editor follows this with the supervisor's bounded global shutdown,
    /// which performs final TERM/KILL escalation for terminal and non-terminal
    /// processes alike.
    pub fn shutdown(&mut self, supervisor: &mut ProcessSupervisor) {
        let process_ids: Vec<ProcessId> = self.process_to_buffer.keys().copied().collect();
        for process_id in process_ids {
            if matches!(
                supervisor.state(process_id),
                Some(
                    ProcessState::Running { .. }
                        | ProcessState::Exiting { .. }
                        | ProcessState::Starting
                )
            ) {
                let _ = supervisor.terminate(process_id);
            }
            self.closing.insert(process_id);
        }
        self.sessions.clear();
        self.process_to_buffer.clear();
        self.views.clear();
        self.controllers.clear();
    }
}

fn unique_terminal_name(core: &EditorCore, base: &str) -> String {
    let registry = core.registry.borrow();
    if registry.find_by_name(base).is_none() {
        return base.to_owned();
    }
    for suffix in 2usize.. {
        let candidate = format!("{base}<{suffix}>");
        if registry.find_by_name(&candidate).is_none() {
            return candidate;
        }
    }
    unreachable!("unbounded terminal suffix search must find a free name")
}

fn finish_session(session: &mut TerminalSession, outcome: TerminalProcessState) {
    if session.annotated {
        session.process = outcome;
        return;
    }
    session.screen.finish_output();
    let annotation = match &outcome {
        TerminalProcessState::Running => return,
        TerminalProcessState::Exited(0) => {
            format!("Process {} exited normally with code 0", session.pid)
        }
        TerminalProcessState::Exited(code) => {
            format!("Process {} exited abnormally with code {code}", session.pid)
        }
        TerminalProcessState::Signaled(signal) => format!(
            "Process {} exited abnormally with signal {signal}",
            session.pid
        ),
        TerminalProcessState::Crashed(error) => {
            format!("Process {} crashed: {error}", session.pid)
        }
    };
    session.screen.append_process_annotation(&annotation);
    session.annotated = true;
    // Publish exit metadata only after final bytes and annotation are applied.
    session.process = outcome;
}

fn validate_size(rows: u16, cols: u16) -> Result<(), TerminalError> {
    let cells = usize::from(rows) * usize::from(cols);
    if rows == 0 || rows > MAX_TERMINAL_ROWS {
        return Err(TerminalError::InvalidSpec(format!(
            "rows must be in 1..={MAX_TERMINAL_ROWS}; got {rows}"
        )));
    }
    if cols == 0 || cols > MAX_TERMINAL_COLS {
        return Err(TerminalError::InvalidSpec(format!(
            "cols must be in 1..={MAX_TERMINAL_COLS}; got {cols}"
        )));
    }
    if cells > MAX_TERMINAL_VISIBLE_CELLS {
        return Err(TerminalError::InvalidSpec(format!(
            "visible cell count {cells} exceeds {MAX_TERMINAL_VISIBLE_CELLS}"
        )));
    }
    Ok(())
}

fn reject_nul(field: &str, bytes: &[u8]) -> Result<(), TerminalError> {
    if bytes.contains(&0) {
        Err(TerminalError::InvalidSpec(format!(
            "{field} must not contain NUL"
        )))
    } else {
        Ok(())
    }
}

fn sanitize_metadata(value: &str) -> String {
    let mut clean = String::with_capacity(value.len().min(MAX_TERMINAL_METADATA_BYTES));
    for ch in value.chars() {
        let ch = if ch == '\r' || ch == '\n' || ch.is_control() {
            ' '
        } else {
            ch
        };
        if clean.len() + ch.len_utf8() > MAX_TERMINAL_METADATA_BYTES {
            break;
        }
        clean.push(ch);
    }
    clean
}

/// The built-in terminal escape chord, and the fallback for an
/// unparseable `terminal.escape-key` (Q#TC4a).
pub(super) fn default_escape_chord() -> Chord {
    Chord::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    )
}
