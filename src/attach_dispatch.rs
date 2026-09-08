// attach_dispatch.rs --- Post-init attach dispatcher (T M5.6g).

//! Post-init attach dispatcher.
//!
//! After `init.lua` runs, [`crate::editor::EditorState`] reads the
//! [`crate::lua_bindings::RequestedAttach`] slot. This module decides
//! what the resulting program should do with a stored attach request:
//!
//! * `None` (init.lua never called `pmacs.attach{...}`) — run the
//!   local TUI as usual.
//! * `Some(LocalSocket(_))` — hand off to [`crate::attach::run_attach`]
//!   against that socket. The local terminal becomes a frontend
//!   talking to the named daemon.
//! * `Some(Ssh|Tls|Custom)` — error. These transports parse and store
//!   in v0.1 (per the spec's "validate locally, defer activation"
//!   rule) but their activation pathway hasn't shipped yet. The
//!   dispatcher surfaces a workaround-pointing error rather than
//!   silently downgrading to local mode.
//!
//! # Threading
//!
//! Pure function. Called once per process from `editor::run` *before*
//! the local [`crate::frontend::Frontend`] is constructed, so a
//! handed-off attach doesn't fight the local-TUI pathway for the
//! terminal.

use std::path::PathBuf;

use crate::protocol::AttachTarget;

/// What the post-init dispatcher decided to do with the requested
/// attach.
///
/// The `editor::run` caller matches on this and either continues the
/// local TUI, hands off to `attach::run_attach`, or surfaces an error
/// pointing at the milestone where the requested transport ships.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AttachDispatch {
    /// No attach was requested in init.lua. Run the local TUI as
    /// usual.
    RunLocal,
    /// Hand off to `attach::run_attach(socket)`. The local frontend
    /// will talk to the daemon at this socket path.
    RunAttachLocalSocket(PathBuf),
    /// Hand off to `attach::run_attach_ssh(target)`. The local
    /// frontend will spawn `ssh ... pmacs --daemon-attach ...` and
    /// drive the attach pump over the SSH child's stdio.
    RunAttachSsh(AttachTarget),
    /// The requested transport is recognized and validated, but its
    /// activation pathway isn't shipped in v0.1. The dispatcher
    /// surfaces a structured deferral; `editor::run` formats it into
    /// a workaround-pointing error message at the boundary.
    DeferredInV01 {
        /// Lower-case transport name: `"tls"`, `"custom"` (SSH used
        /// to defer here pre-M5.7e; now activates via
        /// [`Self::RunAttachSsh`]).
        kind: &'static str,
        /// Target milestone for activation: `"v0.2"` for tls/custom.
        /// Combined with `kind` to form the user-visible error
        /// message.
        milestone: &'static str,
    },
}

impl AttachDispatch {
    /// Render `DeferredInV01` as a single error line. Pulled into a
    /// method so `editor::run` and the acceptance tests share one
    /// canonical message shape.
    ///
    /// Returns `None` for `RunLocal` and `RunAttachLocalSocket` —
    /// only the deferral case has a message.
    #[must_use]
    pub fn deferred_message(&self) -> Option<String> {
        match self {
            Self::DeferredInV01 { kind, milestone } => Some(format!(
                "{kind} attach is not yet implemented in v0.1 (planned for {milestone}); \
                 remove the `pmacs.attach{{...}}` call from init.lua and restart, \
                 or pass `--attach --socket NAME` for a local-socket attach"
            )),
            _ => None,
        }
    }
}

/// Decide what to do with the (optional) attach request consumed
/// from `init.lua`'s `pmacs::lua_bindings::RequestedAttach` slot.
///
/// Pure function: takes the parsed target, returns the dispatch
/// outcome. The wrapping `editor::run` is responsible for actually
/// running the chosen pathway.
#[must_use]
pub fn dispatch_attach(target: Option<AttachTarget>) -> AttachDispatch {
    match target {
        None => AttachDispatch::RunLocal,
        Some(AttachTarget::LocalSocket(path)) => AttachDispatch::RunAttachLocalSocket(path),
        Some(t @ AttachTarget::Ssh { .. }) => AttachDispatch::RunAttachSsh(t),
        Some(AttachTarget::Tls { .. }) => AttachDispatch::DeferredInV01 {
            kind: "tls",
            milestone: "v0.2",
        },
        Some(AttachTarget::Custom { .. }) => AttachDispatch::DeferredInV01 {
            kind: "custom",
            milestone: "v0.2",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn no_request_runs_local() {
        assert_eq!(dispatch_attach(None), AttachDispatch::RunLocal);
    }

    #[test]
    fn local_socket_dispatches_to_attach_run() {
        let p = PathBuf::from("/run/pmacs/work.sock");
        assert_eq!(
            dispatch_attach(Some(AttachTarget::LocalSocket(p.clone()))),
            AttachDispatch::RunAttachLocalSocket(p),
        );
    }

    #[test]
    fn ssh_target_dispatches_to_run_attach_ssh() {
        // M5.7e activates SSH: the dispatcher routes SSH targets
        // straight to `RunAttachSsh` so `attach::run_attach_ssh` can
        // spawn the subprocess. The fields round-trip unchanged.
        let target = AttachTarget::Ssh {
            host: "host".into(),
            user: Some("alice".into()),
            instance_name: Some("research".into()),
        };
        match dispatch_attach(Some(target.clone())) {
            AttachDispatch::RunAttachSsh(round_tripped) => {
                assert_eq!(round_tripped, target);
            }
            other => panic!("expected RunAttachSsh, got {other:?}"),
        }
    }

    #[test]
    fn tls_target_defers_to_v0_2() {
        let target = AttachTarget::Tls {
            endpoint: "example:9999".into(),
            cert: PathBuf::from("/etc/p.crt"),
        };
        match dispatch_attach(Some(target)) {
            AttachDispatch::DeferredInV01 { kind, milestone } => {
                assert_eq!(kind, "tls");
                assert_eq!(milestone, "v0.2");
            }
            other => panic!("expected DeferredInV01, got {other:?}"),
        }
    }

    #[test]
    fn custom_target_defers_to_v0_2() {
        let target = AttachTarget::Custom {
            command: vec!["docker".into(), "exec".into()],
        };
        match dispatch_attach(Some(target)) {
            AttachDispatch::DeferredInV01 { kind, milestone } => {
                assert_eq!(kind, "custom");
                assert_eq!(milestone, "v0.2");
            }
            other => panic!("expected DeferredInV01, got {other:?}"),
        }
    }

    #[test]
    fn deferred_message_present_only_on_deferred_variant() {
        assert!(AttachDispatch::RunLocal.deferred_message().is_none());
        assert!(
            AttachDispatch::RunAttachLocalSocket(PathBuf::from("/x"))
                .deferred_message()
                .is_none(),
        );
        assert!(
            AttachDispatch::RunAttachSsh(AttachTarget::Ssh {
                host: "h".into(),
                user: None,
                instance_name: None,
            })
            .deferred_message()
            .is_none(),
        );
        let m = AttachDispatch::DeferredInV01 {
            kind: "tls",
            milestone: "v0.2",
        }
        .deferred_message();
        let m = m.expect("deferred variant has a message");
        assert!(m.contains("tls attach"), "{m}");
        assert!(m.contains("v0.2"), "{m}");
        assert!(
            m.contains("--attach"),
            "message must point at workaround: {m}"
        );
    }

    #[test]
    fn deferred_message_for_tls_names_v0_2_and_pointer_to_workaround() {
        let m = AttachDispatch::DeferredInV01 {
            kind: "tls",
            milestone: "v0.2",
        }
        .deferred_message()
        .unwrap();
        assert!(m.contains("tls"));
        assert!(m.contains("v0.2"));
        assert!(m.contains("init.lua"));
        assert!(m.contains("--attach"));
    }

    #[test]
    fn each_deferred_variant_yields_distinct_kind() {
        // Defends against a copy-paste bug that would map two
        // deferred variants to the same kind tag. Post-M5.7e only
        // tls and custom defer; ssh now activates.
        let kinds: std::collections::HashSet<&'static str> = [
            AttachTarget::Tls {
                endpoint: "e".into(),
                cert: PathBuf::from("/x"),
            },
            AttachTarget::Custom {
                command: vec!["c".into()],
            },
        ]
        .into_iter()
        .map(|t| match dispatch_attach(Some(t)) {
            AttachDispatch::DeferredInV01 { kind, .. } => kind,
            other => panic!("expected DeferredInV01, got {other:?}"),
        })
        .collect();
        assert_eq!(kinds.len(), 2, "kind tags must be distinct: {kinds:?}");
    }
}
