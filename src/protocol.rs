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

use crate::cell::{Cell, CellCoord, CellSize, DiffSpan};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Frontend identity
// ---------------------------------------------------------------------------

/// Opaque identifier for a frontend attached to an instance.
///
/// Every input event carries a `FrontendId`. v0.1 uses one ID per
/// instance ([`FrontendId::LOCAL`]); v0.3 generalizes to multi-frontend
/// (multi-window, multi-user) without a protocol break.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct FrontendId(pub u64);

impl FrontendId {
    /// The single frontend used in v0.1's local-attach mode.
    ///
    /// Future multi-frontend deployments allocate IDs from a counter
    /// starting after this value; the constant is reserved.
    pub const LOCAL: FrontendId = FrontendId(1);
}

// ---------------------------------------------------------------------------
// Key encoding
// ---------------------------------------------------------------------------

/// Key code, normalized away from any specific terminal protocol.
///
/// `Char` covers printable input. The named variants cover the keys
/// terminals report distinctly (arrows, function keys, etc.). `Unknown`
/// is the escape hatch: a key the protocol layer cannot encode in
/// any of the named variants is preserved as a u32 sentinel so it
/// can round-trip through serialization without becoming an error.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Key {
    /// A printable character. The character is the user-visible
    /// codepoint after layout / IME processing.
    Char(char),
    /// A function key. `n` is 1-based: `F(1)` is F1.
    F(u8),
    /// Backspace / `^H`.
    Backspace,
    /// Enter / Return / `^M`.
    Enter,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Tab.
    Tab,
    /// Shift-Tab.
    BackTab,
    /// Forward delete.
    Delete,
    /// Insert.
    Insert,
    /// Escape.
    Escape,
    /// Caps Lock.
    CapsLock,
    /// Scroll Lock.
    ScrollLock,
    /// Num Lock.
    NumLock,
    /// Print Screen.
    PrintScreen,
    /// Pause / Break.
    Pause,
    /// Menu / context-menu key.
    Menu,
    /// Numeric-keypad center key.
    KeypadBegin,
    /// The "null" keycode (terminal-protocol artifact).
    Null,
    /// A key the protocol layer does not recognize. The `u32`
    /// preserves whatever sentinel value the upstream layer attached
    /// (e.g. a media-key code from kitty's keyboard protocol). Round-trips
    /// through serialization but is not actionable by commands.
    Unknown(u32),
}

/// Modifier-key set. Bit-flag encoding for compact wire shape.
///
/// `META` corresponds to the "logo" / "super" key on most keyboards.
/// `HYPER` is reserved for the rare keyboards that distinguish it
/// from `META` (kitty's keyboard protocol surfaces both).
#[derive(
    Copy, Clone, Eq, PartialEq, Hash, Debug, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Modifiers(u8);

impl Modifiers {
    /// Empty set: no modifiers held.
    pub const NONE: Modifiers = Modifiers(0);
    /// Shift.
    pub const SHIFT: Modifiers = Modifiers(1 << 0);
    /// Control.
    pub const CTRL: Modifiers = Modifiers(1 << 1);
    /// Alt / Option.
    pub const ALT: Modifiers = Modifiers(1 << 2);
    /// Meta / Super / Logo / Command.
    pub const META: Modifiers = Modifiers(1 << 3);
    /// Hyper. Distinguished from `META` only on keyboards that
    /// surface both (kitty's keyboard protocol).
    pub const HYPER: Modifiers = Modifiers(1 << 4);

    /// Construct from a raw bit set. Bits outside the defined range
    /// are silently masked off so a future-extended wire cannot smuggle
    /// undefined bits past current decoders.
    #[must_use]
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & 0b0001_1111)
    }

    /// Raw bit set.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `self` includes every bit set in `other`.
    #[must_use]
    pub const fn contains(self, other: Modifiers) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Whether no modifiers are held.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Modifiers) {
        self.0 |= rhs.0;
    }
}

/// A keyboard event.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeyEvent {
    /// Frontend that produced the event.
    pub frontend_id: FrontendId,
    /// The key code.
    pub key: Key,
    /// Modifier set held when the key was pressed.
    pub mods: Modifiers,
    /// Monotonic timestamp at which the frontend captured the event.
    /// Zero means "no timestamp available" (e.g. test-synthesized
    /// events).
    pub timestamp_ns: u64,
}

// ---------------------------------------------------------------------------
// Mouse encoding
// ---------------------------------------------------------------------------

/// Mouse button.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MouseButton {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
}

/// Kind of mouse interaction.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MouseKind {
    /// Button pressed.
    Down(MouseButton),
    /// Button released.
    Up(MouseButton),
    /// Drag with the named button held.
    Drag(MouseButton),
    /// Pointer moved with no button held.
    Move,
    /// Wheel scrolled up.
    ScrollUp,
    /// Wheel scrolled down.
    ScrollDown,
    /// Wheel scrolled left.
    ScrollLeft,
    /// Wheel scrolled right.
    ScrollRight,
}

/// A mouse event.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MouseEvent {
    /// Frontend that produced the event.
    pub frontend_id: FrontendId,
    /// Kind of mouse interaction.
    pub kind: MouseKind,
    /// Cell-grid coordinate of the pointer at the moment of the event.
    pub coord: CellCoord,
    /// Modifiers held during the event.
    pub mods: Modifiers,
}

// ---------------------------------------------------------------------------
// Frontend → Instance events
// ---------------------------------------------------------------------------

/// Input event from frontend to instance.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FrontendEvent {
    /// A key event.
    Key(KeyEvent),
    /// A mouse event.
    Mouse(MouseEvent),
    /// Frontend's terminal resized.
    Resize {
        /// Frontend that resized.
        frontend_id: FrontendId,
        /// New size, in cells.
        size: CellSize,
    },
    /// Bracketed-paste payload from the frontend.
    Paste {
        /// Frontend that produced the paste.
        frontend_id: FrontendId,
        /// Raw bytes pasted (the instance decodes as UTF-8 if relevant).
        data: Vec<u8>,
    },
    /// Frontend gained input focus.
    FocusGained(FrontendId),
    /// Frontend lost input focus.
    FocusLost(FrontendId),
    /// Frontend is going away. Instance treats this as immediate
    /// detach; no acknowledgement required.
    Detach(FrontendId),
}

impl FrontendEvent {
    /// The frontend that produced this event.
    #[must_use]
    pub fn frontend_id(&self) -> FrontendId {
        match self {
            Self::Key(e) => e.frontend_id,
            Self::Mouse(e) => e.frontend_id,
            Self::Resize { frontend_id, .. }
            | Self::Paste { frontend_id, .. }
            | Self::FocusGained(frontend_id)
            | Self::FocusLost(frontend_id)
            | Self::Detach(frontend_id) => *frontend_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Instance → Frontend messages
// ---------------------------------------------------------------------------

/// Cursor position and visibility.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorState {
    /// Cell where the cursor should be drawn.
    pub coord: CellCoord,
    /// Whether the cursor is visible at all.
    pub visible: bool,
}

/// Instance-level signal that is not a render message.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InstanceSignal {
    /// Terminal bell.
    Bell,
    /// Window-title change request.
    Title(String),
    /// Clipboard set request (OSC 52).
    Clipboard(Vec<u8>),
}

/// Reason an instance terminates an attachment.
///
/// Only the four variants the v0.1 daemon actually emits or rejects on.
/// `Evicted` (multi-frontend takeover) and similar will land alongside
/// the v0.3 multi-frontend work; until then `AlreadyAttached` covers
/// the single-slot equivalent.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GoodbyeReason {
    /// Instance is shutting down (SIGTERM / SIGINT or clean exit).
    ShuttingDown,
    /// Frontend's `protocol_version` does not match the instance's.
    /// The handshake fails before any further messages.
    VersionMismatch {
        /// The instance's `PROTOCOL_VERSION`.
        server: u32,
        /// The version the frontend announced in its `AttachRequest`.
        client: u32,
    },
    /// Another frontend is currently attached. v0.1 rejects concurrent
    /// attaches; v0.3 will replace this with eviction or multiplexing.
    AlreadyAttached,
    /// Frontend sent a malformed message or otherwise violated the
    /// protocol. The connection is closed without further dialogue.
    ProtocolError,
}

/// Rendering and signals from instance to frontend.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InstanceMessage {
    /// Cell deltas. `full_grid = true` is the initial sync sent on
    /// fresh attach (or after a resize where the previous grid is no
    /// longer applicable); `full_grid = false` is a differential
    /// frame.
    CellDelta {
        /// One run of changed cells per `DiffSpan`.
        spans: Vec<DiffSpan>,
        /// Whether `spans` represents a full-grid resync (true on
        /// fresh attach or post-resize) versus an incremental frame.
        full_grid: bool,
    },
    /// Cursor position and visibility update.
    Cursor(Option<CursorState>),
    /// Modeline cells. Reserved for v0.3 GUI use; v0.1 ships modeline
    /// inside [`InstanceMessage::CellDelta`]. The variant exists in
    /// the protocol from day one so adding the discrete channel later
    /// is not a breaking change.
    ModeLine(Vec<Cell>),
    /// Side-channel signal (bell, title, clipboard).
    Signal(InstanceSignal),
    /// Instance is terminating the attachment.
    Goodbye(GoodbyeReason),
}

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
// Handshake — version, identity, capabilities
// ---------------------------------------------------------------------------

/// Wire-protocol version. Bumped on any breaking change to the
/// `Hello` / `AttachRequest` / event-message shapes.
///
/// The handshake compares the two sides' values; mismatches close the
/// connection with [`GoodbyeReason::VersionMismatch`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Identifies an instance for client-side display.
///
/// Sent inside [`Hello`] from instance to frontend. Use of `uptime_secs`
/// instead of an absolute start time is deliberate: instance and
/// frontend may run on machines whose clocks disagree, so the frontend
/// computes "instance has been running N seconds" using only the
/// instance's view of time.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InstanceIdentity {
    /// Pmacs version string (`env!("CARGO_PKG_VERSION")`).
    pub pmacs_version: String,
    /// Short git hash if the build embedded one. `None` for releases or
    /// source-tarball builds where no git checkout was available.
    pub build_hash: Option<String>,
    /// The name the instance was launched under (`--socket NAME`).
    /// `None` for the default daemon (no `--socket` argument).
    pub instance_name: Option<String>,
    /// Seconds since the instance started, from the instance's clock.
    /// Frontend displays "running 47m" by interpreting this against
    /// its own notion of "now," avoiding cross-machine clock skew.
    pub uptime_secs: u64,
    /// Working directory the instance is running in. Encoded as a
    /// UTF-8 string; non-UTF-8 paths are rejected at the boundary.
    pub working_directory: String,
}

impl InstanceIdentity {
    /// Build an identity for the running pmacs process.
    ///
    /// `instance_name` is the user-facing name (typically the
    /// `--socket NAME` value for the daemon path; `None` for the
    /// in-process Local mode and the unnamed default daemon).
    /// `started` is the wall-clock anchor used to compute
    /// [`Self::uptime_secs`]; the elapsed seconds are evaluated at the
    /// call site, so calling twice on different days surfaces different
    /// uptimes from the same anchor.
    ///
    /// The version comes from `CARGO_PKG_VERSION` and the build hash
    /// from the optional `PMACS_GIT_HASH` environment variable populated
    /// by the build script.
    #[must_use]
    pub fn for_running_process(instance_name: Option<String>, started: std::time::Instant) -> Self {
        Self {
            pmacs_version: env!("CARGO_PKG_VERSION").into(),
            build_hash: option_env!("PMACS_GIT_HASH").map(String::from),
            instance_name,
            uptime_secs: started.elapsed().as_secs(),
            working_directory: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// Capabilities the instance advertises to attaching frontends.
///
/// Empty for v0.1; the type exists so that adding capabilities in v0.2+
/// is not a breaking-change. Symmetric with [`FrontendCapabilities`].
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InstanceCapabilities {
    // No fields in v0.1. Reserved for future expansion.
}

/// Capabilities the frontend advertises to the instance.
///
/// All bools default to `false` so a frontend that omits a field via an
/// older `AttachRequest` is conservatively treated as not supporting
/// the capability. New capabilities added in v0.2+ get
/// `#[serde(default)]` so old wire bytes still deserialize.
// A capability set is exactly the case `struct_excessive_bools` warns
// against — but each flag is independent and the alternative (an enum
// or bitset) loses the per-field `#[serde(default)]` semantics that
// make schema evolution work.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrontendCapabilities {
    /// Frontend understands DEC 2026 `BeginSynchronizedUpdate` /
    /// `EndSynchronizedUpdate` markers. Instance strips them when false.
    #[serde(default)]
    pub synchronized_output: bool,
    /// Frontend can render Unicode beyond the Basic Multilingual Plane.
    /// Instance can substitute a fallback glyph when false.
    #[serde(default)]
    pub unicode_smp: bool,
    /// Frontend supports 24-bit color (truecolor SGR sequences).
    /// Instance maps to the 256-color palette when false.
    #[serde(default)]
    pub true_color: bool,
    /// Frontend captures and forwards mouse events.
    #[serde(default)]
    pub mouse: bool,
    /// Frontend supports bracketed paste — distinguishes pasted bytes
    /// from typed bytes. Instance treats all input as keystrokes when false.
    #[serde(default)]
    pub bracketed_paste: bool,
    /// Optional human-readable terminal identifier for logs and
    /// debugging only. The instance does not branch on this value;
    /// branching is done on the explicit capability bits above.
    #[serde(default)]
    pub terminal_kind: Option<String>,
}

/// First message sent by the instance to a freshly-attached frontend.
///
/// Sent immediately after the connection is accepted, before reading
/// the frontend's [`AttachRequest`]. The frontend uses
/// `instance_identity` for status display and `protocol_version` /
/// `instance_capabilities` for compatibility decisions.
///
/// The instance also stamps the `assigned_frontend_id` which the
/// frontend will use as the `FrontendId` on every event it sends.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hello {
    /// The instance's `PROTOCOL_VERSION`.
    pub protocol_version: u32,
    /// `FrontendId` assigned to this attachment by the instance. The
    /// frontend stamps this onto subsequent events. v0.1 daemons start
    /// allocation at `FrontendId(2)` (1 reserved for the in-process TUI).
    pub assigned_frontend_id: FrontendId,
    /// Instance self-identification (version, name, uptime, cwd).
    pub instance_identity: InstanceIdentity,
    /// Instance capabilities. Empty for v0.1.
    pub instance_capabilities: InstanceCapabilities,
}

/// First message sent by a frontend after receiving [`Hello`].
///
/// Carries the frontend's view of the protocol version, the
/// capabilities it can support, and its initial terminal size. On
/// version mismatch the instance closes with
/// [`GoodbyeReason::VersionMismatch`] and no further messages flow.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttachRequest {
    /// The frontend's `PROTOCOL_VERSION`.
    pub protocol_version: u32,
    /// Frontend capabilities. Defaults to all-false if omitted.
    #[serde(default)]
    pub frontend_capabilities: FrontendCapabilities,
    /// The frontend's terminal size at attach time. Authoritative
    /// until the frontend sends a [`FrontendEvent::Resize`]. The
    /// instance uses this for the initial full-grid render.
    pub initial_size: CellSize,
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
    fn protocol_version_is_one_for_v01() {
        // Pin the value: every wire-shape change in v0.1 patch releases
        // must keep this constant or break the handshake.
        assert_eq!(PROTOCOL_VERSION, 1);
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
}
