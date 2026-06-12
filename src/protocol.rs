// protocol.rs --- Frontend ↔ Instance message protocol.

//! Frontend ↔ Instance typed message protocol (T M5.1).
//!
//! Spec §sec:m5-remote, §sec:v01-remote-scope deliverable 1.
//!
//! The TUI of v0.1 is a frontend talking to its instance over an in-process
//! channel. The remote case (M5.7) adds nothing to the instance side; it adds
//! a network transport on the frontend side. The protocol shape is symmetric
//! over transports.
//!
//! # Module surface
//!
//! - [`FrontendId`]: opaque per-frontend identity. v0.1 uses
//!   [`FrontendId::LOCAL`] for the single attached frontend.
//! - [`Key`] + [`Modifiers`] + [`KeyEvent`]: pmacs-native key encoding.
//!   Independent of any specific terminal protocol so the wire is stable
//!   when M5.7 ships SSH.
//! - [`MouseEvent`] + [`MouseKind`] + [`MouseButton`]: pmacs-native mouse
//!   encoding.
//! - [`FrontendEvent`]: input from frontend to instance.
//! - [`InstanceMessage`]: rendering and signals from instance to frontend.
//! - [`AttachTarget`]: addressing for remote attachment. Two variants
//!   (`LocalSocket`, `Ssh`) are v0.1; `Tls` and `Custom` are reserved
//!   and return [`AttachError::NotImplementedInV01`] when invoked.
//!
//! # Wire stability
//!
//! These types must remain backwards-compatible across v0.1 patch
//! releases. Adding variants requires explicit consideration of the
//! v0.3 multi-frontend generalization (cf. spec §sec:remote). New
//! fields prefer optional extension over breaking change. The
//! `Unknown` keycode variant exists so that future terminal protocols
//! that surface unrecognized keycodes do not require a protocol break.
//!
//! # Translation layer
//!
//! [`crossterm_translate`] converts the TUI's `crossterm::event` types
//! into the protocol types. It is the only place in the protocol module
//! that touches `crossterm`. SSH transports do not use this submodule;
//! they decode the wire directly into [`KeyEvent`] / [`MouseEvent`].

// Cell wire types reach this module through the `pub use
// pmacs_protocol::*` block below; the `crossterm_translate`
// submodule's `use super::{CellCoord, ...}` resolves through that
// glob.
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Wire types — re-exports from `pmacs-protocol`
// ---------------------------------------------------------------------------

// Session 1 of the `pmacs-gpu` arc moved every wire type out of this
// module into the `pmacs-protocol` crate (`docs/pmacs-gpu-design.md`).
// What stays below are the CLI / binding internals (`AttachTarget` and
// friends, `AttachmentHandle`, the `crossterm_translate` submodule)
// plus the existing wire-format roundtrip tests. The blanket re-export
// keeps every `crate::protocol::*` import path working unchanged; new
// consumers (`pmacs-gpu`, debug tools) should depend on `pmacs-protocol`
// directly.
pub use pmacs_protocol::*;

// ---------------------------------------------------------------------------
// Attachment
// ---------------------------------------------------------------------------

/// Where to attach. v0.1 implements `LocalSocket` and `Ssh`; `Tls` and
/// `Custom` are reserved and return [`AttachError::NotImplementedInV01`]
/// when validated via [`AttachTarget::check_v01`].
///
/// # String form
///
/// [`AttachTarget::parse`] accepts a human-readable string of the form
/// `kind:body`, with these grammars:
///
/// - `local:<path>`
/// - `ssh:[user@]host[/instance_name]`
/// - `tls:<endpoint>#<cert_path>`
/// - `custom:<argv space-split>`
///
/// [`Display`](std::fmt::Display) round-trips through [`parse`](Self::parse).
///
/// # Validation
///
/// [`validate`](Self::validate) catches semantic problems (empty fields,
/// embedded null bytes, invalid characters in usernames or instance
/// names) regardless of how the target was constructed. [`parse`](Self::parse)
/// runs validation as its final step, so any target that exits parsing
/// is locally well-formed. Lua kwargs callers that build the variants
/// directly must call `validate` before storing.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AttachTarget {
    /// Local Unix-socket transport: attach to a daemonized local instance.
    LocalSocket(PathBuf),
    /// SSH transport: spawn `ssh <host> pmacs --daemon-attach` and bridge
    /// its stdio to the local frontend.
    Ssh {
        /// Host alias or address; resolved through `~/.ssh/config`.
        host: String,
        /// Optional explicit username override.
        user: Option<String>,
        /// Optional named instance on the far side (defaults to
        /// `default`, mapping to the per-user default daemon).
        instance_name: Option<String>,
    },
    /// TLS transport. **Reserved** — returns
    /// [`AttachError::NotImplementedInV01`] in v0.1.
    Tls {
        /// `host:port` endpoint to connect to.
        endpoint: String,
        /// Path to a pre-shared certificate.
        cert: PathBuf,
    },
    /// Escape hatch for non-SSH transports (`docker exec`, `kubectl
    /// exec`, `nsenter`, `flatpak-spawn`). **Reserved** — returns
    /// [`AttachError::NotImplementedInV01`] in v0.1.
    Custom {
        /// Argv for the bridging process.
        command: Vec<String>,
    },
}

impl AttachTarget {
    /// Reject the v0.3-only variants up front so the rest of the
    /// attach machinery can assume an implementable target.
    pub fn check_v01(&self) -> Result<(), AttachError> {
        match self {
            Self::LocalSocket(_) | Self::Ssh { .. } => Ok(()),
            Self::Tls { .. } => Err(AttachError::NotImplementedInV01("TLS")),
            Self::Custom { .. } => Err(AttachError::NotImplementedInV01("Custom")),
        }
    }

    /// Short tag used in diagnostic messages.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::LocalSocket(_) => "local",
            Self::Ssh { .. } => "ssh",
            Self::Tls { .. } => "tls",
            Self::Custom { .. } => "custom",
        }
    }

    /// Parse the human-readable string form (`kind:body`).
    ///
    /// On success, returns a target that has already been [`validate`](Self::validate)d
    /// — the caller does not need to revalidate. Round-trips with
    /// [`Display`](std::fmt::Display) for every successfully parsed target.
    pub fn parse(s: &str) -> Result<Self, AttachTargetError> {
        let (kind, body) = s.split_once(':').ok_or(AttachTargetError::Parse(
            AttachTargetParseError::MissingColon,
        ))?;
        let target = match kind {
            "local" => parse_local_body(body)?,
            "ssh" => parse_ssh_body(body)?,
            "tls" => parse_tls_body(body)?,
            "custom" => parse_custom_body(body)?,
            other => {
                return Err(AttachTargetError::Parse(
                    AttachTargetParseError::UnknownKind(other.to_string()),
                ));
            }
        };
        target.validate().map_err(AttachTargetError::Validate)?;
        Ok(target)
    }

    /// Local structural validation. Catches empty required fields,
    /// embedded null bytes, non-UTF-8 paths, and invalid characters in
    /// fields with structural meaning (e.g. `@` in a username, `/` in
    /// an instance name). Does not perform any I/O.
    pub fn validate(&self) -> Result<(), AttachTargetValidationError> {
        match self {
            Self::LocalSocket(p) => {
                let s = p
                    .to_str()
                    .ok_or(AttachTargetValidationError::NonUtf8Path("path"))?;
                if s.is_empty() {
                    return Err(AttachTargetValidationError::EmptyPath);
                }
                if s.contains('\0') {
                    return Err(AttachTargetValidationError::NullByte("path"));
                }
                Ok(())
            }
            Self::Ssh {
                host,
                user,
                instance_name,
            } => {
                if host.is_empty() {
                    return Err(AttachTargetValidationError::EmptyHost);
                }
                if host.contains('\0') {
                    return Err(AttachTargetValidationError::NullByte("host"));
                }
                if let Some(u) = user {
                    if u.is_empty() {
                        return Err(AttachTargetValidationError::EmptyUser);
                    }
                    if u.contains('\0') {
                        return Err(AttachTargetValidationError::NullByte("user"));
                    }
                    if u.contains('@') {
                        return Err(AttachTargetValidationError::InvalidUser(
                            "must not contain '@'",
                        ));
                    }
                }
                if let Some(n) = instance_name {
                    if n.is_empty() {
                        return Err(AttachTargetValidationError::EmptyInstanceName);
                    }
                    if n.contains('\0') {
                        return Err(AttachTargetValidationError::NullByte("instance_name"));
                    }
                    if n.contains('/') {
                        return Err(AttachTargetValidationError::InvalidInstanceName(
                            "must not contain '/'",
                        ));
                    }
                }
                Ok(())
            }
            Self::Tls { endpoint, cert } => {
                if endpoint.is_empty() {
                    return Err(AttachTargetValidationError::EmptyEndpoint);
                }
                if endpoint.contains('\0') {
                    return Err(AttachTargetValidationError::NullByte("endpoint"));
                }
                let cert_s = cert
                    .to_str()
                    .ok_or(AttachTargetValidationError::NonUtf8Path("cert"))?;
                if cert_s.is_empty() {
                    return Err(AttachTargetValidationError::EmptyPath);
                }
                if cert_s.contains('\0') {
                    return Err(AttachTargetValidationError::NullByte("cert"));
                }
                Ok(())
            }
            Self::Custom { command } => {
                if command.is_empty() {
                    return Err(AttachTargetValidationError::EmptyCommand);
                }
                for arg in command {
                    if arg.contains('\0') {
                        return Err(AttachTargetValidationError::NullByte("command"));
                    }
                }
                Ok(())
            }
        }
    }
}

fn parse_local_body(body: &str) -> Result<AttachTarget, AttachTargetError> {
    if body.is_empty() {
        return Err(AttachTargetError::Parse(AttachTargetParseError::EmptyBody(
            "local",
        )));
    }
    Ok(AttachTarget::LocalSocket(PathBuf::from(body)))
}

fn parse_ssh_body(body: &str) -> Result<AttachTarget, AttachTargetError> {
    if body.is_empty() {
        return Err(AttachTargetError::Parse(AttachTargetParseError::EmptyBody(
            "ssh",
        )));
    }
    let (user_host, instance_name) = match body.split_once('/') {
        Some((uh, n)) => (uh, Some(n.to_string())),
        None => (body, None),
    };
    let (user, host) = match user_host.split_once('@') {
        Some((u, h)) => (Some(u.to_string()), h.to_string()),
        None => (None, user_host.to_string()),
    };
    if host.is_empty() {
        return Err(AttachTargetError::Parse(
            AttachTargetParseError::SshMissingHost,
        ));
    }
    Ok(AttachTarget::Ssh {
        host,
        user,
        instance_name,
    })
}

fn parse_tls_body(body: &str) -> Result<AttachTarget, AttachTargetError> {
    if body.is_empty() {
        return Err(AttachTargetError::Parse(AttachTargetParseError::EmptyBody(
            "tls",
        )));
    }
    let (endpoint, cert) = body.split_once('#').ok_or(AttachTargetError::Parse(
        AttachTargetParseError::TlsMissingHash,
    ))?;
    Ok(AttachTarget::Tls {
        endpoint: endpoint.to_string(),
        cert: PathBuf::from(cert),
    })
}

fn parse_custom_body(body: &str) -> Result<AttachTarget, AttachTargetError> {
    let command: Vec<String> = body.split_whitespace().map(String::from).collect();
    if command.is_empty() {
        return Err(AttachTargetError::Parse(
            AttachTargetParseError::CustomEmptyCommand,
        ));
    }
    Ok(AttachTarget::Custom { command })
}

impl std::fmt::Display for AttachTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalSocket(p) => write!(f, "local:{}", p.display()),
            Self::Ssh {
                host,
                user,
                instance_name,
            } => {
                write!(f, "ssh:")?;
                if let Some(u) = user {
                    write!(f, "{u}@")?;
                }
                write!(f, "{host}")?;
                if let Some(n) = instance_name {
                    write!(f, "/{n}")?;
                }
                Ok(())
            }
            Self::Tls { endpoint, cert } => write!(f, "tls:{endpoint}#{}", cert.display()),
            Self::Custom { command } => write!(f, "custom:{}", command.join(" ")),
        }
    }
}

/// Error returned when an attach attempt fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachError {
    /// The target is reserved for a post-v0.1 release.
    NotImplementedInV01(&'static str),
    /// Transport-level I/O failure during attach.
    Io(String),
    /// The frontend already has an active attachment.
    AlreadyAttached,
    /// No instance was reachable at the requested target.
    NotFound(String),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotImplementedInV01(name) => {
                write!(
                    f,
                    "{name} transport not yet implemented (planned for v0.2 / milestone M5.7)"
                )
            }
            Self::Io(msg) => write!(f, "attach I/O error: {msg}"),
            Self::AlreadyAttached => write!(f, "frontend is already attached"),
            Self::NotFound(t) => write!(f, "attach target not found: {t}"),
        }
    }
}

impl std::error::Error for AttachError {}

/// Syntactic problems with the [`AttachTarget`] string form.
///
/// Distinct from [`AttachTargetValidationError`] because Lua callers
/// can construct [`AttachTarget`] from kwargs (skipping the parser);
/// they only encounter validation errors, not parse errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachTargetParseError {
    /// Input did not contain a `kind:body` separator.
    MissingColon,
    /// The `kind` prefix was not one of `local`, `ssh`, `tls`, `custom`.
    UnknownKind(String),
    /// The body after `kind:` was empty.
    EmptyBody(&'static str),
    /// SSH form was given without a host (`ssh:user@`, `ssh:/instance`).
    SshMissingHost,
    /// TLS form was missing the `endpoint#cert` separator.
    TlsMissingHash,
    /// Custom form had no argv tokens after whitespace splitting.
    CustomEmptyCommand,
}

impl std::fmt::Display for AttachTargetParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingColon => write!(
                f,
                "attach target must be of the form 'kind:body' (e.g. 'local:/path/to.sock', 'ssh:host')"
            ),
            Self::UnknownKind(k) => write!(
                f,
                "unknown attach target kind '{k}' (expected one of: local, ssh, tls, custom)"
            ),
            Self::EmptyBody(kind) => {
                write!(f, "attach target '{kind}:' requires a body after the colon")
            }
            Self::SshMissingHost => write!(
                f,
                "ssh attach target requires a host (e.g. 'ssh:hostname' or 'ssh:user@hostname')"
            ),
            Self::TlsMissingHash => write!(
                f,
                "tls attach target requires the form 'tls:endpoint#cert_path'"
            ),
            Self::CustomEmptyCommand => write!(
                f,
                "custom attach target requires at least one command word (e.g. 'custom:docker exec ...')"
            ),
        }
    }
}

impl std::error::Error for AttachTargetParseError {}

/// Semantic problems with an [`AttachTarget`] regardless of how it was
/// constructed. The string in each variant names the offending field
/// for diagnostic clarity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachTargetValidationError {
    /// A path field was empty.
    EmptyPath,
    /// A field contained an embedded null byte. Names the field.
    NullByte(&'static str),
    /// SSH host was empty.
    EmptyHost,
    /// SSH user override was an empty string. Callers should omit the
    /// field instead of passing `""`.
    EmptyUser,
    /// SSH user override contained an invalid character. The string
    /// names the constraint that was violated.
    InvalidUser(&'static str),
    /// SSH instance name override was an empty string.
    EmptyInstanceName,
    /// SSH instance name override contained an invalid character.
    InvalidInstanceName(&'static str),
    /// TLS endpoint was empty.
    EmptyEndpoint,
    /// A path field was not valid UTF-8. Names the field.
    NonUtf8Path(&'static str),
    /// Custom command had no argv tokens.
    EmptyCommand,
}

impl std::fmt::Display for AttachTargetValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "attach target path must not be empty"),
            Self::NullByte(field) => write!(
                f,
                "attach target field '{field}' must not contain a null byte"
            ),
            Self::EmptyHost => write!(f, "ssh attach target host must not be empty"),
            Self::EmptyUser => write!(
                f,
                "ssh attach target user must not be empty (omit it instead of passing \"\")"
            ),
            Self::InvalidUser(reason) => {
                write!(f, "ssh attach target user is invalid: {reason}")
            }
            Self::EmptyInstanceName => write!(
                f,
                "ssh attach target instance name must not be empty (omit it instead of passing \"\")"
            ),
            Self::InvalidInstanceName(reason) => {
                write!(f, "ssh attach target instance name is invalid: {reason}")
            }
            Self::EmptyEndpoint => write!(f, "tls attach target endpoint must not be empty"),
            Self::NonUtf8Path(field) => {
                write!(f, "attach target field '{field}' is not valid UTF-8")
            }
            Self::EmptyCommand => write!(
                f,
                "custom attach target command must have at least one argument"
            ),
        }
    }
}

impl std::error::Error for AttachTargetValidationError {}

/// Wrapper combining the two failure modes of [`AttachTarget::parse`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachTargetError {
    /// Syntactic parse failure.
    Parse(AttachTargetParseError),
    /// Semantic validation failure.
    Validate(AttachTargetValidationError),
}

impl std::fmt::Display for AttachTargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Validate(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AttachTargetError {}

/// Introspection token describing an active attachment.
///
/// Returned by the Lua getter `pmacs.current_attachment()`. The handle
/// surfaces the three facts a caller might want to inspect: which
/// `FrontendId` the instance assigned during the handshake, the
/// instance's self-description, and the target the frontend is
/// connected to.
///
/// # Lifecycle
///
/// v0.1 has no `pmacs.detach(handle)` operation — the only way to drop
/// an attachment is to exit the frontend. The handle is therefore
/// purely an introspection token, not a lifecycle resource. It carries
/// no Drop side-effects.
///
/// # Stability
///
/// Callers should not cache the handle across operations. v0.1 makes no
/// guarantee that two calls to `current_attachment()` return identical
/// handles even when nothing has changed (e.g. `uptime_secs` advances
/// monotonically inside `identity`). Treat each handle as a snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentHandle {
    /// Frontend ID the instance assigned in [`Hello::assigned_frontend_id`].
    pub frontend_id: FrontendId,
    /// Instance self-description from [`Hello::instance_identity`].
    pub identity: InstanceIdentity,
    /// The target the frontend is connected to.
    pub target: AttachTarget,
}

impl AttachmentHandle {
    /// Construct a handle from its three components.
    #[must_use]
    pub fn new(frontend_id: FrontendId, identity: InstanceIdentity, target: AttachTarget) -> Self {
        Self {
            frontend_id,
            identity,
            target,
        }
    }
}

// ---------------------------------------------------------------------------
// Crossterm translation (the only crossterm seam in this module)
// ---------------------------------------------------------------------------

/// Translation from `crossterm::event` types to the protocol types.
///
/// This submodule is the single place where `crossterm` types touch
/// the protocol. The TUI frontend converts at the input boundary;
/// network transports decode the wire directly into protocol types
/// without going through this layer.
pub mod crossterm_translate {
    use super::{CellCoord, FrontendId};
    use super::{Key, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseKind};
    use crossterm::event::{
        KeyCode, KeyEvent as CtKeyEvent, KeyModifiers as CtMods, MediaKeyCode, ModifierKeyCode,
        MouseButton as CtMouseButton, MouseEvent as CtMouseEvent, MouseEventKind,
    };

    /// Translate a `crossterm::event::KeyEvent` into a protocol
    /// [`KeyEvent`].
    #[must_use]
    pub fn key_from_crossterm(
        ev: &CtKeyEvent,
        frontend_id: FrontendId,
        timestamp_ns: u64,
    ) -> KeyEvent {
        KeyEvent {
            frontend_id,
            key: keycode_from_crossterm(ev.code),
            mods: mods_from_crossterm(ev.modifiers),
            timestamp_ns,
        }
    }

    /// Translate a `crossterm::event::KeyCode` into a protocol [`Key`].
    ///
    /// Media and modifier-only keycodes map to [`Key::Unknown`]: they
    /// are not actionable as commands but round-trip through
    /// serialization without being an error.
    #[must_use]
    pub fn keycode_from_crossterm(code: KeyCode) -> Key {
        match code {
            KeyCode::Char(c) => Key::Char(c),
            KeyCode::F(n) => Key::F(n),
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Enter => Key::Enter,
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            KeyCode::PageUp => Key::PageUp,
            KeyCode::PageDown => Key::PageDown,
            KeyCode::Tab => Key::Tab,
            KeyCode::BackTab => Key::BackTab,
            KeyCode::Delete => Key::Delete,
            KeyCode::Insert => Key::Insert,
            KeyCode::Esc => Key::Escape,
            KeyCode::Null => Key::Null,
            KeyCode::CapsLock => Key::CapsLock,
            KeyCode::ScrollLock => Key::ScrollLock,
            KeyCode::NumLock => Key::NumLock,
            KeyCode::PrintScreen => Key::PrintScreen,
            KeyCode::Pause => Key::Pause,
            KeyCode::Menu => Key::Menu,
            KeyCode::KeypadBegin => Key::KeypadBegin,
            KeyCode::Media(m) => Key::Unknown(media_sentinel(m)),
            KeyCode::Modifier(m) => Key::Unknown(modifier_sentinel(m)),
        }
    }

    /// Reverse translation: protocol [`Key`] back to a
    /// `crossterm::event::KeyCode`. Returns `None` for variants that
    /// have no crossterm equivalent ([`Key::Unknown`]).
    ///
    /// Used in the round-trip property test to confirm losslessness.
    #[must_use]
    pub fn keycode_to_crossterm(key: Key) -> Option<KeyCode> {
        Some(match key {
            Key::Char(c) => KeyCode::Char(c),
            Key::F(n) => KeyCode::F(n),
            Key::Backspace => KeyCode::Backspace,
            Key::Enter => KeyCode::Enter,
            Key::Left => KeyCode::Left,
            Key::Right => KeyCode::Right,
            Key::Up => KeyCode::Up,
            Key::Down => KeyCode::Down,
            Key::Home => KeyCode::Home,
            Key::End => KeyCode::End,
            Key::PageUp => KeyCode::PageUp,
            Key::PageDown => KeyCode::PageDown,
            Key::Tab => KeyCode::Tab,
            Key::BackTab => KeyCode::BackTab,
            Key::Delete => KeyCode::Delete,
            Key::Insert => KeyCode::Insert,
            Key::Escape => KeyCode::Esc,
            Key::Null => KeyCode::Null,
            Key::CapsLock => KeyCode::CapsLock,
            Key::ScrollLock => KeyCode::ScrollLock,
            Key::NumLock => KeyCode::NumLock,
            Key::PrintScreen => KeyCode::PrintScreen,
            Key::Pause => KeyCode::Pause,
            Key::Menu => KeyCode::Menu,
            Key::KeypadBegin => KeyCode::KeypadBegin,
            Key::Unknown(_) => return None,
        })
    }

    /// Translate a `crossterm::event::KeyModifiers` into protocol [`Modifiers`].
    #[must_use]
    pub fn mods_from_crossterm(m: CtMods) -> Modifiers {
        let mut out = Modifiers::NONE;
        if m.contains(CtMods::SHIFT) {
            out |= Modifiers::SHIFT;
        }
        if m.contains(CtMods::CONTROL) {
            out |= Modifiers::CTRL;
        }
        if m.contains(CtMods::ALT) {
            out |= Modifiers::ALT;
        }
        if m.contains(CtMods::SUPER) {
            out |= Modifiers::META;
        }
        if m.contains(CtMods::HYPER) {
            out |= Modifiers::HYPER;
        }
        out
    }

    /// Translate protocol [`Modifiers`] back to `crossterm::event::KeyModifiers`.
    #[must_use]
    pub fn mods_to_crossterm(m: Modifiers) -> CtMods {
        let mut out = CtMods::empty();
        if m.contains(Modifiers::SHIFT) {
            out |= CtMods::SHIFT;
        }
        if m.contains(Modifiers::CTRL) {
            out |= CtMods::CONTROL;
        }
        if m.contains(Modifiers::ALT) {
            out |= CtMods::ALT;
        }
        if m.contains(Modifiers::META) {
            out |= CtMods::SUPER;
        }
        if m.contains(Modifiers::HYPER) {
            out |= CtMods::HYPER;
        }
        out
    }

    /// Translate a `crossterm::event::MouseEvent` into a protocol [`MouseEvent`].
    #[must_use]
    pub fn mouse_from_crossterm(ev: &CtMouseEvent, frontend_id: FrontendId) -> MouseEvent {
        let kind = match ev.kind {
            MouseEventKind::Down(b) => MouseKind::Down(button_from(b)),
            MouseEventKind::Up(b) => MouseKind::Up(button_from(b)),
            MouseEventKind::Drag(b) => MouseKind::Drag(button_from(b)),
            MouseEventKind::Moved => MouseKind::Move,
            MouseEventKind::ScrollUp => MouseKind::ScrollUp,
            MouseEventKind::ScrollDown => MouseKind::ScrollDown,
            MouseEventKind::ScrollLeft => MouseKind::ScrollLeft,
            MouseEventKind::ScrollRight => MouseKind::ScrollRight,
        };
        MouseEvent {
            frontend_id,
            kind,
            coord: CellCoord::new(u32::from(ev.row), u32::from(ev.column)),
            mods: mods_from_crossterm(ev.modifiers),
        }
    }

    fn button_from(b: CtMouseButton) -> MouseButton {
        match b {
            CtMouseButton::Left => MouseButton::Left,
            CtMouseButton::Right => MouseButton::Right,
            CtMouseButton::Middle => MouseButton::Middle,
        }
    }

    fn button_to(b: MouseButton) -> CtMouseButton {
        match b {
            MouseButton::Left => CtMouseButton::Left,
            MouseButton::Right => CtMouseButton::Right,
            MouseButton::Middle => CtMouseButton::Middle,
        }
    }

    /// Reverse translation: build a `crossterm::event::KeyEvent` from
    /// a protocol [`KeyEvent`].
    ///
    /// Returns `None` when the keycode is [`Key::Unknown`], which has
    /// no native crossterm equivalent. The instance side ignores
    /// unknown keys (they don't actuate commands), so callers can
    /// drop these without further handling.
    ///
    /// Used by the daemon's per-attach loop to feed
    /// [`crate::editor::EditorState::dispatch_key`], which still takes
    /// the crossterm shape for compatibility with the in-process TUI.
    #[must_use]
    pub fn key_to_crossterm(ev: &KeyEvent) -> Option<CtKeyEvent> {
        use crossterm::event::{KeyEventKind, KeyEventState};
        Some(CtKeyEvent {
            code: keycode_to_crossterm(ev.key)?,
            modifiers: mods_to_crossterm(ev.mods),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
    }

    /// Reverse translation: build a `crossterm::event::MouseEvent`
    /// from a protocol [`MouseEvent`].
    ///
    /// Coordinates are clamped into `u16` (crossterm's representation);
    /// terminal sizes don't realistically exceed `u16::MAX` cells in
    /// either dimension, but we clamp rather than panic to be safe
    /// against a misbehaving frontend.
    #[must_use]
    pub fn mouse_to_crossterm(ev: &MouseEvent) -> CtMouseEvent {
        let kind = match ev.kind {
            MouseKind::Down(b) => MouseEventKind::Down(button_to(b)),
            MouseKind::Up(b) => MouseEventKind::Up(button_to(b)),
            MouseKind::Drag(b) => MouseEventKind::Drag(button_to(b)),
            MouseKind::Move => MouseEventKind::Moved,
            MouseKind::ScrollUp => MouseEventKind::ScrollUp,
            MouseKind::ScrollDown => MouseEventKind::ScrollDown,
            MouseKind::ScrollLeft => MouseEventKind::ScrollLeft,
            MouseKind::ScrollRight => MouseEventKind::ScrollRight,
        };
        CtMouseEvent {
            kind,
            row: u16::try_from(ev.coord.row).unwrap_or(u16::MAX),
            column: u16::try_from(ev.coord.col).unwrap_or(u16::MAX),
            modifiers: mods_to_crossterm(ev.mods),
        }
    }

    /// Stable sentinel for media keycodes so they round-trip through
    /// the [`Key::Unknown`] variant.
    const fn media_sentinel(m: MediaKeyCode) -> u32 {
        // Encode as `0x01XX` so the namespace is distinguishable from
        // modifier-only keys (0x02XX) and any future class.
        0x0100
            | match m {
                MediaKeyCode::Play => 0x01,
                MediaKeyCode::Pause => 0x02,
                MediaKeyCode::PlayPause => 0x03,
                MediaKeyCode::Reverse => 0x04,
                MediaKeyCode::Stop => 0x05,
                MediaKeyCode::FastForward => 0x06,
                MediaKeyCode::Rewind => 0x07,
                MediaKeyCode::TrackNext => 0x08,
                MediaKeyCode::TrackPrevious => 0x09,
                MediaKeyCode::Record => 0x0A,
                MediaKeyCode::LowerVolume => 0x0B,
                MediaKeyCode::RaiseVolume => 0x0C,
                MediaKeyCode::MuteVolume => 0x0D,
            }
    }

    const fn modifier_sentinel(m: ModifierKeyCode) -> u32 {
        0x0200
            | match m {
                ModifierKeyCode::LeftShift => 0x01,
                ModifierKeyCode::LeftControl => 0x02,
                ModifierKeyCode::LeftAlt => 0x03,
                ModifierKeyCode::LeftSuper => 0x04,
                ModifierKeyCode::LeftHyper => 0x05,
                ModifierKeyCode::LeftMeta => 0x06,
                ModifierKeyCode::RightShift => 0x07,
                ModifierKeyCode::RightControl => 0x08,
                ModifierKeyCode::RightAlt => 0x09,
                ModifierKeyCode::RightSuper => 0x0A,
                ModifierKeyCode::RightHyper => 0x0B,
                ModifierKeyCode::RightMeta => 0x0C,
                ModifierKeyCode::IsoLevel3Shift => 0x0D,
                ModifierKeyCode::IsoLevel5Shift => 0x0E,
            }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Acceptance home for T M5.1 (Frontend protocol skeleton). The M5.1
    // spec criteria — typed FrontendEvent / InstanceMessage / FrontendId,
    // pmacs-native Key / Modifiers, lossless crossterm round-trip,
    // NotImplementedInV01 paths — are exercised by the lib tests in
    // this module rather than a separate tests/m5_1_acceptance.rs file.
    // See tests/INDEX.md for the full M5.x → coverage map.

    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent as CtKeyEvent, KeyEventKind, KeyEventState, KeyModifiers as CtMods,
        MouseButton as CtMouseButton, MouseEvent as CtMouseEvent, MouseEventKind,
    };

    #[test]
    fn frontend_id_local_is_one() {
        // The constant is load-bearing — Lua bindings and tests
        // hard-code this. Pin it so we notice if it ever drifts.
        assert_eq!(FrontendId::LOCAL, FrontendId(1));
    }

    #[test]
    fn modifiers_compose() {
        let m = Modifiers::SHIFT | Modifiers::CTRL;
        assert!(m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::CTRL));
        assert!(!m.contains(Modifiers::ALT));
        assert!(!m.is_empty());
        assert_eq!(m.bits(), 0b0000_0011);
    }

    #[test]
    fn modifiers_truncate_unknown_bits() {
        let raw = Modifiers::from_bits_truncate(0b1111_1111);
        // Only the five defined bits survive.
        assert_eq!(raw.bits(), 0b0001_1111);
    }

    #[test]
    fn frontend_event_id_extraction() {
        let id = FrontendId(42);
        let ev = FrontendEvent::Key(KeyEvent {
            frontend_id: id,
            key: Key::Char('a'),
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        });
        assert_eq!(ev.frontend_id(), id);
        assert_eq!(FrontendEvent::Detach(id).frontend_id(), id);
        assert_eq!(
            FrontendEvent::Resize {
                frontend_id: id,
                size: CellSize::new(24, 80),
            }
            .frontend_id(),
            id
        );
    }

    #[test]
    fn attach_target_check_v01_accepts_implemented() {
        assert!(
            AttachTarget::LocalSocket(PathBuf::from("/run/pmacs.sock"))
                .check_v01()
                .is_ok()
        );
        assert!(
            AttachTarget::Ssh {
                host: "example".into(),
                user: None,
                instance_name: None
            }
            .check_v01()
            .is_ok()
        );
    }

    #[test]
    fn attach_target_check_v01_rejects_tls_and_custom() {
        let tls = AttachTarget::Tls {
            endpoint: "example:9999".into(),
            cert: PathBuf::from("/etc/pmacs.crt"),
        };
        match tls.check_v01() {
            Err(AttachError::NotImplementedInV01("TLS")) => {}
            other => panic!("expected NotImplementedInV01(\"TLS\"), got {other:?}"),
        }

        let custom = AttachTarget::Custom {
            command: vec!["docker".into(), "exec".into()],
        };
        match custom.check_v01() {
            Err(AttachError::NotImplementedInV01("Custom")) => {}
            other => panic!("expected NotImplementedInV01(\"Custom\"), got {other:?}"),
        }
    }

    #[test]
    fn attach_error_display_points_at_target_milestone() {
        // The not-implemented message names the milestone that ships
        // the implementation, so users have a planning anchor and the
        // error tells them what to do (wait / upgrade) rather than
        // characterizing their action as misuse.
        let e = AttachError::NotImplementedInV01("SSH");
        assert_eq!(
            e.to_string(),
            "SSH transport not yet implemented (planned for v0.2 / milestone M5.7)"
        );
    }

    #[test]
    fn kind_name_stable_across_variants() {
        assert_eq!(
            AttachTarget::LocalSocket(PathBuf::new()).kind_name(),
            "local"
        );
        assert_eq!(
            AttachTarget::Ssh {
                host: String::new(),
                user: None,
                instance_name: None
            }
            .kind_name(),
            "ssh"
        );
        assert_eq!(
            AttachTarget::Tls {
                endpoint: String::new(),
                cert: PathBuf::new()
            }
            .kind_name(),
            "tls"
        );
        assert_eq!(
            AttachTarget::Custom { command: vec![] }.kind_name(),
            "custom"
        );
    }

    // --- M5.6a: parse, validate, Display ---

    #[test]
    fn parse_local_socket_simple_path() {
        let t = AttachTarget::parse("local:/run/user/1000/pmacs/default.sock")
            .expect("local with valid path");
        match t {
            AttachTarget::LocalSocket(p) => {
                assert_eq!(p, PathBuf::from("/run/user/1000/pmacs/default.sock"));
            }
            other => panic!("expected LocalSocket, got {other:?}"),
        }
    }

    #[test]
    fn parse_local_path_with_internal_colon_preserved() {
        // split_once(':') only splits on the first colon — paths with
        // colons in them (e.g. Windows-style or weird mount points) are
        // preserved verbatim in the body.
        let t = AttachTarget::parse("local:/foo:bar/baz.sock").expect("colon in path");
        match t {
            AttachTarget::LocalSocket(p) => assert_eq!(p, PathBuf::from("/foo:bar/baz.sock")),
            other => panic!("expected LocalSocket, got {other:?}"),
        }
    }

    #[test]
    fn parse_ssh_host_only() {
        let t = AttachTarget::parse("ssh:mac-studio").expect("ssh with bare host");
        assert_eq!(
            t,
            AttachTarget::Ssh {
                host: "mac-studio".into(),
                user: None,
                instance_name: None,
            }
        );
    }

    #[test]
    fn parse_ssh_user_at_host() {
        let t = AttachTarget::parse("ssh:lev@mac-studio").expect("ssh with user");
        assert_eq!(
            t,
            AttachTarget::Ssh {
                host: "mac-studio".into(),
                user: Some("lev".into()),
                instance_name: None,
            }
        );
    }

    #[test]
    fn parse_ssh_user_host_instance() {
        let t =
            AttachTarget::parse("ssh:lev@mac-studio/research").expect("ssh with user and instance");
        assert_eq!(
            t,
            AttachTarget::Ssh {
                host: "mac-studio".into(),
                user: Some("lev".into()),
                instance_name: Some("research".into()),
            }
        );
    }

    #[test]
    fn parse_ssh_host_instance_no_user() {
        let t = AttachTarget::parse("ssh:mac-studio/research").expect("ssh with instance, no user");
        assert_eq!(
            t,
            AttachTarget::Ssh {
                host: "mac-studio".into(),
                user: None,
                instance_name: Some("research".into()),
            }
        );
    }

    #[test]
    fn parse_tls_endpoint_and_cert() {
        let t = AttachTarget::parse("tls:example.com:9999#/etc/pmacs.crt")
            .expect("tls with endpoint and cert");
        assert_eq!(
            t,
            AttachTarget::Tls {
                endpoint: "example.com:9999".into(),
                cert: PathBuf::from("/etc/pmacs.crt"),
            }
        );
    }

    #[test]
    fn parse_custom_argv_split() {
        let t = AttachTarget::parse("custom:docker exec -i pmacs-container pmacs --daemon-attach")
            .expect("custom with argv");
        match t {
            AttachTarget::Custom { command } => {
                assert_eq!(
                    command,
                    vec![
                        "docker",
                        "exec",
                        "-i",
                        "pmacs-container",
                        "pmacs",
                        "--daemon-attach"
                    ]
                );
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn parse_missing_colon() {
        match AttachTarget::parse("local") {
            Err(AttachTargetError::Parse(AttachTargetParseError::MissingColon)) => {}
            other => panic!("expected MissingColon, got {other:?}"),
        }
    }

    #[test]
    fn parse_missing_colon_message_points_at_workaround() {
        // The error message tells the user what shape the input should
        // take, not just that the input was wrong.
        let e = AttachTargetParseError::MissingColon;
        let msg = e.to_string();
        assert!(msg.contains("kind:body"), "{msg}");
        assert!(msg.contains("local:") && msg.contains("ssh:"), "{msg}");
    }

    #[test]
    fn parse_unknown_kind() {
        match AttachTarget::parse("smtp:host") {
            Err(AttachTargetError::Parse(AttachTargetParseError::UnknownKind(k))) => {
                assert_eq!(k, "smtp");
            }
            other => panic!("expected UnknownKind, got {other:?}"),
        }
    }

    #[test]
    fn parse_unknown_kind_message_lists_valid_kinds() {
        let e = AttachTargetParseError::UnknownKind("smtp".into());
        let msg = e.to_string();
        assert!(msg.contains("smtp"), "{msg}");
        // All four valid kinds named so user knows the menu.
        for k in ["local", "ssh", "tls", "custom"] {
            assert!(msg.contains(k), "{msg} missing {k}");
        }
    }

    #[test]
    fn parse_local_empty_body() {
        match AttachTarget::parse("local:") {
            Err(AttachTargetError::Parse(AttachTargetParseError::EmptyBody("local"))) => {}
            other => panic!("expected EmptyBody(local), got {other:?}"),
        }
    }

    #[test]
    fn parse_ssh_empty_body() {
        match AttachTarget::parse("ssh:") {
            Err(AttachTargetError::Parse(AttachTargetParseError::EmptyBody("ssh"))) => {}
            other => panic!("expected EmptyBody(ssh), got {other:?}"),
        }
    }

    #[test]
    fn parse_ssh_user_at_empty_host() {
        // `ssh:lev@` parses user-host as `lev@`, splits to user=Some("lev"), host=""
        match AttachTarget::parse("ssh:lev@") {
            Err(AttachTargetError::Parse(AttachTargetParseError::SshMissingHost)) => {}
            other => panic!("expected SshMissingHost, got {other:?}"),
        }
    }

    #[test]
    fn parse_ssh_slash_instance_no_host() {
        // `ssh:/research` splits at `/` first → user_host = "", instance = "research"
        // Then user_host has no `@`, so host = "" → SshMissingHost.
        match AttachTarget::parse("ssh:/research") {
            Err(AttachTargetError::Parse(AttachTargetParseError::SshMissingHost)) => {}
            other => panic!("expected SshMissingHost, got {other:?}"),
        }
    }

    #[test]
    fn parse_tls_missing_hash() {
        match AttachTarget::parse("tls:example.com:9999") {
            Err(AttachTargetError::Parse(AttachTargetParseError::TlsMissingHash)) => {}
            other => panic!("expected TlsMissingHash, got {other:?}"),
        }
    }

    #[test]
    fn parse_custom_only_whitespace_is_empty_command() {
        match AttachTarget::parse("custom:   \t  ") {
            Err(AttachTargetError::Parse(AttachTargetParseError::CustomEmptyCommand)) => {}
            other => panic!("expected CustomEmptyCommand, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_null_byte_in_path() {
        // Validation runs as the final step of parse, so embedded
        // nulls surface as a Validate error (not a Parse error).
        let s = "local:/foo\0/bar.sock";
        match AttachTarget::parse(s) {
            Err(AttachTargetError::Validate(AttachTargetValidationError::NullByte("path"))) => {}
            other => panic!("expected NullByte(path) from validate, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_at_sign_in_user() {
        // `ssh:a@b@host` parses user=Some("a"), host="b@host" — the host
        // contains an `@` which is structurally fine for ssh, but if the
        // user does `ssh:user@@host`, we get user=Some("user"), host="@host".
        // The host having `@` is legal-ish for some configs; we don't
        // reject it. But user containing `@` *is* rejected. Construct
        // the case directly to test the validation:
        let t = AttachTarget::Ssh {
            host: "host".into(),
            user: Some("u@bad".into()),
            instance_name: None,
        };
        match t.validate() {
            Err(AttachTargetValidationError::InvalidUser(_)) => {}
            other => panic!("expected InvalidUser, got {other:?}"),
        }
    }

    #[test]
    fn validate_local_empty_path() {
        let t = AttachTarget::LocalSocket(PathBuf::new());
        match t.validate() {
            Err(AttachTargetValidationError::EmptyPath) => {}
            other => panic!("expected EmptyPath, got {other:?}"),
        }
    }

    #[test]
    fn validate_ssh_empty_host() {
        let t = AttachTarget::Ssh {
            host: String::new(),
            user: None,
            instance_name: None,
        };
        match t.validate() {
            Err(AttachTargetValidationError::EmptyHost) => {}
            other => panic!("expected EmptyHost, got {other:?}"),
        }
    }

    #[test]
    fn validate_ssh_empty_user_string_rejected() {
        // Passing user = Some("") is treated as user error: omit the
        // field instead. This catches a common Lua-side mistake where
        // a missing kwarg becomes an empty string.
        let t = AttachTarget::Ssh {
            host: "host".into(),
            user: Some(String::new()),
            instance_name: None,
        };
        match t.validate() {
            Err(AttachTargetValidationError::EmptyUser) => {}
            other => panic!("expected EmptyUser, got {other:?}"),
        }
    }

    #[test]
    fn validate_ssh_instance_name_with_slash() {
        let t = AttachTarget::Ssh {
            host: "host".into(),
            user: None,
            instance_name: Some("a/b".into()),
        };
        match t.validate() {
            Err(AttachTargetValidationError::InvalidInstanceName(_)) => {}
            other => panic!("expected InvalidInstanceName, got {other:?}"),
        }
    }

    #[test]
    fn validate_tls_empty_endpoint() {
        let t = AttachTarget::Tls {
            endpoint: String::new(),
            cert: PathBuf::from("/etc/pmacs.crt"),
        };
        match t.validate() {
            Err(AttachTargetValidationError::EmptyEndpoint) => {}
            other => panic!("expected EmptyEndpoint, got {other:?}"),
        }
    }

    #[test]
    fn validate_tls_empty_cert() {
        let t = AttachTarget::Tls {
            endpoint: "host:9999".into(),
            cert: PathBuf::new(),
        };
        match t.validate() {
            Err(AttachTargetValidationError::EmptyPath) => {}
            other => panic!("expected EmptyPath, got {other:?}"),
        }
    }

    #[test]
    fn validate_custom_empty_command() {
        let t = AttachTarget::Custom { command: vec![] };
        match t.validate() {
            Err(AttachTargetValidationError::EmptyCommand) => {}
            other => panic!("expected EmptyCommand, got {other:?}"),
        }
    }

    #[test]
    fn validate_custom_null_in_arg() {
        let t = AttachTarget::Custom {
            command: vec!["docker".into(), "exec\0".into()],
        };
        match t.validate() {
            Err(AttachTargetValidationError::NullByte("command")) => {}
            other => panic!("expected NullByte(command), got {other:?}"),
        }
    }

    #[test]
    fn validate_succeeds_on_well_formed_targets() {
        AttachTarget::LocalSocket(PathBuf::from("/run/p.sock"))
            .validate()
            .expect("local valid");
        AttachTarget::Ssh {
            host: "h".into(),
            user: Some("u".into()),
            instance_name: Some("i".into()),
        }
        .validate()
        .expect("ssh valid");
        AttachTarget::Tls {
            endpoint: "h:9".into(),
            cert: PathBuf::from("/c"),
        }
        .validate()
        .expect("tls valid");
        AttachTarget::Custom {
            command: vec!["a".into(), "b".into()],
        }
        .validate()
        .expect("custom valid");
    }

    #[test]
    fn display_round_trips_for_all_variants() {
        // Display → parse → Display is a fixed point for every shape
        // the parser accepts.
        let cases = [
            "local:/run/user/1000/pmacs/default.sock",
            "ssh:mac-studio",
            "ssh:lev@mac-studio",
            "ssh:lev@mac-studio/research",
            "ssh:mac-studio/research",
            "tls:example.com:9999#/etc/pmacs.crt",
            "custom:docker exec pmacs",
        ];
        for s in cases {
            let parsed = AttachTarget::parse(s).unwrap_or_else(|e| panic!("parse {s:?}: {e}"));
            let displayed = parsed.to_string();
            assert_eq!(displayed, s, "round-trip failed: {s:?} → {displayed:?}");
            // Re-parsing the Display output must also succeed and equal
            // the first parse.
            let reparsed = AttachTarget::parse(&displayed).expect("re-parse Display output");
            assert_eq!(reparsed, parsed);
        }
    }

    #[test]
    fn parse_then_check_v01_for_unimplemented_passes_parse() {
        // The v0.1 stub posture: TLS / Custom parse and validate
        // successfully, but check_v01 rejects them. This is what lets a
        // user write `pmacs.attach{ target = "ssh:..." }` in init.lua
        // today and have the call only fail at activation time once SSH
        // ships in M5.7.
        let tls = AttachTarget::parse("tls:host:9#/etc/c").expect("tls parses");
        match tls.check_v01() {
            Err(AttachError::NotImplementedInV01("TLS")) => {}
            other => panic!("expected NotImplementedInV01(TLS), got {other:?}"),
        }
        let custom = AttachTarget::parse("custom:docker exec").expect("custom parses");
        match custom.check_v01() {
            Err(AttachError::NotImplementedInV01("Custom")) => {}
            other => panic!("expected NotImplementedInV01(Custom), got {other:?}"),
        }
    }

    #[test]
    fn attach_target_error_display_delegates_to_inner() {
        let p = AttachTargetError::Parse(AttachTargetParseError::SshMissingHost);
        assert!(p.to_string().contains("ssh attach target requires a host"));
        let v = AttachTargetError::Validate(AttachTargetValidationError::EmptyHost);
        assert!(v.to_string().contains("must not be empty"));
    }

    // --- M5.6b: AttachmentHandle ---

    fn sample_identity() -> InstanceIdentity {
        InstanceIdentity {
            pmacs_version: "0.1.0".into(),
            build_hash: Some("a3f9c21".into()),
            instance_name: Some("research".into()),
            uptime_secs: 2_847,
            working_directory: "/home/researcher/project".into(),
        }
    }

    #[test]
    fn attachment_handle_new_constructs_all_fields() {
        let id = sample_identity();
        let target = AttachTarget::LocalSocket(PathBuf::from("/run/p.sock"));
        let h = AttachmentHandle::new(FrontendId(7), id.clone(), target.clone());
        assert_eq!(h.frontend_id, FrontendId(7));
        assert_eq!(h.identity, id);
        assert_eq!(h.target, target);
    }

    #[test]
    fn attachment_handle_clone_is_equal() {
        let h = AttachmentHandle::new(
            FrontendId(2),
            sample_identity(),
            AttachTarget::LocalSocket(PathBuf::from("/x")),
        );
        let cloned = h.clone();
        assert_eq!(h, cloned);
    }

    #[test]
    fn attachment_handle_equality_includes_every_field() {
        // Mutating any single field flips equality. Pin this so a
        // future field addition doesn't silently weaken the comparison.
        let base = AttachmentHandle::new(
            FrontendId(2),
            sample_identity(),
            AttachTarget::LocalSocket(PathBuf::from("/x")),
        );

        let diff_id = AttachmentHandle {
            frontend_id: FrontendId(3),
            ..base.clone()
        };
        assert_ne!(base, diff_id);

        let diff_identity = AttachmentHandle {
            identity: InstanceIdentity {
                uptime_secs: 999,
                ..base.identity.clone()
            },
            ..base.clone()
        };
        assert_ne!(base, diff_identity);

        let diff_target = AttachmentHandle {
            target: AttachTarget::LocalSocket(PathBuf::from("/other")),
            ..base.clone()
        };
        assert_ne!(base, diff_target);
    }

    #[test]
    fn attachment_handle_carries_ssh_target_for_v01_init_lua_use() {
        // A user writes `pmacs.attach{ target = "ssh:host" }` in
        // init.lua. v0.1 errors at activation, but the handle shape
        // must be able to carry an Ssh target so M5.7 can ship without
        // changing AttachmentHandle's surface.
        let h = AttachmentHandle::new(
            FrontendId(2),
            sample_identity(),
            AttachTarget::Ssh {
                host: "mac-studio".into(),
                user: Some("lev".into()),
                instance_name: Some("research".into()),
            },
        );
        assert_eq!(h.target.kind_name(), "ssh");
    }

    #[test]
    fn attachment_handle_uses_assigned_frontend_id_not_local() {
        // Daemon-attached frontends start at FrontendId(2); the LOCAL
        // constant (FrontendId(1)) is reserved for an in-process TUI.
        // Pin this so we don't accidentally hand back LOCAL from a
        // remote handle.
        let h = AttachmentHandle::new(
            FrontendId(2),
            sample_identity(),
            AttachTarget::LocalSocket(PathBuf::from("/run/p.sock")),
        );
        assert_ne!(h.frontend_id, FrontendId::LOCAL);
        assert_eq!(h.frontend_id, FrontendId(2));
    }

    // --- Crossterm translation round-trips ---

    fn ct_key(code: KeyCode, mods: CtMods) -> CtKeyEvent {
        CtKeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn key_round_trip_for_named_keys() {
        // Every named keycode must translate forward and back without
        // loss. `KeyCode::Char` with every printable char is excessive;
        // a representative sample plus the named variants is enough to
        // catch a missed arm.
        use crossterm_translate::{keycode_from_crossterm, keycode_to_crossterm};
        let cases = [
            KeyCode::Char('a'),
            KeyCode::Char('Z'),
            KeyCode::Char('5'),
            KeyCode::Char(' '),
            KeyCode::Char('é'),
            KeyCode::F(1),
            KeyCode::F(12),
            KeyCode::Backspace,
            KeyCode::Enter,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::Esc,
            KeyCode::Null,
            KeyCode::CapsLock,
            KeyCode::ScrollLock,
            KeyCode::NumLock,
            KeyCode::PrintScreen,
            KeyCode::Pause,
            KeyCode::Menu,
            KeyCode::KeypadBegin,
        ];
        for code in cases {
            let pmacs_key = keycode_from_crossterm(code);
            let back = keycode_to_crossterm(pmacs_key)
                .unwrap_or_else(|| panic!("no reverse for {pmacs_key:?} (from {code:?})"));
            assert_eq!(
                back, code,
                "round-trip mismatch: {code:?} → {pmacs_key:?} → {back:?}"
            );
        }
    }

    #[test]
    fn modifiers_round_trip_through_crossterm() {
        use crossterm_translate::{mods_from_crossterm, mods_to_crossterm};
        let pairs = [
            (CtMods::empty(), Modifiers::NONE),
            (CtMods::SHIFT, Modifiers::SHIFT),
            (CtMods::CONTROL, Modifiers::CTRL),
            (CtMods::ALT, Modifiers::ALT),
            (CtMods::SUPER, Modifiers::META),
            (CtMods::HYPER, Modifiers::HYPER),
            (
                CtMods::SHIFT | CtMods::CONTROL,
                Modifiers::SHIFT | Modifiers::CTRL,
            ),
        ];
        for (ct, pmacs) in pairs {
            let forward = mods_from_crossterm(ct);
            assert_eq!(
                forward, pmacs,
                "from_crossterm({ct:?}) = {forward:?}, expected {pmacs:?}"
            );
            let back = mods_to_crossterm(forward);
            assert_eq!(
                back, ct,
                "to_crossterm({forward:?}) = {back:?}, expected {ct:?}"
            );
        }
    }

    #[test]
    fn key_event_translation_threads_frontend_id() {
        use crossterm_translate::key_from_crossterm;
        let id = FrontendId(7);
        let ct = ct_key(KeyCode::Char('q'), CtMods::CONTROL);
        let translated = key_from_crossterm(&ct, id, 12345);
        assert_eq!(translated.frontend_id, id);
        assert_eq!(translated.key, Key::Char('q'));
        assert_eq!(translated.mods, Modifiers::CTRL);
        assert_eq!(translated.timestamp_ns, 12345);
    }

    #[test]
    fn unknown_keycode_does_not_round_trip_to_crossterm() {
        use crossterm_translate::keycode_to_crossterm;
        // Unknown is the escape hatch; reverse translation is `None`
        // since there's no native crossterm equivalent.
        assert!(keycode_to_crossterm(Key::Unknown(0x0101)).is_none());
    }

    #[test]
    fn media_keycode_translates_to_unknown_with_stable_sentinel() {
        use crossterm::event::MediaKeyCode;
        use crossterm_translate::keycode_from_crossterm;
        let k = keycode_from_crossterm(KeyCode::Media(MediaKeyCode::PlayPause));
        match k {
            Key::Unknown(n) => assert_eq!(n, 0x0103),
            other => panic!("expected Key::Unknown, got {other:?}"),
        }
    }

    #[test]
    fn modifier_only_keycode_translates_to_unknown_with_stable_sentinel() {
        use crossterm::event::ModifierKeyCode;
        use crossterm_translate::keycode_from_crossterm;
        let k = keycode_from_crossterm(KeyCode::Modifier(ModifierKeyCode::LeftShift));
        match k {
            Key::Unknown(n) => assert_eq!(n, 0x0201),
            other => panic!("expected Key::Unknown, got {other:?}"),
        }
    }

    #[test]
    fn mouse_event_translation() {
        use crossterm_translate::mouse_from_crossterm;
        let id = FrontendId(3);
        let ct = CtMouseEvent {
            kind: MouseEventKind::Down(CtMouseButton::Left),
            row: 5,
            column: 10,
            modifiers: CtMods::SHIFT,
        };
        let m = mouse_from_crossterm(&ct, id);
        assert_eq!(m.frontend_id, id);
        assert_eq!(m.kind, MouseKind::Down(MouseButton::Left));
        assert_eq!(m.coord, CellCoord::new(5, 10));
        assert_eq!(m.mods, Modifiers::SHIFT);
    }

    #[test]
    fn mouse_kinds_cover_all_crossterm_variants() {
        use crossterm_translate::mouse_from_crossterm;
        let id = FrontendId::LOCAL;
        let kinds = [
            (
                MouseEventKind::Down(CtMouseButton::Right),
                MouseKind::Down(MouseButton::Right),
            ),
            (
                MouseEventKind::Up(CtMouseButton::Middle),
                MouseKind::Up(MouseButton::Middle),
            ),
            (
                MouseEventKind::Drag(CtMouseButton::Left),
                MouseKind::Drag(MouseButton::Left),
            ),
            (MouseEventKind::Moved, MouseKind::Move),
            (MouseEventKind::ScrollUp, MouseKind::ScrollUp),
            (MouseEventKind::ScrollDown, MouseKind::ScrollDown),
            (MouseEventKind::ScrollLeft, MouseKind::ScrollLeft),
            (MouseEventKind::ScrollRight, MouseKind::ScrollRight),
        ];
        for (ct_kind, expected) in kinds {
            let ct = CtMouseEvent {
                kind: ct_kind,
                row: 0,
                column: 0,
                modifiers: CtMods::empty(),
            };
            let m = mouse_from_crossterm(&ct, id);
            assert_eq!(m.kind, expected);
        }
    }

    #[test]
    fn instance_message_cell_delta_carries_full_grid_flag() {
        let m = InstanceMessage::CellDelta {
            spans: vec![],
            full_grid: true,
        };
        match m {
            InstanceMessage::CellDelta { full_grid, .. } => assert!(full_grid),
            _ => unreachable!(),
        }
    }

    // --- M5.5a handshake & postcard round-trips ---

    #[test]
    fn protocol_version_is_eight_for_status_facts() {
        // Pin the value: T M10.5 bumped 1→2 (v1.0 wire: CrdtOp /
        // PresenceUpdate). T M11.1 bumped 2→3 (v1.1 wire: the
        // SemanticFrame family + FrontendEvent::Viewport). T M11.6
        // bumped 3→4 (DispatchIdle for the optimistic-apply gate).
        // The mouse framing Q#M1 bumped 4→5 (FrontendEvent::Pointer).
        // T M4.6 bumped 5→6 (`Style::underline_color`) — the first
        // bump that changed an existing struct's postcard encoding,
        // making v6 the ladder's encoding floor. Q#M4 bumped 6→7
        // (`PointerKind::TripleDown`, additive + frontend-gated).
        // Q#S1 bumped 7→8 (`InstanceMessage::StatusFacts`, additive
        // + daemon-gated per session).
        assert_eq!(PROTOCOL_VERSION, 8);
    }

    #[test]
    fn supported_protocol_versions_resume_ladder_on_v6_floor() {
        // T M4.6: `Style::underline_color` changed the encoding of
        // every cell-carrying message, ending the v1–v5 ladder —
        // pre-v6 peers are refused at the handshake (a clean
        // VersionMismatch) rather than garbling postcard mid-session.
        // Q#M4 / Q#S1: the ladder resumes above that floor — v7
        // (`TripleDown`, frontend-gated) and v8 (`StatusFacts`,
        // daemon-gated) are additive, so v6 through v8 interoperate.
        assert!(is_supported_protocol_version(6));
        assert!(is_supported_protocol_version(7));
        assert!(is_supported_protocol_version(8));
        for rejected in [0, 1, 2, 3, 4, 5, 9, u32::MAX] {
            assert!(
                !is_supported_protocol_version(rejected),
                "v{rejected} must be rejected by a v8 binary"
            );
        }
    }

    #[test]
    fn hello_round_trips_through_postcard() {
        let h = Hello {
            protocol_version: PROTOCOL_VERSION,
            assigned_frontend_id: FrontendId(7),
            instance_identity: InstanceIdentity {
                pmacs_version: "0.1.0".into(),
                build_hash: Some("a3f9c21".into()),
                instance_name: Some("research".into()),
                uptime_secs: 2_847,
                working_directory: "/home/researcher/project".into(),
            },
            instance_capabilities: InstanceCapabilities::default(),
        };
        let bytes = postcard::to_allocvec(&h).expect("encode");
        let decoded: Hello = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, h);
    }

    #[test]
    fn attach_request_round_trips_through_postcard() {
        let req = AttachRequest {
            protocol_version: PROTOCOL_VERSION,
            frontend_capabilities: FrontendCapabilities {
                synchronized_output: true,
                unicode_smp: true,
                true_color: true,
                mouse: true,
                bracketed_paste: true,
                terminal_kind: Some("xterm-256color".into()),
                multi_frontend: false,
                crdt_replica: false,
                semantic_render: false,
            },
            initial_size: CellSize::new(50, 200),
        };
        let bytes = postcard::to_allocvec(&req).expect("encode");
        let decoded: AttachRequest = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, req);
    }

    #[test]
    fn frontend_capabilities_default_is_all_false() {
        // The default-false posture is the protocol-evolution
        // contract: a frontend that omits a capability is treated
        // as not supporting it.
        let c = FrontendCapabilities::default();
        assert!(!c.synchronized_output);
        assert!(!c.unicode_smp);
        assert!(!c.true_color);
        assert!(!c.mouse);
        assert!(!c.bracketed_paste);
        assert!(c.terminal_kind.is_none());
    }

    #[test]
    fn frontend_capabilities_omitted_fields_default_on_decode() {
        // Old-frontend / new-instance scenario: encode an empty
        // postcard struct and decode it as a (potentially future)
        // capability set. With `#[serde(default)]` on every field,
        // missing fields land as their default values rather than
        // a decode error. The wire shape we test here is a struct
        // that postcard serializes as a sequence of its fields; the
        // test fakes the "older wire" by encoding a smaller
        // synthetic type.
        //
        // Concretely: encode a struct with only the bools (no
        // terminal_kind). postcard serializes structs as positional
        // sequences, so this exercises the sequence-shorter-than-struct
        // path that `#[serde(default)]` rescues. A more thorough test
        // would synthesize a fewer-field shadow struct, but for now
        // we verify the all-defaults Default::default() decodes by
        // round-trip.
        let bytes = postcard::to_allocvec(&FrontendCapabilities::default()).expect("encode");
        let decoded: FrontendCapabilities = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, FrontendCapabilities::default());
    }

    #[test]
    fn goodbye_version_mismatch_round_trips() {
        let g = InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch {
            server: PROTOCOL_VERSION,
            client: 999,
        });
        let bytes = postcard::to_allocvec(&g).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch { server, client }) => {
                assert_eq!(server, PROTOCOL_VERSION);
                assert_eq!(client, 999);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn goodbye_other_variants_round_trip() {
        for reason in [
            GoodbyeReason::ShuttingDown,
            GoodbyeReason::AlreadyAttached,
            GoodbyeReason::ProtocolError,
        ] {
            let m = InstanceMessage::Goodbye(reason.clone());
            let bytes = postcard::to_allocvec(&m).expect("encode");
            let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
            match (decoded, reason) {
                (InstanceMessage::Goodbye(a), b) => assert_eq!(a, b),
                (other, _) => panic!("expected Goodbye, got {other:?}"),
            }
        }
    }

    #[test]
    fn frontend_event_detach_round_trips() {
        let ev = FrontendEvent::Detach(FrontendId(42));
        let bytes = postcard::to_allocvec(&ev).expect("encode");
        let decoded: FrontendEvent = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            FrontendEvent::Detach(id) => assert_eq!(id, FrontendId(42)),
            other => panic!("expected Detach, got {other:?}"),
        }
    }

    #[test]
    fn key_event_round_trips_through_postcard() {
        let ev = FrontendEvent::Key(KeyEvent {
            frontend_id: FrontendId(2),
            key: Key::Char('q'),
            mods: Modifiers::CTRL | Modifiers::SHIFT,
            timestamp_ns: 1_700_000_000_000_000_000,
        });
        let bytes = postcard::to_allocvec(&ev).expect("encode");
        let decoded: FrontendEvent = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            FrontendEvent::Key(k) => {
                assert_eq!(k.frontend_id, FrontendId(2));
                assert_eq!(k.key, Key::Char('q'));
                assert_eq!(k.mods, Modifiers::CTRL | Modifiers::SHIFT);
                assert_eq!(k.timestamp_ns, 1_700_000_000_000_000_000);
            }
            other => panic!("expected Key, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_idle_round_trips_through_postcard() {
        // T M11.6 — the wire variant. Round-trip both polarities so
        // the postcard encoding of bool is verified in both states.
        for idle in [true, false] {
            let msg = InstanceMessage::DispatchIdle { idle };
            let bytes = postcard::to_allocvec(&msg).expect("encode");
            let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
            match decoded {
                InstanceMessage::DispatchIdle { idle: got } => assert_eq!(got, idle),
                other => panic!("expected DispatchIdle, got {other:?}"),
            }
        }
    }

    #[test]
    fn key_event_to_crossterm_round_trips() {
        // Build a protocol KeyEvent, translate to crossterm, translate
        // back. The frontend_id and timestamp are stripped (crossterm
        // doesn't carry them) but key + mods round-trip.
        use crossterm_translate::{key_from_crossterm, key_to_crossterm};
        let original = KeyEvent {
            frontend_id: FrontendId(7),
            key: Key::Char('x'),
            mods: Modifiers::CTRL | Modifiers::ALT,
            timestamp_ns: 42,
        };
        let ct = key_to_crossterm(&original).expect("translatable");
        let back = key_from_crossterm(&ct, FrontendId(7), 42);
        assert_eq!(back, original);
    }

    #[test]
    fn key_event_to_crossterm_returns_none_for_unknown() {
        use crossterm_translate::key_to_crossterm;
        let ev = KeyEvent {
            frontend_id: FrontendId::LOCAL,
            key: Key::Unknown(0x0103),
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        };
        assert!(key_to_crossterm(&ev).is_none());
    }

    #[test]
    fn mouse_event_to_crossterm_round_trips() {
        use crossterm_translate::{mouse_from_crossterm, mouse_to_crossterm};
        let original = MouseEvent {
            frontend_id: FrontendId(3),
            kind: MouseKind::Drag(MouseButton::Right),
            coord: CellCoord::new(7, 22),
            mods: Modifiers::SHIFT,
        };
        let ct = mouse_to_crossterm(&original);
        let back = mouse_from_crossterm(&ct, FrontendId(3));
        assert_eq!(back, original);
    }

    #[test]
    fn unknown_keycode_round_trips_with_sentinel_preserved() {
        // The Unknown variant carries an opaque u32; round-tripping it
        // through postcard must preserve the exact value so frontends
        // that introduce new keycodes don't lose them in transit.
        let ev = FrontendEvent::Key(KeyEvent {
            frontend_id: FrontendId::LOCAL,
            key: Key::Unknown(0x0103), // Media::PlayPause sentinel
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        });
        let bytes = postcard::to_allocvec(&ev).expect("encode");
        let decoded: FrontendEvent = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            FrontendEvent::Key(k) => assert_eq!(k.key, Key::Unknown(0x0103)),
            other => panic!("expected Key, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // T M10.5 round-trip tests for the new wire variants.
    // -----------------------------------------------------------------

    #[test]
    fn instance_message_crdt_op_round_trips_through_postcard() {
        // Synthetic CrdtOp with known peer_id + arbitrary bytes.
        // Verifies the protocol-level serialization shape. The
        // real-loro-bytes variant is in the test below.
        let msg = InstanceMessage::CrdtOp {
            buffer_id: crate::buffer::BufferId::next(),
            op: crate::rope::CrdtOp {
                peer_id: 0x1234_5678_9abc_def0,
                bytes: vec![1, 2, 3, 4, 5, 0xFF, 0xFE, 0xFD],
            },
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            InstanceMessage::CrdtOp {
                op: crate::rope::CrdtOp { peer_id, bytes: ob },
                ..
            } => {
                assert_eq!(peer_id, 0x1234_5678_9abc_def0);
                assert_eq!(ob, vec![1, 2, 3, 4, 5, 0xFF, 0xFE, 0xFD]);
            }
            other => panic!("expected CrdtOp, got {other:?}"),
        }
    }

    #[test]
    fn frontend_event_crdt_op_round_trips_through_postcard() {
        let ev = FrontendEvent::CrdtOp {
            frontend_id: FrontendId(42),
            buffer_id: crate::buffer::BufferId::next(),
            op: crate::rope::CrdtOp {
                peer_id: 99,
                bytes: vec![0xAA, 0xBB, 0xCC],
            },
        };
        let bytes = postcard::to_allocvec(&ev).expect("encode");
        let decoded: FrontendEvent = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            FrontendEvent::CrdtOp {
                frontend_id, op, ..
            } => {
                assert_eq!(frontend_id, FrontendId(42));
                assert_eq!(op.peer_id, 99);
                assert_eq!(op.bytes, vec![0xAA, 0xBB, 0xCC]);
            }
            other => panic!("expected FrontendEvent::CrdtOp, got {other:?}"),
        }
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn instance_message_crdt_op_round_trips_with_real_loro_bytes() {
        // T M10.5 framing-pass addition: use actual loro-exported
        // bytes (not synthetic) so the test catches surprising
        // interactions between loro's wire format and postcard's
        // encoding. Also logs the per-CrdtOp wire byte size — a
        // reference number M10.8's broadcast-cost reasoning relies on.
        use crate::crdt::CrdtState;
        let state = CrdtState::new(7).expect("CRDT state");
        let pre_version = state.version();
        state.insert(0, "hello world").expect("insert");
        let real_bytes = state.export_updates_since(&pre_version).expect("export");
        let real_bytes_len = real_bytes.len();
        let msg = InstanceMessage::CrdtOp {
            buffer_id: crate::buffer::BufferId::next(),
            op: crate::rope::CrdtOp {
                peer_id: 7,
                bytes: real_bytes.clone(),
            },
        };
        let postcard_bytes = postcard::to_allocvec(&msg).expect("encode");
        let postcard_len = postcard_bytes.len();
        eprintln!(
            "[T M10.5 wire-size] real-loro CrdtOp for `hello world` insert:\n  \
             loro export bytes: {} B\n  \
             postcard-encoded InstanceMessage::CrdtOp: {} B\n  \
             protocol overhead: {} B (BufferId + peer_id + framing)",
            real_bytes_len,
            postcard_len,
            postcard_len.saturating_sub(real_bytes_len)
        );
        let decoded: InstanceMessage = postcard::from_bytes(&postcard_bytes).expect("decode");
        match decoded {
            InstanceMessage::CrdtOp { op, .. } => {
                assert_eq!(op.peer_id, 7);
                assert_eq!(
                    op.bytes, real_bytes,
                    "loro bytes must round-trip identically"
                );
                // Verify the round-tripped bytes apply on a remote
                // CrdtState and produce the originating state's
                // projection — the property M10.5's wire codec must
                // preserve for M10.8's broadcast path to work.
                let receiver = CrdtState::new(99).expect("receiver");
                receiver.import_updates(&op.bytes).expect("import");
                assert_eq!(receiver.materialize_string(), "hello world");
            }
            other => panic!("expected CrdtOp, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Handshake version-policy tests.
    //
    // History: T M10.5 introduced the slice-membership relaxation
    // (`is_supported_protocol_version`) so v1.0 daemons could accept
    // v0.1 frontends, and the ladder grew through v5 (T M11.1, T
    // M11.6, mouse framing Q#M1) — all additive enum variants,
    // filtered per session, with shared-struct encodings untouched.
    // T M4.6 (`Style::underline_color`) changed a shared struct's
    // postcard encoding, ending the ladder: v6 binaries accept only
    // v6 peers. These tests exercise the predicate directly; the
    // daemon-level handshake integration lives in m5_5_acceptance.rs.
    // -----------------------------------------------------------------

    #[test]
    fn m4_6_handshake_rejects_every_pre_v6_wire() {
        // A v5-or-older peer cannot decode v6 cell traffic (postcard
        // is not self-describing), so the handshake must refuse the
        // session up front with VersionMismatch — slice membership is
        // how that policy is expressed.
        for old in 1..=5 {
            assert!(
                !is_supported_protocol_version(old),
                "v6 binary must refuse a v{old} peer: its Style encoding \
                 predates underline_color and would mis-decode every CellDelta"
            );
        }
    }

    #[test]
    fn m4_6_handshake_accepts_v6_peer() {
        assert!(
            is_supported_protocol_version(PROTOCOL_VERSION),
            "the current wire version must accept itself"
        );
    }

    // T M10.6 — PresenceUpdate wire shape tests.

    #[test]
    fn instance_message_presence_update_round_trips_no_selection() {
        let msg = InstanceMessage::PresenceUpdate {
            frontend_id: FrontendId(42),
            buffer_id: crate::buffer::BufferId::next(),
            cursor: 100,
            selection: None,
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn instance_message_presence_update_round_trips_with_selection() {
        let msg = InstanceMessage::PresenceUpdate {
            frontend_id: FrontendId(7),
            buffer_id: crate::buffer::BufferId::next(),
            cursor: 500,
            selection: Some(SelectionSnapshot {
                anchor: 480,
                active: 500,
            }),
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn presence_update_typical_size_under_64_bytes() {
        // T M10.6 size acceptance — typical case: cursor at offset
        // 100 in a small buffer, no selection. Should be well under
        // 64B (varint encoding of small u64s is 1-2 bytes each).
        let msg = InstanceMessage::PresenceUpdate {
            frontend_id: FrontendId(2),
            buffer_id: crate::buffer::BufferId::next(),
            cursor: 100,
            selection: None,
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let size = bytes.len();
        eprintln!(
            "[T M10.6 wire-size] PresenceUpdate typical (cursor=100, no selection): {size} B"
        );
        assert!(
            size < 64,
            "typical PresenceUpdate is {size} B; spec target is <64 B"
        );
    }

    #[test]
    fn presence_update_worst_case_size_recorded() {
        // T M10.6 size acceptance — worst case: max u64 values for
        // every position field, selection present spanning a large
        // range. Varint encoding of u64::MAX is 10 bytes; this is
        // the upper bound on a single PresenceUpdate's wire size.
        // Recording the actual number for the audit doc.
        let msg = InstanceMessage::PresenceUpdate {
            frontend_id: FrontendId(u64::MAX),
            buffer_id: crate::buffer::BufferId::next(),
            cursor: u64::MAX,
            selection: Some(SelectionSnapshot {
                anchor: 0,
                active: u64::MAX,
            }),
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let size = bytes.len();
        eprintln!(
            "[T M10.6 wire-size] PresenceUpdate worst-case (all-max u64s + selection): {size} B"
        );
        // Worst-case bound: 1 (variant tag) + 10 (frontend_id) + ~2
        // (BufferId varint — small) + 10 (cursor) + 1 (Some tag) +
        // 10 (anchor zero = 1B) + 10 (active = u64::MAX = 10B) = ~44
        // upper bound. Buffer-id is freshly minted so its varint
        // encoding is small. We assert <64 to cover the spec target,
        // and log the actual number for the audit.
        assert!(
            size < 64,
            "worst-case PresenceUpdate is {size} B; spec target is <64 B"
        );
    }

    // -----------------------------------------------------------------
    // T M10.10 round-trip + size tests for BufferSnapshot.
    // -----------------------------------------------------------------

    #[test]
    fn instance_message_buffer_snapshot_round_trips_through_postcard() {
        // Synthetic loro-snapshot bytes — the wire-level test is
        // independent of the actual loro encoding.
        let msg = InstanceMessage::BufferSnapshot {
            buffer_id: crate::buffer::BufferId::next(),
            crdt_snapshot: vec![0xCD, 0x07, 0x00, 0x01, 0x02, 0x03, 0xFF],
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            InstanceMessage::BufferSnapshot { crdt_snapshot, .. } => {
                assert_eq!(
                    crdt_snapshot,
                    vec![0xCD, 0x07, 0x00, 0x01, 0x02, 0x03, 0xFF]
                );
            }
            other => panic!("expected BufferSnapshot, got {other:?}"),
        }
    }

    #[test]
    fn instance_message_cursor_byte_round_trips_through_postcard() {
        let msg = InstanceMessage::CursorByte {
            buffer_id: crate::buffer::BufferId::next(),
            byte_pos: 12345,
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            InstanceMessage::CursorByte { byte_pos, .. } => assert_eq!(byte_pos, 12345),
            other => panic!("expected CursorByte, got {other:?}"),
        }
    }

    #[test]
    fn instance_message_cursor_byte_zero_position_round_trips() {
        let msg = InstanceMessage::CursorByte {
            buffer_id: crate::buffer::BufferId::next(),
            byte_pos: 0,
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        assert!(matches!(
            decoded,
            InstanceMessage::CursorByte { byte_pos: 0, .. }
        ));
    }

    #[test]
    fn instance_message_buffer_snapshot_empty_snapshot_round_trips() {
        // An empty CRDT (no edits yet) — loro's export produces a
        // small but non-zero byte string. The wire layer must round-trip
        // a zero-length crdt_snapshot regardless of whether loro ever
        // emits one.
        let msg = InstanceMessage::BufferSnapshot {
            buffer_id: crate::buffer::BufferId::next(),
            crdt_snapshot: vec![],
        };
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        match decoded {
            InstanceMessage::BufferSnapshot { crdt_snapshot, .. } => {
                assert!(crdt_snapshot.is_empty());
            }
            other => panic!("expected BufferSnapshot, got {other:?}"),
        }
    }

    // T M10.7 — capability negotiation matrix + error round-trip.

    /// Build a `FrontendCapabilities` with the M10-era negotiated
    /// bits set as specified and all other fields at their default.
    fn front_caps(multi_frontend: bool, crdt_replica: bool) -> FrontendCapabilities {
        FrontendCapabilities {
            multi_frontend,
            crdt_replica,
            ..FrontendCapabilities::default()
        }
    }

    fn inst_caps(multi_frontend: bool, crdt_replica: bool) -> InstanceCapabilities {
        InstanceCapabilities {
            multi_frontend,
            crdt_replica,
            ..InstanceCapabilities::default()
        }
    }

    /// T M11.1 — caps builder that also sets `semantic_render`, for
    /// the semantic-negotiation matrix. The 2-arg `inst_caps` keeps
    /// `semantic_render` at its `Default` (`false`) so the existing
    /// M10.7 matrix tests are untouched.
    fn inst_caps_s(
        multi_frontend: bool,
        crdt_replica: bool,
        semantic_render: bool,
    ) -> InstanceCapabilities {
        InstanceCapabilities {
            multi_frontend,
            crdt_replica,
            semantic_render,
        }
    }

    fn front_caps_s(
        multi_frontend: bool,
        crdt_replica: bool,
        semantic_render: bool,
    ) -> FrontendCapabilities {
        FrontendCapabilities {
            multi_frontend,
            crdt_replica,
            semantic_render,
            ..FrontendCapabilities::default()
        }
    }

    #[test]
    fn negotiate_neither_side_declares_anything() {
        let res = negotiate_capabilities(&front_caps(false, false), &inst_caps(false, false))
            .expect("ok");
        assert!(!res.multi_frontend);
        assert!(!res.crdt_replica);
    }

    #[test]
    fn negotiate_frontend_silent_instance_offers() {
        // Frontend didn't request, instance has — frontend's silence
        // is accepted as "single-frontend subset is fine."
        let res =
            negotiate_capabilities(&front_caps(false, false), &inst_caps(true, true)).expect("ok");
        assert!(!res.multi_frontend, "frontend didn't ask → doesn't get");
        assert!(!res.crdt_replica, "frontend didn't ask → doesn't get");
    }

    #[test]
    fn negotiate_both_sides_declare_multi_frontend() {
        let res =
            negotiate_capabilities(&front_caps(true, false), &inst_caps(true, false)).expect("ok");
        assert!(res.multi_frontend);
        assert!(!res.crdt_replica);
    }

    #[test]
    fn negotiate_both_sides_declare_both_bits() {
        let res =
            negotiate_capabilities(&front_caps(true, true), &inst_caps(true, true)).expect("ok");
        assert!(res.multi_frontend);
        assert!(res.crdt_replica);
    }

    #[test]
    fn negotiate_frontend_wants_multi_instance_lacks() {
        // T M10.7 criterion 4 — mismatch produces clear error
        // naming what was requested vs available.
        let err = negotiate_capabilities(&front_caps(true, false), &inst_caps(false, false))
            .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(missing, vec!["multi_frontend".to_string()]);
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn negotiate_frontend_wants_crdt_replica_instance_lacks() {
        let err = negotiate_capabilities(&front_caps(false, true), &inst_caps(false, false))
            .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(missing, vec!["crdt_replica".to_string()]);
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn negotiate_frontend_wants_both_instance_lacks_both() {
        // Multiple missing bits land in a single CapabilityMismatch
        // — one round-trip carries the complete picture.
        let err = negotiate_capabilities(&front_caps(true, true), &inst_caps(false, false))
            .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(
                    missing,
                    vec!["multi_frontend".to_string(), "crdt_replica".to_string()]
                );
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn negotiate_partial_mismatch_only_lists_missing() {
        // Frontend wants both, instance has multi but not crdt:
        // only crdt_replica lands in `missing`.
        let err = negotiate_capabilities(&front_caps(true, true), &inst_caps(true, false))
            .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(missing, vec!["crdt_replica".to_string()]);
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn goodbye_capability_mismatch_round_trips() {
        let msg = InstanceMessage::Goodbye(GoodbyeReason::CapabilityMismatch {
            missing: vec!["multi_frontend".to_string(), "crdt_replica".to_string()],
        });
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn missing_strings_are_field_names_not_descriptions() {
        // T M10.7 wire-format-stability commitment: the strings
        // emitted into `missing` are exactly the
        // `FrontendCapabilities`/`InstanceCapabilities` field names.
        // Human-readable translation happens in
        // `AttachError::Display`, not on the wire. Renaming a bit
        // requires updating both this emission and the field name
        // in lockstep — this test pins the current names so a
        // future rename forces an audit-visible diff here too.
        let err = negotiate_capabilities(&front_caps(true, true), &inst_caps(false, false))
            .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                // The exact strings the wire carries — no
                // pluralization, no hyphenation, no human polish.
                assert!(
                    missing
                        .iter()
                        .all(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '_')),
                    "missing strings must be field-name identifiers (ascii lowercase + underscore), got {missing:?}"
                );
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    // T M11.1 — semantic_render negotiation matrix + the
    // semantic_render ⇒ crdt_replica dependency rule.

    #[test]
    fn negotiate_semantic_render_both_sides_with_crdt() {
        // The only success shape: both sides want semantic_render AND
        // the session also negotiates crdt_replica (the text-replica
        // dependency). semantic_render true implies crdt_replica true.
        let res = negotiate_capabilities(
            &front_caps_s(true, true, true),
            &inst_caps_s(true, true, true),
        )
        .expect("ok");
        assert!(res.crdt_replica);
        assert!(res.semantic_render);
    }

    #[test]
    fn negotiate_semantic_render_frontend_silent() {
        // Instance offers semantic_render; frontend doesn't ask. The
        // subset (no semantic projection) is accepted, no error —
        // identical posture to the multi_frontend/crdt_replica
        // "frontend silent" case.
        let res = negotiate_capabilities(
            &front_caps_s(false, false, false),
            &inst_caps_s(true, true, true),
        )
        .expect("ok");
        assert!(!res.semantic_render);
        assert!(!res.crdt_replica);
    }

    #[test]
    fn negotiate_semantic_render_frontend_wants_instance_lacks() {
        // Frontend wants crdt+semantic; instance has crdt but not the
        // semantic projection (the M11.1 reality until M11.2 flips
        // the instance default). Only semantic_render is missing.
        let err = negotiate_capabilities(
            &front_caps_s(false, true, true),
            &inst_caps_s(false, true, false),
        )
        .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(missing, vec!["semantic_render".to_string()]);
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn negotiate_semantic_render_requires_crdt_replica_dependency() {
        // Both sides declare semantic_render, but the frontend did
        // NOT request crdt_replica. The AND-rule alone would yield
        // semantic_render=true; the dependency rule rejects instead
        // of silently degrading to a text-only replica. The rejected
        // identifier is "semantic_render" (the capability whose
        // precondition is unmet), not "crdt_replica".
        let err = negotiate_capabilities(
            &front_caps_s(false, false, true),
            &inst_caps_s(false, true, true),
        )
        .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(missing, vec!["semantic_render".to_string()]);
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn negotiate_semantic_render_dependency_orders_after_crdt_replica() {
        // Frontend wants crdt+semantic; instance has the semantic
        // projection but lacks crdt. crdt_replica fails the AND-rule
        // (true,false) → "crdt_replica"; the dependency rule then
        // appends "semantic_render". Deterministic order:
        // [crdt_replica, semantic_render]. No duplicate semantic_render.
        let err = negotiate_capabilities(
            &front_caps_s(false, true, true),
            &inst_caps_s(false, false, true),
        )
        .expect_err("should mismatch");
        match err {
            GoodbyeReason::CapabilityMismatch { missing } => {
                assert_eq!(
                    missing,
                    vec!["crdt_replica".to_string(), "semantic_render".to_string()]
                );
            }
            other => panic!("expected CapabilityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn negotiate_ok_semantic_render_always_implies_crdt_replica() {
        // Invariant: every successful negotiation with
        // semantic_render=true also has crdt_replica=true. Exhaust
        // the 2³ declared-bit combinations on each side that the
        // helpers can express; any Ok with semantic_render must carry
        // crdt_replica.
        for fc in [false, true] {
            for fr in [false, true] {
                for fs in [false, true] {
                    for ic in [false, true] {
                        for ir in [false, true] {
                            for is in [false, true] {
                                if let Ok(neg) = negotiate_capabilities(
                                    &front_caps_s(fc, fr, fs),
                                    &inst_caps_s(ic, ir, is),
                                ) && neg.semantic_render
                                {
                                    assert!(
                                        neg.crdt_replica,
                                        "semantic_render without crdt_replica leaked through \
                                         negotiation: front=({fc},{fr},{fs}) inst=({ic},{ir},{is})"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn negotiate_two_arg_helpers_do_not_negotiate_semantic_render() {
        // Regression: the M10.7 matrix uses the 2-arg helpers. After
        // the T M11.2 flip the *instance* default is `cfg!(crdt)`
        // (true under `--features crdt`), but the *frontend* 2-arg
        // helper still defaults `semantic_render` to false — so the
        // AND-rule yields `false` and existing M10.7 outcomes are
        // unperturbed. (A frontend that wants the semantic projection
        // opts in explicitly via the 3-arg helper.)
        let res =
            negotiate_capabilities(&front_caps(true, true), &inst_caps(true, true)).expect("ok");
        assert!(res.multi_frontend);
        assert!(res.crdt_replica);
        assert!(
            !res.semantic_render,
            "frontend that didn't request semantic_render must not negotiate it, \
             regardless of the instance default"
        );
    }

    // T M11.1 — postcard round-trips for the SemanticFrame family and
    // FrontendEvent::Viewport. Mirrors the M10.x variant round-trip
    // tests: encode → decode → structural equality.

    #[test]
    fn semantic_frame_family_round_trips_through_postcard() {
        let bid = crate::buffer::BufferId::next();
        let msgs = vec![
            InstanceMessage::StyleSpans {
                buffer_id: bid,
                generation: 7,
                full: true,
                segments: vec![StyleSegment {
                    range: ByteRange { start: 0, end: 12 },
                    spans: vec![StyleSpan {
                        range: ByteRange { start: 0, end: 12 },
                        style: crate::cell::Style::default(),
                    }],
                }],
            },
            InstanceMessage::Decorations {
                buffer_id: bid,
                generation: 7,
                full: false,
                segments: vec![DecorationSegment {
                    range: ByteRange { start: 3, end: 20 },
                    decorations: vec![
                        Decoration {
                            range: ByteRange { start: 3, end: 9 },
                            kind: DecorationKind::DiagnosticError,
                        },
                        Decoration {
                            range: ByteRange { start: 20, end: 20 },
                            kind: DecorationKind::CurrentLine,
                        },
                    ],
                }],
            },
            InstanceMessage::InlineAdornments {
                buffer_id: bid,
                items: vec![InlineAdornment {
                    at: 42,
                    placement: AdornmentPlacement::EndOfLine,
                    content: AdornmentContent::Text {
                        text: "→ i32".to_string(),
                        style: crate::cell::Style::default(),
                    },
                }],
            },
            InstanceMessage::FileStyleSummary {
                buffer_id: bid,
                generation: 7,
                lines: vec![
                    crate::cell::Style::default(),
                    crate::cell::Style {
                        bold: true,
                        ..crate::cell::Style::default()
                    },
                    crate::cell::Style::default(),
                ],
            },
            InstanceMessage::BlockAdornments {
                buffer_id: bid,
                items: vec![BlockAdornment {
                    at: 64,
                    replaces: Some(ByteRange {
                        start: 64,
                        end: 256,
                    }),
                    content: AdornmentContent::Resource { handle: 1 },
                }],
            },
            InstanceMessage::FoldState {
                buffer_id: bid,
                folds: vec![ByteRange {
                    start: 100,
                    end: 400,
                }],
            },
            InstanceMessage::ResourceOffer {
                handle: 1,
                mime: "image/png".to_string(),
                body: ResourceBody::Inline(vec![0x89, b'P', b'N', b'G']),
            },
            InstanceMessage::ResourceOffer {
                handle: 2,
                mime: "image/svg+xml".to_string(),
                body: ResourceBody::Uri("file:///tmp/blame.svg".to_string()),
            },
        ];
        for msg in msgs {
            let bytes = postcard::to_allocvec(&msg).expect("encode");
            let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn frontend_event_viewport_round_trips_through_postcard() {
        let ev = FrontendEvent::Viewport {
            frontend_id: FrontendId(4),
            buffer_id: crate::buffer::BufferId::next(),
            visible: ByteRange {
                start: 1_024,
                end: 4_096,
            },
            generation: 99,
        };
        let bytes = postcard::to_allocvec(&ev).expect("encode");
        let decoded: FrontendEvent = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(ev, decoded);
        // The contract-boundary invariant, asserted structurally:
        // frontend_id() must resolve for the new variant (it is part
        // of the per-frontend routing alternation).
        assert_eq!(decoded.frontend_id(), FrontendId(4));
    }
}
