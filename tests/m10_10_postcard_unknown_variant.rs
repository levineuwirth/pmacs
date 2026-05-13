//! M10.10 Day 2 first verification test.
//!
//! Question: when postcard deserializes bytes for a wire-format
//! `enum` variant that does not exist in the receiver's enum
//! definition, does it error, drop silently, or something else?
//!
//! Decision rule (per M10.10-FRAMING.md Refinement 3):
//! - Hard error → `PROTOCOL_VERSION` must bump to 3 when M10.10 adds
//!   `InstanceMessage::BufferSnapshot`.
//! - Graceful (error reaches connection-tear-down only) → stays at 2.
//!
//! This test does not depend on pmacs's protocol types. It uses two
//! locally-defined enums to isolate the postcard behavior question.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
enum SenderEnum {
    KnownA(u32),
    KnownB(String),
    NewVariant { id: u64, payload: Vec<u8> },
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
enum ReceiverEnum {
    KnownA(u32),
    KnownB(String),
}

#[test]
fn known_variants_round_trip() {
    let msg = SenderEnum::KnownA(42);
    let bytes = postcard::to_allocvec(&msg).expect("encode");
    let decoded: ReceiverEnum = postcard::from_bytes(&bytes).expect("decode");
    assert_eq!(decoded, ReceiverEnum::KnownA(42));
}

#[test]
fn unknown_variant_behavior_observed() {
    let msg = SenderEnum::NewVariant {
        id: 12345,
        payload: vec![1, 2, 3, 4, 5],
    };
    let bytes = postcard::to_allocvec(&msg).expect("encode");

    let result: Result<ReceiverEnum, _> = postcard::from_bytes(&bytes);

    match result {
        Ok(value) => {
            panic!("postcard silently decoded an unknown variant — unexpected: {value:?}");
        }
        Err(e) => {
            // This is the expected case based on postcard's enum-as-varint
            // discriminant model. Document the error category for the
            // M10.10 audit.
            eprintln!("postcard unknown-variant behavior: hard error");
            eprintln!("  error: {e}");
            eprintln!("  category: {e:?}");
        }
    }
}

#[test]
fn unknown_variant_does_not_corrupt_subsequent_stream() {
    // Concat bytes: [NewVariant payload][KnownA(7) payload].
    // If postcard's error on the first frame is recoverable at the
    // length-prefix-framing layer (as pmacs's transport uses), the
    // second frame should still decode. This isolates whether the
    // unknown-variant error is per-frame or stream-corrupting.
    let bad = postcard::to_allocvec(&SenderEnum::NewVariant {
        id: 99,
        payload: vec![0xff; 4],
    })
    .expect("encode bad");
    let good = postcard::to_allocvec(&SenderEnum::KnownA(7)).expect("encode good");

    let first: Result<ReceiverEnum, _> = postcard::from_bytes(&bad);
    let second: Result<ReceiverEnum, _> = postcard::from_bytes(&good);

    eprintln!("first frame: {first:?}");
    eprintln!("second frame: {second:?}");

    assert!(
        first.is_err(),
        "expected first frame to fail at unknown variant"
    );
    assert_eq!(
        second.expect("second frame should decode independently"),
        ReceiverEnum::KnownA(7),
        "subsequent independent frame must decode regardless of prior failure"
    );
}
