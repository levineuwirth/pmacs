// hook.rs --- Hook registry and run engine (T M2.6).

//! Typed hooks per spec §4.4.
//!
//! A hook is a named extension point. Lua code defines a hook with
//! [`HookRegistry::define`], attaches callbacks via [`HookRegistry::add`],
//! and the editor (or Lua) fires it via [`run_snapshot`] which executes
//! every callback in registration order according to the hook's
//! [`HookKind`].
//!
//! # Composition kinds
//!
//! * [`HookKind::ShortCircuit`] --- callbacks are veto-able. The first
//!   callback that returns `false` stops the run; the [`HookOutcome`]
//!   reports the veto. Used for `before-save`, `before-quit`: any
//!   listener can refuse the action. A raised error counts as a veto.
//! * [`HookKind::AllMustSucceed`] --- run every callback. Any errors
//!   are collected but do not stop later callbacks. The outcome is
//!   "ok" iff no callback raised. Used for `after-load`-style
//!   notifications where every listener should run.
//! * [`HookKind::Accumulate`] --- thread a single value through the
//!   callbacks. The first callback receives the run's input args; each
//!   subsequent callback receives the previous callback's first return
//!   as its first argument (with the original trailing args). The run's
//!   return value is the final callback's first return. An error
//!   aborts the run; the partial accumulator is discarded.
//!
//! # Threading
//!
//! Single-threaded, like the buffer / command / keymap registries.
//! Lives behind `Rc<RefCell<...>>` next to them inside
//! [`crate::lua::LuaHost`]. Callbacks are `mlua::Function`s, which
//! borrow Lua state; firing a hook re-enters Lua.

use std::collections::HashMap;

use mlua::{Function, MultiValue, Value};
use thiserror::Error;

use crate::command::SourceLocation;

/// Composition semantics, selected at [`HookRegistry::define`] time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HookKind {
    /// Stop on the first `false` return or raised error. Used for
    /// veto-able lifecycle hooks.
    ShortCircuit,
    /// Run every callback; aggregate errors. Used for notifications
    /// where each listener is independent.
    AllMustSucceed,
    /// Thread the first return value through the callbacks as their
    /// first argument. Used for transformation pipelines.
    Accumulate,
}

impl HookKind {
    /// Stable string identifier used by the Lua API and in introspection.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShortCircuit => "short-circuit",
            Self::AllMustSucceed => "all-must-succeed",
            Self::Accumulate => "accumulate",
        }
    }

    /// Parse a Lua-side identifier into a kind.
    pub fn parse(s: &str) -> Result<Self, HookError> {
        match s {
            "short-circuit" => Ok(Self::ShortCircuit),
            "all-must-succeed" => Ok(Self::AllMustSucceed),
            "accumulate" => Ok(Self::Accumulate),
            other => Err(HookError::UnknownKind {
                got: other.to_owned(),
            }),
        }
    }
}

/// One callback registered against a hook.
#[derive(Clone)]
pub struct HookCallback {
    /// The Lua function body. Cloning is cheap (mlua refcounts internally).
    pub body: Function,
    /// Where the call to `pmacs.hook.add` originated.
    pub source: SourceLocation,
}

/// A defined hook: name, description, registered callbacks (in
/// registration order).
#[derive(Clone)]
pub struct Hook {
    /// Unique name (e.g. `buffer.before-save`).
    pub name: String,
    /// One-line description (R42, mandatory).
    pub description: String,
    /// Composition semantics.
    pub kind: HookKind,
    /// Where the call to `pmacs.hook.define` originated.
    pub source: SourceLocation,
    /// Callbacks in registration order. Run in this order on every
    /// [`run_snapshot`] invocation.
    pub callbacks: Vec<HookCallback>,
}

/// Errors raised by the hook registry.
#[derive(Debug, Error)]
pub enum HookError {
    /// `pmacs.hook.define` was called with no name.
    #[error("hook name must be non-empty")]
    EmptyName,

    /// R42: `define` was called without a description, or with one
    /// that's empty after trimming.
    #[error("hook \"{name}\" requires a non-empty description (R42)")]
    MissingDescription {
        /// The offending hook name.
        name: String,
    },

    /// A hook with this name is already defined.
    #[error("hook \"{name}\" is already defined (refusing to redefine)")]
    DuplicateName {
        /// The offending hook name.
        name: String,
    },

    /// `pmacs.hook.add` referenced an undefined hook.
    #[error("hook \"{name}\" is not defined")]
    NotFound {
        /// The offending hook name.
        name: String,
    },

    /// R50: a spec table contained a key the registry doesn't know
    /// about.
    #[error("unknown field `{field}` in hook spec; supported: name, description, kind for define")]
    UnknownField {
        /// The offending key.
        field: String,
    },

    /// `kind` was set to a value outside the [`HookKind`] vocabulary.
    #[error(
        "unknown hook kind `{got}`; expected one of: short-circuit, all-must-succeed, accumulate"
    )]
    UnknownKind {
        /// The offending kind string.
        got: String,
    },
}

/// Result of running a hook.
#[derive(Debug)]
pub struct HookOutcome {
    /// `false` iff a [`HookKind::ShortCircuit`] callback returned
    /// `false` or raised. Always `true` for the other kinds (their
    /// "did it run cleanly" answer is encoded in [`Self::errors`]).
    pub proceed: bool,
    /// Final value:
    /// * [`HookKind::Accumulate`]: the last successful callback's
    ///   first return; [`Value::Nil`] if there were no callbacks.
    /// * Other kinds: [`Value::Nil`].
    pub value: Value,
    /// Errors raised during the run. For [`HookKind::ShortCircuit`]
    /// this is at most one (the first); for the others it lists every
    /// failure in callback order.
    pub errors: Vec<HookCallbackError>,
}

/// One callback's failure during a hook run.
#[derive(Debug)]
pub struct HookCallbackError {
    /// Where the failed callback was registered.
    pub source: SourceLocation,
    /// The mlua error raised (or synthesized for a veto).
    pub error: mlua::Error,
}

/// Registry of named hooks and their callbacks.
#[derive(Default)]
pub struct HookRegistry {
    by_name: HashMap<String, Hook>,
    /// Insertion order for stable listing.
    order: Vec<String>,
}

impl HookRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Define a new hook.
    pub fn define(
        &mut self,
        name: String,
        description: String,
        kind: HookKind,
        source: SourceLocation,
    ) -> Result<(), HookError> {
        if name.is_empty() {
            return Err(HookError::EmptyName);
        }
        if description.trim().is_empty() {
            return Err(HookError::MissingDescription { name });
        }
        if self.by_name.contains_key(&name) {
            return Err(HookError::DuplicateName { name });
        }
        self.order.push(name.clone());
        self.by_name.insert(
            name.clone(),
            Hook {
                name,
                description,
                kind,
                source,
                callbacks: Vec::new(),
            },
        );
        Ok(())
    }

    /// Attach `body` to the hook named `name`. Returns
    /// [`HookError::NotFound`] if the hook hasn't been defined.
    pub fn add(
        &mut self,
        name: &str,
        body: Function,
        source: SourceLocation,
    ) -> Result<(), HookError> {
        let hook = self
            .by_name
            .get_mut(name)
            .ok_or_else(|| HookError::NotFound {
                name: name.to_owned(),
            })?;
        hook.callbacks.push(HookCallback { body, source });
        Ok(())
    }

    /// Look up a hook by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Hook> {
        self.by_name.get(name)
    }

    /// Snapshot the kind + callbacks of a hook so the caller can drop
    /// the registry borrow before invoking user code (which may
    /// re-enter the registry, e.g. another `pmacs.hook.run` from
    /// inside a callback).
    #[must_use]
    pub fn snapshot(&self, name: &str) -> Option<(HookKind, Vec<HookCallback>)> {
        self.by_name
            .get(name)
            .map(|h| (h.kind, h.callbacks.clone()))
    }

    /// Hook names in definition order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Number of defined hooks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// True iff no hooks are defined.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Run a snapshot of a hook's callbacks per its [`HookKind`]. The
/// caller takes the snapshot via [`HookRegistry::snapshot`] and then
/// drops the registry borrow before calling here, so callbacks may
/// freely re-enter the registry.
///
/// `args` is the input arg list; it is consumed (and rebuilt for each
/// callback in [`HookKind::Accumulate`] mode).
pub fn run_snapshot(kind: HookKind, callbacks: &[HookCallback], args: MultiValue) -> HookOutcome {
    match kind {
        HookKind::ShortCircuit => run_short_circuit(callbacks, &args),
        HookKind::AllMustSucceed => run_all_must_succeed(callbacks, &args),
        HookKind::Accumulate => run_accumulate(callbacks, args),
    }
}

fn run_short_circuit(callbacks: &[HookCallback], args: &MultiValue) -> HookOutcome {
    for cb in callbacks {
        match cb.body.call::<MultiValue>(args.clone()) {
            Ok(rets) => {
                // A literal `false` veto stops the run; `nil` is a no-op
                // (callbacks that don't return are equivalent to "proceed").
                if let Some(Value::Boolean(false)) = rets.iter().next() {
                    return HookOutcome {
                        proceed: false,
                        value: Value::Nil,
                        errors: Vec::new(),
                    };
                }
            }
            Err(e) => {
                return HookOutcome {
                    proceed: false,
                    value: Value::Nil,
                    errors: vec![HookCallbackError {
                        source: cb.source.clone(),
                        error: e,
                    }],
                };
            }
        }
    }
    HookOutcome {
        proceed: true,
        value: Value::Nil,
        errors: Vec::new(),
    }
}

fn run_all_must_succeed(callbacks: &[HookCallback], args: &MultiValue) -> HookOutcome {
    let mut errors = Vec::new();
    for cb in callbacks {
        if let Err(e) = cb.body.call::<MultiValue>(args.clone()) {
            errors.push(HookCallbackError {
                source: cb.source.clone(),
                error: e,
            });
        }
    }
    HookOutcome {
        proceed: errors.is_empty(),
        value: Value::Nil,
        errors,
    }
}

fn run_accumulate(callbacks: &[HookCallback], args: MultiValue) -> HookOutcome {
    let trailing: Vec<Value> = args.iter().skip(1).cloned().collect();
    let mut acc: Value = args.into_iter().next().unwrap_or(Value::Nil);
    for cb in callbacks {
        let mut next_args = MultiValue::new();
        next_args.push_back(acc.clone());
        for v in &trailing {
            next_args.push_back(v.clone());
        }
        match cb.body.call::<MultiValue>(next_args) {
            Ok(rets) => {
                acc = rets.into_iter().next().unwrap_or(Value::Nil);
            }
            Err(e) => {
                return HookOutcome {
                    proceed: false,
                    value: Value::Nil,
                    errors: vec![HookCallbackError {
                        source: cb.source.clone(),
                        error: e,
                    }],
                };
            }
        }
    }
    HookOutcome {
        proceed: true,
        value: acc,
        errors: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn src(line: i32) -> SourceLocation {
        SourceLocation {
            file: "test.lua".into(),
            line,
        }
    }

    #[test]
    fn define_then_get_round_trips() {
        let mut r = HookRegistry::new();
        r.define(
            "buffer.before-save".into(),
            "Run before save.".into(),
            HookKind::ShortCircuit,
            src(1),
        )
        .unwrap();
        let h = r.get("buffer.before-save").unwrap();
        assert_eq!(h.description, "Run before save.");
        assert_eq!(h.kind, HookKind::ShortCircuit);
        assert!(h.callbacks.is_empty());
    }

    #[test]
    fn add_appends_in_registration_order() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        r.define("h".into(), "desc".into(), HookKind::AllMustSucceed, src(1))
            .unwrap();
        for line in 10..14 {
            let f = lua.create_function(|_, ()| Ok(())).unwrap();
            r.add("h", f, src(line)).unwrap();
        }
        let cbs = &r.get("h").unwrap().callbacks;
        assert_eq!(cbs.len(), 4);
        assert_eq!(
            cbs.iter().map(|c| c.source.line).collect::<Vec<_>>(),
            vec![10, 11, 12, 13]
        );
    }

    #[test]
    fn empty_name_rejected() {
        let mut r = HookRegistry::new();
        let err = r
            .define(String::new(), "ok".into(), HookKind::AllMustSucceed, src(1))
            .unwrap_err();
        assert!(matches!(err, HookError::EmptyName));
    }

    #[test]
    fn whitespace_description_rejected() {
        let mut r = HookRegistry::new();
        let err = r
            .define(
                "h".into(),
                "   \n\t  ".into(),
                HookKind::AllMustSucceed,
                src(1),
            )
            .unwrap_err();
        assert!(matches!(err, HookError::MissingDescription { .. }));
    }

    #[test]
    fn duplicate_define_rejected() {
        let mut r = HookRegistry::new();
        r.define("h".into(), "ok".into(), HookKind::AllMustSucceed, src(1))
            .unwrap();
        let err = r
            .define("h".into(), "ok".into(), HookKind::AllMustSucceed, src(2))
            .unwrap_err();
        assert!(matches!(err, HookError::DuplicateName { name } if name == "h"));
    }

    #[test]
    fn add_to_undefined_hook_errors() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        let f = lua.create_function(|_, ()| Ok(())).unwrap();
        let err = r.add("nope", f, src(1)).unwrap_err();
        assert!(matches!(err, HookError::NotFound { name } if name == "nope"));
    }

    #[test]
    fn names_in_definition_order() {
        let mut r = HookRegistry::new();
        r.define("a".into(), "a".into(), HookKind::AllMustSucceed, src(1))
            .unwrap();
        r.define("b".into(), "b".into(), HookKind::AllMustSucceed, src(2))
            .unwrap();
        r.define("c".into(), "c".into(), HookKind::AllMustSucceed, src(3))
            .unwrap();
        assert_eq!(r.names(), &["a".to_owned(), "b".into(), "c".into()]);
    }

    #[test]
    fn parse_kind_round_trips() {
        for k in [
            HookKind::ShortCircuit,
            HookKind::AllMustSucceed,
            HookKind::Accumulate,
        ] {
            assert_eq!(HookKind::parse(k.as_str()).unwrap(), k);
        }
        assert!(matches!(
            HookKind::parse("nope"),
            Err(HookError::UnknownKind { .. })
        ));
    }

    // ---- runner -------------------------------------------------------------

    fn snap(r: &HookRegistry, name: &str) -> (HookKind, Vec<HookCallback>) {
        r.snapshot(name).expect("hook defined")
    }

    #[test]
    fn short_circuit_proceeds_when_all_return_true() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        r.define("h".into(), "x".into(), HookKind::ShortCircuit, src(1))
            .unwrap();
        for _ in 0..3 {
            let f = lua.create_function(|_, ()| Ok(true)).unwrap();
            r.add("h", f, src(0)).unwrap();
        }
        let (k, cbs) = snap(&r, "h");
        let out = run_snapshot(k, &cbs, MultiValue::new());
        assert!(out.proceed);
        assert!(out.errors.is_empty());
    }

    #[test]
    fn short_circuit_vetoes_on_false() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        r.define("h".into(), "x".into(), HookKind::ShortCircuit, src(1))
            .unwrap();
        let f1 = lua.create_function(|_, ()| Ok(true)).unwrap();
        let f2 = lua.create_function(|_, ()| Ok(false)).unwrap();
        let f3 = lua
            .create_function(|_, ()| -> mlua::Result<()> { panic!("must not run after veto") })
            .unwrap();
        r.add("h", f1, src(0)).unwrap();
        r.add("h", f2, src(0)).unwrap();
        r.add("h", f3, src(0)).unwrap();
        let (k, cbs) = snap(&r, "h");
        let out = run_snapshot(k, &cbs, MultiValue::new());
        assert!(!out.proceed);
        assert!(out.errors.is_empty());
    }

    #[test]
    fn short_circuit_vetoes_on_error_and_records_it() {
        let mut r = HookRegistry::new();
        r.define("h".into(), "x".into(), HookKind::ShortCircuit, src(1))
            .unwrap();
        // Build the failing callback through Lua so the error is a
        // genuine mlua::Error::CallbackError, like real user code.
        let lua = Lua::new();
        let f: Function = lua.load("function() error('boom') end").eval().unwrap();
        r.add("h", f, src(7)).unwrap();
        let (k, cbs) = snap(&r, "h");
        let out = run_snapshot(k, &cbs, MultiValue::new());
        assert!(!out.proceed);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].source.line, 7);
    }

    #[test]
    fn all_must_succeed_runs_every_callback_collecting_errors() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        r.define("h".into(), "x".into(), HookKind::AllMustSucceed, src(1))
            .unwrap();
        let raise: Function = lua.load("function() error('boom') end").eval().unwrap();
        let counter = lua
            .load(
                "
                _G.hits = 0
                return function() _G.hits = _G.hits + 1 end
                ",
            )
            .eval::<Function>()
            .unwrap();
        r.add("h", raise.clone(), src(11)).unwrap();
        r.add("h", counter.clone(), src(12)).unwrap();
        r.add("h", raise, src(13)).unwrap();
        r.add("h", counter, src(14)).unwrap();
        let (k, cbs) = snap(&r, "h");
        let out = run_snapshot(k, &cbs, MultiValue::new());
        assert!(!out.proceed);
        assert_eq!(out.errors.len(), 2);
        let hits: i64 = lua.load("return _G.hits").eval().unwrap();
        assert_eq!(hits, 2, "every successful callback must run");
    }

    #[test]
    fn accumulate_threads_value_through_callbacks() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        r.define("h".into(), "x".into(), HookKind::Accumulate, src(1))
            .unwrap();
        let plus_one: Function = lua.load("function(n) return n + 1 end").eval().unwrap();
        for _ in 0..4 {
            r.add("h", plus_one.clone(), src(0)).unwrap();
        }
        let (k, cbs) = snap(&r, "h");
        let mut args = MultiValue::new();
        args.push_back(Value::Integer(10));
        let out = run_snapshot(k, &cbs, args);
        assert!(out.proceed);
        match out.value {
            Value::Integer(n) => assert_eq!(n, 14),
            other => panic!("expected integer, got {other:?}"),
        }
    }

    #[test]
    fn accumulate_aborts_on_error() {
        let lua = Lua::new();
        let mut r = HookRegistry::new();
        r.define("h".into(), "x".into(), HookKind::Accumulate, src(1))
            .unwrap();
        let plus_one: Function = lua.load("function(n) return n + 1 end").eval().unwrap();
        let raise: Function = lua.load("function(_) error('boom') end").eval().unwrap();
        r.add("h", plus_one.clone(), src(0)).unwrap();
        r.add("h", raise, src(0)).unwrap();
        r.add("h", plus_one, src(0)).unwrap();
        let (k, cbs) = snap(&r, "h");
        let mut args = MultiValue::new();
        args.push_back(Value::Integer(0));
        let out = run_snapshot(k, &cbs, args);
        assert!(!out.proceed);
        assert_eq!(out.errors.len(), 1);
    }
}
