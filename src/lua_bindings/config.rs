// lua_bindings/config.rs --- pmacs.config: the configuration registry surface.

//! `pmacs.config.*` --- the Lua surface over [`crate::config_registry`].
//! Per `docs/config-registry-framing.md`: two scopes, no wire surface,
//! and no runtime chunk (Q#CR14) --- `pmacs.config` is installed here,
//! entirely from Rust, before any `builtin/runtime/*.lua` chunk
//! evaluates, exactly like `pmacs.command` and `pmacs.hook` have no
//! Lua-side companion file either.
//!
//! ```lua
//! pmacs.config.define { name = ..., description = ..., type = ...,
//!                        default = ..., min = ..., max = ...,
//!                        choices = ..., allow_empty = ...,
//!                        mutability = ... }
//! pmacs.config.get(name [, buf])
//! pmacs.config.set(name, value)
//! pmacs.config.set_local(buf, name, value)
//! pmacs.config.reset(name [, buf])
//! pmacs.config.is_set(name [, buf])
//! pmacs.config.describe(name [, buf])
//! pmacs.config.list()
//! local handle = pmacs.config.on_change(name, function(new, old, buf) ... end)
//! handle:dispose()
//! ```
//!
//! # Two scopes, no ambient buffer (Q#CR4, F9)
//!
//! `set` writes the global override; `set_local` writes one buffer's
//! override. `get(name, buf)` resolves buffer-local -> global ->
//! default. **`get(name)` with no buffer argument resolves the global
//! chain only** --- global override, then default --- and never
//! consults an "active" buffer. There is no ambient buffer at this
//! layer; a caller that wants buffer-aware behavior must pass one.
//! `describe` and `list` follow the identical rule for their `value`
//! field.
//!
//! # Define before set; owner-defines, not a runtime chunk (Q#CR10, Q#CR14)
//!
//! Every name must be `define`d before `get`/`set`/`set_local`/
//! `reset`/`is_set`/`describe`/`on_change` touches it --- an undefined
//! name raises `NotFound`, the same posture `pmacs.hook.add` already
//! has. Definitions live with the module that owns the setting, not in
//! a shared helper: `pair.lua` defines `editing.auto-pair`,
//! `editops.lua` defines `editing.trim-on-save`, `autosave.lua`
//! defines `autosave.interval-ms`. [`SourceLocation`] is captured from
//! Lua debug info at the `define` call site (see [`caller_source`]),
//! so a shared wrapper would point every builtin setting's reported
//! source at the wrapper instead of its owner --- which is exactly why
//! no such wrapper exists here.
//!
//! # Strict specs, lenient wrappers (Q#CR3, Q#CR8)
//!
//! `type` is one of `boolean | integer | number | string | enum` ---
//! no list/table type in stage 1. `define`'s spec table is read with
//! **raw** access only (typo-detection via [`Table::pairs`], field
//! reads via [`Table::raw_get`]), so neither an unknown key nor a
//! metatable-provided value can smuggle a field in (R50). Unknown
//! keys, a missing-or-whitespace-only `description` (R42), and a
//! `default` that violates its own `min`/`max`/`choices` are all
//! rejected before anything is registered. `set`/`set_local` enforce
//! the identical type and bounds on every write --- this module
//! converts a raw [`Value`] into a [`ConfigValue`] and lets
//! [`ConfigRegistry`] own the value-level check (see that module's
//! "value-validation seam").
//!
//! That strictness is deliberate and is not this module's to relax for
//! any particular adopter: `pmacs.editops.trim_on_save("yes")` and
//! `pmacs.autosave.interval_ms(1500.7)` keep their legacy coercion
//! (flooring, `~= false`) in their own builtin files, calling `set`
//! only with an already-conforming value. This registry never coerces.
//!
//! # Integer exactness, by value, never `math.type` (acceptance 6)
//!
//! `LuaJIT` (Lua 5.1 semantics) never produces `Value::Integer` --- every
//! `LuaJIT` number arrives as `Value::Number(f64)`. Lua 5.4 produces
//! `Value::Integer(i64)` for integer literals, which are already exact
//! and read straight through with no float round-trip (round-tripping
//! a large `i64` through `f64` can silently change its value).
//! `Value::Number` on either backend goes through
//! [`ConfigValue::int_from_f64`], which checks exactness by the
//! numeric value alone. Neither backend's arm inspects which backend
//! produced the value or what `math.type` would say.
//!
//! # Listener dispatch (Q#CR6)
//!
//! `set`/`set_local`/`reset` commit inside the registry borrow, then
//! (iff the effective value changed) drop that borrow completely
//! before invoking any listener body --- see [`dispatch_config_listeners`].
//! A raising listener is logged and does not stop later listeners or
//! roll back the committed value. [`ConfigDispatchDepth`] bounds
//! re-entrant dispatch so an accidental listener cycle raises a
//! pointed error instead of recursing forever. Listeners persist until
//! explicitly disposed (F3): there is no `MetaMethod::Gc` anywhere in
//! this module, matching the rest of the codebase's explicit-dispose
//! posture.
//!
//! # The startup freeze, without touching `editor.rs`
//!
//! [`ConfigMutability::StartupOnly`] enforcement is entirely
//! [`ConfigRegistry`]'s job (`StartupOnlyAfterFreeze`); this module's
//! only responsibility is calling [`ConfigRegistry::freeze`] at the
//! right moment. `set` and `reset` do so lazily: if
//! [`super::InitCompleteFlag`] reports the init phase complete and the
//! registry isn't frozen yet, they freeze it before proceeding. That
//! keeps the single source of truth for "init is done" in the flag
//! `EditorState::new` already flips, with no new call added to
//! `editor.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mlua::{Function, Lua, Table, UserData, UserDataMethods, Value};
use thiserror::Error;

use super::{BufferIdLua, InitCompleteFlag, SharedRegistry, caller_source, require_string_key};
use crate::buffer::EditOp;
use crate::command::SourceLocation;
use crate::config_registry::{
    ConfigChange, ConfigError, ConfigKind, ConfigMutability, ConfigRegistry, ConfigValue,
};

/// Shared, single-threaded handle to the configuration registry.
/// Lives behind `Rc<RefCell<...>>` as Lua app data, exactly like the
/// command and hook registries (`mod.rs:2265-2271`).
pub type SharedConfigRegistry = Rc<RefCell<ConfigRegistry>>;

/// Recursion bound for re-entrant listener dispatch (Q#CR6). A
/// listener is free to call `pmacs.config.set`/`set_local`/`reset`
/// re-entrantly --- the registry borrow is always released before any
/// listener body runs (see [`dispatch_config_listeners`]) --- but an
/// accidental cycle (A's listener sets B, B's listener sets A, ...)
/// must not hang the editor. Chosen generously above any legitimate
/// re-entrant chain a real adopter would produce.
const MAX_DISPATCH_DEPTH: u32 = 32;

/// Per-VM re-entrant listener-dispatch depth counter. Newtype app data
/// (mirrors [`InitCompleteFlag`]'s `Rc<Cell<...>>` shape) so
/// `set`/`set_local`/`reset` dispatch calls made from arbitrarily
/// nested re-entrant `pmacs.config.*` calls all see the same counter.
#[derive(Clone)]
struct ConfigDispatchDepth(Rc<Cell<u32>>);

impl ConfigDispatchDepth {
    fn new() -> Self {
        Self(Rc::new(Cell::new(0)))
    }
}

/// RAII guard: increments on construction, decrements on drop ---
/// including on an early `?` return from a failed Lua value
/// conversion between the increment and the dispatch loop --- so the
/// counter can never get stuck above zero.
struct DispatchDepthGuard(Rc<Cell<u32>>);

impl Drop for DispatchDepthGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// Errors raised by this Lua-boundary module itself, distinct from
/// [`ConfigError`]: these describe a failure to make sense of a raw
/// Lua value (an unrecognized `type =` tag, a missing one, a listener
/// cycle) rather than a failure of already-typed config data. See the
/// module doc's "value-validation seam" cross-reference.
#[derive(Debug, Error)]
enum ConfigBindingError {
    /// `define` was called without a `type` field.
    #[error(
        "config spec requires a \"type\" field (one of boolean, integer, number, string, enum)"
    )]
    MissingType,

    /// `define`'s `type` field wasn't one of the closed vocabulary.
    #[error("unknown config type `{got}`; expected one of: boolean, integer, number, string, enum")]
    UnknownType {
        /// The offending type tag.
        got: String,
    },

    /// `define`'s `type = "enum"` had no (or an empty) `choices` table.
    #[error("config \"{name}\" (type = \"enum\") requires a `choices` table of strings")]
    MissingChoices {
        /// The config name being defined.
        name: String,
    },

    /// `define` carried a spec field that means nothing for the
    /// declared `type` --- `choices` on a string, `min` on a boolean.
    /// Correctly spelled, wrongly placed: the R50 whitelist cannot see
    /// it, so it is checked against the kind instead.
    #[error(
        "config field `{field}` is meaningless for type = \"{got_type}\"; \
         `min`/`max` require integer or number, `choices` requires enum, \
         `allow_empty` requires string"
    )]
    FieldIrrelevantForType {
        /// The offending field name.
        field: &'static str,
        /// The declared type it was paired with.
        got_type: String,
    },

    /// `define`'s `mutability` field wasn't `"live"` or `"startup"`.
    #[error("unknown config mutability `{got}`; expected one of: live, startup")]
    UnknownMutability {
        /// The offending mutability tag.
        got: String,
    },

    /// Re-entrant listener dispatch exceeded [`MAX_DISPATCH_DEPTH`] ---
    /// almost certainly an accidental listener cycle.
    #[error(
        "config listener dispatch exceeded the recursion bound ({max}); a listener cycle is likely"
    )]
    ListenerCycle {
        /// The bound that was exceeded.
        max: u32,
    },
}

// ---------------------------------------------------------------------------
// Lua Value <-> ConfigValue conversion
// ---------------------------------------------------------------------------

fn type_mismatch(name: &str, expected: &'static str, got: &Value) -> mlua::Error {
    mlua::Error::external(ConfigError::TypeMismatch {
        name: name.to_owned(),
        expected,
        got: got.type_name(),
    })
}

/// Convert a raw Lua number into an exact `i64`. See the module doc's
/// "integer exactness" section: `Value::Integer` (lua54 only) is
/// already exact and is never round-tripped through `f64`;
/// `Value::Number` (both backends) goes through
/// [`ConfigValue::int_from_f64`].
fn lua_exact_i64(name: &str, value: Value) -> mlua::Result<i64> {
    match value {
        Value::Integer(i) => Ok(i),
        Value::Number(f) => match ConfigValue::int_from_f64(name, f) {
            Ok(ConfigValue::Int(i)) => Ok(i),
            Ok(_) => unreachable!("int_from_f64 always returns ConfigValue::Int"),
            Err(e) => Err(mlua::Error::external(e)),
        },
        other => Err(type_mismatch(name, "integer", &other)),
    }
}

fn lua_to_f64(name: &str, value: Value) -> mlua::Result<f64> {
    match value {
        Value::Integer(i) => Ok(i as f64),
        Value::Number(f) => Ok(f),
        other => Err(type_mismatch(name, "number", &other)),
    }
}

/// Convert a raw Lua value into a [`ConfigValue`] under an
/// already-known [`ConfigKind`]. Used for both `define`'s `default`
/// field and `set`/`set_local`'s value argument --- the one converter
/// both paths share, so there is exactly one place Lua-level type
/// coercion can drift (mirrors [`ConfigRegistry::validate`] being the
/// single value-level seam on the Rust side).
fn lua_to_config_value(name: &str, kind: &ConfigKind, value: Value) -> mlua::Result<ConfigValue> {
    match kind {
        ConfigKind::Boolean => match value {
            Value::Boolean(b) => Ok(ConfigValue::Bool(b)),
            other => Err(type_mismatch(name, "boolean", &other)),
        },
        ConfigKind::Integer { .. } => Ok(ConfigValue::Int(lua_exact_i64(name, value)?)),
        ConfigKind::Number { .. } => Ok(ConfigValue::Num(lua_to_f64(name, value)?)),
        ConfigKind::String { .. } => match value {
            Value::String(s) => Ok(ConfigValue::Str(s.to_str()?.to_owned())),
            other => Err(type_mismatch(name, "string", &other)),
        },
        ConfigKind::Enum { .. } => match value {
            Value::String(s) => Ok(ConfigValue::Str(s.to_str()?.to_owned())),
            other => Err(type_mismatch(name, "enum", &other)),
        },
    }
}

fn config_value_to_lua(lua: &Lua, v: &ConfigValue) -> mlua::Result<Value> {
    Ok(match v {
        ConfigValue::Bool(b) => Value::Boolean(*b),
        ConfigValue::Int(i) => Value::Integer(*i),
        ConfigValue::Num(f) => Value::Number(*f),
        ConfigValue::Str(s) => Value::String(lua.create_string(s)?),
    })
}

// ---------------------------------------------------------------------------
// define(): strict raw-table spec parsing
// ---------------------------------------------------------------------------

/// The closed set of keys `pmacs.config.define {...}` accepts. Checked
/// with raw table access (R50) --- see [`check_unknown_fields`].
const DEFINE_SPEC_FIELDS: &[&str] = &[
    "name",
    "description",
    "type",
    "default",
    "min",
    "max",
    "choices",
    "allow_empty",
    "mutability",
];

/// R50 typo-detection: every key actually present in the raw table
/// (via [`Table::pairs`], which --- like [`Table::raw_get`] below ---
/// never invokes `__pairs`/`__index`) must be in `allowed`, or the
/// spec is rejected naming the offender and the supported-key list.
/// This alone doesn't stop a metatable `__index` from answering a
/// `raw_get` for a key the table itself never had; that's why every
/// field read below uses `raw_get`, not `get`, too.
fn check_unknown_fields(spec: &Table, allowed: &[&str]) -> mlua::Result<()> {
    for pair in spec.clone().pairs::<Value, Value>() {
        let (k, _) = pair?;
        let key = require_string_key(k)?;
        if !allowed.contains(&key.as_str()) {
            return Err(mlua::Error::external(ConfigError::UnknownField {
                field: key,
                supported: allowed.join(", "),
            }));
        }
    }
    Ok(())
}

fn read_bound_i64(spec: &Table, field: &'static str, name: &str) -> mlua::Result<Option<i64>> {
    match spec.raw_get::<Value>(field)? {
        Value::Nil => Ok(None),
        other => Ok(Some(lua_exact_i64(name, other)?)),
    }
}

fn read_bound_f64(spec: &Table, field: &'static str, name: &str) -> mlua::Result<Option<f64>> {
    match spec.raw_get::<Value>(field)? {
        Value::Nil => Ok(None),
        other => Ok(Some(lua_to_f64(name, other)?)),
    }
}

/// Read `choices` as a raw sequence of Lua strings --- no numeric
/// coercion (unlike `Table::sequence_values::<String>()`, which would
/// silently stringify a numeric entry): a non-string choice is a
/// definition bug, not a value to paper over.
fn read_choices(spec: &Table, name: &str) -> mlua::Result<Vec<String>> {
    let Some(t) = spec.raw_get::<Option<Table>>("choices")? else {
        return Err(mlua::Error::external(ConfigBindingError::MissingChoices {
            name: name.to_owned(),
        }));
    };
    let mut choices = Vec::new();
    for v in t.sequence_values::<Value>() {
        match v? {
            Value::String(s) => choices.push(s.to_str()?.to_owned()),
            other => return Err(type_mismatch(name, "enum", &other)),
        }
    }
    Ok(choices)
}

/// Reject spec fields that are meaningless for the declared `type`
/// (review round 1, finding 3).
///
/// `DEFINE_SPEC_FIELDS` whitelists every key for every type, and the
/// kind parser below only reads the ones its own arm cares about ---
/// so without this check `{ type = "string", choices = {"a","b"} }`
/// silently defines a string that accepts anything (the author meant
/// `enum`), and `{ type = "boolean", min = 1 }` silently drops the
/// bound. Both are typo-shaped bugs of exactly the class R50 exists to
/// catch; the whitelist alone only catches misspelled keys, not
/// correctly-spelled keys on the wrong type.
fn check_fields_relevant_to_kind(spec: &Table, type_str: &str) -> mlua::Result<()> {
    let numeric = matches!(type_str, "integer" | "number");
    for (field, allowed_for) in [
        ("min", numeric),
        ("max", numeric),
        ("choices", type_str == "enum"),
        ("allow_empty", type_str == "string"),
    ] {
        if !allowed_for && !matches!(spec.raw_get::<Value>(field)?, Value::Nil) {
            return Err(mlua::Error::external(
                ConfigBindingError::FieldIrrelevantForType {
                    field,
                    got_type: type_str.to_owned(),
                },
            ));
        }
    }
    Ok(())
}

fn parse_kind_and_default(spec: &Table, name: &str) -> mlua::Result<(ConfigKind, ConfigValue)> {
    let type_str: Option<String> = spec.raw_get("type")?;
    let type_str =
        type_str.ok_or_else(|| mlua::Error::external(ConfigBindingError::MissingType))?;
    check_fields_relevant_to_kind(spec, &type_str)?;

    let kind = match type_str.as_str() {
        "boolean" => ConfigKind::Boolean,
        "integer" => ConfigKind::Integer {
            min: read_bound_i64(spec, "min", name)?,
            max: read_bound_i64(spec, "max", name)?,
        },
        "number" => ConfigKind::Number {
            min: read_bound_f64(spec, "min", name)?,
            max: read_bound_f64(spec, "max", name)?,
        },
        "string" => ConfigKind::String {
            allow_empty: spec
                .raw_get::<Option<bool>>("allow_empty")?
                .unwrap_or(false),
        },
        "enum" => ConfigKind::Enum {
            choices: read_choices(spec, name)?,
        },
        other => {
            return Err(mlua::Error::external(ConfigBindingError::UnknownType {
                got: other.to_owned(),
            }));
        }
    };

    let raw_default: Value = spec.raw_get("default")?;
    let default = lua_to_config_value(name, &kind, raw_default)?;
    Ok((kind, default))
}

fn parse_mutability(spec: &Table) -> mlua::Result<ConfigMutability> {
    match spec.raw_get::<Option<String>>("mutability")?.as_deref() {
        None | Some("live") => Ok(ConfigMutability::Live),
        Some("startup") => Ok(ConfigMutability::StartupOnly),
        Some(other) => Err(mlua::Error::external(
            ConfigBindingError::UnknownMutability {
                got: other.to_owned(),
            },
        )),
    }
}

// ---------------------------------------------------------------------------
// Listener dispatch (Q#CR6)
// ---------------------------------------------------------------------------

/// Snapshot `name`'s listeners, drop the registry borrow, then invoke
/// each in registration order with copied `(new, old, buf)` values.
///
/// The borrow-release is the whole point (Q#CR6): by the time any
/// listener body runs, `reg.borrow()` from *within* that body (e.g. a
/// re-entrant `pmacs.config.set`) sees no outstanding borrow from us.
/// A raising listener is logged to the `*errors*` buffer and does not
/// stop later listeners or affect the already-committed value.
/// [`MAX_DISPATCH_DEPTH`] bounds re-entrant dispatch depth.
fn dispatch_config_listeners(
    lua: &Lua,
    reg: &SharedConfigRegistry,
    name: &str,
    change: &ConfigChange,
    buf: Option<BufferIdLua>,
) -> mlua::Result<()> {
    let listeners = reg.borrow().snapshot(name);
    if listeners.is_empty() {
        return Ok(());
    }

    let depth_cell = lua
        .app_data_ref::<ConfigDispatchDepth>()
        .expect("ConfigDispatchDepth installed by install_config")
        .0
        .clone();
    let depth = depth_cell.get();
    if depth >= MAX_DISPATCH_DEPTH {
        return Err(mlua::Error::external(ConfigBindingError::ListenerCycle {
            max: MAX_DISPATCH_DEPTH,
        }));
    }
    depth_cell.set(depth + 1);
    let _guard = DispatchDepthGuard(depth_cell);

    let new_lua = config_value_to_lua(lua, &change.new)?;
    let old_lua = config_value_to_lua(lua, &change.old)?;
    for listener in listeners {
        if let Err(err) = listener
            .body
            .call::<()>((new_lua.clone(), old_lua.clone(), buf))
        {
            log_config_listener_error(lua, &listener.source, &err);
        }
    }
    Ok(())
}

/// Append a one-line entry to the `*errors*` buffer naming the
/// listener's source. Mirrors `log_hook_error` / `log_buffer_removed_error`
/// in `mod.rs`; a no-op (rather than a panic) if the buffer registry
/// app data isn't installed, matching those precedents.
fn log_config_listener_error(lua: &Lua, source: &SourceLocation, err: &mlua::Error) {
    let line = format!(
        "[config] on_change listener at {} raised: {err}\n",
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
        super::notify_buffer_edit_to_windows(lua, id, &edit);
    }
}

/// If the init phase has completed and the registry isn't frozen yet,
/// freeze it now. Called from `set`/`reset` only (see the module doc's
/// "startup freeze" section) --- `set_local` never consults the frozen
/// flag at all, so freezing ahead of it would change nothing.
fn maybe_freeze_after_init(lua: &Lua, reg: &SharedConfigRegistry) {
    let complete = lua
        .app_data_ref::<InitCompleteFlag>()
        .is_some_and(|f| f.is_complete());
    if complete {
        let mut r = reg.borrow_mut();
        if !r.is_frozen() {
            r.freeze();
        }
    }
}

// ---------------------------------------------------------------------------
// describe() / list()
// ---------------------------------------------------------------------------

/// Build a fresh descriptor table for `name`, shared by `describe` and
/// `list`. Never returns a handle onto registry state (Q#CR3): every
/// field is copied out.
fn describe_one(
    lua: &Lua,
    reg: &ConfigRegistry,
    name: &str,
    buf: Option<BufferIdLua>,
) -> mlua::Result<Table> {
    let def = reg.get_definition(name).ok_or_else(|| {
        mlua::Error::external(ConfigError::NotFound {
            name: name.to_owned(),
        })
    })?;

    let t = lua.create_table()?;
    t.set("name", def.name.clone())?;
    t.set("description", def.description.clone())?;
    t.set("type", def.kind.type_name())?;
    t.set("default", config_value_to_lua(lua, &def.default)?)?;
    match &def.kind {
        ConfigKind::Boolean => {}
        ConfigKind::Integer { min, max } => {
            if let Some(v) = min {
                t.set("min", *v)?;
            }
            if let Some(v) = max {
                t.set("max", *v)?;
            }
        }
        ConfigKind::Number { min, max } => {
            if let Some(v) = min {
                t.set("min", *v)?;
            }
            if let Some(v) = max {
                t.set("max", *v)?;
            }
        }
        ConfigKind::String { allow_empty } => {
            t.set("allow_empty", *allow_empty)?;
        }
        ConfigKind::Enum { choices } => {
            let choices_t = lua.create_table_with_capacity(choices.len(), 0)?;
            for (i, c) in choices.iter().enumerate() {
                choices_t.set(i + 1, c.as_str())?;
            }
            t.set("choices", choices_t)?;
        }
    }
    t.set(
        "mutability",
        match def.mutability {
            ConfigMutability::Live => "live",
            ConfigMutability::StartupOnly => "startup",
        },
    )?;

    // `value` follows the same buf-argument contract as `get` (F9):
    // with `buf`, buffer-local -> global -> default; with `None`, the
    // global chain only. `global` is always the global-chain
    // resolution, independent of `buf`, so a caller can always see
    // "what would this be with no override at all on my buffer."
    let value = reg
        .get(name, buf.map(BufferIdLua::id))
        .map_err(mlua::Error::external)?;
    t.set("value", config_value_to_lua(lua, value)?)?;
    let global = reg.get(name, None).map_err(mlua::Error::external)?;
    t.set("global", config_value_to_lua(lua, global)?)?;

    // F7: `buffer_local`, never `local` (a Lua keyword). Present only
    // when a buffer was given AND that buffer holds an override.
    if let Some(b) = buf
        && let Some(v) = reg.local_override(name, b.id())
    {
        t.set("buffer_local", config_value_to_lua(lua, v)?)?;
    }

    t.set("source", def.source.render())?;
    Ok(t)
}

// ---------------------------------------------------------------------------
// on_change() handle
// ---------------------------------------------------------------------------

/// Userdata handle returned by `pmacs.config.on_change`. Models the
/// `dispose` binding at `mod.rs:1867` (the compile-mode style-overlay
/// handle): explicit, idempotent teardown, no `MetaMethod::Gc` ---
/// `ConfigRegistry::dispose` is itself idempotent and generation-safe
/// by listener id, so this wrapper adds no extra state of its own. A
/// dropped-but-never-disposed handle keeps firing (F3): there is
/// nothing here that would stop it.
struct ConfigListenerHandleLua {
    id: u64,
    registry: SharedConfigRegistry,
}

impl UserData for ConfigListenerHandleLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dispose", |_, this, ()| {
            this.registry.borrow_mut().dispose(this.id);
            Ok(())
        });
    }
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

/// Install `pmacs.config.*` over `registry`.
///
/// Called from [`super::install`] while the `pmacs` table is being
/// built, so `pmacs.config` exists before any `builtin/runtime/*.lua`
/// chunk evaluates (Q#CR14) --- the same ordering guarantee
/// `pmacs.command` and `pmacs.hook` already have.
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings; splitting fragments a coherent surface"
)]
pub fn install_config(lua: &Lua, registry: &SharedConfigRegistry) -> mlua::Result<Table> {
    lua.set_app_data(ConfigDispatchDepth::new());
    let config_mod = lua.create_table()?;

    {
        let reg = registry.clone();
        config_mod.set(
            "define",
            lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
                check_unknown_fields(&spec, DEFINE_SPEC_FIELDS)?;
                let name: String = spec.raw_get::<Option<String>>("name")?.unwrap_or_default();
                let description: String = spec
                    .raw_get::<Option<String>>("description")?
                    .unwrap_or_default();
                let (kind, default) = parse_kind_and_default(&spec, &name)?;
                let mutability = parse_mutability(&spec)?;
                reg.borrow_mut()
                    .define(
                        name,
                        description,
                        kind,
                        default,
                        mutability,
                        caller_source(lua, 2),
                    )
                    .map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "get",
            lua.create_function(move |lua, (name, buf): (String, Option<BufferIdLua>)| {
                let r = reg.borrow();
                let v = r
                    .get(&name, buf.map(BufferIdLua::id))
                    .map_err(mlua::Error::external)?;
                config_value_to_lua(lua, v)
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "is_set",
            lua.create_function(move |_, (name, buf): (String, Option<BufferIdLua>)| {
                reg.borrow()
                    .is_set(&name, buf.map(BufferIdLua::id))
                    .map_err(mlua::Error::external)
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "set",
            lua.create_function(
                move |lua, (name, value): (String, Value)| -> mlua::Result<()> {
                    maybe_freeze_after_init(lua, &reg);
                    let kind = {
                        let r = reg.borrow();
                        r.get_definition(&name)
                            .ok_or_else(|| {
                                mlua::Error::external(ConfigError::NotFound { name: name.clone() })
                            })?
                            .kind
                            .clone()
                    };
                    let cv = lua_to_config_value(&name, &kind, value)?;
                    let change = reg
                        .borrow_mut()
                        .set(&name, cv)
                        .map_err(mlua::Error::external)?;
                    if change.changed {
                        dispatch_config_listeners(lua, &reg, &name, &change, None)?;
                    }
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "set_local",
            lua.create_function(
                move |lua, (buf, name, value): (BufferIdLua, String, Value)| -> mlua::Result<()> {
                    let kind = {
                        let r = reg.borrow();
                        r.get_definition(&name)
                            .ok_or_else(|| {
                                mlua::Error::external(ConfigError::NotFound { name: name.clone() })
                            })?
                            .kind
                            .clone()
                    };
                    let cv = lua_to_config_value(&name, &kind, value)?;
                    let change = reg
                        .borrow_mut()
                        .set_local(buf.id(), &name, cv)
                        .map_err(mlua::Error::external)?;
                    if change.changed {
                        dispatch_config_listeners(lua, &reg, &name, &change, Some(buf))?;
                    }
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "reset",
            lua.create_function(
                move |lua, (name, buf): (String, Option<BufferIdLua>)| -> mlua::Result<()> {
                    maybe_freeze_after_init(lua, &reg);
                    let change = reg
                        .borrow_mut()
                        .reset(&name, buf.map(BufferIdLua::id))
                        .map_err(mlua::Error::external)?;
                    if change.changed {
                        dispatch_config_listeners(lua, &reg, &name, &change, buf)?;
                    }
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "describe",
            lua.create_function(move |lua, (name, buf): (String, Option<BufferIdLua>)| {
                describe_one(lua, &reg.borrow(), &name, buf)
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "list",
            lua.create_function(move |lua, ()| {
                let r = reg.borrow();
                let out = lua.create_table_with_capacity(r.names().len(), 0)?;
                for (i, name) in r.names().iter().enumerate() {
                    out.set(i + 1, describe_one(lua, &r, name, None)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let reg = registry.clone();
        config_mod.set(
            "on_change",
            lua.create_function(move |lua, (name, body): (String, Function)| {
                let id = reg
                    .borrow_mut()
                    .on_change(&name, body, caller_source(lua, 2))
                    .map_err(mlua::Error::external)?;
                Ok(ConfigListenerHandleLua {
                    id,
                    registry: reg.clone(),
                })
            })?,
        )?;
    }

    Ok(config_mod)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BufferId;

    fn fresh() -> (Lua, SharedConfigRegistry) {
        let lua = Lua::new();
        let registry: SharedConfigRegistry = Rc::new(RefCell::new(ConfigRegistry::new()));
        let config_mod = install_config(&lua, &registry).expect("install_config");
        let pmacs = lua.create_table().expect("pmacs table");
        pmacs.set("config", config_mod).expect("pmacs.config");
        lua.globals().set("pmacs", pmacs).expect("globals");
        (lua, registry)
    }

    /// Execute a chunk purely for side effects; discards any return.
    fn run(lua: &Lua, src: &str) -> mlua::Result<()> {
        lua.load(src).exec()
    }

    /// Evaluate a chunk and convert its return value(s) to `T`.
    fn eval<T: mlua::FromLuaMulti>(lua: &Lua, src: &str) -> mlua::Result<T> {
        lua.load(src).eval::<T>()
    }

    /// Like [`run`], but under an explicit chunk name --- for
    /// exercising `caller_source`'s capture of a "real" file location.
    fn run_named(lua: &Lua, name: &str, src: &str) -> mlua::Result<()> {
        lua.load(src).set_name(name).exec()
    }

    // ---- acceptance 1: round-trip every kind, via Lua ----------------------

    /// The names below are ILLUSTRATIVE — one per `ConfigKind`, chosen
    /// to read plausibly. Only `editing.auto-pair` and
    /// `autosave.interval-ms` are real settings (`builtin/runtime/`); the
    /// other three are defined nowhere but this test and its twin in
    /// `config_registry.rs`. `editing.fill-column` in particular has
    /// never shipped, so no user can get it, set it, or discover it.
    #[test]
    fn define_then_get_round_trips_every_kind_via_lua() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="editing.auto-pair", description="d", type="boolean", default=true }
            pmacs.config.define{ name="autosave.interval-ms", description="d", type="integer", default=30000, min=1000 }
            pmacs.config.define{ name="editing.fill-column", description="d", type="number", default=80.0, min=1.0, max=1000.0 }
            pmacs.config.define{ name="editing.comment-prefix", description="d", type="string", default="", allow_empty=true }
            pmacs.config.define{ name="editing.line-ending", description="d", type="enum", default="lf", choices={"lf","crlf"} }
        "#,
        )
        .unwrap();

        assert!(eval::<bool>(&lua, "return pmacs.config.get('editing.auto-pair')").unwrap());
        assert_eq!(
            eval::<i64>(&lua, "return pmacs.config.get('autosave.interval-ms')").unwrap(),
            30_000
        );
        assert!(
            (eval::<f64>(&lua, "return pmacs.config.get('editing.fill-column')").unwrap() - 80.0)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            eval::<String>(&lua, "return pmacs.config.get('editing.comment-prefix')").unwrap(),
            ""
        );
        assert_eq!(
            eval::<String>(&lua, "return pmacs.config.get('editing.line-ending')").unwrap(),
            "lf"
        );
    }

    // ---- acceptance 2: R50 unknown field + metatable smuggling -------------

    #[test]
    fn define_rejects_unknown_field_naming_offender_and_supported_keys() {
        let (lua, _reg) = fresh();
        let err = run(
            &lua,
            r#"pmacs.config.define{ name="x.y", description="d", type="boolean", default=true, typo_field=1 }"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("typo_field"), "{msg}");
        assert!(msg.contains("supported"), "{msg}");
        assert!(msg.contains("name"), "{msg}");
    }

    #[test]
    fn define_rejects_missing_or_whitespace_description() {
        let (lua, _reg) = fresh();
        let err = run(
            &lua,
            r#"pmacs.config.define{ name="x.y", description="   ", type="boolean", default=true }"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("R42"), "{err}");
    }

    #[test]
    fn define_ignores_metatable_provided_default_via_raw_access() {
        // R50's harder half: a spec table whose OWN keys never include
        // `default` at all -- a metatable `__index` answers for it
        // instead. Raw access must not pick that up: the define must
        // fail as though `default` were absent (nil), not silently
        // succeed with the metatable-smuggled value.
        let (lua, _reg) = fresh();
        let err = run(
            &lua,
            r#"
            local spec = setmetatable({ name="x.y", description="d", type="boolean" }, {
                __index = function(_, k) if k == "default" then return true end end,
            })
            pmacs.config.define(spec)
        "#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("boolean"),
            "expected a boolean-vs-nil type mismatch, got: {err}"
        );
        // And the definition must not have been registered.
        let names: i64 = eval(&lua, "return #pmacs.config.list()").unwrap();
        assert_eq!(
            names, 0,
            "the rejected define must not have registered anything"
        );
    }

    // ---- review round 1, finding 3: correctly spelled, wrongly typed --------

    #[test]
    fn define_rejects_spec_fields_meaningless_for_the_declared_type() {
        // The R50 whitelist accepts all nine keys for every type, so
        // these are invisible to it: each field below is spelled
        // correctly but means nothing for the type it is paired with.
        // Before the kind cross-check, `type = "string"` with `choices`
        // silently defined a string that accepts ANYTHING -- the author
        // plainly meant `enum` -- and `min` on a boolean was dropped.
        for (spec, offender) in [
            (
                r#"{ name="a.b", description="d", type="string", default="x", choices={"a"} }"#,
                "choices",
            ),
            (
                r#"{ name="a.b", description="d", type="boolean", default=true, min=1 }"#,
                "min",
            ),
            (
                r#"{ name="a.b", description="d", type="boolean", default=true, max=1 }"#,
                "max",
            ),
            (
                r#"{ name="a.b", description="d", type="integer", default=1, allow_empty=true }"#,
                "allow_empty",
            ),
            (
                r#"{ name="a.b", description="d", type="enum", choices={"x"}, default="x", min=1 }"#,
                "min",
            ),
        ] {
            let (lua, _reg) = fresh();
            let err = run(&lua, &format!("pmacs.config.define {spec}")).unwrap_err();
            assert!(
                err.to_string().contains(offender),
                "the error must name the misplaced field {offender}: {err}"
            );
            let n: i64 = eval(&lua, "return #pmacs.config.list()").unwrap();
            assert_eq!(n, 0, "a rejected define registers nothing");
        }
    }

    #[test]
    fn define_still_accepts_each_field_on_its_own_type() {
        // The guard must not over-reject: every field remains legal
        // where it belongs, including on `number` as well as `integer`.
        let (lua, _reg) = fresh();
        for spec in [
            r#"{ name="a.int", description="d", type="integer", default=5, min=1, max=9 }"#,
            r#"{ name="a.num", description="d", type="number", default=0.5, min=0.0, max=1.0 }"#,
            r#"{ name="a.str", description="d", type="string", default="", allow_empty=true }"#,
            r#"{ name="a.enum", description="d", type="enum", default="x", choices={"x","y"} }"#,
            r#"{ name="a.bool", description="d", type="boolean", default=true }"#,
        ] {
            run(&lua, &format!("pmacs.config.define {spec}"))
                .unwrap_or_else(|e| panic!("{spec} must be accepted: {e}"));
        }
        let n: i64 = eval(&lua, "return #pmacs.config.list()").unwrap();
        assert_eq!(n, 5);
    }

    // ---- review round 1, finding 1: the saturating i64 boundary, via Lua ----

    #[test]
    fn set_rejects_the_saturating_i64_boundary_from_lua() {
        // 2^63 is a float on BOTH backends (LuaJIT has no integer
        // subtype; Lua 5.4's `2^63` is a float too). Before the `>=`
        // fix this stored i64::MAX -- a silent off-by-one against the
        // module's own "never silently change its value" contract.
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="a.n", description="d", type="integer", default=0 }"#,
        )
        .unwrap();
        let err = run(&lua, "pmacs.config.set('a.n', 2^63)").unwrap_err();
        assert!(
            err.to_string().contains("integer") || err.to_string().contains("integral"),
            "2^63 must be refused, not saturated: {err}"
        );
        let still: i64 = eval(&lua, "return pmacs.config.get('a.n')").unwrap();
        assert_eq!(still, 0, "the refused set must not have stored anything");
    }

    #[test]
    fn define_rejects_the_saturating_i64_boundary_in_a_bound() {
        // `min`/`max` go through read_bound_i64 -> lua_exact_i64 ->
        // int_from_f64, so the same boundary must be refused in a spec.
        let (lua, _reg) = fresh();
        let err = run(
            &lua,
            r#"pmacs.config.define{ name="a.n", description="d", type="integer", default=0, max=2^63 }"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("integer") || err.to_string().contains("integral"),
            "a 2^63 bound must be refused: {err}"
        );
    }

    // ---- acceptance 9: SourceLocation from the DEFINING module --------------

    #[test]
    fn source_location_is_captured_from_the_defining_chunk_not_a_helper() {
        let (lua, _reg) = fresh();
        run_named(
            &lua,
            "@pmacs/builtin/runtime/pair.lua",
            "\n\npmacs.config.define{ name='editing.auto-pair', description='d', type='boolean', default=true }\n",
        )
        .expect("define from a named chunk");

        let source: String = eval(
            &lua,
            "return pmacs.config.describe('editing.auto-pair').source",
        )
        .unwrap();
        assert!(
            source.starts_with("pmacs/builtin/runtime/pair.lua:"),
            "expected the defining chunk's own path, got {source:?}"
        );
        assert!(
            source.ends_with(":3"),
            "expected line 3 (the call sits after two blank lines), got {source:?}"
        );
    }

    // ---- F9: get(name) with no buffer resolves the global chain only -------

    #[test]
    fn get_with_no_buffer_ignores_any_local_override() {
        let (lua, reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }"#,
        )
        .unwrap();
        let buf = BufferId::next();
        reg.borrow_mut()
            .set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap();

        lua.globals().set("__buf", BufferIdLua(buf)).unwrap();
        assert!(
            eval::<bool>(&lua, "return pmacs.config.get('editing.x')").unwrap(),
            "no-buffer get must see the global chain only"
        );
        assert!(!eval::<bool>(&lua, "return pmacs.config.get('editing.x', __buf)").unwrap());
    }

    // ---- listeners: the borrow-release bite test ----------------------------

    #[test]
    fn listener_runs_after_borrow_release_and_can_reentrantly_write() {
        // Bite-verified: if `dispatch_config_listeners` still held a
        // `RefCell` borrow on the registry while calling the listener
        // body, the listener's own `pmacs.config.set` below would hit
        // a `BorrowMutError` panic. No panic, plus observing the
        // second setting's new value, is the proof the borrow was
        // released before Lua ran.
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="a", description="d", type="boolean", default=true }
            pmacs.config.define{ name="b", description="d", type="boolean", default=true }
            pmacs.config.on_change('a', function(new, old, buf)
                pmacs.config.set('b', false)
            end)
            pmacs.config.set('a', false)
        "#,
        )
        .unwrap();

        assert!(
            !eval::<bool>(&lua, "return pmacs.config.get('b')").unwrap(),
            "the re-entrant set from inside the listener must have committed"
        );
    }

    // ---- Q#CR6 (a)/(b): fires once with buf=nil, not for a shadowed buffer -

    #[test]
    fn global_set_fires_once_with_nil_buf_and_not_for_a_shadowed_buffer() {
        let (lua, reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }"#,
        )
        .unwrap();
        let buf = BufferId::next();
        reg.borrow_mut()
            .set_local(buf, "editing.x", ConfigValue::Bool(true))
            .unwrap();

        run(
            &lua,
            r"
            calls = {}
            pmacs.config.on_change('editing.x', function(new, old, buf)
                table.insert(calls, { new = new, buf = buf })
            end)
            pmacs.config.set('editing.x', false)
        ",
        )
        .unwrap();

        assert_eq!(
            eval::<i64>(&lua, "return #calls").unwrap(),
            1,
            "must fire exactly once, not once per buffer"
        );
        assert!(
            eval::<bool>(&lua, "return calls[1].buf == nil").unwrap(),
            "global set must report buf = nil"
        );

        // The shadowed buffer's effective value must not have moved.
        assert_eq!(
            reg.borrow().get("editing.x", Some(buf)).unwrap(),
            &ConfigValue::Bool(true)
        );
    }

    // ---- Q#CR6 (c): remove_buffer purges without firing --------------------

    #[test]
    fn remove_buffer_purges_locals_without_firing_a_listener() {
        let (lua, reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }
            fire_count = 0
            pmacs.config.on_change('editing.x', function() fire_count = fire_count + 1 end)
        "#,
        )
        .unwrap();
        let buf = BufferId::next();

        // set_local DOES fire (the buffer's effective value changes) --
        // establishes a nonzero baseline so the next assertion is
        // meaningful (not just "still zero because nothing happened").
        reg.borrow_mut()
            .set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap();
        // set_local was driven directly through the core (not the Lua
        // binding above), so it did not dispatch our Lua listener --
        // that's fine; this test is about remove_buffer specifically.
        let before: i64 = eval(&lua, "return fire_count").unwrap();

        reg.borrow_mut().remove_buffer(buf);
        let after: i64 = eval(&lua, "return fire_count").unwrap();
        assert_eq!(before, after, "remove_buffer must never fire a listener");
    }

    // ---- Q#CR6 (d): on_change on an undefined name raises NotFound ---------

    #[test]
    fn on_change_on_undefined_name_raises_not_found() {
        let (lua, _reg) = fresh();
        let err = run(&lua, "pmacs.config.on_change('nope', function() end)").unwrap_err();
        assert!(err.to_string().contains("not defined"), "{err}");
    }

    // ---- one raising listener is logged and does not block later ones ------

    #[test]
    fn one_raising_listener_does_not_block_later_listeners_or_the_value() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }
            second_ran = false
            pmacs.config.on_change('editing.x', function() error("boom") end)
            pmacs.config.on_change('editing.x', function() second_ran = true end)
            pmacs.config.set('editing.x', false)
        "#,
        )
        .unwrap();

        assert!(
            eval::<bool>(&lua, "return second_ran").unwrap(),
            "a raising listener must not block later ones"
        );
        assert!(
            !eval::<bool>(&lua, "return pmacs.config.get('editing.x')").unwrap(),
            "the committed value must stay authoritative despite the raise"
        );
    }

    // ---- recursion bound: an accidental listener cycle is stopped ----------

    #[test]
    fn listener_cycle_hits_the_recursion_bound_instead_of_hanging() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="a", description="d", type="integer", default=0 }
            pmacs.config.define{ name="b", description="d", type="integer", default=0 }
            n = 0
            pmacs.config.on_change('a', function(new)
                n = n + 1
                pmacs.config.set('b', new)
            end)
            pmacs.config.on_change('b', function(new)
                n = n + 1
                pmacs.config.set('a', new + 1)
            end)
        "#,
        )
        .unwrap();

        // The top-level call itself must return successfully -- each
        // nested listener failure is logged and absorbed by the level
        // above, so the cycle unwinds cleanly rather than raising all
        // the way out.
        run(&lua, "pmacs.config.set('a', 1)").expect("top-level call must not raise");

        let n: i64 = eval(&lua, "return n").unwrap();
        assert_eq!(
            n,
            i64::from(MAX_DISPATCH_DEPTH),
            "exactly MAX_DISPATCH_DEPTH listener invocations run before the bound stops the cycle"
        );
    }

    // ---- dispose: idempotent, generation-safe, no GC dependency (F3, 22) ---

    #[test]
    fn dispose_is_idempotent_and_a_dropped_undisposed_handle_keeps_firing() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }
            fires = 0
            do
                local h = pmacs.config.on_change('editing.x', function() fires = fires + 1 end)
                h:dispose()
                h:dispose() -- idempotent: no error
            end
            collectgarbage("collect")
            pmacs.config.set('editing.x', false)
        "#,
        )
        .unwrap();
        assert_eq!(
            eval::<i64>(&lua, "return fires").unwrap(),
            0,
            "disposed listener must not fire"
        );

        run(
            &lua,
            r#"
            pmacs.config.define{ name="editing.y", description="d", type="boolean", default=true }
            fires2 = 0
            do
                local h2 = pmacs.config.on_change('editing.y', function() fires2 = fires2 + 1 end)
            end
            collectgarbage("collect")
            pmacs.config.set('editing.y', false)
        "#,
        )
        .unwrap();
        assert_eq!(
            eval::<i64>(&lua, "return fires2").unwrap(),
            1,
            "a dropped-but-undisposed handle must keep firing (F3)"
        );
    }

    #[test]
    fn dispose_is_generation_safe_a_stale_id_never_disposes_a_newer_listener() {
        let (lua, reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }"#,
        )
        .unwrap();
        run(
            &lua,
            r"
            first_fires = 0
            h1 = pmacs.config.on_change('editing.x', function() first_fires = first_fires + 1 end)
            h1:dispose()
            second_fires = 0
            h2 = pmacs.config.on_change('editing.x', function() second_fires = second_fires + 1 end)
            h1:dispose() -- stale: must not touch h2's listener
        ",
        )
        .unwrap();
        assert_eq!(reg.borrow().snapshot("editing.x").len(), 1);
        run(&lua, "pmacs.config.set('editing.x', false)").unwrap();
        assert_eq!(eval::<i64>(&lua, "return first_fires").unwrap(), 0);
        assert_eq!(eval::<i64>(&lua, "return second_fires").unwrap(), 1);
    }

    // ---- startup freeze, lazy via InitCompleteFlag --------------------------

    #[test]
    fn set_after_init_complete_freezes_and_rejects_startup_only_writes() {
        let (lua, _reg) = fresh();
        let flag = InitCompleteFlag::new();
        lua.set_app_data(flag.clone());
        run(
            &lua,
            r#"pmacs.config.define{ name="lsp.root-markers", description="d", type="boolean", default=true, mutability="startup" }"#,
        )
        .unwrap();

        // Before init completes: writable.
        run(&lua, "pmacs.config.set('lsp.root-markers', false)").unwrap();

        flag.set_complete();
        let err = run(&lua, "pmacs.config.set('lsp.root-markers', true)").unwrap_err();
        assert!(err.to_string().contains("startup-only"), "{err}");
    }

    #[test]
    fn a_set_local_first_does_not_open_a_startup_only_write_window() {
        // Review round 1, finding 2. `set_local` does not call
        // `maybe_freeze_after_init`, so the concern was that a
        // `set_local` as the first post-init operation defers the freeze
        // and lets a subsequent `StartupOnly` write slip through.
        //
        // It cannot: `set` and `reset` call `maybe_freeze_after_init` as
        // their FIRST statement, before the definition lookup and before
        // the mutator runs, so the freeze always lands ahead of the
        // check in the very same call. This test drives that exact
        // ordering -- post-init `set_local` on a Live key, then a
        // `StartupOnly` write -- and asserts the write is still refused.
        let (lua, reg) = fresh();
        let flag = InitCompleteFlag::new();
        lua.set_app_data(flag.clone());
        run(
            &lua,
            r#"pmacs.config.define{ name="editing.live-one", description="d", type="boolean", default=true }"#,
        )
        .unwrap();
        run(
            &lua,
            r#"pmacs.config.define{ name="lsp.root-markers", description="d", type="boolean", default=true, mutability="startup" }"#,
        )
        .unwrap();

        flag.set_complete();
        assert!(
            !reg.borrow().is_frozen(),
            "precondition: nothing has triggered the lazy freeze yet"
        );

        // The first post-init operation is a set_local on a Live key.
        let buf = BufferId::next();
        lua.globals().set("__buf", BufferIdLua(buf)).unwrap();
        run(
            &lua,
            "pmacs.config.set_local(__buf, 'editing.live-one', false)",
        )
        .unwrap();

        // The StartupOnly write must still be refused.
        let err = run(&lua, "pmacs.config.set('lsp.root-markers', false)").unwrap_err();
        assert!(
            err.to_string().contains("startup-only"),
            "a set_local first must not open a write window: {err}"
        );
        assert!(
            reg.borrow().is_frozen(),
            "the set that was refused is itself what triggered the freeze"
        );
    }

    #[test]
    fn define_startup_only_always_rejects_set_local() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="lsp.root-markers", description="d", type="boolean", default=true, mutability="startup" }"#,
        )
        .unwrap();
        let buf = BufferId::next();
        lua.globals().set("__buf", BufferIdLua(buf)).unwrap();
        let err = run(
            &lua,
            "pmacs.config.set_local(__buf, 'lsp.root-markers', false)",
        )
        .unwrap_err();
        assert!(err.to_string().contains("buffer-local"), "{err}");
    }

    // ---- describe(): field shape, buffer_local naming (F7), fresh table ----

    #[test]
    fn describe_has_buffer_local_field_only_when_a_buffer_override_exists() {
        let (lua, reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }"#,
        )
        .unwrap();
        let buf = BufferId::next();
        lua.globals().set("__buf", BufferIdLua(buf)).unwrap();

        assert!(
            eval::<bool>(
                &lua,
                "return pmacs.config.describe('editing.x', __buf).buffer_local == nil"
            )
            .unwrap(),
            "buffer_local must be absent with no override"
        );

        reg.borrow_mut()
            .set_local(buf, "editing.x", ConfigValue::Bool(false))
            .unwrap();
        assert!(
            eval::<bool>(
                &lua,
                "return pmacs.config.describe('editing.x', __buf).buffer_local == false"
            )
            .unwrap(),
            "buffer_local must reflect the stored override"
        );

        // Every documented field is present, and describe() returns a
        // FRESH table each call (mutating one call's result cannot
        // affect the next).
        assert!(
            eval::<bool>(
                &lua,
                r#"
                local info = pmacs.config.describe('editing.x', __buf)
                info.name = "mutated"
                local info2 = pmacs.config.describe('editing.x', __buf)
                return info2.name == 'editing.x'
                    and info2.description == 'd'
                    and info2.type == 'boolean'
                    and info2.default == true
                    and info2.mutability == 'live'
                    and info2.value == false
                    and info2.global == true
                    and type(info2.source) == 'string'
            "#
            )
            .unwrap()
        );
    }

    #[test]
    fn list_is_stable_and_returns_full_descriptor_tables() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="c", description="d", type="boolean", default=true }
            pmacs.config.define{ name="a", description="d", type="boolean", default=true }
            pmacs.config.define{ name="b", description="d", type="boolean", default=true }
        "#,
        )
        .unwrap();

        let check = r"
            local l = pmacs.config.list()
            return #l == 3 and l[1].name == 'c' and l[2].name == 'a' and l[3].name == 'b'
                and l[1].description == 'd'
        ";
        assert!(
            eval::<bool>(&lua, check).unwrap(),
            "list() must be stable across repeated calls"
        );
        assert!(eval::<bool>(&lua, check).unwrap(), "and on a second call");
    }

    // ---- acceptance 6 (generic across luajit/lua54): exactness by value ----

    #[test]
    fn integer_exactness_is_checked_by_value_not_math_type() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="n", description="d", type="integer", default=0 }"#,
        )
        .unwrap();
        run(&lua, "pmacs.config.set('n', 1500)").unwrap();
        assert_eq!(
            eval::<i64>(&lua, "return pmacs.config.get('n')").unwrap(),
            1500
        );

        let err = run(&lua, "pmacs.config.set('n', 1500.7)").unwrap_err();
        assert!(err.to_string().contains("exact integer"), "{err}");

        // A whole-numbered float is accepted exactly (matters most
        // under LuaJIT, where this is the ONLY way an integer literal
        // ever arrives -- Lua 5.1 has no integer subtype).
        run(&lua, "pmacs.config.set('n', 42.0)").unwrap();
        assert_eq!(
            eval::<i64>(&lua, "return pmacs.config.get('n')").unwrap(),
            42
        );

        // Out-of-i64-range / non-finite floats are rejected too.
        let err2 = run(&lua, "pmacs.config.set('n', 1/0)").unwrap_err();
        assert!(err2.to_string().contains("finite"), "{err2}");
    }

    #[test]
    fn is_set_reports_override_presence_and_reset_drops_exactly_one_layer() {
        let (lua, reg) = fresh();
        run(
            &lua,
            r#"pmacs.config.define{ name="editing.x", description="d", type="boolean", default=true }"#,
        )
        .unwrap();
        let buf = BufferId::next();
        lua.globals().set("__buf", BufferIdLua(buf)).unwrap();

        assert!(!eval::<bool>(&lua, "return pmacs.config.is_set('editing.x')").unwrap());
        run(&lua, "pmacs.config.set('editing.x', true)").unwrap(); // equal to default (F1)
        assert!(
            eval::<bool>(&lua, "return pmacs.config.is_set('editing.x')").unwrap(),
            "an equal-valued override is still stored (F1)"
        );

        run(&lua, "pmacs.config.set_local(__buf, 'editing.x', false)").unwrap();
        assert!(eval::<bool>(&lua, "return pmacs.config.is_set('editing.x', __buf)").unwrap());

        run(&lua, "pmacs.config.reset('editing.x', __buf)").unwrap();
        assert!(!eval::<bool>(&lua, "return pmacs.config.is_set('editing.x', __buf)").unwrap());
        assert!(
            eval::<bool>(&lua, "return pmacs.config.is_set('editing.x')").unwrap(),
            "reset(name, buf) drops only the local layer"
        );

        run(&lua, "pmacs.config.reset('editing.x')").unwrap();
        assert!(!eval::<bool>(&lua, "return pmacs.config.is_set('editing.x')").unwrap());
        drop(reg);
    }

    // ---- define-before-use (Q#CR10) -----------------------------------------

    #[test]
    fn ops_on_an_undefined_name_raise_not_found() {
        let (lua, _reg) = fresh();
        for src in [
            "pmacs.config.get('nope')",
            "pmacs.config.set('nope', true)",
            "pmacs.config.is_set('nope')",
            "pmacs.config.describe('nope')",
            "pmacs.config.reset('nope')",
        ] {
            let err = run(&lua, src).unwrap_err();
            assert!(err.to_string().contains("not defined"), "{src}: {err}");
        }
    }

    // ---- enum / bounds validation surfaces through the Lua boundary --------

    #[test]
    fn enum_and_bounds_validation_surfaces_through_lua() {
        let (lua, _reg) = fresh();
        run(
            &lua,
            r#"
            pmacs.config.define{ name="editing.eol", description="d", type="enum", default="lf", choices={"lf","crlf"} }
            pmacs.config.define{ name="autosave.interval-ms", description="d", type="integer", default=30000, min=1000 }
        "#,
        )
        .unwrap();

        let err = run(&lua, "pmacs.config.set('editing.eol', 'cr')").unwrap_err();
        assert!(err.to_string().contains("cr"), "{err}");

        let err2 = run(&lua, "pmacs.config.set('autosave.interval-ms', 500)").unwrap_err();
        assert!(err2.to_string().contains("range"), "{err2}");
    }
}
