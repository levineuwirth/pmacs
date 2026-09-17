// tests/undo_arbiter_differential_acceptance.rs --- E6c: the arbiter
// against the plain stack.

//! A differential witness for the cross-peer undo arbiter: with one
//! source and no concurrent peer, undoing through the arbiter must
//! agree byte for byte with the v0.1 rope stack, step for step, over a
//! long random history of inserts, deletes and replaces --- and so
//! must redoing it back. The plain stack is the oracle because it
//! restores whole pre-image ropes and cannot be wrong about a single
//! step; the arbiter computes each step as the inverse of op spans
//! transformed against everything after, which is where a compensation
//! can go wrong by exactly the replaced span (the checkpoint's
//! `Rep(3, 8, 14 bytes)` divergence at undo 184, which no hand-written
//! probe would have found).
//!
//! Seeded deterministically, so a divergence names its step and is
//! reproducible; the assertion message carries the step, the op and the
//! first differing byte.

#![cfg(feature = "crdt")]

use pmacs::buffer::{Buffer, BufferId, EditOp};
use pmacs::rope::Range;

fn collect(b: &Buffer) -> Vec<u8> {
    let len = b.len();
    let mut out = vec![0u8; len as usize];
    if len > 0 {
        b.snapshot_rope().slice(0, len, &mut out);
    }
    out
}

#[derive(Clone, Debug)]
enum Op {
    Ins(u64, Vec<u8>),
    Del(u64, u64),
    Rep(u64, u64, Vec<u8>),
}

/// A deterministic random history of `count` ops against a length
/// model, all in ASCII so every position is a codepoint boundary.
fn random_ops(seed: u64, count: usize, initial_len: u64) -> Vec<Op> {
    let mut rng_state = seed;
    let mut rng = || {
        rng_state = rng_state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (rng_state >> 33) as u32
    };
    let mut model_len = initial_len;
    let mut ops: Vec<Op> = Vec::new();
    while ops.len() < count {
        match rng() % 3 {
            0 => {
                let pos = u64::from(rng()) % (model_len + 1);
                let n = (rng() % 32 + 1) as usize;
                let bytes: Vec<u8> = (0..n)
                    .map(|i| ((rng() & 0x7F) as u8).wrapping_add(i as u8) & 0x7F)
                    .collect();
                model_len += n as u64;
                ops.push(Op::Ins(pos, bytes));
            }
            1 if model_len > 0 => {
                let s = u64::from(rng()) % model_len;
                let e = (s + u64::from(rng()) % (model_len - s + 1).max(1)).min(model_len);
                if s < e {
                    model_len -= e - s;
                    ops.push(Op::Del(s, e));
                }
            }
            _ if model_len > 0 => {
                let s = u64::from(rng()) % model_len;
                let e = (s + u64::from(rng()) % (model_len - s + 1).max(1)).min(model_len);
                if s < e {
                    let n = (rng() % 16) as usize;
                    let bytes: Vec<u8> = (0..n)
                        .map(|i| ((rng() & 0x7F) as u8).wrapping_add(i as u8) & 0x7F)
                        .collect();
                    model_len = model_len - (e - s) + n as u64;
                    ops.push(Op::Rep(s, e, bytes));
                }
            }
            _ => {}
        }
    }
    ops
}

fn apply(b: &mut Buffer, op: &Op) {
    match op {
        Op::Ins(pos, bytes) => b.apply_edit(EditOp::Insert { pos: *pos, bytes }),
        Op::Del(s, e) => b.apply_edit(EditOp::Delete {
            range: Range::new(*s, *e),
        }),
        Op::Rep(s, e, bytes) => b.apply_edit(EditOp::Replace {
            range: Range::new(*s, *e),
            bytes,
        }),
    }
    .expect("forward edit");
}

fn first_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

/// Two hundred random ops applied to a CRDT-backed buffer and to a
/// plain one; then every op undone on both, comparing after each step;
/// then every op redone on both, comparing after each step.
#[test]
fn arbiter_undo_and_redo_agree_with_the_plain_stack_step_for_step() {
    let original: Vec<u8> = (0..512u32).map(|i| (i % 128) as u8).collect();
    let ops = random_ops(0x1234_5678, 200, original.len() as u64);
    assert_eq!(ops.len(), 200);

    let mut arbiter = Buffer::from_bytes(BufferId::next(), "arbiter", &original);
    arbiter.upgrade_to_crdt(1).expect("upgrade");
    let mut oracle = Buffer::from_bytes(BufferId::next(), "oracle", &original);
    for op in &ops {
        apply(&mut arbiter, op);
        apply(&mut oracle, op);
    }
    assert_eq!(collect(&arbiter), collect(&oracle), "forward divergence");

    for (i, op) in ops.iter().enumerate().rev() {
        arbiter
            .undo()
            .unwrap_or_else(|e| panic!("arbiter undo {i} ({op:?}) failed: {e:?}"));
        oracle
            .undo()
            .unwrap_or_else(|e| panic!("oracle undo {i} ({op:?}) failed: {e:?}"));
        let a = collect(&arbiter);
        let o = collect(&oracle);
        assert!(
            a == o,
            "DIVERGE at undo {i} ({op:?}): arbiter len {} oracle len {} first difference at byte {}",
            a.len(),
            o.len(),
            first_difference(&a, &o)
        );
    }
    assert_eq!(
        collect(&arbiter),
        original,
        "every undo taken, back to the original"
    );
    assert!(arbiter.undo().is_err(), "nothing left to undo");

    for (i, op) in ops.iter().enumerate() {
        arbiter
            .redo()
            .unwrap_or_else(|e| panic!("arbiter redo {i} ({op:?}) failed: {e:?}"));
        oracle
            .redo()
            .unwrap_or_else(|e| panic!("oracle redo {i} ({op:?}) failed: {e:?}"));
        let a = collect(&arbiter);
        let o = collect(&oracle);
        assert!(
            a == o,
            "DIVERGE at redo {i} ({op:?}): arbiter len {} oracle len {} first difference at byte {}",
            a.len(),
            o.len(),
            first_difference(&a, &o)
        );
    }
    assert!(arbiter.redo().is_err(), "nothing left to redo");
}

/// The same agreement under a second seed, with undo and redo
/// interleaved: three back, two forward, repeatedly, so the redo
/// stack is exercised while undo groups remain below it.
#[test]
fn arbiter_agrees_with_the_plain_stack_under_interleaved_undo_and_redo() {
    let original: Vec<u8> = b"the quick brown fox jumps over the lazy dog".to_vec();
    let ops = random_ops(0xdead_beef, 120, original.len() as u64);

    let mut arbiter = Buffer::from_bytes(BufferId::next(), "arbiter", &original);
    arbiter.upgrade_to_crdt(1).expect("upgrade");
    let mut oracle = Buffer::from_bytes(BufferId::next(), "oracle", &original);
    for op in &ops {
        apply(&mut arbiter, op);
        apply(&mut oracle, op);
    }

    let mut step = 0usize;
    loop {
        let mut undone = 0;
        for _ in 0..3 {
            let (a, o) = (arbiter.undo(), oracle.undo());
            assert_eq!(a.is_ok(), o.is_ok(), "undo availability at step {step}");
            if a.is_err() {
                break;
            }
            undone += 1;
            let (a, o) = (collect(&arbiter), collect(&oracle));
            assert!(
                a == o,
                "DIVERGE at step {step} (undo): first difference at byte {}",
                first_difference(&a, &o)
            );
            step += 1;
        }
        if undone < 3 {
            // The bottom: fewer than three left means the walk is
            // done, or two forward would undo the same two forever.
            break;
        }
        for _ in 0..2 {
            let (a, o) = (arbiter.redo(), oracle.redo());
            assert_eq!(a.is_ok(), o.is_ok(), "redo availability at step {step}");
            let (a, o) = (collect(&arbiter), collect(&oracle));
            assert!(
                a == o,
                "DIVERGE at step {step} (redo): first difference at byte {}",
                first_difference(&a, &o)
            );
            step += 1;
        }
    }
    while arbiter.undo().is_ok() {
        oracle.undo().expect("oracle has as many steps left");
        let (a, o) = (collect(&arbiter), collect(&oracle));
        assert!(a == o, "DIVERGE on the final walk down at step {step}");
        step += 1;
    }
    assert!(
        oracle.undo().is_err(),
        "the oracle bottoms out with the arbiter"
    );
    assert_eq!(collect(&arbiter), original, "walked back to the original");
    assert!(
        step > 200,
        "the interleaving covered the history ({step} steps)"
    );
}
