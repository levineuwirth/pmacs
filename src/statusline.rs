//! Lua statusline provider registry and borrow-released evaluator.
//!
//! The registry is editor-global, while evaluation is scoped to one frontend's
//! visible windows (grid) or one declared active buffer (semantic).  Callers
//! must interpret [`StatuslineEvaluationOutcome::Invalidated`] as an
//! authoritative empty replacement and [`StatuslineEvaluationOutcome::NoMessage`]
//! as no publication at all.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use mlua::{Function, Lua, Value};
use pmacs_protocol::{
    FrontendId, MAX_STATUSLINE_FACE_BYTES, MAX_STATUSLINE_PROVIDER_NAME_BYTES,
    MAX_STATUSLINE_PROVIDERS, MAX_STATUSLINE_SEGMENT_BYTES, is_modeline_face_name,
};

use crate::buffer::BufferId;
use crate::command::SourceLocation;
use crate::editor_core::EditorCore;
use crate::lua_bindings::BufferIdLua;
use crate::window::WindowId;

/// Cheaply cloneable, single-threaded statusline registry handle.
pub type SharedStatuslineRegistry = Rc<RefCell<StatuslineRegistry>>;

/// Stable, monotonic provider identity. IDs are never reused.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StatuslineProviderId(u64);

impl StatuslineProviderId {
    /// Numeric representation for diagnostics and Lua introspection.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for StatuslineProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "statusline-provider:{}", self.0)
    }
}

/// Side of the modeline populated by a provider.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StatuslineSide {
    /// After the protected buffer-identity group.
    Left,
    /// Before the protected diagnostic/cursor/scroll group.
    Right,
}

impl StatuslineSide {
    /// Lua/API spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Fresh metadata returned by registry introspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatuslineProviderMetadata {
    /// Stable registration handle.
    pub id: StatuslineProviderId,
    /// Non-unique display/debug label.
    pub name: String,
    /// Compositor side.
    pub side: StatuslineSide,
    /// Survival priority; visual ordering is side-dependent.
    pub priority: i32,
    /// Static `ui.modeline` face name.
    pub face: String,
    /// Whether evaluation currently invokes the callback.
    pub enabled: bool,
    /// Registration callsite used for error attribution.
    pub source: SourceLocation,
}

#[derive(Clone)]
struct StatuslineProviderDefinition {
    metadata: StatuslineProviderMetadata,
    callback: Function,
}

/// Registration validation/capacity failure. Failed registration never mutates
/// providers, IDs, or epochs.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StatuslineRegistryError {
    #[error("statusline provider name must not be empty")]
    /// The display label was empty.
    EmptyName,
    #[error("statusline provider name contains a control character")]
    /// The display label contained a control scalar.
    NameControl,
    #[error("statusline provider name exceeds {MAX_STATUSLINE_PROVIDER_NAME_BYTES} bytes")]
    /// The display label exceeded the shared byte limit.
    NameTooLong,
    #[error("statusline face must be ui.modeline or a ui.modeline.* child")]
    /// The face was outside the reserved modeline namespace.
    InvalidFace,
    #[error("statusline face contains a control character")]
    /// The face contained a control scalar.
    FaceControl,
    #[error("statusline face exceeds {MAX_STATUSLINE_FACE_BYTES} bytes")]
    /// The face exceeded the shared byte limit.
    FaceTooLong,
    #[error("at most {MAX_STATUSLINE_PROVIDERS} statusline providers may be registered")]
    /// The live-provider structural limit was reached.
    ProviderLimit,
}

/// Full identity of one callback invocation and its failure latch.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct StatuslineContext {
    /// Frontend being rendered.
    pub frontend_id: FrontendId,
    /// Visible window being rendered.
    pub window_id: WindowId,
    /// Buffer displayed by that window.
    pub buffer_id: BufferId,
    /// Whether this is the frontend's focused window.
    pub active: bool,
}

/// One successful, non-empty provider result. Vectors containing these are
/// already in the exact Q#SL4 display/survival order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluatedStatuslineSegment {
    /// Registration that produced the segment.
    pub provider_id: StatuslineProviderId,
    /// Sanitized, non-empty one-line UTF-8 text.
    pub text: String,
    /// Static face copied from the registration.
    pub face: String,
}

/// Complete custom output for one window context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatuslineWindowSegments {
    /// Context to which both vectors belong.
    pub context: StatuslineContext,
    /// Left segments in priority-descending/id-ascending order.
    pub left: Vec<EvaluatedStatuslineSegment>,
    /// Right segments in priority-ascending/id-ascending order.
    pub right: Vec<EvaluatedStatuslineSegment>,
}

/// Why phase 1 intentionally produced no message.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StatuslineNoMessageReason {
    /// The target frontend, view, or one of its layout windows no longer exists.
    ContextUnavailable,
    /// A layout window points at a buffer that has been removed.
    BufferUnavailable,
    /// The semantic frontend's daemon window does not match its declared
    /// viewport. `BufferSnapshot` already cleared the remote mirror.
    DeclaredBufferMismatch,
    /// A core borrow was unexpectedly held before evaluation could begin.
    CoreBorrowConflict,
}

/// Publication decision from one three-phase evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StatuslineEvaluationOutcome {
    /// Valid owned results. Empty vectors are authoritative truth.
    Ready(Vec<StatuslineWindowSegments>),
    /// A callback changed registry/layout context after phase 1. Callers MUST
    /// publish empty left/right vectors for these old contexts (subject to
    /// their normal payload baseline) and MUST discard all evaluated text.
    Invalidated {
        /// Phase-1 contexts whose previous published output must be cleared.
        authoritative_empty: Vec<StatuslineContext>,
    },
    /// Phase 1 was already stale. Callers MUST NOT publish a segment message.
    NoMessage(StatuslineNoMessageReason),
}

/// A newly armed provider failure. The evaluator also appends it to the
/// canonical `*errors*` buffer; returning it keeps tests/telemetry explicit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatuslineProviderFailure {
    /// Registration that failed.
    pub provider_id: StatuslineProviderId,
    /// Display label captured from the registration.
    pub provider_name: String,
    /// Registration callsite.
    pub source: SourceLocation,
    /// Exact per-window latch key.
    pub context: StatuslineContext,
    /// Callback/return-validation error text.
    pub message: String,
}

/// Evaluator result including only first-in-consecutive-run failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatuslineEvaluation {
    /// Publication decision.
    pub outcome: StatuslineEvaluationOutcome,
    /// Newly armed failures already appended to `*errors*`.
    pub new_failures: Vec<StatuslineProviderFailure>,
}

/// Target shape for the shared evaluator.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StatuslineEvaluationTarget {
    /// Every visible leaf in the frontend's layout, in layout order.
    Grid {
        /// Frontend whose entire visible layout is evaluated.
        frontend_id: FrontendId,
    },
    /// Only the frontend's active window, iff it still displays the declared
    /// semantic viewport buffer.
    Semantic {
        /// Frontend whose focused daemon window is evaluated.
        frontend_id: FrontendId,
        /// Buffer declared by the semantic viewport.
        declared_buffer: BufferId,
    },
}

/// Registry state. Provider callbacks are never invoked while this is borrowed.
pub struct StatuslineRegistry {
    next_id: u64,
    layout_epoch: u64,
    face_set_epoch: u64,
    providers: BTreeMap<StatuslineProviderId, StatuslineProviderDefinition>,
    enabled_face_counts: HashMap<String, usize>,
    failure_latches: HashSet<(StatuslineProviderId, StatuslineContext)>,
}

impl Default for StatuslineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StatuslineRegistry {
    /// Construct an empty registry with epochs at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            layout_epoch: 0,
            face_set_epoch: 0,
            providers: BTreeMap::new(),
            enabled_face_counts: HashMap::new(),
            failure_latches: HashSet::new(),
        }
    }

    /// Current monotonic ordering/context guard epoch.
    #[must_use]
    pub fn layout_epoch(&self) -> u64 {
        self.layout_epoch
    }

    /// Current monotonic enabled-face-inventory epoch.
    #[must_use]
    pub fn face_set_epoch(&self) -> u64 {
        self.face_set_epoch
    }

    /// Number of live registrations, including disabled ones.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether there are no live registrations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Fresh metadata in stable registration order.
    #[must_use]
    pub fn providers(&self) -> Vec<StatuslineProviderMetadata> {
        self.providers
            .values()
            .map(|definition| definition.metadata.clone())
            .collect()
    }

    /// Sorted/deduplicated enabled face inventory for `ThemeFacts` expansion.
    #[must_use]
    pub fn enabled_face_names(&self) -> Vec<String> {
        let mut faces: Vec<_> = self.enabled_face_counts.keys().cloned().collect();
        faces.sort_unstable();
        faces
    }

    /// Register one enabled provider after validating the shared structural
    /// limits. Lua-specific strict type/table validation happens in the binding.
    pub fn register(
        &mut self,
        name: String,
        side: StatuslineSide,
        priority: i32,
        face: String,
        callback: Function,
        source: SourceLocation,
    ) -> Result<StatuslineProviderId, StatuslineRegistryError> {
        validate_name(&name)?;
        validate_face(&face)?;
        if self.providers.len() >= MAX_STATUSLINE_PROVIDERS {
            return Err(StatuslineRegistryError::ProviderLimit);
        }

        let id = StatuslineProviderId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("statusline provider id exhausted");
        let face_set_changed = self.add_enabled_face(&face);
        let metadata = StatuslineProviderMetadata {
            id,
            name,
            side,
            priority,
            face,
            enabled: true,
            source,
        };
        self.providers
            .insert(id, StatuslineProviderDefinition { metadata, callback });
        self.bump_layout_epoch();
        if face_set_changed {
            self.bump_face_set_epoch();
        }
        Ok(id)
    }

    /// Idempotently remove a provider and all its failure latches.
    pub fn unregister(&mut self, id: StatuslineProviderId) -> bool {
        let Some(definition) = self.providers.remove(&id) else {
            return false;
        };
        self.failure_latches.retain(|(provider, _)| *provider != id);
        self.bump_layout_epoch();
        if definition.metadata.enabled && self.remove_enabled_face(&definition.metadata.face) {
            self.bump_face_set_epoch();
        }
        true
    }

    /// Change survival priority. No-op and stale writes do not advance epochs.
    pub fn set_priority(&mut self, id: StatuslineProviderId, priority: i32) -> bool {
        let Some(definition) = self.providers.get_mut(&id) else {
            return false;
        };
        if definition.metadata.priority != priority {
            definition.metadata.priority = priority;
            self.bump_layout_epoch();
        }
        true
    }

    /// Enable/disable a provider. Disabling clears all its failure latches.
    pub fn set_enabled(&mut self, id: StatuslineProviderId, enabled: bool) -> bool {
        let Some(definition) = self.providers.get_mut(&id) else {
            return false;
        };
        if definition.metadata.enabled == enabled {
            return true;
        }
        definition.metadata.enabled = enabled;
        let face = definition.metadata.face.clone();
        if !enabled {
            self.failure_latches.retain(|(provider, _)| *provider != id);
        }
        self.bump_layout_epoch();
        let face_set_changed = if enabled {
            self.add_enabled_face(&face)
        } else {
            self.remove_enabled_face(&face)
        };
        if face_set_changed {
            self.bump_face_set_epoch();
        }
        true
    }

    /// Drop every failure latch for a detached frontend.
    pub fn detach_frontend(&mut self, frontend_id: FrontendId) {
        self.failure_latches
            .retain(|(_, context)| context.frontend_id != frontend_id);
    }

    /// Defense-in-depth stale-window/buffer/focus latch sweep.
    pub fn retain_live_contexts(&mut self, live: &HashSet<StatuslineContext>) {
        self.failure_latches
            .retain(|(_, context)| live.contains(context));
    }

    fn snapshot_enabled(&self) -> (u64, Vec<StatuslineProviderDefinition>) {
        let mut definitions: Vec<_> = self
            .providers
            .values()
            .filter(|definition| definition.metadata.enabled)
            .cloned()
            .collect();
        definitions.sort_by(|a, b| match (a.metadata.side, b.metadata.side) {
            (StatuslineSide::Left, StatuslineSide::Right) => std::cmp::Ordering::Less,
            (StatuslineSide::Right, StatuslineSide::Left) => std::cmp::Ordering::Greater,
            (StatuslineSide::Left, StatuslineSide::Left) => b
                .metadata
                .priority
                .cmp(&a.metadata.priority)
                .then_with(|| a.metadata.id.cmp(&b.metadata.id)),
            (StatuslineSide::Right, StatuslineSide::Right) => a
                .metadata
                .priority
                .cmp(&b.metadata.priority)
                .then_with(|| a.metadata.id.cmp(&b.metadata.id)),
        });
        (self.layout_epoch, definitions)
    }

    fn note_success(&mut self, id: StatuslineProviderId, context: StatuslineContext) {
        self.failure_latches.remove(&(id, context));
    }

    fn note_failure(&mut self, id: StatuslineProviderId, context: StatuslineContext) -> bool {
        if self.failure_latches.contains(&(id, context)) {
            return false;
        }
        if self
            .providers
            .get(&id)
            .is_some_and(|definition| definition.metadata.enabled)
        {
            self.failure_latches.insert((id, context));
        }
        true
    }

    fn add_enabled_face(&mut self, face: &str) -> bool {
        let count = self.enabled_face_counts.entry(face.to_owned()).or_default();
        let changed = *count == 0;
        *count += 1;
        changed
    }

    fn remove_enabled_face(&mut self, face: &str) -> bool {
        let Some(count) = self.enabled_face_counts.get_mut(face) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            self.enabled_face_counts.remove(face);
            true
        } else {
            false
        }
    }

    fn bump_layout_epoch(&mut self) {
        self.layout_epoch = self
            .layout_epoch
            .checked_add(1)
            .expect("statusline layout epoch exhausted");
    }

    fn bump_face_set_epoch(&mut self) {
        self.face_set_epoch = self
            .face_set_epoch
            .checked_add(1)
            .expect("statusline face-set epoch exhausted");
    }
}

fn validate_name(name: &str) -> Result<(), StatuslineRegistryError> {
    if name.is_empty() {
        return Err(StatuslineRegistryError::EmptyName);
    }
    if name.len() > MAX_STATUSLINE_PROVIDER_NAME_BYTES {
        return Err(StatuslineRegistryError::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(StatuslineRegistryError::NameControl);
    }
    Ok(())
}

fn validate_face(face: &str) -> Result<(), StatuslineRegistryError> {
    if face.len() > MAX_STATUSLINE_FACE_BYTES {
        return Err(StatuslineRegistryError::FaceTooLong);
    }
    if face.chars().any(char::is_control) {
        return Err(StatuslineRegistryError::FaceControl);
    }
    if !is_modeline_face_name(face) {
        return Err(StatuslineRegistryError::InvalidFace);
    }
    Ok(())
}

/// Evaluate one target without holding a core or registry borrow across Lua.
///
/// Three phases are enforced internally: capture contexts, snapshot definitions
/// and invoke them, then compare epoch and contexts before publication.
pub fn evaluate_statusline(
    lua: &Lua,
    core: &Rc<RefCell<EditorCore>>,
    registry: &SharedStatuslineRegistry,
    target: StatuslineEvaluationTarget,
) -> StatuslineEvaluation {
    let initial_contexts = match capture_target_contexts(core, target) {
        Ok(contexts) => contexts,
        Err(reason) => {
            return StatuslineEvaluation {
                outcome: StatuslineEvaluationOutcome::NoMessage(reason),
                new_failures: Vec::new(),
            };
        }
    };

    let (layout_epoch, definitions) = registry.borrow().snapshot_enabled();
    let mut windows = initial_contexts
        .iter()
        .copied()
        .map(|context| StatuslineWindowSegments {
            context,
            left: Vec::new(),
            right: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut new_failures = Vec::new();

    for window in &mut windows {
        for definition in &definitions {
            match call_provider(lua, definition, window.context) {
                Ok(Some(text)) => {
                    registry
                        .borrow_mut()
                        .note_success(definition.metadata.id, window.context);
                    let segment = EvaluatedStatuslineSegment {
                        provider_id: definition.metadata.id,
                        text,
                        face: definition.metadata.face.clone(),
                    };
                    match definition.metadata.side {
                        StatuslineSide::Left => window.left.push(segment),
                        StatuslineSide::Right => window.right.push(segment),
                    }
                }
                Ok(None) => registry
                    .borrow_mut()
                    .note_success(definition.metadata.id, window.context),
                Err(message) => {
                    if registry
                        .borrow_mut()
                        .note_failure(definition.metadata.id, window.context)
                    {
                        let failure = StatuslineProviderFailure {
                            provider_id: definition.metadata.id,
                            provider_name: definition.metadata.name.clone(),
                            source: definition.metadata.source.clone(),
                            context: window.context,
                            message,
                        };
                        crate::lua_bindings::log_statusline_provider_error(lua, &failure);
                        new_failures.push(failure);
                    }
                }
            }
        }
    }

    let final_contexts = capture_target_contexts(core, target).ok();
    let live_contexts = capture_all_live_contexts(core);
    let final_epoch = registry.borrow().layout_epoch();
    if let Some(live) = live_contexts {
        registry.borrow_mut().retain_live_contexts(&live);
    }

    let outcome = if final_epoch == layout_epoch
        && final_contexts.as_deref() == Some(initial_contexts.as_slice())
    {
        StatuslineEvaluationOutcome::Ready(windows)
    } else {
        StatuslineEvaluationOutcome::Invalidated {
            authoritative_empty: initial_contexts,
        }
    };
    StatuslineEvaluation {
        outcome,
        new_failures,
    }
}

fn capture_target_contexts(
    core: &Rc<RefCell<EditorCore>>,
    target: StatuslineEvaluationTarget,
) -> Result<Vec<StatuslineContext>, StatuslineNoMessageReason> {
    let core = core
        .try_borrow()
        .map_err(|_| StatuslineNoMessageReason::CoreBorrowConflict)?;
    let buffers = core
        .registry
        .try_borrow()
        .map_err(|_| StatuslineNoMessageReason::CoreBorrowConflict)?;
    match target {
        StatuslineEvaluationTarget::Grid { frontend_id } => {
            let view = core
                .views
                .get(&frontend_id)
                .ok_or(StatuslineNoMessageReason::ContextUnavailable)?;
            let mut contexts = Vec::new();
            for window_id in view.layout.iter_ids() {
                let window = core
                    .windows
                    .get(&window_id)
                    .ok_or(StatuslineNoMessageReason::ContextUnavailable)?;
                if buffers.get(window.buffer_id).is_err() {
                    return Err(StatuslineNoMessageReason::BufferUnavailable);
                }
                contexts.push(StatuslineContext {
                    frontend_id,
                    window_id,
                    buffer_id: window.buffer_id,
                    active: window_id == view.active,
                });
            }
            Ok(contexts)
        }
        StatuslineEvaluationTarget::Semantic {
            frontend_id,
            declared_buffer,
        } => {
            let view = core
                .views
                .get(&frontend_id)
                .ok_or(StatuslineNoMessageReason::ContextUnavailable)?;
            let window = core
                .windows
                .get(&view.active)
                .ok_or(StatuslineNoMessageReason::ContextUnavailable)?;
            if buffers.get(window.buffer_id).is_err() {
                return Err(StatuslineNoMessageReason::BufferUnavailable);
            }
            if window.buffer_id != declared_buffer {
                return Err(StatuslineNoMessageReason::DeclaredBufferMismatch);
            }
            Ok(vec![StatuslineContext {
                frontend_id,
                window_id: window.id,
                buffer_id: window.buffer_id,
                active: true,
            }])
        }
    }
}

fn capture_all_live_contexts(core: &Rc<RefCell<EditorCore>>) -> Option<HashSet<StatuslineContext>> {
    let core = core.try_borrow().ok()?;
    let buffers = core.registry.try_borrow().ok()?;
    let mut live = HashSet::new();
    for (&frontend_id, view) in &core.views {
        for window_id in view.layout.iter_ids() {
            if let Some(window) = core.windows.get(&window_id)
                && buffers.get(window.buffer_id).is_ok()
            {
                live.insert(StatuslineContext {
                    frontend_id,
                    window_id,
                    buffer_id: window.buffer_id,
                    active: window_id == view.active,
                });
            }
        }
    }
    Some(live)
}

fn call_provider(
    lua: &Lua,
    definition: &StatuslineProviderDefinition,
    context: StatuslineContext,
) -> Result<Option<String>, String> {
    let table = lua.create_table().map_err(|error| error.to_string())?;
    table
        .raw_set("frontend", context.frontend_id.0)
        .map_err(|error| error.to_string())?;
    table
        .raw_set("window", context.window_id.raw())
        .map_err(|error| error.to_string())?;
    table
        .raw_set("buffer", BufferIdLua(context.buffer_id))
        .map_err(|error| error.to_string())?;
    table
        .raw_set("active", context.active)
        .map_err(|error| error.to_string())?;

    let value: Value = definition
        .callback
        .call(table)
        .map_err(|error| error.to_string())?;
    let bytes = match value {
        Value::Nil => return Ok(None),
        Value::String(value) => value
            .to_str()
            .map_err(|_| "callback returned a string that is not valid UTF-8".to_owned())?
            .as_bytes()
            .to_vec(),
        other => {
            return Err(format!(
                "callback must return a string or nil, got {}",
                other.type_name()
            ));
        }
    };
    let text = String::from_utf8(bytes).expect("mlua to_str validated UTF-8");
    let text = sanitize_provider_text(&text);
    if text.len() > MAX_STATUSLINE_SEGMENT_BYTES {
        return Err(format!(
            "callback result exceeds {MAX_STATUSLINE_SEGMENT_BYTES} bytes after sanitation"
        ));
    }
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

/// Apply the shared one-line provider-output policy.
#[must_use]
pub fn sanitize_provider_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' {
            break;
        }
        if ch.is_control() {
            output.push(' ');
        } else {
            output.push(ch);
        }
    }
    output
}

/// Flatten a provider failure into one durable `*errors*` entry while
/// retaining multi-line traceback content.
#[must_use]
pub(crate) fn sanitize_provider_error_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_registry::BufferRegistry;
    use crate::lua::LuaHost;
    use crate::lua_bindings::{SharedRegistry, statusline_registry};

    fn harness() -> (LuaHost, Rc<RefCell<EditorCore>>, SharedStatuslineRegistry) {
        let buffers: SharedRegistry = Rc::new(RefCell::new(BufferRegistry::new()));
        let core = Rc::new(RefCell::new(EditorCore::new(buffers.clone())));
        let host = LuaHost::with_registry(buffers).expect("Lua host");
        let statusline = statusline_registry(host.lua()).expect("statusline registry");
        (host, core, statusline)
    }

    #[test]
    fn provider_output_and_error_log_have_distinct_newline_policies() {
        let multiline = "boom\nstack\ttrace\r\0";
        assert_eq!(sanitize_provider_text(multiline), "boom");
        assert_eq!(
            sanitize_provider_error_text(multiline),
            "boom stack trace  "
        );

        let (host, core, registry) = harness();
        host.lua()
            .load(
                r"
                pmacs.statusline.register {
                  name='trace', side='left',
                  fn=function() error('boom\nstack detail') end,
                }
                ",
            )
            .exec()
            .unwrap();
        let evaluation = evaluate_statusline(
            host.lua(),
            &core,
            &registry,
            StatuslineEvaluationTarget::Grid {
                frontend_id: FrontendId::LOCAL,
            },
        );
        assert_eq!(evaluation.new_failures.len(), 1);
        let errors = host.errors_buffer_text();
        assert!(
            errors.contains("boom stack detail") && errors.contains("stack traceback"),
            "flattened provider traceback must retain every line: {errors:?}"
        );
        assert_eq!(
            errors.lines().count(),
            1,
            "one failure run must remain one durable error entry"
        );
    }

    #[test]
    fn registry_epochs_track_layout_and_distinct_enabled_faces() {
        let lua = Lua::new();
        let callback = lua.create_function(|_, ()| Ok(())).unwrap();
        let mut registry = StatuslineRegistry::new();
        let first = registry
            .register(
                "a".to_owned(),
                StatuslineSide::Left,
                0,
                "ui.modeline.same".to_owned(),
                callback.clone(),
                SourceLocation::default(),
            )
            .unwrap();
        let second = registry
            .register(
                "b".to_owned(),
                StatuslineSide::Right,
                0,
                "ui.modeline.same".to_owned(),
                callback,
                SourceLocation::default(),
            )
            .unwrap();
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), (2, 1));
        assert!(registry.set_priority(first, 0));
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), (2, 1));
        assert!(registry.set_priority(first, 1));
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), (3, 1));
        assert!(registry.set_enabled(first, false));
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), (4, 1));
        assert!(registry.set_enabled(second, false));
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), (5, 2));
        assert!(registry.unregister(first));
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), (6, 2));
        assert!(!registry.unregister(first));
    }

    #[test]
    fn lua_registration_is_strict_raw_and_lifecycle_is_introspectable() {
        let (host, _, registry) = harness();
        host.lua()
            .load(
                r#"
                local touched = false
                local mt = {
                  __index = function() touched = true; error("no __index") end,
                  __pairs = function() touched = true; error("no __pairs") end,
                }
                local spec = setmetatable({
                  name = "mine", side = "left", priority = 3,
                  face = "ui.modeline.mine", fn = function() return "ok" end,
                }, mt)
                H = pmacs.statusline.register(spec)
                P = pmacs.statusline.providers()
                assert(not touched)
                assert(#P == 1 and P[1].handle == H and P[1].name == "mine")
                assert(pmacs.statusline.set_priority(H, -4))
                assert(pmacs.statusline.set_enabled(H, false))
                assert(pmacs.statusline.unregister(H))
                assert(not pmacs.statusline.unregister(H))
                assert(not pmacs.statusline.set_enabled(H, true))
                "#,
            )
            .exec()
            .unwrap();
        assert!(registry.borrow().is_empty());
        for script in [
            "pmacs.statusline.register{name='x',side='left',fn=function()end,wat=1}",
            "pmacs.statusline.register{name='x',side='middle',fn=function()end}",
            "pmacs.statusline.register{name='x',side='left',priority=1.5,fn=function()end}",
            "pmacs.statusline.register{name='x',side='left',face='ui.statusline',fn=function()end}",
            "pmacs.statusline.register{name='x',side='left',fn=true}",
        ] {
            assert!(host.lua().load(script).exec().is_err(), "accepted {script}");
            assert!(registry.borrow().is_empty());
        }
    }
    #[test]
    fn lua_registration_enforces_shared_capacity_without_partial_mutation() {
        let (host, _, registry) = harness();
        host.lua()
            .load(
                r#"
                for i = 1, 64 do
                  pmacs.statusline.register {
                    name = "p" .. i,
                    side = "left",
                    fn = function() return nil end,
                  }
                end
                "#,
            )
            .exec()
            .unwrap();
        let epochs = {
            let registry = registry.borrow();
            (registry.layout_epoch(), registry.face_set_epoch())
        };
        let error = host
            .lua()
            .load("pmacs.statusline.register{name='overflow',side='right',fn=function()end}")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("64"), "{error}");
        let registry = registry.borrow();
        assert_eq!(registry.len(), MAX_STATUSLINE_PROVIDERS);
        assert_eq!((registry.layout_epoch(), registry.face_set_epoch()), epochs);
    }

    #[test]
    fn evaluation_orders_sanitizes_and_exposes_owned_context() {
        let (host, core, registry) = harness();
        host.lua()
            .load(
                r"
                A = pmacs.statusline.register{name='a',side='left',priority=0,fn=function(ctx)
                  assert(type(ctx.frontend)=='number' and type(ctx.window)=='number')
                  assert(ctx.buffer and ctx.active == true)
                  return 'a\rX\nignored'
                end}
                B = pmacs.statusline.register{name='b',side='left',priority=9,fn=function() return 'b' end}
                C = pmacs.statusline.register{name='c',side='right',priority=9,fn=function() return 'c' end}
                D = pmacs.statusline.register{name='d',side='right',priority=-2,fn=function() return 'd' end}
                ",
            )
            .exec()
            .unwrap();
        let evaluation = evaluate_statusline(
            host.lua(),
            &core,
            &registry,
            StatuslineEvaluationTarget::Grid {
                frontend_id: FrontendId::LOCAL,
            },
        );
        let StatuslineEvaluationOutcome::Ready(windows) = evaluation.outcome else {
            panic!("expected ready outcome");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0]
                .left
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            ["b", "a X"]
        );
        assert_eq!(
            windows[0]
                .right
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            ["d", "c"]
        );
    }
    #[test]
    fn invalid_results_are_isolated_omitted_and_latched() {
        let (host, core, registry) = harness();
        host.lua()
            .load(
                r"
                pmacs.statusline.register{name='good-a',side='left',priority=2,fn=function() return 'a' end}
                pmacs.statusline.register{name='number',side='left',priority=1,fn=function() return 42 end}
                pmacs.statusline.register{name='good-b',side='left',priority=0,fn=function() return 'b' end}
                pmacs.statusline.register{name='bytes',side='right',fn=function() return string.char(255) end}
                pmacs.statusline.register{name='huge',side='right',fn=function() return string.rep('x', 1025) end}
                ",
            )
            .exec()
            .unwrap();
        let target = StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        };
        let first = evaluate_statusline(host.lua(), &core, &registry, target);
        assert_eq!(first.new_failures.len(), 3);
        let StatuslineEvaluationOutcome::Ready(windows) = first.outcome else {
            panic!("expected ready output despite isolated failures");
        };
        assert_eq!(
            windows[0]
                .left
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(windows[0].right.is_empty());
        let repeated = evaluate_statusline(host.lua(), &core, &registry, target);
        assert!(repeated.new_failures.is_empty());
    }

    #[test]
    fn reentrant_unregister_is_invalidated_but_initial_mismatch_is_no_message() {
        let (host, core, registry) = harness();
        host.lua()
            .load(
                r"
                H = pmacs.statusline.register{name='self',side='left',fn=function()
                  pmacs.statusline.unregister(H)
                  return 'stale'
                end}
                ",
            )
            .exec()
            .unwrap();
        let active_buffer = core.borrow().active_buffer_id();
        let evaluation = evaluate_statusline(
            host.lua(),
            &core,
            &registry,
            StatuslineEvaluationTarget::Semantic {
                frontend_id: FrontendId::LOCAL,
                declared_buffer: active_buffer,
            },
        );
        let StatuslineEvaluationOutcome::Invalidated {
            authoritative_empty,
        } = evaluation.outcome
        else {
            panic!("expected invalidated outcome");
        };
        assert_eq!(authoritative_empty.len(), 1);

        let other_buffer = core.borrow().registry.borrow_mut().create("other");
        let mismatch = evaluate_statusline(
            host.lua(),
            &core,
            &registry,
            StatuslineEvaluationTarget::Semantic {
                frontend_id: FrontendId::LOCAL,
                declared_buffer: other_buffer,
            },
        );
        assert_eq!(
            mismatch.outcome,
            StatuslineEvaluationOutcome::NoMessage(
                StatuslineNoMessageReason::DeclaredBufferMismatch
            )
        );
    }

    #[test]
    fn editor_bootstrap_registers_the_pure_lsp_provider() {
        let state = crate::editor::EditorState::new();
        let registry = statusline_registry(state.lua_host.lua()).unwrap();
        let providers = registry.borrow().providers();
        let lsp = providers
            .iter()
            .find(|provider| provider.name == "lsp")
            .expect("builtin LSP provider");
        assert_eq!(lsp.side, StatuslineSide::Right);
        assert_eq!(lsp.priority, 0);
        assert_eq!(lsp.face, "ui.modeline.lsp");
        assert!(lsp.enabled);
    }

    #[test]
    fn failure_latch_success_and_detach_each_rearm() {
        let (host, core, registry) = harness();
        host.lua()
            .load(
                r#"
                MODE = "fail"
                pmacs.statusline.register {
                  name = "bad", side = "left",
                  fn = function()
                    if MODE == "fail" then error("boom") end
                    return nil
                  end,
                }
                "#,
            )
            .exec()
            .unwrap();
        let target = StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        };
        let first = evaluate_statusline(host.lua(), &core, &registry, target);
        assert_eq!(first.new_failures.len(), 1);
        let second = evaluate_statusline(host.lua(), &core, &registry, target);
        assert!(second.new_failures.is_empty());

        host.lua().load("MODE = 'ok'").exec().unwrap();
        let success = evaluate_statusline(host.lua(), &core, &registry, target);
        assert!(success.new_failures.is_empty());
        host.lua().load("MODE = 'fail'").exec().unwrap();
        let rearmed = evaluate_statusline(host.lua(), &core, &registry, target);
        assert_eq!(rearmed.new_failures.len(), 1);

        registry.borrow_mut().detach_frontend(FrontendId::LOCAL);
        let detached = evaluate_statusline(host.lua(), &core, &registry, target);
        assert_eq!(detached.new_failures.len(), 1);
    }
}
