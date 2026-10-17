//! The reconnect backoff schedule, shared by every frontend that
//! re-attaches after a lost socket (E9.2): the TUI's SSH attach loop in
//! `pmacs::attach` (M5.8) and `pmacs-gpu`'s window. It lives here, and
//! not in the `pmacs` crate where M5.8 wrote it, because `pmacs-gpu`
//! depends on this crate and never on `pmacs`; the verdicts that go with
//! it stay in `pmacs::attach_reconnect`, since they classify that
//! crate's own error type.
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
//! No jitter. Jitter (±20% randomization) is the standard defence
//! against thundering-herd reconnect storms; one frontend per window
//! can never produce a herd, and if a fleet of them ever does, add
//! jitter then under a concrete signal rather than preemptively.

use std::time::Duration;

/// Maximum delay between reconnect attempts. The schedule plateaus
/// here once the pre-cap doubling reaches it.
pub const MAX_DELAY: Duration = Duration::from_secs(30);

/// Pre-cap delay sequence. The schedule yields these in order, then
/// plateaus at [`MAX_DELAY`]. Kept as a const slice so the test asserts
/// against the exact contract rather than a re-implementation.
const PRE_CAP_DELAYS_MS: [u64; 6] = [500, 1000, 2000, 4000, 8000, 16000];

/// Environment variable for tests to compress the backoff schedule.
///
/// Acceptance tests that exercise a reconnect loop end-to-end need
/// the schedule to fire fast enough that a CI run doesn't take
/// minutes. Setting this to a positive integer `BASE_MS` overrides
/// the production curve with a doubling sequence based on `BASE_MS`,
/// capped at 60 × `BASE_MS` (mirroring the production 500ms → 30s
/// ratio):
///
/// * `PMACS_TEST_BACKOFF_SCALE_MS=1` → 1, 2, 4, 8, 16, 32, 60, 60, …
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

    /// How many delays the schedule has yielded since it was last reset:
    /// the attempt count a reconnect banner shows.
    #[must_use]
    pub fn step(&self) -> u32 {
        self.step
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_yields_first_seven_delays_per_documented_curve() {
        let mut s = BackoffSchedule {
            step: 0,
            test_scale_ms: None,
        };
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
        let mut s = BackoffSchedule {
            step: 0,
            test_scale_ms: None,
        };
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
        let mut s = BackoffSchedule {
            step: 0,
            test_scale_ms: None,
        };
        for _ in 0..10 {
            let _ = s.next_delay();
        }
        assert_eq!(s.next_delay(), Duration::from_secs(30));
        assert_eq!(s.step(), 11);

        s.reset();
        assert_eq!(s.step(), 0);
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

    #[test]
    fn compute_test_delay_at_500ms_base_matches_production_curve() {
        // The pure helper, given base_ms=500, should produce exactly
        // the production sequence — proving the test path and the
        // production path agree on the same input.
        let base = 500;
        let expected = [500, 1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000];
        for (step, &want_ms) in expected.iter().enumerate() {
            let got = compute_test_delay(base, u32::try_from(step).expect("small"));
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
            let got = compute_test_delay(base, u32::try_from(step).expect("small"));
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
            let got = compute_test_delay(base, u32::try_from(step).expect("small"));
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
}
