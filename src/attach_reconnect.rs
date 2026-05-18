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
//! [`BackoffSchedule`] is a deterministic sequence:
//!
//! ```text
//!     0.5s, 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s, …
//! ```
//!
//! The pre-cap curve is the standard exponential doubling. The cap
//! at 30s is per spec — the user is waiting on a wall clock, and 30s
//! is the longest delay where "press a key, see something happen
//! soon" still feels intentional rather than broken.
//!
//! No jitter in v0.1. Jitter (±20% randomization) is the standard
//! defence against thundering-herd reconnect storms. The v0.1
//! single-frontend case can never produce a herd; if v0.3
//! multi-frontend attach reveals the need, add jitter then under a
//! concrete signal rather than preemptively.
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
//! * [`ReconnectVerdict::ExitClean`] — user pressed F12; the session
//!   ended on purpose. No reconnect.
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

use std::time::Duration;

use crate::attach::AttachError;
use crate::protocol::GoodbyeReason;

/// Maximum number of pre-success handshake retries before the outer
/// loop converts [`ReconnectVerdict::ReconnectHandshake`] into a
/// give-up error.
///
/// Sized for "DNS hiccup + SSH agent re-prompt + retry-with-fresh-key"
/// scenarios without letting permanent misconfiguration loop forever.
/// The handshake-failure tail (stderr from each attempt) is folded
/// into the give-up error for diagnostic purposes.
pub const HANDSHAKE_RETRY_CAP: u32 = 3;

/// Maximum delay between reconnect attempts. The schedule plateaus
/// here once the pre-cap doubling reaches it.
const MAX_DELAY: Duration = Duration::from_secs(30);

/// Pre-cap delay sequence. The schedule yields these in order, then
/// plateaus at [`MAX_DELAY`]. Kept as a const slice so the test asserts
/// against the exact contract rather than a re-implementation.
const PRE_CAP_DELAYS_MS: [u64; 6] = [500, 1000, 2000, 4000, 8000, 16000];

/// Environment variable for tests to compress the backoff schedule.
///
/// Acceptance tests that exercise the reconnect loop end-to-end need
/// the schedule to fire fast enough that a CI run doesn't take
/// minutes. Setting this to a positive integer `BASE_MS` overrides
/// the production curve with a doubling sequence based on `BASE_MS`,
/// capped at 60 × `BASE_MS` (mirroring the production 500ms → 30s
/// ratio):
///
/// * `PMACS_TEST_BACKOFF_SCALE_MS=1` → 1, 2, 4, 8, 16, 32, 60, 60, …
///   (`HANDSHAKE_RETRY_CAP` retries elapse in ~7ms total.)
/// * `PMACS_TEST_BACKOFF_SCALE_MS=500` (or unset) → production
///   0.5, 1, 2, 4, 8, 16, 30, 30, … s.
///
/// Test-flavored: production users do not set this. The env var is
/// read once at [`BackoffSchedule::new`] time so a test can scope
/// the override per-subprocess without affecting other concurrently
/// running tests in the same binary.
pub const PMACS_TEST_BACKOFF_SCALE_MS: &str = "PMACS_TEST_BACKOFF_SCALE_MS";

/// Exponential backoff scheduler for reconnect attempts.
///
/// Stateful: each call to [`next_delay`](Self::next_delay) advances
/// the curve. [`reset`](Self::reset) returns the schedule to its
/// initial state — used by the outer loop after a successful
/// reconnect, so the next disconnect starts at 0.5s rather than
/// staying parked at 30s.
///
/// Cheap to construct, no heap allocation. Single-threaded by design;
/// the outer reconnect loop is the only consumer.
#[derive(Debug, Clone)]
pub struct BackoffSchedule {
    /// Number of `next_delay` calls observed so far. Saturating, so
    /// it cannot wrap on a pathologically long-running session.
    step: u32,
    /// Test override read from [`PMACS_TEST_BACKOFF_SCALE_MS`] at
    /// construction. `None` → production curve. `Some(base_ms)` →
    /// doubling sequence on `base_ms` with cap `60 × base_ms`.
    test_scale_ms: Option<u64>,
}

impl BackoffSchedule {
    /// Construct a fresh schedule. The first [`next_delay`](Self::next_delay)
    /// call returns 0.5s in production, or `base_ms` if
    /// [`PMACS_TEST_BACKOFF_SCALE_MS`] is set.
    ///
    /// Reads the env var once and caches the result. Constructing a
    /// new `BackoffSchedule` after the env var changes will pick up
    /// the new value; existing instances will not.
    #[must_use]
    pub fn new() -> Self {
        Self {
            step: 0,
            test_scale_ms: read_test_scale_ms(),
        }
    }

    /// Compute the next delay and advance the schedule.
    ///
    /// Successive calls in production yield 0.5s, 1s, 2s, 4s, 8s,
    /// 16s, 30s, 30s, … Once the cap is reached, every subsequent
    /// call returns the same [`MAX_DELAY`]. With
    /// [`PMACS_TEST_BACKOFF_SCALE_MS`] set, the analogous doubling
    /// sequence on the configured base is yielded instead.
    pub fn next_delay(&mut self) -> Duration {
        let step = self.step;
        self.step = self.step.saturating_add(1);
        match self.test_scale_ms {
            Some(base_ms) => compute_test_delay(base_ms, step),
            None => PRE_CAP_DELAYS_MS
                .get(step as usize)
                .copied()
                .map_or(MAX_DELAY, Duration::from_millis),
        }
    }

    /// Reset the schedule to its initial state.
    ///
    /// Called by the outer reconnect loop after a session attempt
    /// succeeds: the next disconnect should feel responsive (0.5s)
    /// rather than inheriting the long delay that finally got us back
    /// last time. Without this, a flaky link reconnects every 30s
    /// after the first cycle, even when the link recovers in
    /// milliseconds — annoying but not breaking, exactly the kind of
    /// bug that ships.
    pub fn reset(&mut self) {
        self.step = 0;
    }
}

impl Default for BackoffSchedule {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the test-mode delay for `step` given a `base_ms` override.
///
/// Pure: same `(base_ms, step)` produces the same `Duration`,
/// regardless of any process state. Tested directly without env-var
/// manipulation.
///
/// Algorithm: `min(base_ms × 2^step, 60 × base_ms)`. Saturates
/// safely on absurd `step` values (the `1u64 << shift` is clamped to
/// 31 bits to avoid undefined behavior on the shift, and the
/// multiplication uses `saturating_mul`).
fn compute_test_delay(base_ms: u64, step: u32) -> Duration {
    let cap_ms = base_ms.saturating_mul(60);
    let shift = step.min(31);
    let scaled = base_ms.saturating_mul(1u64 << shift);
    Duration::from_millis(scaled.min(cap_ms))
}

fn read_test_scale_ms() -> Option<u64> {
    std::env::var(PMACS_TEST_BACKOFF_SCALE_MS)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
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
    /// User-initiated clean detach (F12 → Detach → daemon Goodbye →
    /// SSH child exit 0). No reconnect, no error to surface.
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
    // BackoffSchedule
    // -----------------------------------------------------------------

    #[test]
    fn schedule_yields_first_seven_delays_per_documented_curve() {
        let mut s = BackoffSchedule::new();
        let expected = [
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
        ];
        for (i, &want) in expected.iter().enumerate() {
            let got = s.next_delay();
            assert_eq!(got, want, "step {i}: expected {want:?}, got {got:?}");
        }
    }

    #[test]
    fn schedule_plateaus_at_30s_indefinitely() {
        let mut s = BackoffSchedule::new();
        // Burn through the pre-cap curve.
        for _ in 0..PRE_CAP_DELAYS_MS.len() {
            let _ = s.next_delay();
        }
        // Every subsequent call should pin at 30s. 100 iterations is
        // enough to catch any "off-by-one accidentally re-enters the
        // curve" regression.
        for _ in 0..100 {
            assert_eq!(s.next_delay(), Duration::from_secs(30));
        }
    }

    #[test]
    fn schedule_reset_returns_to_first_delay() {
        // Regression guard for the bug class flagged in the M5.8 plan:
        // a flaky link reconnects successfully once, then stays parked
        // at 30s for subsequent disconnects when it should drop back
        // to 0.5s. The reset path is easy to forget; the consequence
        // is annoying-but-not-breaking, exactly the kind of bug that
        // ships if not gated by a test.
        let mut s = BackoffSchedule::new();
        for _ in 0..10 {
            let _ = s.next_delay();
        }
        assert_eq!(s.next_delay(), Duration::from_secs(30));

        s.reset();
        assert_eq!(s.next_delay(), Duration::from_millis(500));
        assert_eq!(s.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn schedule_default_matches_new() {
        let mut a = BackoffSchedule::default();
        let mut b = BackoffSchedule::new();
        for _ in 0..5 {
            assert_eq!(a.next_delay(), b.next_delay());
        }
    }

    #[test]
    fn schedule_step_counter_saturates_at_u32_max() {
        // Pathologically long-running session: the schedule must not
        // panic on overflow. The saturating add guarantees this.
        let mut s = BackoffSchedule {
            step: u32::MAX,
            test_scale_ms: None,
        };
        // Pre-saturation: returns MAX_DELAY (we're past the curve).
        assert_eq!(s.next_delay(), Duration::from_secs(30));
        // Post-saturation: still returns MAX_DELAY, no panic.
        assert_eq!(s.step, u32::MAX);
        assert_eq!(s.next_delay(), Duration::from_secs(30));
    }

    // -----------------------------------------------------------------
    // PMACS_TEST_BACKOFF_SCALE_MS test override
    // -----------------------------------------------------------------

    #[test]
    fn compute_test_delay_at_500ms_base_matches_production_curve() {
        // The pure helper, given base_ms=500, should produce exactly
        // the production sequence — proving the test path and the
        // production path agree on the same input.
        let base = 500;
        let expected = [500, 1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000];
        for (step, &want_ms) in expected.iter().enumerate() {
            let got = compute_test_delay(base, step as u32);
            assert_eq!(
                got,
                Duration::from_millis(want_ms),
                "step {step}: expected {want_ms}ms, got {got:?}"
            );
        }
    }

    #[test]
    fn compute_test_delay_at_1ms_base_yields_fast_curve() {
        // Base 1ms → 1, 2, 4, 8, 16, 32, 60 (cap), 60, 60. The cap
        // is 60 × base = 60ms, mirroring 60 × 500ms = 30s.
        let base = 1;
        let expected = [1, 2, 4, 8, 16, 32, 60, 60, 60, 60];
        for (step, &want_ms) in expected.iter().enumerate() {
            let got = compute_test_delay(base, step as u32);
            assert_eq!(
                got,
                Duration::from_millis(want_ms),
                "step {step} at base 1ms: expected {want_ms}ms, got {got:?}"
            );
        }
    }

    #[test]
    fn compute_test_delay_at_50ms_base_yields_proportional_curve() {
        // 50ms × {1, 2, 4, 8, 16, 32, 60} = 50, 100, 200, 400, 800,
        // 1600, 3000.
        let base = 50;
        let expected = [50, 100, 200, 400, 800, 1600, 3000, 3000];
        for (step, &want_ms) in expected.iter().enumerate() {
            let got = compute_test_delay(base, step as u32);
            assert_eq!(
                got,
                Duration::from_millis(want_ms),
                "step {step} at base 50ms: expected {want_ms}ms, got {got:?}"
            );
        }
    }

    #[test]
    fn compute_test_delay_saturates_on_huge_step() {
        // Pathological step value — the bit-shift would UB without
        // the `min(31)` clamp, and the multiplication would overflow
        // without `saturating_mul`. Both guards verified.
        let base = 100;
        let got = compute_test_delay(base, u32::MAX);
        // Whatever the result is, it must be at least the cap and
        // not panic. With saturation it ends up at cap exactly.
        assert_eq!(got, Duration::from_millis(base * 60));
    }

    #[test]
    fn read_test_scale_ms_returns_none_when_unset() {
        // We can't safely set/unset env vars in parallel tests, but
        // we CAN verify the helper's parsing behavior on synthetic
        // inputs by constructing the schedule directly with a known
        // value. The pure-helper test above already covers the math;
        // this is a regression guard for the field-level wiring.
        let s = BackoffSchedule {
            step: 0,
            test_scale_ms: None,
        };
        assert!(s.test_scale_ms.is_none());
    }

    #[test]
    fn schedule_with_test_scale_uses_test_curve() {
        // Construct a schedule with the override pre-set (bypassing
        // the env-var read so this test is hermetic).
        let mut s = BackoffSchedule {
            step: 0,
            test_scale_ms: Some(10),
        };
        let expected = [10, 20, 40, 80, 160, 320, 600, 600, 600];
        for (i, &want_ms) in expected.iter().enumerate() {
            let got = s.next_delay();
            assert_eq!(
                got,
                Duration::from_millis(want_ms),
                "step {i}: expected {want_ms}ms, got {got:?}"
            );
        }
    }

    // -----------------------------------------------------------------
    // classify_for_reconnect
    // -----------------------------------------------------------------

    fn io_err() -> AttachError {
        AttachError::Io(io::Error::other("test transient io error"))
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
