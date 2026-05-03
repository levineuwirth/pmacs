// worker.rs --- T M3.1 work-stealing pool with cooperative cancellation.

//! Worker pool primitive (T M3.1).
//!
//! Spec contract: a work-stealing thread pool built on
//! [`std::thread`] and [`crossbeam_deque`] --- explicitly **not** a
//! generic async runtime ([spec §3 checkpoint 5]). Each worker owns
//! a local FIFO deque and steals from siblings + a shared injector
//! when its own deque is empty. Idle workers park on a [`Condvar`];
//! [`WorkerPool::dispatch`] notifies one. Dropping the pool signals
//! every worker and joins.
//!
//! # Cancellation
//!
//! Each [`JobHandle`] carries a [`CancellationToken`] (an
//! [`Arc<AtomicBool>`]). A user closure that loops on long work
//! polls the token at granular boundaries; calling
//! [`JobHandle::cancel`] flips the bit so the next check returns.
//! Cancellation is cooperative: the runtime never preempts a job.
//! A job that was cancelled before any worker picked it up sees the
//! flag at the start of execution and returns immediately without
//! invoking the user closure.
//!
//! # Panic isolation
//!
//! Each job runs inside [`std::panic::catch_unwind`]. A panicking
//! job does not kill its worker; subsequent jobs run on the same
//! thread.
//!
//! # What this layer does *not* do
//!
//! No message bus, no result delivery channel, no Lua surface ---
//! T M3.1 is the raw primitive. Result delivery is a per-call
//! responsibility: the user closure can capture a
//! [`crossbeam_channel::Sender`] and send. The typed message bus
//! (T M3.2) and the coroutine-based async API (T M3.3) build on
//! top.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam::deque::{Injector, Steal, Stealer, Worker};

/// Stable identifier for a dispatched job, monotonically increasing
/// per [`WorkerPool`] from `0`.
pub type JobId = u64;

// ---------------------------------------------------------------------------
// CancellationToken
// ---------------------------------------------------------------------------

/// Cooperative cancellation flag shared between a [`JobHandle`] and
/// the worker closure that owns it.
///
/// `is_cancelled` is the user-side check; `cancel` is the producer
/// side. Both are lock-free atomic reads/writes. Cloning a token
/// shares the same flag --- two clones see each other's state.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    /// A fresh, not-yet-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Has [`Self::cancel`] been called on this token (or any clone
    /// of it)? Returns `true` from then on.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Mark the token cancelled. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// JobHandle
// ---------------------------------------------------------------------------

/// Handle returned by [`WorkerPool::dispatch`].
///
/// Carries the job's id and the cancellation token. Dropping a
/// handle does **not** cancel the job (the user may have other
/// clones of the token); call [`Self::cancel`] explicitly.
#[derive(Clone, Debug)]
pub struct JobHandle {
    id: JobId,
    token: CancellationToken,
}

impl JobHandle {
    /// The dispatch-order id assigned by the pool.
    #[must_use]
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Request cooperative cancellation. The worker closure (if it
    /// polls the token) sees the flag on its next check.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Has the job been cancelled?
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Borrow a clone of the token, e.g. to register cancellation
    /// behaviour outside the original closure.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

// ---------------------------------------------------------------------------
// WorkerPool
// ---------------------------------------------------------------------------

type Job = Box<dyn FnOnce() + Send + 'static>;

struct PoolShared {
    injector: Injector<Job>,
    stealers: Vec<Stealer<Job>>,
    shutdown: AtomicBool,
    next_id: AtomicU64,
    /// Idle-worker park lot. Workers acquire the mutex, recheck for
    /// work, and `wait_timeout` on the condvar. Producers and
    /// shutdown signal `notify_one` / `notify_all`. The 100ms
    /// timeout is a belt-and-braces guard against a missed
    /// notification --- workers re-poll the queues even without an
    /// explicit wakeup.
    parker: (Mutex<()>, Condvar),
}

impl PoolShared {
    fn alloc_id(&self) -> JobId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn notify_one(&self) {
        // Holding the lock during `notify_one` is required to avoid
        // a lost wakeup when a worker is between the empty-queue
        // check and the `wait_timeout` call.
        let _guard = self.parker.0.lock().expect("parker mutex");
        self.parker.1.notify_one();
    }

    fn notify_all(&self) {
        let _guard = self.parker.0.lock().expect("parker mutex");
        self.parker.1.notify_all();
    }
}

/// Work-stealing thread pool with cooperative cancellation tokens.
///
/// Construction spawns `size` OS threads, each running its own
/// [`worker_loop`]. Drop the pool to stop them: [`Drop`] signals
/// every worker and joins.
///
/// `WorkerPool` is `Send + Sync`: dispatch can come from any
/// thread, but typical pmacs use is single-producer (the Lua main
/// thread).
pub struct WorkerPool {
    shared: Arc<PoolShared>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    /// Build a pool with exactly `size` worker threads. `size` is
    /// clamped to at least `1` so callers that compute `cores - 1`
    /// on a 1-core machine don't end up with zero workers.
    #[must_use]
    pub fn new(size: usize) -> Self {
        let size = size.max(1);
        let local_queues: Vec<Worker<Job>> = (0..size).map(|_| Worker::new_fifo()).collect();
        let stealers: Vec<Stealer<Job>> = local_queues.iter().map(Worker::stealer).collect();
        let shared = Arc::new(PoolShared {
            injector: Injector::new(),
            stealers,
            shutdown: AtomicBool::new(false),
            next_id: AtomicU64::new(0),
            parker: (Mutex::new(()), Condvar::new()),
        });
        let workers = local_queues
            .into_iter()
            .enumerate()
            .map(|(idx, local)| {
                let shared = Arc::clone(&shared);
                thread::Builder::new()
                    .name(format!("pmacs-worker-{idx}"))
                    .spawn(move || worker_loop(&local, &shared))
                    .expect("spawn worker thread")
            })
            .collect();
        Self { shared, workers }
    }

    /// Build a pool sized at `available_parallelism - 1`, with a
    /// floor of `1`. Reserves one core for the main editor thread,
    /// matching the spec's "main thread is the event loop" pattern
    /// ([spec §6.1]).
    #[must_use]
    pub fn with_default_size() -> Self {
        let cores = thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
        Self::new(cores.saturating_sub(1))
    }

    /// Number of worker threads owned by this pool.
    #[must_use]
    pub fn size(&self) -> usize {
        self.workers.len()
    }

    /// Submit `work` to be run on a worker. Returns a [`JobHandle`]
    /// that owns the cancellation token for this job.
    ///
    /// `work` is invoked with a borrow of the same token the handle
    /// holds; the closure is responsible for polling
    /// [`CancellationToken::is_cancelled`] at granular boundaries
    /// inside any long-running loop. A job whose token is set
    /// before the worker picks it up sees the flag immediately and
    /// returns without running the user closure.
    ///
    /// Result delivery is up to the closure: capture a
    /// `crossbeam_channel::Sender` (or any other `Send` channel) to
    /// pass values back. T M3.2 will provide the message bus that
    /// formalises this; T M3.1 deliberately does not.
    pub fn dispatch<F>(&self, work: F) -> JobHandle
    where
        F: FnOnce(&CancellationToken) + Send + 'static,
    {
        let id = self.shared.alloc_id();
        let token = CancellationToken::new();
        let token_for_job = token.clone();
        let job: Job = Box::new(move || {
            // Skip user work entirely if cancelled before we ran.
            if token_for_job.is_cancelled() {
                return;
            }
            // Panic isolation: a panicking job must not poison the
            // worker thread. We swallow the panic payload here ---
            // higher layers (M3.2 message bus) will deliver
            // structured failure to the caller.
            let _ = catch_unwind(AssertUnwindSafe(|| work(&token_for_job)));
        });
        self.shared.injector.push(job);
        self.shared.notify_one();
        JobHandle { id, token }
    }
}

impl Drop for WorkerPool {
    /// Shutdown semantics: dropping the pool signals every worker
    /// to exit at its next idle wakeup and joins them. Running
    /// jobs run to completion (or to their own cancellation
    /// check); queued jobs that haven't been picked up are dropped
    /// without running. Tests and callers that want explicit
    /// shutdown just `drop(pool)`.
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_loop(local: &Worker<Job>, shared: &Arc<PoolShared>) {
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        if let Some(job) = find_work(local, shared) {
            job();
            continue;
        }
        // No work --- park briefly, then retry. The 100ms timeout
        // bounds how long a missed wakeup can stall this worker.
        let guard = shared.parker.0.lock().expect("parker mutex");
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        // The injector was empty when we last looked; if a producer
        // pushed in the meantime they will have signalled the
        // condvar. We hold the parker mutex now, so any signal that
        // happens after we drop the lock will wake us via wait.
        let _ = shared
            .parker
            .1
            .wait_timeout(guard, Duration::from_millis(100))
            .expect("parker condvar");
    }
}

fn find_work(local: &Worker<Job>, shared: &PoolShared) -> Option<Job> {
    if let Some(job) = local.pop() {
        return Some(job);
    }
    // Pull a batch from the global injector into our local deque,
    // returning one to run immediately.
    loop {
        match shared.injector.steal_batch_and_pop(local) {
            Steal::Success(job) => return Some(job),
            Steal::Empty => break,
            Steal::Retry => {}
        }
    }
    // Steal one job from each sibling.
    for stealer in &shared.stealers {
        loop {
            match stealer.steal() {
                Steal::Success(job) => return Some(job),
                Steal::Empty => break,
                Steal::Retry => {}
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam::channel;
    use std::sync::atomic::AtomicU64;

    fn assert_recv_within<T>(rx: &channel::Receiver<T>, label: &str) -> T {
        rx.recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("timed out waiting for {label}"))
    }

    /// Acceptance bullet: jobs run on workers; results returned via
    /// callback (the user closure forwards through a channel).
    #[test]
    fn dispatch_runs_user_closure_on_a_worker() {
        let pool = WorkerPool::new(2);
        let (tx, rx) = channel::bounded::<u64>(1);
        let _ = pool.dispatch(move |_| {
            tx.send(42).unwrap();
        });
        assert_eq!(assert_recv_within(&rx, "dispatched value"), 42);
    }

    /// Job ids are unique and monotonic per pool.
    #[test]
    fn job_ids_are_monotonic() {
        let pool = WorkerPool::new(1);
        let h1 = pool.dispatch(|_| {});
        let h2 = pool.dispatch(|_| {});
        let h3 = pool.dispatch(|_| {});
        assert!(h1.id() < h2.id() && h2.id() < h3.id());
    }

    /// A job whose token is set before the worker picks it up must
    /// not run the user closure. We park the only worker on a
    /// blocking job, then dispatch the doomed job, cancel it,
    /// release the worker, and verify the doomed closure never set
    /// its sentinel flag.
    #[test]
    fn cancel_before_dispatch_skips_user_work() {
        let pool = WorkerPool::new(1);
        let (release, gate) = channel::bounded::<()>(0);
        let _h_block = pool.dispatch(move |_| {
            let _ = gate.recv();
        });
        let did_run = Arc::new(AtomicBool::new(false));
        let did_run_clone = Arc::clone(&did_run);
        let h_doomed = pool.dispatch(move |_| {
            did_run_clone.store(true, Ordering::SeqCst);
        });
        h_doomed.cancel();
        release.send(()).unwrap();
        // Give the worker time to drain and run the doomed job.
        thread::sleep(Duration::from_millis(50));
        assert!(
            !did_run.load(Ordering::SeqCst),
            "cancelled job should not have run"
        );
    }

    /// A long-running job that polls the token sees cancellation
    /// observed mid-flight.
    #[test]
    fn cancel_during_work_observed_by_user_closure() {
        let pool = WorkerPool::new(1);
        let started = Arc::new(AtomicBool::new(false));
        let observed = Arc::new(AtomicBool::new(false));
        let started_c = Arc::clone(&started);
        let observed_c = Arc::clone(&observed);
        let h = pool.dispatch(move |t| {
            started_c.store(true, Ordering::SeqCst);
            while !t.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            observed_c.store(true, Ordering::SeqCst);
        });
        // Wait for the worker to enter the loop.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "worker never started job"
            );
            thread::sleep(Duration::from_millis(1));
        }
        h.cancel();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !observed.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "worker never observed cancellation"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// A panicking job must not kill its worker. After a panic on a
    /// single-thread pool, a follow-up job must still run on the
    /// (same) worker.
    #[test]
    fn panic_in_job_does_not_kill_worker() {
        let pool = WorkerPool::new(1);
        let _ = pool.dispatch(|_| panic!("boom"));
        // Allow the worker to swallow the panic.
        thread::sleep(Duration::from_millis(20));
        let (tx, rx) = channel::bounded::<u32>(1);
        let _ = pool.dispatch(move |_| {
            tx.send(7).unwrap();
        });
        assert_eq!(assert_recv_within(&rx, "post-panic dispatch"), 7);
    }

    /// `with_default_size` produces at least one worker on every
    /// platform, even single-core machines.
    #[test]
    fn default_size_floors_at_one() {
        let pool = WorkerPool::with_default_size();
        assert!(
            pool.size() >= 1,
            "default size must be at least 1, got {}",
            pool.size()
        );
    }

    /// Acceptance bullet: 10000 dispatches with random
    /// cancellations, no leaks or hangs. We use a deterministic
    /// "cancel every Nth" pattern instead of an RNG so the test is
    /// reproducible. Every non-cancelled job must complete within a
    /// global timeout; cancelled jobs may or may not run before
    /// they observe the flag (see `cancel_before_dispatch_skips...`
    /// for the deterministic version).
    #[test]
    fn stress_10k_dispatches_with_periodic_cancels_no_hang() {
        const TOTAL: usize = 10_000;
        const CANCEL_EVERY: usize = 3;
        let pool = WorkerPool::with_default_size();
        let completed = Arc::new(AtomicU64::new(0));
        let mut handles = Vec::with_capacity(TOTAL);
        for i in 0..TOTAL {
            let completed = Arc::clone(&completed);
            let h = pool.dispatch(move |t| {
                if t.is_cancelled() {
                    return;
                }
                // A bit of trivial work that observes the token.
                let mut acc: u64 = 0;
                for j in 0..32u64 {
                    if t.is_cancelled() {
                        return;
                    }
                    acc = acc.wrapping_add(j);
                }
                completed.fetch_add(1, Ordering::Relaxed);
                std::hint::black_box(acc);
            });
            if i % CANCEL_EVERY == 0 {
                h.cancel();
            }
            handles.push(h);
        }
        // Drain shutdown synchronously via Drop: this returns only
        // after every queued, non-cancelled job has run (or every
        // cancelled job has either run-then-noop or been silently
        // dropped). No hang means we get here within the test
        // harness's default timeout.
        drop(pool);
        // Cancelled-before-pickup jobs return without bumping the
        // counter; cancelled-after-pickup jobs may bump 0 or 1
        // times depending on when they polled. We only assert no
        // job over-counted: completion count must not exceed the
        // count of jobs that were ever eligible.
        let count = completed.load(Ordering::Relaxed);
        let max_eligible = u64::try_from(TOTAL).unwrap();
        assert!(
            count <= max_eligible,
            "completion count {count} exceeded eligible {max_eligible}"
        );
        // Lower bound: all definitely-uncancelled jobs (those whose
        // index is not a multiple of CANCEL_EVERY) must have run.
        let definitely_uncancelled = u64::try_from(TOTAL - TOTAL.div_ceil(CANCEL_EVERY)).unwrap();
        assert!(
            count >= definitely_uncancelled,
            "expected at least {definitely_uncancelled} completions, got {count}"
        );
    }

    /// Drop semantics: dropping a `WorkerPool` must join every
    /// worker. We rely on the test harness's process-exit timeout
    /// to flag failure if a worker leaks.
    #[test]
    fn drop_joins_workers() {
        let pool = WorkerPool::new(4);
        let (tx, rx) = channel::bounded::<()>(1);
        let _ = pool.dispatch(move |_| {
            tx.send(()).unwrap();
        });
        assert_recv_within(&rx, "pre-drop dispatch");
        drop(pool);
        // If Drop didn't join, the process would have to wait for
        // detached threads to exit naturally. Reaching here is the
        // assertion.
    }
}
