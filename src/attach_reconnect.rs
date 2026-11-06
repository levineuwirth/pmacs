// attach_reconnect.rs --- Reconnect controller types for the SSH attach (T M5.8a).

//! Reconnect controller types: backoff schedule + verdict enum.
//!
//! M5.8 adds mosh-style resilience to the SSH attach path: a dropped
//! channel is treated as temporary detachment, the frontend retains
//! the last-rendered cell grid, and we re-attempt connection on an
//! exponential backoff bounded at ~30s. This module owns the *pure*
//! pieces of that machinery — the schedule and the classification —
//! so the IO-heavy outer loop in [`crate::attach::run_attach_ssh`]
//! can be reasoned about and tested separately from the policy.
//!
//! # The schedule
//!
//! [`BackoffSchedule`] is `pmacs_protocol::reconnect`'s (E9.2 moved it
//! there so `pmacs-gpu`'s window reconnects on the same curve), a
//! deterministic sequence:
//!
//! ```text
//!     0.5s, 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s, …
//! ```
//!
//! # Verdicts and the handshake cap
//!
//! After each session attempt — successful or not — the outer loop
//! consults [`classify_for_reconnect`] with the resulting
//! [`crate::attach::AttachError`] to produce a [`ReconnectVerdict`].
//! The verdict tells the loop what to do next:
//!
//! * [`ReconnectVerdict::Reconnect`] — mid-session disconnect, full
//!   exponential backoff, indefinite retries. The user has a working
//!   session somewhere on the remote and we're going to get them back
//!   to it.
//! * [`ReconnectVerdict::ReconnectHandshake`] — pre-session failure
//!   (initial connect, Hello / `AttachRequest` exchange). Bounded by
//!   [`HANDSHAKE_RETRY_CAP`] so a permanently misconfigured target
//!   doesn't loop forever.
//! * [`ReconnectVerdict::ExitClean`] — F12 or the daemon's `Goodbye`; the
//!   session ended on purpose. No reconnect. The bridge's clean exit alone is
//!   not this: it is also what the daemon closing its socket looks like, and
//!   `attach::remote_verdict` makes that a reconnect.
//! * [`ReconnectVerdict::ExitProtocolError`] — version skew, transport
//!   corruption, terminal failure. Retry will not help.
//! * [`ReconnectVerdict::ExitPolicy`] — v0.1-policy rejection
//!   (currently `Rejected(AlreadyAttached)`). The split from
//!   `ExitProtocolError` is forward-looking: v0.2 multi-frontend
//!   will reclassify this as transient (the other frontend may
//!   detach), and a separate variant lets that be a one-line policy
//!   change rather than an audit through the whole error surface.
//!
//! The `initial_handshake_complete` flag in [`classify_for_reconnect`]
//! is the seam: once a session has succeeded even once in a single
//! `run_attach_ssh` invocation, all subsequent transient failures
//! are classified as full `Reconnect` (no cap), regardless of which
//! stage they happen in. Pre-success transient failures are
//! `ReconnectHandshake` (capped). The rationale is the user-perceived
//! contract: if you've ever seen the editor render, you expect
//! reconnect to keep trying; if you've never gotten past the
//! handshake, three quick attempts is courtesy and after that we owe
//! you a clear failure message.

use std::time::{Duration, Instant};

use crate::attach::AttachError;
use crate::protocol::{FrontendId, GoodbyeReason};

/// The backoff schedule, its test override and the cap, shared with
/// `pmacs-gpu` through the protocol crate (the GPU never depends on `pmacs`);
/// re-exported so this module's callers keep their paths. The cap is also the
/// maximum number of pre-success handshake retries before the outer loop
/// converts [`ReconnectVerdict::ReconnectHandshake`] into a give-up error, the
/// handshake-failure tail (stderr from each attempt) folded into that error for
/// diagnostic purposes.
pub use pmacs_protocol::reconnect::{
    BackoffSchedule, HANDSHAKE_RETRY_CAP, PMACS_TEST_BACKOFF_SCALE_MS,
};

/// How far a `Hello`'s age may fall short of the age the last daemon would have
/// reached by now and still be that daemon. Read on the real bridge at `b25e779`,
/// 15 connects over 97 s: 0.01 to 0.86 s short, since `uptime_secs` is whole seconds.
/// The rest is for a `Hello` that takes longer to arrive than the last one did.
pub const DAEMON_AGE_SLACK: Duration = Duration::from_secs(5);

/// What a later `Hello` says of the daemon an earlier one came from. Ordered by how
/// much it says, so that the strongest reading since a session last showed one is
/// `max`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Succession {
    /// The daemon the last `Hello` came from, as far as anything here can tell.
    #[default]
    Same,
    /// Perhaps another daemon: the age says so, or cannot say, and nothing is certain.
    /// A stalled link's `Hello` reads this way on a daemon holding all it held.
    Maybe,
    /// Another daemon, on no clock.
    Certain,
}

/// What a session's `Hello` says of its daemon, and when it was read: what tells the
/// daemon a session was on from a new one at the same address, with no wire change.
#[derive(Clone, Copy, Debug)]
pub struct DaemonSeen {
    frontend_id: FrontendId,
    uptime_secs: u64,
    read_at: Instant,
}

impl DaemonSeen {
    /// A `Hello`'s `assigned_frontend_id` and `uptime_secs`, read at `read_at`.
    #[must_use]
    pub fn new(frontend_id: FrontendId, uptime_secs: u64, read_at: Instant) -> Self {
        Self {
            frontend_id,
            uptime_secs,
            read_at,
        }
    }

    /// What `later`, a `Hello` read after this one, says of this one's daemon.
    ///
    /// A daemon numbers its frontends upward, so a number not past this one is another
    /// daemon's, on no clock: [`Succession::Certain`]. A number past it says nothing
    /// by itself, since other frontends may have got to a new daemon first, so the
    /// age is read, and it can only ever say [`Succession::Maybe`]: a `Hello` younger
    /// than this daemon would be by then, by more than [`DAEMON_AGE_SLACK`], or a
    /// daemon that would still be inside the slack, too young for an age to tell.
    ///
    /// What the age cannot tell, each said so a notice is not read for more than it
    /// is. Both clocks are `Instant`s, which stop while a machine sleeps, so on one
    /// machine they agree; over SSH, a remote host that slept past the slack reads as
    /// `Maybe`, and a client that slept may read a new daemon others got to first as
    /// [`Succession::Same`]. A `Hello` held past the slack, by a stop or a stalled link,
    /// reads as `Maybe` on a daemon holding everything it held, which is why that
    /// reading says `may be`. A new daemon others got to first, begun within the slack
    /// of the last one's start and read when the last would be past it, reads as
    /// `Same`: nothing here tells it from the same daemon's `Hello` held a moment.
    #[must_use]
    pub fn succeeded_by(&self, later: &Self) -> Succession {
        if later.frontend_id.0 <= self.frontend_id.0 {
            return Succession::Certain;
        }
        let elapsed = later.read_at.saturating_duration_since(self.read_at);
        let expected = Duration::from_secs(self.uptime_secs).saturating_add(elapsed);
        let younger =
            Duration::from_secs(later.uptime_secs).saturating_add(DAEMON_AGE_SLACK) < expected;
        if younger || expected <= DAEMON_AGE_SLACK {
            Succession::Maybe
        } else {
            Succession::Same
        }
    }
}

/// What the outer reconnect loop should do after a session attempt
/// returns.
///
/// Each variant carries the [`AttachError`] that produced the verdict
/// so the loop can fold per-attempt diagnostic context into a final
/// give-up message (handshake retries) or surface it directly to the
/// user (terminal cases). The `Reconnect*` variants pass the error
/// through so the modeline indicator can show *why* the last attempt
/// failed during the next sleep.
#[derive(Debug)]
pub enum ReconnectVerdict {
    /// Mid-session disconnect: the session previously succeeded and
    /// we should reconnect with full exponential backoff.
    Reconnect {
        /// The error that ended the session (broken pipe, EOF, daemon
        /// shutdown, etc.). Surfaced on the modeline indicator and
        /// folded into a give-up message only on user-driven exit.
        error: AttachError,
    },
    /// Pre-success handshake failure: bounded by
    /// [`HANDSHAKE_RETRY_CAP`]. The outer loop tracks attempts and
    /// converts to a give-up error past the cap.
    ReconnectHandshake {
        /// The error from this handshake attempt. Each attempt's
        /// error is logged (stderr tail preserved internally) so the
        /// final give-up message can summarize the full sequence.
        error: AttachError,
    },
    /// The attempt's channel ended with no error, which the outer loop reads
    /// beside how the session ended: on purpose (F12 → Detach, or the daemon's
    /// `Goodbye`), no reconnect and no error to surface; lost, a reconnect.
    ExitClean,
    /// Protocol-permanent error: version mismatch, transport
    /// corruption, terminal failure. Retry will not help; surface to
    /// the user immediately and exit.
    ExitProtocolError {
        /// The terminal error.
        error: AttachError,
    },
    /// v0.1-policy rejection: currently `Rejected(AlreadyAttached)`.
    /// Distinct from [`Self::ExitProtocolError`] so v0.2's
    /// multi-frontend reclassification (other frontend may detach,
    /// retry could succeed) is a one-line policy change in
    /// [`classify_for_reconnect`] rather than a sweep through the
    /// error-handling surface.
    ExitPolicy {
        /// The rejection error.
        error: AttachError,
    },
}

/// Classify an [`AttachError`] into a [`ReconnectVerdict`].
///
/// `initial_handshake_complete` is the outer-loop's "have we ever
/// successfully attached on this `run_attach_ssh` invocation" flag.
/// Once true, transient failures become [`ReconnectVerdict::Reconnect`]
/// (no cap) regardless of which stage of the attempt they happened
/// in — the user expects mid-session reconnect to keep trying. While
/// false, transient failures are [`ReconnectVerdict::ReconnectHandshake`]
/// so a permanently misconfigured target doesn't loop forever.
///
/// Permanent errors (`VersionMismatch`, `Rejected(VersionMismatch | ProtocolError)`,
/// `Terminal`) ignore the flag and always exit.
#[must_use]
pub fn classify_for_reconnect(
    error: AttachError,
    initial_handshake_complete: bool,
) -> ReconnectVerdict {
    match &error {
        // Permanent errors:
        // * Version skew (locally observed or daemon-reported) and
        //   protocol corruption — retry won't fix the wire mismatch.
        // * `Terminal` — crossterm broke. Even if SSH recovered, we'd
        //   have nowhere to draw the recovered grid.
        AttachError::VersionMismatch { .. }
        | AttachError::Rejected(
            GoodbyeReason::VersionMismatch { .. } | GoodbyeReason::ProtocolError,
        )
        | AttachError::Terminal(_) => ReconnectVerdict::ExitProtocolError { error },
        // v0.2 multi-frontend will reclassify this as transient
        // (other frontend may detach). Keep the variant distinct so
        // that change is one match arm, not a survey.
        AttachError::Rejected(GoodbyeReason::AlreadyAttached) => {
            ReconnectVerdict::ExitPolicy { error }
        }
        // Everything else — daemon shutting down, broken pipes,
        // transport / decode errors, SSH child exits / spawn
        // failures — is transient. The cap depends on whether we've
        // ever had a working session.
        _ => {
            if initial_handshake_complete {
                ReconnectVerdict::Reconnect { error }
            } else {
                ReconnectVerdict::ReconnectHandshake { error }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    // -----------------------------------------------------------------
    // classify_for_reconnect
    // -----------------------------------------------------------------

    fn io_err() -> AttachError {
        AttachError::Io(io::Error::other("test transient io error"))
    }

    // -----------------------------------------------------------------
    // DaemonSeen
    // -----------------------------------------------------------------

    /// What a `Hello` of `later` seconds, read `after` seconds on with a higher number,
    /// says after one of `first`.
    fn reads(first: u64, after: f64, later: u64) -> Succession {
        let start = Instant::now();
        DaemonSeen::new(FrontendId(5), first, start).succeeded_by(&DaemonSeen::new(
            FrontendId(6),
            later,
            start + Duration::from_secs_f64(after),
        ))
    }

    #[test]
    fn a_frontend_number_not_past_the_last_is_certain_on_no_clock() {
        let start = Instant::now();
        for id in [2, 5] {
            assert_eq!(
                DaemonSeen::new(FrontendId(5), 1_000, start).succeeded_by(&DaemonSeen::new(
                    FrontendId(id),
                    1_000,
                    start
                )),
                Succession::Certain,
                "{id}"
            );
        }
        assert_ne!(
            DaemonSeen::new(FrontendId(5), 1_000, start).succeeded_by(&DaemonSeen::new(
                FrontendId(6),
                1_000,
                start
            )),
            Succession::Certain,
        );
    }

    #[test]
    fn a_daemon_that_went_on_is_the_same_one() {
        assert_eq!(reads(100, 10.0, 110), Succession::Same);
        // Whole seconds, each way, the worst of what the real bridge read.
        assert_eq!(reads(10, 3.86, 13), Succession::Same);
        // Older than expected: a client that slept, its clock stopped.
        assert_eq!(reads(100, 10.0, 8_000), Succession::Same);
    }

    #[test]
    fn a_younger_age_alone_is_only_maybe() {
        assert_eq!(reads(100, 10.0, 0), Succession::Maybe);
        // The real run: a `Hello` held 7 s by a stalled link, on the same daemon.
        assert_eq!(reads(100, 7.5, 100), Succession::Maybe);
    }

    #[test]
    fn an_old_daemon_inside_the_slack_is_only_maybe() {
        // The real run: killed 2.5 s after it started, its successor's number past.
        assert_eq!(reads(0, 3.0, 0), Succession::Maybe);
        // The edge of it, and its first second out.
        assert_eq!(reads(0, 5.0, 5), Succession::Maybe);
        assert_eq!(reads(0, 6.0, 6), Succession::Same);
    }

    #[test]
    fn the_age_slack_is_what_a_daemon_may_fall_short_by() {
        assert_eq!(reads(100, 5.0, 100), Succession::Same);
        assert_eq!(reads(100, 5.5, 100), Succession::Maybe);
    }

    #[test]
    fn a_daemons_age_past_a_duration_does_not_panic() {
        assert_eq!(reads(u64::MAX, 1.0, u64::MAX), Succession::Same);
        assert_eq!(reads(u64::MAX, 1.0, 0), Succession::Maybe);
    }

    #[test]
    fn the_strongest_reading_is_the_greatest() {
        assert!(Succession::Same < Succession::Maybe);
        assert!(Succession::Maybe < Succession::Certain);
    }

    #[test]
    fn version_mismatch_is_protocol_error_regardless_of_session_state() {
        for &flag in &[false, true] {
            let err = AttachError::VersionMismatch {
                server: 99,
                client: 1,
            };
            match classify_for_reconnect(err, flag) {
                ReconnectVerdict::ExitProtocolError { .. } => {}
                other => panic!("flag={flag}: expected ExitProtocolError, got {other:?}"),
            }
        }
    }

    #[test]
    fn rejected_version_mismatch_is_protocol_error() {
        let err = AttachError::Rejected(GoodbyeReason::VersionMismatch {
            server: 99,
            client: 1,
        });
        match classify_for_reconnect(err, false) {
            ReconnectVerdict::ExitProtocolError { .. } => {}
            other => panic!("expected ExitProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn rejected_protocol_error_is_protocol_error() {
        let err = AttachError::Rejected(GoodbyeReason::ProtocolError);
        match classify_for_reconnect(err, true) {
            ReconnectVerdict::ExitProtocolError { .. } => {}
            other => panic!("expected ExitProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn rejected_already_attached_is_policy_in_v0_1() {
        // The deliberate forward-compatibility seam: keep this case
        // separate from ExitProtocolError so v0.2 multi-frontend
        // (other frontend may detach, retry could succeed) is a
        // one-arm change, not a refactor.
        let err = AttachError::Rejected(GoodbyeReason::AlreadyAttached);
        match classify_for_reconnect(err, true) {
            ReconnectVerdict::ExitPolicy { .. } => {}
            other => panic!("expected ExitPolicy, got {other:?}"),
        }
    }

    #[test]
    fn terminal_error_is_protocol_error_regardless_of_session_state() {
        for &flag in &[false, true] {
            let err = AttachError::Terminal(io::Error::other("crossterm broke"));
            match classify_for_reconnect(err, flag) {
                ReconnectVerdict::ExitProtocolError { .. } => {}
                other => panic!("flag={flag}: expected ExitProtocolError, got {other:?}"),
            }
        }
    }

    #[test]
    fn rejected_shutting_down_is_transient() {
        // ShuttingDown (daemon SIGTERM'd or restarting) is transient:
        // the daemon may come back, retry should succeed.
        let err = AttachError::Rejected(GoodbyeReason::ShuttingDown);
        match classify_for_reconnect(err, true) {
            ReconnectVerdict::Reconnect { .. } => {}
            other => panic!("expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn io_error_pre_session_is_handshake_reconnect() {
        match classify_for_reconnect(io_err(), false) {
            ReconnectVerdict::ReconnectHandshake { .. } => {}
            other => panic!("expected ReconnectHandshake, got {other:?}"),
        }
    }

    #[test]
    fn io_error_post_session_is_unbounded_reconnect() {
        match classify_for_reconnect(io_err(), true) {
            ReconnectVerdict::Reconnect { .. } => {}
            other => panic!("expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn ssh_spawn_failed_is_handshake_reconnect_pre_session() {
        let err = AttachError::SshSpawnFailed {
            command: std::path::PathBuf::from("ssh"),
            source: io::Error::other("ENOENT"),
        };
        match classify_for_reconnect(err, false) {
            ReconnectVerdict::ReconnectHandshake { .. } => {}
            other => panic!("expected ReconnectHandshake, got {other:?}"),
        }
    }

    #[test]
    fn ssh_spawn_failed_post_session_is_unbounded_reconnect() {
        // A reconnect attempt's spawn fails, but we've had a working
        // session before — the user expects retries to keep going.
        let err = AttachError::SshSpawnFailed {
            command: std::path::PathBuf::from("ssh"),
            source: io::Error::other("ENOENT"),
        };
        match classify_for_reconnect(err, true) {
            ReconnectVerdict::Reconnect { .. } => {}
            other => panic!("expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn ssh_child_exited_127_is_transient_in_both_states() {
        // 127 = command not found on remote. Treated like any other
        // SSH-child exit: handshake-bounded pre-session, unbounded
        // post-session. The user might be installing pmacs on the
        // remote while we wait.
        let make = || AttachError::SshChildExited {
            code: Some(127),
            stderr_tail: String::new(),
        };
        match classify_for_reconnect(make(), false) {
            ReconnectVerdict::ReconnectHandshake { .. } => {}
            other => panic!("pre-session: expected ReconnectHandshake, got {other:?}"),
        }
        match classify_for_reconnect(make(), true) {
            ReconnectVerdict::Reconnect { .. } => {}
            other => panic!("post-session: expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn transport_error_is_transient() {
        let make = || {
            AttachError::Transport(crate::transport::TransportError::Io(io::Error::other(
                "decode fail",
            )))
        };
        match classify_for_reconnect(make(), false) {
            ReconnectVerdict::ReconnectHandshake { .. } => {}
            other => panic!("pre-session: expected ReconnectHandshake, got {other:?}"),
        }
        match classify_for_reconnect(make(), true) {
            ReconnectVerdict::Reconnect { .. } => {}
            other => panic!("post-session: expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn handshake_retry_cap_is_three() {
        // The cap is part of the user contract (per M5.8 plan). Pin
        // it so a casual edit changes the test, not the user
        // experience. Bumping the cap is fine; doing it accidentally
        // is not.
        assert_eq!(HANDSHAKE_RETRY_CAP, 3);
    }
}
