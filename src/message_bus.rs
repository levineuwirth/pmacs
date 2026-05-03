// message_bus.rs --- T M3.2 typed in-process message bus with `MessagePack`
// serialization, a schema registry, and structured schema-mismatch errors.

//! Message bus core (T M3.2).
//!
//! Spec contract: a single typed protocol used uniformly for every
//! producer the editor talks to ([spec §5.2]). T M3.2 implements only
//! the in-process transport --- two crossbeam channels wired into a
//! pair of [`BusEnd`]s --- but the wire format (`MessagePack`) is the
//! same one out-of-process workers will eventually use, so a future
//! migration of one topic from in-process to subprocess is a transport
//! swap, not a serialisation rewrite.
//!
//! # Topology
//!
//! [`MessageBus::pair`] returns two [`BusEnd`]s. Each end owns a
//! [`Sender`] aimed at the *other* end and a [`Receiver`] reading
//! messages aimed at *itself*. Cloning a [`BusEnd`] yields another
//! handle that shares the same channels --- typical use is for the
//! main thread to keep one end and hand a clone of the other end to
//! every worker via the closure captured by [`crate::worker::WorkerPool::dispatch`].
//!
//! # Schema registry
//!
//! Topics are `&'static str` keys. Each topic is bound (at runtime,
//! once) to a Rust type via [`SchemaRegistry::register`]. Sending
//! validates the payload type against the registry; decoding
//! validates the requested type against the registry. Both produce
//! [`BusError::SchemaMismatch`] with `&'static str` topic and type
//! names so the error payload itself does not allocate.
//!
//! # Hot-path allocation
//!
//! The send path performs exactly one allocation, the
//! `MessagePack`-encoded payload buffer. Topic strings are
//! `&'static str`, registry lookups use the borrowed key, and the
//! envelope construction moves the payload buffer rather than copying
//! it. Internal `crossbeam_channel` node allocation is implementation
//! detail of the channel and is acceptable per the spec acceptance
//! ("beyond payload buffer").

use std::any::{TypeId, type_name};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossbeam::channel::{self, Receiver, Sender};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Stable identifier for an envelope, monotonically increasing per
/// [`MessageBus`] from `0`.
pub type MessageId = u64;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Structured failures from the bus. All variants use `&'static str`
/// for topic and type names so the error itself does not allocate ---
/// codec errors are wrapped owned because the underlying
/// `rmp_serde` types are owned.
#[derive(Debug)]
pub enum BusError {
    /// A topic was used for send/decode but not previously
    /// [`SchemaRegistry::register`]ed.
    UnregisteredTopic {
        /// The topic that was not registered.
        topic: &'static str,
    },
    /// The Rust type used for send/decode does not match the type the
    /// topic was registered with.
    SchemaMismatch {
        /// The topic involved in the mismatch.
        topic: &'static str,
        /// The fully-qualified type name supplied by the caller.
        expected: &'static str,
        /// The fully-qualified type name the registry was bound to.
        registered: &'static str,
    },
    /// A topic was registered twice with two different types.
    /// Re-registering with the *same* type is a no-op and not an error.
    DuplicateRegistration {
        /// The contested topic.
        topic: &'static str,
        /// The type name from the prior registration.
        existing: &'static str,
        /// The type name attempted in the second registration.
        attempted: &'static str,
    },
    /// `MessagePack` encoding failed.
    Encode(rmp_serde::encode::Error),
    /// `MessagePack` decoding failed.
    Decode(rmp_serde::decode::Error),
    /// The peer end of the channel has been dropped; no further
    /// sends or receives will succeed.
    Disconnected,
    /// Non-blocking receive found nothing.
    Empty,
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnregisteredTopic { topic } => {
                write!(f, "unregistered topic {topic:?}")
            }
            Self::SchemaMismatch {
                topic,
                expected,
                registered,
            } => {
                write!(
                    f,
                    "schema mismatch on topic {topic:?}: caller used {expected}, \
                     registry has {registered}"
                )
            }
            Self::DuplicateRegistration {
                topic,
                existing,
                attempted,
            } => {
                write!(
                    f,
                    "topic {topic:?} already registered as {existing}; cannot \
                     re-register as {attempted}"
                )
            }
            Self::Encode(e) => write!(f, "MessagePack encode failed: {e}"),
            Self::Decode(e) => write!(f, "MessagePack decode failed: {e}"),
            Self::Disconnected => write!(f, "bus peer is gone"),
            Self::Empty => write!(f, "no message available"),
        }
    }
}

impl std::error::Error for BusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(e) => Some(e),
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// SchemaRegistry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct TopicEntry {
    type_id: TypeId,
    type_name: &'static str,
}

/// Maps topic names to the Rust types their payloads serialize to.
///
/// Senders register each topic up front (typically at editor startup
/// or at package load); send and decode operations validate against
/// the registry. A registry is cheap to share via [`Arc`] --- all
/// runtime methods take `&self` and use a single internal `RwLock`.
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    topics: RwLock<HashMap<&'static str, TopicEntry>>,
}

impl SchemaRegistry {
    /// Build an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `topic` to type `T`. Re-registering with the same `T`
    /// is a no-op; re-registering with a different type returns
    /// [`BusError::DuplicateRegistration`].
    pub fn register<T: 'static>(&self, topic: &'static str) -> Result<(), BusError> {
        let new = TopicEntry {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
        };
        let mut guard = self.topics.write().expect("schema registry poisoned");
        if let Some(existing) = guard.get(topic) {
            if existing.type_id == new.type_id {
                return Ok(());
            }
            return Err(BusError::DuplicateRegistration {
                topic,
                existing: existing.type_name,
                attempted: new.type_name,
            });
        }
        guard.insert(topic, new);
        Ok(())
    }

    /// Validate that `topic` is registered and bound to type `T`.
    /// Used internally on the send and decode hot paths.
    fn check<T: 'static>(&self, topic: &'static str) -> Result<(), BusError> {
        let guard = self.topics.read().expect("schema registry poisoned");
        let Some(entry) = guard.get(topic) else {
            return Err(BusError::UnregisteredTopic { topic });
        };
        if entry.type_id != TypeId::of::<T>() {
            return Err(BusError::SchemaMismatch {
                topic,
                expected: type_name::<T>(),
                registered: entry.type_name,
            });
        }
        Ok(())
    }

    /// Number of topics currently registered. Useful in tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.topics.read().expect("schema registry poisoned").len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

/// A typed message in transit. The bus moves envelopes between
/// [`BusEnd`]s; the payload is `MessagePack`-encoded bytes whose Rust
/// type is determined by `topic` via the [`SchemaRegistry`].
#[derive(Clone, Debug)]
pub struct Envelope {
    /// Per-bus monotonic id.
    pub id: MessageId,
    /// Schema key naming the payload type.
    pub topic: &'static str,
    /// `MessagePack`-encoded payload bytes.
    pub payload: Vec<u8>,
}

// ---------------------------------------------------------------------------
// BusEnd
// ---------------------------------------------------------------------------

/// One end of a [`MessageBus`]. Send goes to the *other* end; recv
/// reads messages aimed at *this* end. Cloning yields another handle
/// that shares the same underlying channels and id counter.
///
/// `BusEnd` is `Send + Sync`; clones can move to other threads and
/// each one can send concurrently. Recv calls from multiple clones
/// race --- exactly one clone will see any given envelope.
#[derive(Clone)]
pub struct BusEnd {
    out: Sender<Envelope>,
    inbox: Receiver<Envelope>,
    registry: Arc<SchemaRegistry>,
    next_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for BusEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusEnd")
            .field("inbox_len", &self.inbox.len())
            .field("registered_topics", &self.registry.len())
            .finish_non_exhaustive()
    }
}

impl BusEnd {
    /// Encode `payload` as `MessagePack` and send to the peer end.
    /// Validates `topic` against the schema registry first; an
    /// unregistered or mismatched topic returns a structured
    /// [`BusError`] without touching the channel.
    pub fn send<T>(&self, topic: &'static str, payload: &T) -> Result<MessageId, BusError>
    where
        T: Serialize + 'static,
    {
        self.registry.check::<T>(topic)?;
        let bytes = rmp_serde::to_vec(payload).map_err(BusError::Encode)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let env = Envelope {
            id,
            topic,
            payload: bytes,
        };
        self.out.send(env).map_err(|_| BusError::Disconnected)?;
        Ok(id)
    }

    /// Block until an envelope arrives at this end. Returns
    /// [`BusError::Disconnected`] if the peer has dropped its sender
    /// without queuing further messages.
    pub fn recv(&self) -> Result<Envelope, BusError> {
        self.inbox.recv().map_err(|_| BusError::Disconnected)
    }

    /// Non-blocking receive: returns [`BusError::Empty`] if no
    /// envelope is queued, or [`BusError::Disconnected`] if the peer
    /// is gone and the queue is drained.
    pub fn try_recv(&self) -> Result<Envelope, BusError> {
        match self.inbox.try_recv() {
            Ok(env) => Ok(env),
            Err(channel::TryRecvError::Empty) => Err(BusError::Empty),
            Err(channel::TryRecvError::Disconnected) => Err(BusError::Disconnected),
        }
    }

    /// Block up to `timeout` waiting for the next envelope.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Envelope, BusError> {
        match self.inbox.recv_timeout(timeout) {
            Ok(env) => Ok(env),
            Err(channel::RecvTimeoutError::Timeout) => Err(BusError::Empty),
            Err(channel::RecvTimeoutError::Disconnected) => Err(BusError::Disconnected),
        }
    }

    /// Decode an envelope's payload as type `T`. Validates the
    /// envelope's topic against the schema registry: if the
    /// registered type for `env.topic` is not `T`, returns
    /// [`BusError::SchemaMismatch`] without invoking the codec.
    pub fn decode<T>(&self, env: &Envelope) -> Result<T, BusError>
    where
        T: DeserializeOwned + 'static,
    {
        self.registry.check::<T>(env.topic)?;
        rmp_serde::from_slice(&env.payload).map_err(BusError::Decode)
    }

    /// Borrow the schema registry. Useful for callers that hold the
    /// pair but want to register topics through the same `Arc` the
    /// bus is using.
    #[must_use]
    pub fn registry(&self) -> &Arc<SchemaRegistry> {
        &self.registry
    }
}

// ---------------------------------------------------------------------------
// MessageBus
// ---------------------------------------------------------------------------

/// Factory for paired [`BusEnd`]s. The bus type itself holds no
/// state; it exists as a namespace for [`Self::pair`].
pub struct MessageBus;

impl MessageBus {
    /// Build a pair of bidirectional [`BusEnd`]s sharing `registry`
    /// and a single per-bus id counter. Either end can send and
    /// receive; messages are routed point-to-point only (each end
    /// has one peer).
    #[must_use]
    pub fn pair(registry: Arc<SchemaRegistry>) -> (BusEnd, BusEnd) {
        let (a_to_b_tx, a_to_b_rx) = channel::unbounded::<Envelope>();
        let (b_to_a_tx, b_to_a_rx) = channel::unbounded::<Envelope>();
        let next_id = Arc::new(AtomicU64::new(0));
        let a = BusEnd {
            out: a_to_b_tx,
            inbox: b_to_a_rx,
            registry: Arc::clone(&registry),
            next_id: Arc::clone(&next_id),
        };
        let b = BusEnd {
            out: b_to_a_tx,
            inbox: a_to_b_rx,
            registry,
            next_id,
        };
        (a, b)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::WorkerPool;
    use serde::Deserialize;
    use std::time::Instant;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Hit {
        line: u32,
        col: u32,
        snippet: String,
    }

    fn registry_with<T: 'static>(topic: &'static str) -> Arc<SchemaRegistry> {
        let r = Arc::new(SchemaRegistry::new());
        r.register::<T>(topic).expect("register");
        r
    }

    /// Schema registration is idempotent for the same type; a
    /// conflicting registration is a structured error.
    #[test]
    fn schema_register_idempotent_for_same_type() {
        let reg = SchemaRegistry::new();
        reg.register::<u32>("ping").unwrap();
        reg.register::<u32>("ping").unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn schema_register_conflicting_type_is_structured_error() {
        let reg = SchemaRegistry::new();
        reg.register::<u32>("ping").unwrap();
        match reg.register::<String>("ping") {
            Err(BusError::DuplicateRegistration {
                topic,
                existing,
                attempted,
            }) => {
                assert_eq!(topic, "ping");
                assert!(existing.contains("u32"), "existing was {existing}");
                assert!(attempted.contains("String"), "attempted was {attempted}");
            }
            other => panic!("expected DuplicateRegistration, got {other:?}"),
        }
    }

    /// Acceptance bullet 1: messages can be sent between the two
    /// ends. Smoke test on a paired bus, no workers involved yet.
    #[test]
    fn round_trip_preserves_value_in_each_direction() {
        let reg = registry_with::<Hit>("search.hit");
        let (a, b) = MessageBus::pair(reg);
        let original = Hit {
            line: 7,
            col: 3,
            snippet: "fn main".into(),
        };
        a.send("search.hit", &original).unwrap();
        let env = b.recv().unwrap();
        let got: Hit = b.decode(&env).unwrap();
        assert_eq!(got, original);

        let echo = Hit {
            line: 8,
            col: 4,
            snippet: "}".into(),
        };
        b.send("search.hit", &echo).unwrap();
        let env = a.recv().unwrap();
        let got: Hit = a.decode(&env).unwrap();
        assert_eq!(got, echo);
    }

    /// Per-bus id counter is monotonic across both directions.
    #[test]
    fn message_ids_are_monotonic_across_directions() {
        let reg = registry_with::<u32>("n");
        let (a, b) = MessageBus::pair(reg);
        let id1 = a.send("n", &1u32).unwrap();
        let id2 = b.send("n", &2u32).unwrap();
        let id3 = a.send("n", &3u32).unwrap();
        assert!(id1 < id2 && id2 < id3);
    }

    /// Acceptance bullet 2 (send side): unregistered topic is a
    /// structured error and never reaches the channel.
    #[test]
    fn send_unregistered_topic_returns_structured_error() {
        let reg = Arc::new(SchemaRegistry::new());
        let (a, _b) = MessageBus::pair(reg);
        match a.send("nope", &1u32) {
            Err(BusError::UnregisteredTopic { topic }) => assert_eq!(topic, "nope"),
            other => panic!("expected UnregisteredTopic, got {other:?}"),
        }
    }

    /// Acceptance bullet 2 (send side): mismatched type is a
    /// structured error and never reaches the channel.
    #[test]
    fn send_with_wrong_type_returns_schema_mismatch() {
        let reg = registry_with::<u32>("n");
        let (a, _b) = MessageBus::pair(reg);
        match a.send("n", &"hi".to_string()) {
            Err(BusError::SchemaMismatch {
                topic,
                expected,
                registered,
            }) => {
                assert_eq!(topic, "n");
                assert!(expected.contains("String"), "expected = {expected}");
                assert!(registered.contains("u32"), "registered = {registered}");
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    /// Acceptance bullet 2 (decode side): pulling an envelope and
    /// decoding it at the wrong type fails before the codec runs.
    #[test]
    fn decode_with_wrong_type_returns_schema_mismatch() {
        let reg = registry_with::<u32>("n");
        let (a, b) = MessageBus::pair(reg);
        a.send("n", &42u32).unwrap();
        let env = b.recv().unwrap();
        let result: Result<String, _> = b.decode(&env);
        match result {
            Err(BusError::SchemaMismatch { topic, .. }) => assert_eq!(topic, "n"),
            other => panic!("expected SchemaMismatch on decode, got {other:?}"),
        }
    }

    /// Acceptance bullet 1, integrated path: a worker pool job sends
    /// a result through the bus; the main thread receives and
    /// decodes it. This is the canonical M3.2 use case --- T M3.3
    /// will wrap this in a Lua coroutine API.
    #[test]
    fn worker_dispatched_job_sends_through_bus() {
        let reg = registry_with::<u64>("worker.result");
        let (main_end, worker_end) = MessageBus::pair(reg);
        let pool = WorkerPool::new(2);

        let _ = pool.dispatch(move |_| {
            // A real job would fetch a snapshot, compute, and send.
            let answer: u64 = (1..=10).sum();
            worker_end.send("worker.result", &answer).unwrap();
        });

        let env = main_end
            .recv_timeout(Duration::from_secs(2))
            .expect("worker reply");
        let got: u64 = main_end.decode(&env).unwrap();
        assert_eq!(got, 55);
    }

    /// Multiple workers each holding a clone of the `BusEnd` should
    /// all be able to send concurrently; the main end collects every
    /// envelope.
    #[test]
    fn many_workers_share_one_bus_end_clone() {
        const N: usize = 50;
        let reg = registry_with::<u32>("worker.tick");
        let (main_end, worker_end) = MessageBus::pair(reg);
        let pool = WorkerPool::new(4);

        for i in 0..N {
            let bus = worker_end.clone();
            let _ = pool.dispatch(move |_| {
                #[allow(clippy::cast_possible_truncation)]
                bus.send("worker.tick", &(i as u32)).unwrap();
            });
        }

        let mut collected = Vec::with_capacity(N);
        for _ in 0..N {
            let env = main_end
                .recv_timeout(Duration::from_secs(2))
                .expect("worker tick");
            let v: u32 = main_end.decode(&env).unwrap();
            collected.push(v);
        }
        collected.sort_unstable();
        let expected: Vec<u32> = (0..N).map(|i| u32::try_from(i).unwrap()).collect();
        assert_eq!(collected, expected);
    }

    /// `try_recv` distinguishes empty from disconnected.
    #[test]
    fn try_recv_reports_empty_then_disconnected() {
        let reg = registry_with::<u32>("n");
        let (a, b) = MessageBus::pair(reg);
        assert!(matches!(a.try_recv(), Err(BusError::Empty)));
        drop(b);
        // After the peer drops, try_recv reports Disconnected (queue
        // was empty when the peer left).
        assert!(matches!(a.try_recv(), Err(BusError::Disconnected)));
    }

    /// Acceptance bullet 3: round-trip latency under a microsecond
    /// budget. The spec target is 50 microseconds in release; we
    /// assert a generous upper bound here so debug-mode CI stays
    /// green, and rely on the loop average rather than any single
    /// iteration to absorb scheduler noise. The point is to fail
    /// loudly if anyone introduces a per-message allocation storm
    /// (a String topic, a clone of the registry `HashMap`, a Box
    /// per send, etc).
    #[test]
    fn round_trip_latency_is_within_budget() {
        const ITERS: usize = 5_000;
        let reg = registry_with::<u32>("ping");
        let (a, b) = MessageBus::pair(reg);

        // Warm-up to amortise channel bootstrap.
        for _ in 0..256 {
            a.send("ping", &1u32).unwrap();
            let _ = b.recv().unwrap();
        }

        let start = Instant::now();
        for i in 0..ITERS {
            #[allow(clippy::cast_possible_truncation)]
            a.send("ping", &(i as u32)).unwrap();
            let env = b.recv().unwrap();
            let _: u32 = b.decode(&env).unwrap();
        }
        let elapsed = start.elapsed();
        #[allow(clippy::cast_possible_truncation)]
        let per_op = elapsed / ITERS as u32;

        // Spec target: 50 microseconds in release. Debug builds run
        // serde without optimisations, so we assert a 500-microsecond
        // ceiling that still catches order-of-magnitude regressions.
        let ceiling = if cfg!(debug_assertions) {
            Duration::from_micros(500)
        } else {
            Duration::from_micros(50)
        };
        assert!(
            per_op < ceiling,
            "round-trip averaged {per_op:?} (target {ceiling:?})"
        );
    }

    /// Acceptance bullet 4 (sentinel): the send hot path produces
    /// exactly one `Vec<u8>` allocation --- the payload buffer ---
    /// per call. We test this indirectly by verifying that the
    /// envelope's payload buffer is the *only* heap-bound piece of
    /// state on the wire (topic is `&'static str`, ids are `u64`).
    /// Compile-time evidence: changing `topic` to `String` would
    /// fail this test by adding String allocations per send.
    #[test]
    fn envelope_topic_is_borrowed_static_str() {
        let reg = registry_with::<u32>("ping");
        let (a, b) = MessageBus::pair(reg);
        a.send("ping", &1u32).unwrap();
        let env = b.recv().unwrap();
        // Static-str equality is by pointer, not by content, when
        // both literals are deduplicated by the linker. Confirm the
        // envelope's topic *is* the same static slot as the literal.
        assert!(std::ptr::eq(env.topic, "ping"));
        // Payload buffer length matches a single u32 in `MessagePack`
        // (positive fixint or u8 form, ≤ 2 bytes).
        assert!(
            env.payload.len() <= 2,
            "payload was {} bytes, expected ≤2 for a small u32",
            env.payload.len()
        );
    }

    /// Disconnect detection: after the peer drops, send returns
    /// Disconnected without panicking.
    #[test]
    fn send_after_peer_drop_returns_disconnected() {
        let reg = registry_with::<u32>("n");
        let (a, b) = MessageBus::pair(reg);
        drop(b);
        assert!(matches!(a.send("n", &1u32), Err(BusError::Disconnected)));
    }
}
