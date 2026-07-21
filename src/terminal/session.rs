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
use crate::process::{
    ProcessEventKind, ProcessId, ProcessMode, ProcessSpec, ProcessState, ProcessSupervisor,
    RestartPolicy, StdinMode, TerminalMode,
};
use crate::terminal::screen::TerminalScreen;
use crate::terminal::{
    MAX_TERMINAL_COLS, MAX_TERMINAL_HISTORY_CELLS, MAX_TERMINAL_METADATA_BYTES, MAX_TERMINAL_ROWS,
    MAX_TERMINAL_VISIBLE_CELLS,
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
    /// Environment overrides inherited by the child.
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

/// Process outcome published with an owned terminal snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalProcessState {
    /// Child is running or termination has only been requested.
    Running,
    /// Child exited with a status code.
    Exited(i32),
    /// Child was terminated by a sanitized symbolic signal.
    Signaled(String),
    /// Supervision failed after the session was published.
    Crashed(String),
}

/// One selected terminal-row span. Stage 1 snapshots leave selection empty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSelectionSpan {
    /// Visible row.
    pub row: u32,
    /// Inclusive starting column.
    pub start_col: u32,
    /// Exclusive ending column.
    pub end_col: u32,
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

struct TerminalSession {
    process_id: ProcessId,
    pid: u32,
    screen: TerminalScreen,
    process: TerminalProcessState,
    annotated: bool,
}

/// Owns the one-buffer/one-process/one-screen terminal registry.
#[derive(Default)]
pub struct TerminalManager {
    sessions: HashMap<BufferId, TerminalSession>,
    process_to_buffer: HashMap<ProcessId, BufferId>,
    /// Removed buffers whose children are still being reaped. Their events
    /// remain manager-owned so Lua/LSP/MCP consumers cannot steal a batch.
    closing: HashSet<ProcessId>,
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

        let buffer_name = spec.buffer_name();
        let buffer_id = BufferId::next();
        let mut buffer = Buffer::new(buffer_id, buffer_name.clone());
        buffer.set_read_only(true);
        core.registry.borrow_mut().insert(buffer);

        let mut process_spec = ProcessSpec::new(buffer_name, spec.command);
        process_spec.args = spec.args;
        process_spec.cwd = spec.cwd;
        process_spec.env = spec.env;
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
        supervisor
            .write_stdin(session.process_id, bytes)
            .map_err(TerminalError::Process)
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
    pub fn prune(&mut self, core: &EditorCore, supervisor: &mut ProcessSupervisor) {
        let removed: Vec<BufferId> = {
            let registry = core.registry.borrow();
            self.sessions
                .keys()
                .copied()
                .filter(|buffer_id| !registry.contains(*buffer_id))
                .collect()
        };
        for buffer_id in removed {
            let Some(session) = self.sessions.remove(&buffer_id) else {
                continue;
            };
            self.process_to_buffer.remove(&session.process_id);
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
    }
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
