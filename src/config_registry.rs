// config_registry.rs --- Typed, two-scope, introspectable configuration registry.

//! Configuration settings.
//!
//! Per `docs/config-registry-framing.md` (revision 3), every editor
//! setting is a named, typed [`ConfigDefinition`] registered in a
//! [`ConfigRegistry`], mirroring [`crate::command::CommandRegistry`]
//! and [`crate::hook::HookRegistry`] in shape and error vocabulary:
//! R42 (mandatory description), duplicate-rejection-over-silent-
//! overwrite, and a [`SourceLocation`] captured at definition time.
//! R50 (unknown-field rejection) is enforced by the Lua bindings lane
//! for this registry, since it is the layer that sees the raw spec
//! table --- see [`ConfigError::UnknownField`].
//!
//! # Scopes
//!
//! Exactly two: a global override and, per [`BufferId`], a
//! buffer-local override. [`ConfigRegistry::get`] resolves
//! buffer-local -> global -> default when given a buffer, and
//! global -> default only when given `None` --- there is no ambient
//! "current buffer" at this layer (Q#CR4, F9).
//!
//! # Storage versus change notification (F1)
//!
//! These are two different questions, and conflating them breaks the
//! flagship buffer-local-pin feature:
//!
//! * [`ConfigRegistry::set`] and [`ConfigRegistry::set_local`]
//!   **always store** the override, even when it is equal to the
//!   value it shadows.
//! * The value epoch advances, and a listener dispatch should happen,
//!   **only when the effective value changes** --- reported back via
//!   [`ConfigChange::changed`].
//!
//! Concretely: with `editing.auto-pair` globally `true`, pinning one
//! buffer via `set_local(buf, "editing.auto-pair", true)` must still
//! record an override for that buffer, so that a later
//! `set("editing.auto-pair", false)` does not silently flip the
//! pinned buffer. `equal_valued_local_override_is_still_stored_and_shields_buffer`
//! in the tests below is written to fail against a "true no-op"
//! implementation that declines to store the equal-valued override.
//!
//! # The value-validation seam
//!
//! This module owns *value-level* validation: a [`ConfigValue`]
//! against a [`ConfigKind`] (type match, bounds, enum membership,
//! string emptiness, number finiteness). The Lua bindings lane owns
//! *Lua-level* validation: converting a raw Lua value into a
//! [`ConfigValue`], strict spec-table parsing, and R50 rejection.
//! [`ConfigRegistry::validate`] is the seam --- it lets the bindings
//! lane check a candidate value against an already-registered
//! definition without committing anything, and [`ConfigRegistry::set`]
//! / [`ConfigRegistry::set_local`] call the same path internally, so
//! there is exactly one place the check can drift. For the one
//! numeric-exactness question that must be identical across both Lua
//! backends (`LuaJIT` numbers are always `f64`; Lua 5.4 numbers may
//! carry a native integer subtype), [`ConfigValue::int_from_f64`]
//! gives the bindings lane a by-value exactness check that never
//! consults `math.type`.
//!
//! # Listeners
//!
//! [`ConfigRegistry::on_change`] registers a callback against a
//! setting name and returns a generation-safe `u64` id;
//! [`ConfigRegistry::dispose`] removes it, idempotently. This registry
//! never invokes a listener itself --- [`ConfigRegistry::snapshot`]
//! hands the caller an owned `Vec<ConfigListener>` so it can drop the
//! registry borrow before re-entering Lua, exactly as
//! [`crate::hook::HookRegistry::snapshot`] does. Listeners persist
//! until explicitly disposed; there is no GC-timed lifetime (Q#CR6,
//! F3) --- a dropped-but-undisposed handle keeps firing.
//!
//! # Mutability and the startup freeze
//!
//! A [`ConfigMutability::StartupOnly`] key accepts writes until
//! [`ConfigRegistry::freeze`] is called (at `set_init_complete` time)
//! and rejects them after, via [`ConfigError::StartupOnlyAfterFreeze`].
//! `StartupOnly` and [`ConfigRegistry::set_local`] are mutually
//! exclusive: a buffer-local write against a `StartupOnly` key is
//! always rejected with [`ConfigError::StartupOnlyLocal`], independent
//! of freeze state, because buffer-locals are only ever set at
//! runtime, long after any freeze (Q#CR10, F5).
//!
//! # Buffer-local lifecycle
//!
//! [`ConfigRegistry::remove_buffer`] drops a buffer's entire local
//! map. It fires no listener (Q#CR6 (c)): the buffer is gone, so there
//! is no effective value left for anyone to observe. Callers that skip
//! this (a direct buffer-registry removal bypassing the normal
//! choke point) leak that buffer's locals permanently but harmlessly,
//! since `BufferId`s are never reused (Q#CR5, F8) --- this module does
//! not and cannot fix that; it only owns the happy path.
//!
//! # Threading
//!
//! Single-threaded, like the command / hook registries. Lives behind
//! `Rc<RefCell<...>>` as Lua app data.

use std::collections::{HashMap, HashSet};

use mlua::Function;
use thiserror::Error;

use crate::buffer::BufferId;
use crate::command::SourceLocation;

// ---------------------------------------------------------------------------
// Value vocabulary
// ---------------------------------------------------------------------------

/// The closed value-kind vocabulary (Q#CR3). Deliberately has no
/// list/table variant --- `string-list` was dropped from stage 1
/// (framing F6) and table-valued settings are a deferred arc.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigKind {
    /// A `true`/`false` value.
    Boolean,
    /// A signed 64-bit integer, with optional inclusive bounds.
    Integer {
        /// Inclusive lower bound, if any.
        min: Option<i64>,
        /// Inclusive upper bound, if any.
        max: Option<i64>,
    },
    /// A finite `f64`, with optional inclusive bounds. Bounds must
    /// themselves be finite (checked at `define` time).
    Number {
        /// Inclusive lower bound, if any.
        min: Option<f64>,
        /// Inclusive upper bound, if any.
        max: Option<f64>,
    },
    /// A UTF-8 string.
    String {
        /// Whether an empty string is a valid value.
        allow_empty: bool,
    },
    /// A closed set of string choices. Values are stored as
    /// [`ConfigValue::Str`] and validated against `choices`.
    Enum {
        /// The valid choices. Must be non-empty and duplicate-free
        /// (checked at `define` time).
        choices: Vec<String>,
    },
}

impl ConfigKind {
    /// Stable type-name string used in error messages. Matches
    /// [`ConfigValue::type_name`]'s vocabulary except for `Enum`,
    /// whose values are physically strings but whose *kind* is more
    /// useful to name than its storage representation.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Integer { .. } => "integer",
            Self::Number { .. } => "number",
            Self::String { .. } => "string",
            Self::Enum { .. } => "enum",
        }
    }

    /// Validate that the kind's own constraints are internally
    /// consistent, independent of any candidate value: finite bounds,
    /// `min <= max`, and (for `Enum`) duplicate-free choices. Called
    /// once at `define` time, before the default is checked against
    /// the kind.
    fn validate_self(&self, name: &str) -> Result<(), ConfigError> {
        match self {
            Self::Boolean | Self::String { .. } => Ok(()),
            Self::Integer { min, max } => {
                if let (Some(lo), Some(hi)) = (min, max)
                    && lo > hi
                {
                    return Err(ConfigError::OutOfRange {
                        name: name.to_owned(),
                        detail: format!("minimum {lo} exceeds maximum {hi}"),
                    });
                }
                Ok(())
            }
            Self::Number { min, max } => {
                for bound in [*min, *max].into_iter().flatten() {
                    if !bound.is_finite() {
                        return Err(ConfigError::NonFiniteNumber {
                            name: name.to_owned(),
                            value: bound,
                        });
                    }
                }
                if let (Some(lo), Some(hi)) = (min, max)
                    && lo > hi
                {
                    return Err(ConfigError::OutOfRange {
                        name: name.to_owned(),
                        detail: format!("minimum {lo} exceeds maximum {hi}"),
                    });
                }
                Ok(())
            }
            Self::Enum { choices } => {
                // Rejected directly rather than left to the default's
                // own `NotAChoice` failure: an empty choice list is a
                // malformed *definition*, and reporting it as "the
                // default is not one of []" sends the author looking at
                // the wrong field.
                if choices.is_empty() {
                    return Err(ConfigError::EmptyChoices {
                        name: name.to_owned(),
                    });
                }
                let mut seen = HashSet::new();
                for choice in choices {
                    if !seen.insert(choice.as_str()) {
                        return Err(ConfigError::DuplicateChoice {
                            name: name.to_owned(),
                            choice: choice.clone(),
                        });
                    }
                }
                Ok(())
            }
        }
    }

    /// Validate `value` against this kind: type match, numeric bounds,
    /// enum membership, string emptiness, and number finiteness. Pure
    /// value-vs-kind logic with no knowledge of scope --- resolving
    /// which layer a value lives in is [`ConfigRegistry`]'s job. This
    /// is the half of validation this module owns (see the module
    /// doc's "value-validation seam" section); the Lua bindings lane
    /// calls it (indirectly, via [`ConfigRegistry::validate`]) after
    /// converting a raw Lua value into a [`ConfigValue`].
    pub fn validate(&self, name: &str, value: &ConfigValue) -> Result<(), ConfigError> {
        match (self, value) {
            (Self::Boolean, ConfigValue::Bool(_)) => Ok(()),
            (Self::Integer { min, max }, ConfigValue::Int(v)) => {
                if let Some(lo) = min
                    && v < lo
                {
                    return Err(ConfigError::OutOfRange {
                        name: name.to_owned(),
                        detail: format!("{v} is below the minimum {lo}"),
                    });
                }
                if let Some(hi) = max
                    && v > hi
                {
                    return Err(ConfigError::OutOfRange {
                        name: name.to_owned(),
                        detail: format!("{v} is above the maximum {hi}"),
                    });
                }
                Ok(())
            }
            (Self::Number { min, max }, ConfigValue::Num(v)) => {
                if !v.is_finite() {
                    return Err(ConfigError::NonFiniteNumber {
                        name: name.to_owned(),
                        value: *v,
                    });
                }
                if let Some(lo) = min
                    && v < lo
                {
                    return Err(ConfigError::OutOfRange {
                        name: name.to_owned(),
                        detail: format!("{v} is below the minimum {lo}"),
                    });
                }
                if let Some(hi) = max
                    && v > hi
                {
                    return Err(ConfigError::OutOfRange {
                        name: name.to_owned(),
                        detail: format!("{v} is above the maximum {hi}"),
                    });
                }
                Ok(())
            }
            (Self::String { allow_empty }, ConfigValue::Str(s)) => {
                if !allow_empty && s.is_empty() {
                    return Err(ConfigError::EmptyString {
                        name: name.to_owned(),
                    });
                }
                Ok(())
            }
            (Self::Enum { choices }, ConfigValue::Str(s)) => {
                if choices.iter().any(|c| c == s) {
                    Ok(())
                } else {
                    Err(ConfigError::NotAChoice {
                        name: name.to_owned(),
                        got: s.clone(),
                        choices: choices.clone(),
                    })
                }
            }
            _ => Err(ConfigError::TypeMismatch {
                name: name.to_owned(),
                expected: self.type_name(),
                got: value.type_name(),
            }),
        }
    }
}

/// An owned configuration value. Lua tables, functions and userdata
/// are never stored --- only these four scalars (Q#CR3).
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    /// A boolean value.
    Bool(bool),
    /// A signed 64-bit integer value.
    Int(i64),
    /// A finite `f64` value.
    Num(f64),
    /// A string value. Also used for `Enum`-kind values.
    Str(String),
}

impl ConfigValue {
    /// Stable type-name string used in error messages. Matches
    /// [`ConfigKind::type_name`]'s vocabulary, except an `Enum`
    /// definition's values report as `"string"` here since that is
    /// their physical representation.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "boolean",
            Self::Int(_) => "integer",
            Self::Num(_) => "number",
            Self::Str(_) => "string",
        }
    }

    /// Construct a [`Self::Int`] from an `f64`, checking exactness
    /// **by value**. This is the seam the Lua bindings lane needs for
    /// cross-backend-identical integer handling: `LuaJIT` numbers are
    /// always `f64` (Lua 5.1 has no integer subtype), Lua 5.4 numbers
    /// may carry a native integer subtype --- and this function never
    /// looks at which backend produced `v` or what `math.type` would
    /// say, only at the numeric value itself, so both backends agree
    /// byte-for-byte.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NonFiniteNumber`] if `v` is `NaN` or infinite;
    /// [`ConfigError::NonIntegral`] if `v` has a fractional part or
    /// falls outside the range exactly representable as an `i64`.
    pub fn int_from_f64(name: &str, v: f64) -> Result<Self, ConfigError> {
        if !v.is_finite() {
            return Err(ConfigError::NonFiniteNumber {
                name: name.to_owned(),
                value: v,
            });
        }
        // The bounds are deliberately ASYMMETRIC. `i64::MIN as f64` is
        // -2^63, which round-trips exactly, so `<` correctly admits it.
        // `i64::MAX as f64` rounds UP to 2^63 --- one more than
        // `i64::MAX` --- so a `>` here would admit exactly 2^63 and then
        // `v as i64` would saturate it to `i64::MAX`, silently storing a
        // different number than the caller wrote. `>=` rejects it.
        if v.fract() != 0.0 || v < i64::MIN as f64 || v >= i64::MAX as f64 {
            return Err(ConfigError::NonIntegral {
                name: name.to_owned(),
                value: v,
            });
        }
        Ok(Self::Int(v as i64))
    }
}

/// When a definition's value may be written (Q#CR10).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConfigMutability {
    /// Writable at any time, including after the startup freeze.
    Live,
    /// Writable only while user config is loading. A write after
    /// [`ConfigRegistry::freeze`] returns
    /// [`ConfigError::StartupOnlyAfterFreeze`]. Mutually exclusive
    /// with [`ConfigRegistry::set_local`], which always returns
    /// [`ConfigError::StartupOnlyLocal`] for a key of this mutability
    /// (F5): buffer-locals are only ever set at runtime, long after
    /// any freeze.
    StartupOnly,
}

// ---------------------------------------------------------------------------
// Definitions and listeners
// ---------------------------------------------------------------------------

/// A registered configuration setting.
#[derive(Clone, Debug)]
pub struct ConfigDefinition {
    /// Unique, dotted, kebab-cased name (e.g. `editing.auto-pair`).
    pub name: String,
    /// One-line human-readable description (R42, mandatory,
    /// non-empty after trim).
    pub description: String,
    /// The value's type and constraints.
    pub kind: ConfigKind,
    /// The value used when no override applies at any scope.
    pub default: ConfigValue,
    /// When the value may be written.
    pub mutability: ConfigMutability,
    /// Where the call to `pmacs.config.define` originated.
    pub source: SourceLocation,
}

/// One callback registered against a setting name via
/// [`ConfigRegistry::on_change`].
///
/// Cloning is cheap: `String`s clone trivially and `mlua::Function` is
/// reference-counted internally.
#[derive(Clone)]
pub struct ConfigListener {
    /// Generation-safe id, never reused. Returned by `on_change` and
    /// consumed by [`ConfigRegistry::dispose`].
    pub id: u64,
    /// The setting name this listener watches.
    pub name: String,
    /// The Lua callback body, invoked as `function(new, old, buf)`.
    pub body: Function,
    /// Where the call to `pmacs.config.on_change` originated.
    pub source: SourceLocation,
}

/// Outcome of a [`ConfigRegistry::set`], [`ConfigRegistry::set_local`]
/// or [`ConfigRegistry::reset`] call: the effective value immediately
/// before and after, and whether it actually changed (F1). The
/// override itself is always stored (or, for `reset`, dropped)
/// regardless of `changed` --- only epoch advancement and listener
/// dispatch key on it. A caller dispatching `on_change` uses `new`/
/// `old` directly as the listener's `(new, old, buf)` arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigChange {
    /// `true` iff the effective value differs from before the call.
    pub changed: bool,
    /// The effective value immediately before this call.
    pub old: ConfigValue,
    /// The effective value immediately after this call. Equal to
    /// `old` when `changed` is `false`.
    pub new: ConfigValue,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors raised by the config registry.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// `pmacs.config.define` was called with no name.
    #[error("config name must be non-empty")]
    EmptyName,

    /// Q#CR9: the name failed the dotted, kebab-cased grammar
    /// (`[a-z][a-z0-9]*(-[a-z0-9]+)*` per dot-separated segment) or
    /// exceeded the 128-byte length bound.
    #[error("config name \"{name}\" is invalid: {reason}")]
    InvalidName {
        /// The offending name.
        name: String,
        /// Which grammar rule it broke.
        reason: String,
    },

    /// R42: `define` was called without a description, or with one
    /// that's empty after trimming.
    #[error("config \"{name}\" requires a non-empty description (R42)")]
    MissingDescription {
        /// The offending config name.
        name: String,
    },

    /// A config with this name is already defined with a different
    /// specification. A byte-for-byte identical redefinition is
    /// idempotent and does **not** raise this (Q#CR10).
    #[error(
        "config \"{name}\" is already defined with a different specification (refusing to redefine)"
    )]
    DuplicateName {
        /// The offending config name.
        name: String,
    },

    /// `get`/`set`/`set_local`/`reset`/`is_set`/`on_change` referenced
    /// a name that has not been `define`d (Q#CR10: define-before-use).
    #[error("config \"{name}\" is not defined")]
    NotFound {
        /// The offending config name.
        name: String,
    },

    /// A value's runtime type does not match its definition's
    /// [`ConfigKind`].
    #[error("config \"{name}\" expects a {expected} value, got {got}")]
    TypeMismatch {
        /// The offending config name.
        name: String,
        /// The kind's declared type name.
        expected: &'static str,
        /// The candidate value's type name.
        got: &'static str,
    },

    /// An integer or number value fell outside its definition's
    /// declared `min`/`max`, or the bounds themselves are inverted
    /// (`min > max`) at define time.
    #[error("config \"{name}\" is out of range: {detail}")]
    OutOfRange {
        /// The offending config name.
        name: String,
        /// A human-readable description of which bound was broken.
        detail: String,
    },

    /// An enum-kind value is not one of its definition's `choices`.
    #[error("config \"{name}\" value \"{got}\" is not one of: {choices:?}")]
    NotAChoice {
        /// The offending config name.
        name: String,
        /// The rejected value.
        got: String,
        /// The definition's full choice list.
        choices: Vec<String>,
    },

    /// An enum-kind definition listed the same choice twice. Not part
    /// of the caller-given error vocabulary; added because the
    /// framing's acceptance list (item 2) explicitly requires
    /// rejecting this at define time and none of the other variants
    /// name it precisely.
    #[error("config \"{name}\" enum choices contain a duplicate: \"{choice}\"")]
    DuplicateChoice {
        /// The offending config name.
        name: String,
        /// The choice that appeared more than once.
        choice: String,
    },

    /// An enum-kind definition listed no choices at all. Rejected
    /// directly so the author is pointed at `choices`, rather than
    /// indirectly via the default failing `NotAChoice` against an empty
    /// list.
    #[error("config \"{name}\" is an enum with no choices; `choices` must be non-empty")]
    EmptyChoices {
        /// The offending config name.
        name: String,
    },

    /// A string-kind value was empty and the definition's
    /// `allow_empty` is `false`.
    #[error("config \"{name}\" does not allow an empty string")]
    EmptyString {
        /// The offending config name.
        name: String,
    },

    /// A number-kind value, or a number-kind bound, was not finite
    /// (`NaN` or infinite).
    #[error("config \"{name}\" requires a finite number, got {value}")]
    NonFiniteNumber {
        /// The offending config name.
        name: String,
        /// The non-finite value.
        value: f64,
    },

    /// An integer-kind candidate did not represent an exact integer
    /// **by value** --- see [`ConfigValue::int_from_f64`].
    #[error("config \"{name}\" requires an exact integer, got {value}")]
    NonIntegral {
        /// The offending config name.
        name: String,
        /// The non-integral (or out-of-`i64`-range) candidate.
        value: f64,
    },

    /// Q#CR10/F5: `set_local` targeted a
    /// [`ConfigMutability::StartupOnly`] definition. Always rejected,
    /// independent of freeze state.
    #[error("config \"{name}\" is startup-only and cannot carry a buffer-local override")]
    StartupOnlyLocal {
        /// The offending config name.
        name: String,
    },

    /// Q#CR10: `set`/`reset` targeted a
    /// [`ConfigMutability::StartupOnly`] definition after
    /// [`ConfigRegistry::freeze`] was called.
    #[error("config \"{name}\" is startup-only and cannot be written after startup completes")]
    StartupOnlyAfterFreeze {
        /// The offending config name.
        name: String,
    },

    /// R50: the Lua bindings lane found a key in a raw spec table that
    /// this registry doesn't know about (or the key's value came from
    /// a metatable rather than the table itself). Raised by the
    /// bindings lane, not by this module directly --- `supported` is
    /// the lane's own field list, pre-joined into the message the way
    /// [`crate::hook::HookError::UnknownField`] hardcodes its own.
    #[error("unknown field `{field}` in config spec; supported: {supported}")]
    UnknownField {
        /// The offending key.
        field: String,
        /// The bindings lane's supported-field list.
        supported: String,
    },
}

// ---------------------------------------------------------------------------
// Name grammar
// ---------------------------------------------------------------------------

/// Maximum byte length of a config name (Q#CR9).
const MAX_NAME_LEN: usize = 128;

/// Validate the dotted, kebab-cased name grammar (Q#CR9): each
/// dot-separated segment matches `[a-z][a-z0-9]*(-[a-z0-9]+)*`, ASCII
/// only, at most [`MAX_NAME_LEN`] bytes total. Deliberately rejects a
/// trailing hyphen, a doubled hyphen, an empty segment, and a leading
/// digit.
fn validate_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() {
        return Err(ConfigError::EmptyName);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(ConfigError::InvalidName {
            name: name.to_owned(),
            reason: format!("exceeds the {MAX_NAME_LEN}-byte length limit"),
        });
    }
    for segment in name.split('.') {
        if let Err(reason) = validate_segment(segment) {
            return Err(ConfigError::InvalidName {
                name: name.to_owned(),
                reason: reason.to_owned(),
            });
        }
    }
    Ok(())
}

/// Validate one dot-separated segment against
/// `[a-z][a-z0-9]*(-[a-z0-9]+)*`.
fn validate_segment(segment: &str) -> Result<(), &'static str> {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return Err("contains an empty segment");
    };
    if !first.is_ascii_lowercase() {
        return Err("each segment must start with a lowercase letter");
    }
    let mut prev_hyphen = false;
    let mut trailing_hyphen = false;
    for c in chars {
        if c == '-' {
            if prev_hyphen {
                return Err("contains a doubled hyphen");
            }
            prev_hyphen = true;
            trailing_hyphen = true;
        } else if c.is_ascii_lowercase() || c.is_ascii_digit() {
            prev_hyphen = false;
            trailing_hyphen = false;
        } else {
            return Err("contains a character outside a-z, 0-9, and hyphen");
        }
    }
    if trailing_hyphen {
        return Err("ends with a trailing hyphen");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ConfigRegistry
// ---------------------------------------------------------------------------

/// Registry of named, typed configuration settings with global and
/// buffer-local override scopes.
///
/// Construction goes through [`Self::new`] / [`Self::default`]. Define
/// via [`Self::define`]; read via [`Self::get`]; write via
/// [`Self::set`] / [`Self::set_local`]; drop an override via
/// [`Self::reset`]. See the module doc for the storage-versus-epoch
/// split (F1) and the resolution rules (F9).
#[derive(Default)]
pub struct ConfigRegistry {
    by_name: HashMap<String, ConfigDefinition>,
    /// Definition order, for stable listing.
    order: Vec<String>,
    /// Global overrides only. Absence means "fall through to
    /// default", not "value is the default".
    global: HashMap<String, ConfigValue>,
    /// Per-buffer overrides only, keyed the same way as `global`.
    locals: HashMap<BufferId, HashMap<String, ConfigValue>>,
    /// Registered `on_change` listeners, in registration order.
    listeners: Vec<ConfigListener>,
    next_listener_id: u64,
    frozen: bool,
    definition_epoch: u64,
    value_epoch: u64,
}

impl ConfigRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // -- definitions ---------------------------------------------------

    /// Define a new setting.
    ///
    /// Validates, in order: the name grammar (Q#CR9), R42 (non-empty
    /// description), the kind's internal consistency (finite bounds,
    /// `min <= max`, duplicate-free enum choices), and the default
    /// against its own kind. A byte-for-byte identical redefinition
    /// (same description, kind, default and mutability) succeeds as a
    /// no-op, supporting idempotent config reload; a conflicting
    /// redefinition fails with [`ConfigError::DuplicateName`] and
    /// leaves the original definition --- including its overrides and
    /// its source location --- exactly as it was.
    ///
    /// # Errors
    ///
    /// See the variants above; nothing is mutated and neither epoch
    /// advances on any error path.
    pub fn define(
        &mut self,
        name: String,
        description: String,
        kind: ConfigKind,
        default: ConfigValue,
        mutability: ConfigMutability,
        source: SourceLocation,
    ) -> Result<(), ConfigError> {
        validate_name(&name)?;
        if description.trim().is_empty() {
            return Err(ConfigError::MissingDescription { name });
        }
        kind.validate_self(&name)?;
        kind.validate(&name, &default)?;

        if let Some(existing) = self.by_name.get(&name) {
            if existing.description == description
                && existing.kind == kind
                && existing.default == default
                && existing.mutability == mutability
            {
                return Ok(());
            }
            return Err(ConfigError::DuplicateName { name });
        }

        self.order.push(name.clone());
        self.by_name.insert(
            name.clone(),
            ConfigDefinition {
                name,
                description,
                kind,
                default,
                mutability,
                source,
            },
        );
        self.definition_epoch = self.definition_epoch.saturating_add(1);
        Ok(())
    }

    fn definition(&self, name: &str) -> Result<&ConfigDefinition, ConfigError> {
        self.by_name.get(name).ok_or_else(|| ConfigError::NotFound {
            name: name.to_owned(),
        })
    }

    /// Look up a definition by name.
    #[must_use]
    pub fn get_definition(&self, name: &str) -> Option<&ConfigDefinition> {
        self.by_name.get(name)
    }

    /// True iff `name` is defined.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Names in definition order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Number of defined settings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// True iff no settings are defined.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Validate `value` against `name`'s definition without storing
    /// anything. The seam the Lua bindings lane uses once it has
    /// converted a raw Lua value into a [`ConfigValue`]: call this to
    /// get a properly-vocabularied [`ConfigError`] before deciding
    /// whether to call [`Self::set`] / [`Self::set_local`] --- though
    /// calling `set`/`set_local` directly is also fine, since they run
    /// the identical check internally.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined; otherwise
    /// whatever [`ConfigKind::validate`] returns.
    pub fn validate(&self, name: &str, value: &ConfigValue) -> Result<(), ConfigError> {
        let def = self.definition(name)?;
        def.kind.validate(name, value)
    }

    // -- resolution ------------------------------------------------------

    /// Resolve the effective value of `name`.
    ///
    /// With `buf`, resolution is buffer-local -> global -> default.
    /// With `None`, resolution is global -> default **only** --- there
    /// is no ambient "current buffer" at this layer (Q#CR4, F9): a
    /// caller that wants buffer-aware behavior must pass the buffer.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined.
    pub fn get(&self, name: &str, buf: Option<BufferId>) -> Result<&ConfigValue, ConfigError> {
        let def = self.definition(name)?;
        if let Some(id) = buf
            && let Some(v) = self.locals.get(&id).and_then(|m| m.get(name))
        {
            return Ok(v);
        }
        Ok(self.global.get(name).unwrap_or(&def.default))
    }

    /// True iff an override is present for `name` at the queried
    /// layer: the buffer-local layer if `buf` is given, the global
    /// layer otherwise. Well-defined precisely because overrides are
    /// always stored (F1) --- including one equal to the value it
    /// shadows.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined.
    pub fn is_set(&self, name: &str, buf: Option<BufferId>) -> Result<bool, ConfigError> {
        self.definition(name)?;
        Ok(match buf {
            Some(id) => self.locals.get(&id).is_some_and(|m| m.contains_key(name)),
            None => self.global.contains_key(name),
        })
    }

    /// The global override for `name`, if one is stored. `None` means
    /// "falls through to default", distinct from an override that
    /// happens to equal the default.
    #[must_use]
    pub fn global_override(&self, name: &str) -> Option<&ConfigValue> {
        self.global.get(name)
    }

    /// The buffer-local override for `name` on `buf`, if one is
    /// stored.
    #[must_use]
    pub fn local_override(&self, name: &str, buf: BufferId) -> Option<&ConfigValue> {
        self.locals.get(&buf).and_then(|m| m.get(name))
    }

    // -- writes ------------------------------------------------------------

    /// Set the global override for `name`.
    ///
    /// The override is stored unconditionally, even if `value` equals
    /// the value it shadows (F1). The returned [`ConfigChange`] tells
    /// the caller whether the *effective* global value actually
    /// changed; the value epoch advances iff it did.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined;
    /// [`ConfigError::StartupOnlyAfterFreeze`] if the definition is
    /// [`ConfigMutability::StartupOnly`] and [`Self::freeze`] has
    /// already been called; otherwise whatever [`Self::validate`]
    /// returns for `value`.
    pub fn set(&mut self, name: &str, value: ConfigValue) -> Result<ConfigChange, ConfigError> {
        let (mutability, default) = {
            let def = self.definition(name)?;
            (def.mutability, def.default.clone())
        };
        if mutability == ConfigMutability::StartupOnly && self.frozen {
            return Err(ConfigError::StartupOnlyAfterFreeze {
                name: name.to_owned(),
            });
        }
        self.validate(name, &value)?;

        let old = self.global.get(name).cloned().unwrap_or(default);
        let new = value.clone();
        let changed = old != new;
        self.global.insert(name.to_owned(), value);
        if changed {
            self.value_epoch = self.value_epoch.saturating_add(1);
        }
        Ok(ConfigChange { changed, old, new })
    }

    /// Set the buffer-local override for `name` on `buf`.
    ///
    /// The override is stored unconditionally, even if `value` equals
    /// the value it shadows --- this is the storage half of F1's fix:
    /// a buffer pinned to the current global value must stay pinned
    /// when the global value later changes. The returned
    /// [`ConfigChange`] reports whether `buf`'s effective value
    /// changed; the value epoch advances iff it did.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined;
    /// [`ConfigError::StartupOnlyLocal`] if the definition is
    /// [`ConfigMutability::StartupOnly`] (always, independent of
    /// freeze state); otherwise whatever [`Self::validate`] returns
    /// for `value`.
    pub fn set_local(
        &mut self,
        buf: BufferId,
        name: &str,
        value: ConfigValue,
    ) -> Result<ConfigChange, ConfigError> {
        let (mutability, default) = {
            let def = self.definition(name)?;
            (def.mutability, def.default.clone())
        };
        if mutability == ConfigMutability::StartupOnly {
            return Err(ConfigError::StartupOnlyLocal {
                name: name.to_owned(),
            });
        }
        self.validate(name, &value)?;

        let global_effective = self.global.get(name).cloned().unwrap_or(default);
        let old = self
            .locals
            .get(&buf)
            .and_then(|m| m.get(name))
            .cloned()
            .unwrap_or(global_effective);
        let new = value.clone();
        let changed = old != new;
        self.locals
            .entry(buf)
            .or_default()
            .insert(name.to_owned(), value);
        if changed {
            self.value_epoch = self.value_epoch.saturating_add(1);
        }
        Ok(ConfigChange { changed, old, new })
    }

    /// Drop exactly one override layer for `name`.
    ///
    /// With `buf`, drops only that buffer's local override and
    /// re-exposes the global chain. With `None`, drops the global
    /// override and re-exposes the default. The value epoch advances
    /// iff the effective value actually changes as a result.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined;
    /// [`ConfigError::StartupOnlyAfterFreeze`] if resetting the
    /// *global* layer of a [`ConfigMutability::StartupOnly`]
    /// definition after [`Self::freeze`].
    pub fn reset(
        &mut self,
        name: &str,
        buf: Option<BufferId>,
    ) -> Result<ConfigChange, ConfigError> {
        let (mutability, default) = {
            let def = self.definition(name)?;
            (def.mutability, def.default.clone())
        };
        if let Some(id) = buf {
            let global_effective = self.global.get(name).cloned().unwrap_or(default);
            let old = self
                .locals
                .get(&id)
                .and_then(|m| m.get(name))
                .cloned()
                .unwrap_or_else(|| global_effective.clone());
            if let Some(m) = self.locals.get_mut(&id) {
                m.remove(name);
            }
            let new = global_effective;
            let changed = old != new;
            if changed {
                self.value_epoch = self.value_epoch.saturating_add(1);
            }
            return Ok(ConfigChange { changed, old, new });
        }

        if mutability == ConfigMutability::StartupOnly && self.frozen {
            return Err(ConfigError::StartupOnlyAfterFreeze {
                name: name.to_owned(),
            });
        }
        let old = self
            .global
            .get(name)
            .cloned()
            .unwrap_or_else(|| default.clone());
        self.global.remove(name);
        let changed = old != default;
        if changed {
            self.value_epoch = self.value_epoch.saturating_add(1);
        }
        Ok(ConfigChange {
            changed,
            old,
            new: default,
        })
    }

    /// Drop `id`'s entire buffer-local override map. Called from the
    /// existing `after_buffer_removed` choke point (Q#CR5). Fires no
    /// listener (Q#CR6 (c)): the buffer is gone, so there is no
    /// effective value left to observe. Does not advance the value
    /// epoch either, for the same reason.
    pub fn remove_buffer(&mut self, id: BufferId) {
        self.locals.remove(&id);
    }

    // -- listeners -----------------------------------------------------

    /// Register `body` to run on future effective-value changes to
    /// `name`. Returns a generation-safe id, never reused, for later
    /// [`Self::dispose`].
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] if `name` is undefined (Q#CR6 (d)).
    pub fn on_change(
        &mut self,
        name: &str,
        body: Function,
        source: SourceLocation,
    ) -> Result<u64, ConfigError> {
        self.definition(name)?;
        let id = self.next_listener_id;
        self.next_listener_id = self.next_listener_id.saturating_add(1);
        self.listeners.push(ConfigListener {
            id,
            name: name.to_owned(),
            body,
            source,
        });
        Ok(id)
    }

    /// Remove a listener by id. Idempotent: disposing an id twice, or
    /// an id that was never issued, is a no-op. Generation-safe: since
    /// ids are never reused, a stale handle can never dispose a newer
    /// listener that happens to reuse its slot.
    pub fn dispose(&mut self, id: u64) {
        self.listeners.retain(|l| l.id != id);
    }

    /// Snapshot the listeners registered against `name`, in
    /// registration order, so the caller can drop the registry borrow
    /// before invoking Lua (which may re-enter the registry). This
    /// registry never runs a listener itself --- mirrors
    /// [`crate::hook::HookRegistry::snapshot`] exactly.
    #[must_use]
    pub fn snapshot(&self, name: &str) -> Vec<ConfigListener> {
        self.listeners
            .iter()
            .filter(|l| l.name == name)
            .cloned()
            .collect()
    }

    // -- startup freeze --------------------------------------------------

    /// Flip the startup freeze. One-way in practice: called once, at
    /// `set_init_complete` time, from the tail of `EditorState::new()`.
    /// After this, a write to a [`ConfigMutability::StartupOnly`]
    /// key's global layer returns
    /// [`ConfigError::StartupOnlyAfterFreeze`].
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// True iff [`Self::freeze`] has been called.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    // -- epochs ------------------------------------------------------------

    /// Advances by one on every successful [`Self::define`] of a new
    /// name (not on an idempotent redefinition, which mutates
    /// nothing).
    #[must_use]
    pub fn definition_epoch(&self) -> u64 {
        self.definition_epoch
    }

    /// Advances by one on every write that changes an *effective*
    /// value --- never on a write that only stores an equal-valued
    /// override (F1, acceptance 15).
    #[must_use]
    pub fn value_epoch(&self) -> u64 {
        self.value_epoch
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn src(line: i32) -> SourceLocation {
        SourceLocation {
            file: "test.lua".into(),
            line,
        }
    }

    fn define_bool(
        r: &mut ConfigRegistry,
        name: &str,
        default: bool,
        mutability: ConfigMutability,
    ) {
        r.define(
            name.into(),
            "a boolean setting".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(default),
            mutability,
            src(1),
        )
        .unwrap();
    }

    // ---- acceptance 1: round-trip every kind -------------------------------

    #[test]
    fn define_then_get_round_trips_every_kind() {
        let mut r = ConfigRegistry::new();
        r.define(
            "editing.auto-pair".into(),
            "Pair brackets on insert.".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(true),
            ConfigMutability::Live,
            src(1),
        )
        .unwrap();
        r.define(
            "autosave.interval-ms".into(),
            "Autosave interval.".into(),
            ConfigKind::Integer {
                min: Some(1000),
                max: None,
            },
            ConfigValue::Int(30_000),
            ConfigMutability::Live,
            src(2),
        )
        .unwrap();
        r.define(
            "editing.fill-column".into(),
            "Preferred wrap column.".into(),
            ConfigKind::Number {
                min: Some(1.0),
                max: Some(1000.0),
            },
            ConfigValue::Num(80.0),
            ConfigMutability::Live,
            src(3),
        )
        .unwrap();
        r.define(
            "editing.comment-prefix".into(),
            "Comment prefix string.".into(),
            ConfigKind::String { allow_empty: true },
            ConfigValue::Str(String::new()),
            ConfigMutability::Live,
            src(4),
        )
        .unwrap();
        r.define(
            "editing.line-ending".into(),
            "Line ending style.".into(),
            ConfigKind::Enum {
                choices: vec!["lf".into(), "crlf".into()],
            },
            ConfigValue::Str("lf".into()),
            ConfigMutability::Live,
            src(5),
        )
        .unwrap();

        assert_eq!(
            r.get("editing.auto-pair", None).unwrap(),
            &ConfigValue::Bool(true)
        );
        assert_eq!(
            r.get("autosave.interval-ms", None).unwrap(),
            &ConfigValue::Int(30_000)
        );
        assert_eq!(
            r.get("editing.fill-column", None).unwrap(),
            &ConfigValue::Num(80.0)
        );
        assert_eq!(
            r.get("editing.comment-prefix", None).unwrap(),
            &ConfigValue::Str(String::new())
        );
        assert_eq!(
            r.get("editing.line-ending", None).unwrap(),
            &ConfigValue::Str("lf".into())
        );
    }

    // ---- acceptance 2: define-time rejections ------------------------------

    #[test]
    fn define_rejects_empty_name() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                String::new(),
                "x".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::EmptyName));
        assert_eq!(r.len(), 0);
        assert_eq!(r.definition_epoch(), 0);
    }

    #[test]
    fn define_rejects_whitespace_only_description() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.x".into(),
                "   \n\t  ".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::MissingDescription { name } if name == "editing.x"));
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn define_rejects_conflicting_duplicate_without_mutating() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let epoch_before = r.definition_epoch();

        let err = r
            .define(
                "editing.x".into(),
                "a different description".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(9),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateName { name } if name == "editing.x"));
        assert_eq!(
            r.definition_epoch(),
            epoch_before,
            "no epoch advance on rejection"
        );
        assert_eq!(
            r.get_definition("editing.x").unwrap().description,
            "a boolean setting"
        );
    }

    #[test]
    fn define_rejects_non_finite_bounds() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.x".into(),
                "x".into(),
                ConfigKind::Number {
                    min: Some(f64::NAN),
                    max: None,
                },
                ConfigValue::Num(1.0),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::NonFiniteNumber { name, .. } if name == "editing.x"));
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn define_rejects_inverted_range() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.x".into(),
                "x".into(),
                ConfigKind::Integer {
                    min: Some(10),
                    max: Some(5),
                },
                ConfigValue::Int(7),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::OutOfRange { name, .. } if name == "editing.x"));
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn define_rejects_an_enum_with_no_choices_pointedly() {
        // Without the direct check this is still rejected, but only
        // indirectly: the default fails `NotAChoice` against an empty
        // list, which names the wrong field. Assert the pointed error
        // so a regression to the indirect path is visible.
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.x".into(),
                "x".into(),
                ConfigKind::Enum { choices: vec![] },
                ConfigValue::Str("a".into()),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(
            matches!(err, ConfigError::EmptyChoices { name } if name == "editing.x"),
            "an empty choice list must be reported as such, not as a bad default"
        );
        assert_eq!(r.len(), 0, "a rejected define registers nothing");
    }

    #[test]
    fn define_rejects_duplicate_enum_choices() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.x".into(),
                "x".into(),
                ConfigKind::Enum {
                    choices: vec!["a".into(), "b".into(), "a".into()],
                },
                ConfigValue::Str("a".into()),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(
            matches!(err, ConfigError::DuplicateChoice { name, choice } if name == "editing.x" && choice == "a")
        );
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn define_rejects_default_violating_its_own_contract() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.x".into(),
                "x".into(),
                ConfigKind::Integer {
                    min: Some(1000),
                    max: None,
                },
                ConfigValue::Int(500), // below its own minimum
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::OutOfRange { name, .. } if name == "editing.x"));
        assert_eq!(r.len(), 0);

        let err2 = r
            .define(
                "editing.y".into(),
                "y".into(),
                ConfigKind::Enum {
                    choices: vec!["lf".into(), "crlf".into()],
                },
                ConfigValue::Str("cr".into()), // not a choice
                ConfigMutability::Live,
                src(2),
            )
            .unwrap_err();
        assert!(matches!(err2, ConfigError::NotAChoice { name, .. } if name == "editing.y"));
    }

    // ---- acceptance 3: name grammar edge cases -----------------------------

    #[test]
    fn define_rejects_trailing_hyphen() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "auto-".into(),
                "x".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidName { .. }));
    }

    #[test]
    fn define_rejects_doubled_hyphen() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "a--b".into(),
                "x".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidName { .. }));
    }

    #[test]
    fn define_rejects_empty_segment() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing..auto-pair".into(),
                "x".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidName { .. }));
    }

    #[test]
    fn define_rejects_leading_digit() {
        let mut r = ConfigRegistry::new();
        let err = r
            .define(
                "editing.1auto".into(),
                "x".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidName { .. }));
    }

    #[test]
    fn define_rejects_overlength_name() {
        let mut r = ConfigRegistry::new();
        let long = "a".repeat(129);
        let err = r
            .define(
                long,
                "x".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(true),
                ConfigMutability::Live,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidName { .. }));
    }

    #[test]
    fn define_accepts_well_formed_kebab_dotted_names() {
        let mut r = ConfigRegistry::new();
        for name in [
            "a",
            "editing.auto-pair",
            "autosave.interval-ms",
            "a.b.c-d-e",
        ] {
            define_bool(&mut r, name, true, ConfigMutability::Live);
        }
        assert_eq!(r.len(), 4);
    }

    // ---- acceptance 4: idempotent vs conflicting redefinition --------------

    #[test]
    fn identical_redefinition_is_idempotent_and_keeps_original_source() {
        let mut r = ConfigRegistry::new();
        r.define(
            "editing.x".into(),
            "desc".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(true),
            ConfigMutability::Live,
            src(10),
        )
        .unwrap();
        // Pin an override so we can prove it survives too.
        r.set("editing.x", ConfigValue::Bool(false)).unwrap();
        let epoch_before = r.definition_epoch();

        r.define(
            "editing.x".into(),
            "desc".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(true),
            ConfigMutability::Live,
            src(99), // different call site: same reload, different line
        )
        .unwrap();

        assert_eq!(r.len(), 1);
        assert_eq!(
            r.definition_epoch(),
            epoch_before,
            "idempotent reload adds nothing"
        );
        assert_eq!(r.get_definition("editing.x").unwrap().source.line, 10);
        assert_eq!(
            r.get("editing.x", None).unwrap(),
            &ConfigValue::Bool(false),
            "the override survives an idempotent reload"
        );
    }

    #[test]
    fn conflicting_redefinition_leaves_original_exact() {
        let mut r = ConfigRegistry::new();
        r.define(
            "editing.x".into(),
            "original".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(true),
            ConfigMutability::Live,
            src(10),
        )
        .unwrap();

        let err = r
            .define(
                "editing.x".into(),
                "changed".into(),
                ConfigKind::Boolean,
                ConfigValue::Bool(false), // different default
                ConfigMutability::Live,
                src(20),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateName { .. }));

        let def = r.get_definition("editing.x").unwrap();
        assert_eq!(def.description, "original");
        assert_eq!(def.default, ConfigValue::Bool(true));
        assert_eq!(def.source.line, 10);
    }

    // ---- acceptance 8: stable ordering --------------------------------------

    #[test]
    fn names_are_stable_across_at_least_three_defines() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "c", true, ConfigMutability::Live);
        define_bool(&mut r, "a", true, ConfigMutability::Live);
        define_bool(&mut r, "b", true, ConfigMutability::Live);
        assert_eq!(r.names(), &["c".to_owned(), "a".into(), "b".into()]);
    }

    // ---- acceptance 9 (bonus): source captured verbatim ---------------------

    #[test]
    fn source_location_is_captured_from_the_defining_module() {
        let mut r = ConfigRegistry::new();
        r.define(
            "editing.auto-pair".into(),
            "x".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(true),
            ConfigMutability::Live,
            SourceLocation {
                file: "pair.lua".into(),
                line: 12,
            },
        )
        .unwrap();
        assert_eq!(
            r.get_definition("editing.auto-pair")
                .unwrap()
                .source
                .render(),
            "pair.lua:12"
        );
    }

    // ---- acceptance 10 / 14: resolution and F9 ------------------------------

    #[test]
    fn get_resolves_local_then_global_then_default() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let buf = BufferId::next();

        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(true)
        );

        r.set("editing.x", ConfigValue::Bool(false)).unwrap();
        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(false)
        );

        r.set_local(buf, "editing.x", ConfigValue::Bool(true))
            .unwrap();
        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(true)
        );

        r.reset("editing.x", Some(buf)).unwrap();
        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(false)
        );

        r.reset("editing.x", None).unwrap();
        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(true)
        );
    }

    #[test]
    fn get_with_no_buffer_ignores_any_local_override() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let buf = BufferId::next();
        r.set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap();

        // F9: get(name) with no buffer never consults an "active"
        // buffer. It must see the global chain only, even though a
        // buffer somewhere holds a different local override.
        assert_eq!(r.get("editing.x", None).unwrap(), &ConfigValue::Bool(true));
    }

    // ---- acceptance 11: THE F1 test -----------------------------------------

    #[test]
    fn equal_valued_local_override_is_still_stored_and_shields_buffer() {
        // This is the bite-verified regression test for framing F1.
        //
        // Scenario: a setting defaults to `true`. A buffer is pinned
        // to `true` via set_local -- at the moment of pinning, the
        // local override is *equal* to the effective global value, so
        // an implementation that treats "equal-valued set" as a true
        // no-op would store nothing for that buffer. Then the global
        // value flips to `false`. If the local override was never
        // stored, the "pinned" buffer silently flips too -- the pin
        // never existed. The fix (Q#CR2/F1): overrides are *always*
        // stored, even when they equal the value they shadow; only
        // the value epoch and listener dispatch key on effective
        // change. This test fails against the naive "true no-op"
        // implementation and passes against the always-store one.
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.auto-pair", true, ConfigMutability::Live);
        let buf = BufferId::next();

        // Global effective value is `true` (the default; no override
        // yet). Pin the buffer to that same value.
        let pin = r
            .set_local(buf, "editing.auto-pair", ConfigValue::Bool(true))
            .unwrap();
        assert!(
            !pin.changed,
            "pinning to the current value is observationally silent"
        );
        assert!(
            r.is_set("editing.auto-pair", Some(buf)).unwrap(),
            "but the override IS stored -- is_set must see it"
        );

        // Now flip the global value.
        let flip = r
            .set("editing.auto-pair", ConfigValue::Bool(false))
            .unwrap();
        assert!(flip.changed);

        // The pinned buffer must NOT have flipped.
        assert_eq!(
            r.get("editing.auto-pair", Some(buf)).unwrap(),
            &ConfigValue::Bool(true),
            "F1: the buffer-local pin must survive a later global change"
        );
        // The global (no-buffer) chain must reflect the flip.
        assert_eq!(
            r.get("editing.auto-pair", None).unwrap(),
            &ConfigValue::Bool(false)
        );
    }

    // ---- acceptance 12: per-buffer isolation + is_set -----------------------

    #[test]
    fn set_local_on_one_buffer_does_not_affect_another() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let a = BufferId::next();
        let b = BufferId::next();

        r.set_local(a, "editing.x", ConfigValue::Bool(false))
            .unwrap();

        assert_eq!(
            r.get("editing.x", Some(a)).unwrap(),
            &ConfigValue::Bool(false)
        );
        assert_eq!(
            r.get("editing.x", Some(b)).unwrap(),
            &ConfigValue::Bool(true)
        );
        assert!(r.is_set("editing.x", Some(a)).unwrap());
        assert!(!r.is_set("editing.x", Some(b)).unwrap());
    }

    #[test]
    fn is_set_reports_presence_even_for_an_equal_valued_override() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        assert!(!r.is_set("editing.x", None).unwrap());
        r.set("editing.x", ConfigValue::Bool(true)).unwrap(); // equal to default
        assert!(r.is_set("editing.x", None).unwrap());
    }

    // ---- acceptance 13: buffer-local lifecycle -------------------------------

    #[test]
    fn remove_buffer_drops_its_local_overrides() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let buf = BufferId::next();
        r.set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap();
        assert!(r.is_set("editing.x", Some(buf)).unwrap());

        r.remove_buffer(buf);

        assert!(!r.is_set("editing.x", Some(buf)).unwrap());
        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(true)
        );
    }

    #[test]
    fn locals_persist_until_remove_buffer_is_explicitly_called() {
        // Documents the Q#CR5/F8 contract this module owns: without an
        // explicit remove_buffer call (which in production rides
        // after_buffer_removed), a buffer's locals are never purged.
        // A BufferRegistry::remove bypass that skips that choke point
        // is out of this file's scope (it lives in editor.rs), but the
        // half of the contract this registry is responsible for --
        // "no automatic purge exists" -- is directly testable here.
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let buf = BufferId::next();
        r.set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap();
        // ... time passes, nothing calls remove_buffer ...
        assert!(
            r.is_set("editing.x", Some(buf)).unwrap(),
            "leak is permanent until purged"
        );
    }

    // ---- acceptance 15: epoch discipline -------------------------------------

    #[test]
    fn value_epoch_advances_only_on_effective_change() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let epoch0 = r.value_epoch();

        let noop = r.set("editing.x", ConfigValue::Bool(true)).unwrap(); // equal to default
        assert!(!noop.changed);
        assert_eq!(
            r.value_epoch(),
            epoch0,
            "equal-valued override advances no epoch"
        );

        let real = r.set("editing.x", ConfigValue::Bool(false)).unwrap();
        assert!(real.changed);
        assert_eq!(r.value_epoch(), epoch0 + 1);

        let buf = BufferId::next();
        let local_noop = r
            .set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap(); // equal to current global
        assert!(!local_noop.changed);
        assert_eq!(r.value_epoch(), epoch0 + 1, "still no advance");

        let local_real = r
            .set_local(buf, "editing.x", ConfigValue::Bool(true))
            .unwrap();
        assert!(local_real.changed);
        assert_eq!(r.value_epoch(), epoch0 + 2);
    }

    #[test]
    fn definition_epoch_advances_only_on_new_definitions() {
        let mut r = ConfigRegistry::new();
        assert_eq!(r.definition_epoch(), 0);
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        assert_eq!(r.definition_epoch(), 1);
        // Idempotent redefinition: no advance.
        r.define(
            "editing.x".into(),
            "a boolean setting".into(),
            ConfigKind::Boolean,
            ConfigValue::Bool(true),
            ConfigMutability::Live,
            src(2),
        )
        .unwrap();
        assert_eq!(r.definition_epoch(), 1);
    }

    // ---- the validate() seam --------------------------------------------------

    #[test]
    fn validate_checks_without_committing() {
        let mut r = ConfigRegistry::new();
        r.define(
            "autosave.interval-ms".into(),
            "x".into(),
            ConfigKind::Integer {
                min: Some(1000),
                max: None,
            },
            ConfigValue::Int(30_000),
            ConfigMutability::Live,
            src(1),
        )
        .unwrap();

        let err = r
            .validate("autosave.interval-ms", &ConfigValue::Int(500))
            .unwrap_err();
        assert!(matches!(err, ConfigError::OutOfRange { .. }));
        // Nothing was mutated by the failed validation.
        assert!(!r.is_set("autosave.interval-ms", None).unwrap());
        assert_eq!(r.value_epoch(), 0);

        assert!(
            r.validate("autosave.interval-ms", &ConfigValue::Int(5000))
                .is_ok()
        );
    }

    #[test]
    fn validate_on_undefined_name_is_not_found() {
        let r = ConfigRegistry::new();
        let err = r.validate("nope", &ConfigValue::Bool(true)).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound { name } if name == "nope"));
    }

    // ---- type mismatch and per-kind validation ---------------------------------

    #[test]
    fn set_rejects_wrong_type() {
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let err = r.set("editing.x", ConfigValue::Int(1)).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::TypeMismatch {
                expected: "boolean",
                got: "integer",
                ..
            }
        ));
    }

    #[test]
    fn string_kind_rejects_empty_unless_allowed() {
        let mut r = ConfigRegistry::new();
        r.define(
            "editing.x".into(),
            "x".into(),
            ConfigKind::String { allow_empty: false },
            ConfigValue::Str("nonempty".into()),
            ConfigMutability::Live,
            src(1),
        )
        .unwrap();
        let err = r
            .set("editing.x", ConfigValue::Str(String::new()))
            .unwrap_err();
        assert!(matches!(err, ConfigError::EmptyString { .. }));
    }

    #[test]
    fn enum_kind_rejects_non_choice() {
        let mut r = ConfigRegistry::new();
        r.define(
            "editing.x".into(),
            "x".into(),
            ConfigKind::Enum {
                choices: vec!["lf".into(), "crlf".into()],
            },
            ConfigValue::Str("lf".into()),
            ConfigMutability::Live,
            src(1),
        )
        .unwrap();
        let err = r
            .set("editing.x", ConfigValue::Str("cr".into()))
            .unwrap_err();
        assert!(matches!(err, ConfigError::NotAChoice { got, .. } if got == "cr"));
    }

    // ---- listeners: registration, snapshot, dispose -----------------------

    #[test]
    fn on_change_on_undefined_name_raises_not_found() {
        let lua = mlua::Lua::new();
        let mut r = ConfigRegistry::new();
        let f = lua.create_function(|_, ()| Ok(())).unwrap();
        let err = r.on_change("nope", f, src(1)).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound { name } if name == "nope"));
    }

    #[test]
    fn snapshot_returns_owned_listeners_in_registration_order_and_survives_registry_drop() {
        let lua = mlua::Lua::new();
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        for line in 10..13 {
            let f = lua.create_function(|_, ()| Ok(())).unwrap();
            r.on_change("editing.x", f, src(line)).unwrap();
        }
        let snap = r.snapshot("editing.x");
        assert_eq!(snap.len(), 3);
        assert_eq!(
            snap.iter().map(|l| l.source.line).collect::<Vec<_>>(),
            vec![10, 11, 12]
        );

        // Bite-verify borrow release: the registry itself can be
        // dropped and the snapshot's Lua functions are still callable
        // -- proving the caller genuinely does not need to hold the
        // registry borrow while invoking them.
        drop(r);
        for l in &snap {
            l.body.call::<()>(()).unwrap();
        }
    }

    #[test]
    fn dispose_is_idempotent_and_id_generation_safe() {
        let lua = mlua::Lua::new();
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let f1 = lua.create_function(|_, ()| Ok(())).unwrap();
        let id1 = r.on_change("editing.x", f1, src(1)).unwrap();

        r.dispose(id1);
        assert!(r.snapshot("editing.x").is_empty());
        r.dispose(id1); // idempotent: no panic, no effect
        assert!(r.snapshot("editing.x").is_empty());

        let f2 = lua.create_function(|_, ()| Ok(())).unwrap();
        let id2 = r.on_change("editing.x", f2, src(2)).unwrap();
        assert_ne!(id1, id2);

        // A stale id must never dispose a newer listener.
        r.dispose(id1);
        assert_eq!(r.snapshot("editing.x").len(), 1);
        assert_eq!(r.snapshot("editing.x")[0].id, id2);
    }

    #[test]
    fn global_set_does_not_change_a_shadowed_buffers_effective_value() {
        // Q#CR6 (b): a global `set` fires only the global-scoped
        // notification; a buffer holding its own override is
        // shadowed, so its effective value must not move.
        let mut r = ConfigRegistry::new();
        define_bool(&mut r, "editing.x", true, ConfigMutability::Live);
        let buf = BufferId::next();
        r.set_local(buf, "editing.x", ConfigValue::Bool(true))
            .unwrap();

        let change = r.set("editing.x", ConfigValue::Bool(false)).unwrap();
        assert!(change.changed, "the global effective value did change");
        assert_eq!(
            r.get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(true),
            "the shadowed buffer's effective value must not move"
        );
    }

    // ---- mutability and the startup freeze ---------------------------------

    #[test]
    fn startup_only_key_accepts_writes_before_freeze_and_rejects_after() {
        let mut r = ConfigRegistry::new();
        define_bool(
            &mut r,
            "lsp.root-markers",
            true,
            ConfigMutability::StartupOnly,
        );

        // NOTE: in --lib test builds, set_init_complete never runs and
        // the freeze flag never flips on its own -- this test flips it
        // explicitly, the way mod.rs's own acceptance tests do, so it
        // does not pass vacuously.
        assert!(r.set("lsp.root-markers", ConfigValue::Bool(false)).is_ok());

        r.freeze();
        assert!(r.is_frozen());

        let err = r
            .set("lsp.root-markers", ConfigValue::Bool(true))
            .unwrap_err();
        assert!(
            matches!(err, ConfigError::StartupOnlyAfterFreeze { name } if name == "lsp.root-markers")
        );

        let err2 = r.reset("lsp.root-markers", None).unwrap_err();
        assert!(matches!(err2, ConfigError::StartupOnlyAfterFreeze { .. }));
    }

    #[test]
    fn startup_only_key_always_rejects_set_local() {
        let mut r = ConfigRegistry::new();
        define_bool(
            &mut r,
            "lsp.root-markers",
            true,
            ConfigMutability::StartupOnly,
        );
        let buf = BufferId::next();

        // Rejected even before freeze: the combination is banned
        // outright (F5), not just after startup completes.
        assert!(!r.is_frozen());
        let err = r
            .set_local(buf, "lsp.root-markers", ConfigValue::Bool(false))
            .unwrap_err();
        assert!(
            matches!(err, ConfigError::StartupOnlyLocal { name } if name == "lsp.root-markers")
        );
    }

    // ---- ConfigValue::int_from_f64 (acceptance 6's pure-Rust half) --------

    #[test]
    fn int_from_f64_accepts_exact_values() {
        assert_eq!(
            ConfigValue::int_from_f64("x", 3.0).unwrap(),
            ConfigValue::Int(3)
        );
        assert_eq!(
            ConfigValue::int_from_f64("x", -1500.0).unwrap(),
            ConfigValue::Int(-1500)
        );
        assert_eq!(
            ConfigValue::int_from_f64("x", 0.0).unwrap(),
            ConfigValue::Int(0)
        );
    }

    #[test]
    fn int_from_f64_rejects_fractional_values() {
        let err = ConfigValue::int_from_f64("x", 1500.7).unwrap_err();
        assert!(matches!(err, ConfigError::NonIntegral { .. }));
    }

    #[test]
    fn int_from_f64_rejects_non_finite_values() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = ConfigValue::int_from_f64("x", v).unwrap_err();
            assert!(matches!(err, ConfigError::NonFiniteNumber { .. }));
        }
    }

    #[test]
    fn int_from_f64_rejects_the_saturating_upper_boundary() {
        // Review round 1, finding 1. `i64::MAX as f64` rounds UP to
        // 2^63 = 9223372036854775808.0, one more than i64::MAX. With a
        // `>` guard this value passes validation and `v as i64` then
        // SATURATES to 9223372036854775807 --- the registry silently
        // stores a different number than the caller asked for. This
        // test fails against the `>` form.
        let boundary = i64::MAX as f64;
        let err = ConfigValue::int_from_f64("x", boundary).unwrap_err();
        assert!(
            matches!(err, ConfigError::NonIntegral { .. }),
            "2^63 must be rejected, not saturated to i64::MAX"
        );
        // Anything beyond it too.
        assert!(ConfigValue::int_from_f64("x", boundary * 2.0).is_err());
    }

    #[test]
    fn int_from_f64_accepts_the_exact_lower_boundary() {
        // The bounds are asymmetric on purpose: unlike the upper one,
        // `i64::MIN as f64` IS exactly i64::MIN and round-trips, so
        // tightening the lower comparison to `<=` alongside the upper
        // `>=` would wrongly reject a legitimate value.
        let lo = i64::MIN as f64;
        assert_eq!(
            ConfigValue::int_from_f64("x", lo).unwrap(),
            ConfigValue::Int(i64::MIN),
            "i64::MIN is representable and must still be accepted"
        );
    }

    #[test]
    fn int_from_f64_accepts_the_largest_representable_integer_below_the_boundary() {
        // The next f64 below 2^63 is 2^63 - 1024, which is a valid i64.
        // Pins that the `>=` fix did not over-reject the top of range.
        let below = (i64::MAX as f64) - 1024.0;
        let got = ConfigValue::int_from_f64("x", below).unwrap();
        assert_eq!(got, ConfigValue::Int(9_223_372_036_854_774_784));
    }
}
