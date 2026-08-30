// rope.rs --- Persistent byte-addressed sequence backing every buffer.

//! Persistent byte-addressed sequence backing every buffer.
//!
//! Implements the rope contract from spec §3.1. The interface is byte-addressed
//! (not codepoint-, not grapheme-): grapheme awareness lives in the text view.
//! Edits do not mutate `self`; they return a new [`Rope`] sharing structure
//! with the old via [`std::sync::Arc`], plus an [`Edit`] description so views
//! can update incrementally without diffing.
//!
//! # Implementation
//!
//! A B-tree of immutable byte chunks. Internal nodes hold up to
//! [`MAX_CHILDREN`] children and cache subtree byte length. Leaves hold up to
//! [`MAX_LEAF_BYTES`] bytes in an `Arc<[u8]>`. All leaves are at uniform
//! depth (B-tree invariant). Edits build new nodes along the touched path
//! and share unchanged subtrees with the source.
//!
//! # Threading
//!
//! [`Rope`] is `Send + Sync`. Workers receive their own handle via
//! [`Rope::snapshot`]; the underlying tree is shared via `Arc`, reads are
//! lock-free, and edits never mutate existing nodes. There are no internal
//! locks.

use std::marker::PhantomData;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Soft cap on bytes per leaf chunk. Inserts that overflow split the leaf in
/// half so each half stays at or below this size.
const MAX_LEAF_BYTES: usize = 1024;

/// Soft cap on children per internal node. Inserts that overflow split the
/// internal in half. Together with [`MAX_LEAF_BYTES`] this caps tree depth
/// at roughly `log_MAX_CHILDREN(rope.len() / MAX_LEAF_BYTES)`.
const MAX_CHILDREN: usize = 8;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

// `Position` is re-exported from `pmacs-protocol` (session 1 of the
// `pmacs-gpu` arc — see `docs/pmacs-gpu-design.md`). The type alias
// is a `u64` byte offset into a [`Rope`]: not a codepoint index, not
// a grapheme index. Grapheme awareness is a view-layer concern.
pub use pmacs_protocol::Position;

/// A persistent rope of bytes.
///
/// Cheap to clone (an `Arc` bump). Edits return a new rope sharing structure
/// with the old; the old rope remains valid as long as any handle holds it.
///
/// # Threading
///
/// `Send + Sync`. Any thread that holds a handle may read it; edits return
/// fresh handles, so the original is never mutated.
#[derive(Clone, Debug)]
pub struct Rope {
    /// Shared tree root. The tree is immutable; new edits build new trees
    /// that share unchanged subtrees with the source.
    root: Arc<Node>,
}

impl Rope {
    /// Construct an empty rope.
    ///
    /// Threading: any thread.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Node::empty_leaf(),
        }
    }

    /// Construct a rope holding exactly the given bytes.
    ///
    /// Builds a balanced tree directly: O(n / `MAX_LEAF_BYTES`) leaves,
    /// O(n / `MAX_LEAF_BYTES`) internal nodes. This is the bulk-load path
    /// for opening files. Threading: any thread.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::new();
        }
        let leaves: Vec<Arc<Node>> = bytes
            .chunks(MAX_LEAF_BYTES)
            .map(|c| Arc::new(Node::Leaf(Arc::from(c))))
            .collect();
        Self {
            root: build_balanced_from_nodes(leaves),
        }
    }

    /// Take a snapshot of this rope.
    ///
    /// Equivalent to [`Clone::clone`]. Named explicitly because workers always
    /// go through this entry point so the intent is documented at every call
    /// site (R23). The returned `Rope` is independent of `self`: subsequent
    /// edits to the buffer that produced `self` do not affect the snapshot.
    ///
    /// Threading: any thread. The result is `Send + Sync` and may be passed
    /// to a worker.
    #[must_use]
    pub fn snapshot(&self) -> Self {
        self.clone()
    }

    /// Return the length of the rope in bytes.
    ///
    /// O(1). Threading: any thread.
    #[must_use]
    pub fn len(&self) -> Position {
        self.root.len()
    }

    /// Return true iff the rope holds zero bytes.
    ///
    /// Threading: any thread.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read the byte at `pos`, or `None` if `pos >= len()`.
    ///
    /// O(log n). Threading: any thread.
    #[must_use]
    pub fn byte_at(&self, pos: Position) -> Option<u8> {
        if pos >= self.len() {
            return None;
        }
        Some(byte_at_in(&self.root, pos))
    }

    /// Copy the bytes in `[start, end)` into `out`.
    ///
    /// `out.len()` must equal `end - start`, and the range must be within
    /// the rope; both are enforced by debug assertion. Threading: any thread.
    pub fn slice(&self, start: Position, end: Position, out: &mut [u8]) {
        debug_assert!(start <= end);
        debug_assert!(end <= self.len());
        debug_assert_eq!(out.len() as u64, end - start);
        if out.is_empty() {
            return;
        }
        slice_in(&self.root, start, end, 0, out);
    }

    /// Iterate over the bytes in `[start, end)` as contiguous chunk slices.
    ///
    /// The returned iterator borrows from this handle: the rope must outlive
    /// the iterator. Bytes are not copied; the slices alias chunk memory.
    /// Yields zero items if `start == end`. Threading: any thread, but the
    /// iterator itself is single-threaded.
    #[must_use]
    pub fn chunks(&self, start: Position, end: Position) -> Chunks<'_> {
        let mut out: Vec<&[u8]> = Vec::new();
        if start < end {
            collect_leaves(&self.root, start, end, 0, &mut out);
        }
        Chunks {
            inner: out.into_iter(),
            _phantom: PhantomData,
        }
    }

    /// Insert `bytes` at `pos`.
    ///
    /// Returns a new rope and an [`Edit`] describing the change relative to
    /// `self`. Returns [`RopeError::OutOfBounds`] if `pos > len()`. Threading:
    /// any thread; `self` is not mutated.
    pub fn insert(&self, pos: Position, bytes: &[u8]) -> Result<Edit, RopeError> {
        if pos > self.len() {
            return Err(RopeError::OutOfBounds {
                pos,
                len: self.len(),
            });
        }
        if bytes.is_empty() {
            return Ok(Edit {
                new_rope: self.clone(),
                range: Range::new(pos, pos),
                inserted_len: 0,
                crdt_op: None,
            });
        }

        let mut root = Arc::clone(&self.root);
        let mut at = pos;
        for chunk in bytes.chunks(MAX_LEAF_BYTES) {
            root = match insert_small(&root, at, chunk) {
                NodeOut::One(n) => n,
                NodeOut::Two(a, b) => Arc::new(Node::make_internal(vec![a, b])),
            };
            at += chunk.len() as u64;
        }

        Ok(Edit {
            new_rope: Self { root },
            range: Range::new(pos, pos),
            inserted_len: bytes.len() as u64,
            crdt_op: None,
        })
    }

    /// Delete the byte range `[start, end)`.
    ///
    /// Returns a new rope and an [`Edit`] describing the change. Returns
    /// [`RopeError::OutOfBounds`] if `start > end` or `end > len()`. Threading:
    /// any thread; `self` is not mutated.
    pub fn delete(&self, start: Position, end: Position) -> Result<Edit, RopeError> {
        if start > end {
            return Err(RopeError::OutOfBounds {
                pos: start,
                len: self.len(),
            });
        }
        if end > self.len() {
            return Err(RopeError::OutOfBounds {
                pos: end,
                len: self.len(),
            });
        }
        if start == end {
            return Ok(Edit {
                new_rope: self.clone(),
                range: Range::new(start, end),
                inserted_len: 0,
                crdt_op: None,
            });
        }

        let new_root = if start == 0 && end == self.len() {
            Node::empty_leaf()
        } else {
            collapse_root(delete_in(&self.root, start, end))
        };

        Ok(Edit {
            new_rope: Self { root: new_root },
            range: Range::new(start, end),
            inserted_len: 0,
            crdt_op: None,
        })
    }

    /// Replace the byte range `[start, end)` with `bytes`.
    ///
    /// Equivalent to a delete followed by an insert at `start`, fused into a
    /// single [`Edit`] description. Returns [`RopeError::OutOfBounds`] if the
    /// range is invalid. Threading: any thread; `self` is not mutated.
    pub fn replace(&self, start: Position, end: Position, bytes: &[u8]) -> Result<Edit, RopeError> {
        let after_delete = self.delete(start, end)?;
        let after_insert = after_delete.new_rope.insert(start, bytes)?;
        Ok(Edit {
            new_rope: after_insert.new_rope,
            range: Range::new(start, end),
            inserted_len: bytes.len() as u64,
            crdt_op: None,
        })
    }
}

impl Default for Rope {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over the byte chunks of a rope range.
///
/// Each `next` yields a contiguous `&[u8]` borrowed from the rope's storage;
/// the union of the slices is exactly the requested range. Empty ranges
/// produce zero items.
pub struct Chunks<'a> {
    inner: std::vec::IntoIter<&'a [u8]>,
    _phantom: PhantomData<&'a Rope>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// The result of a successful edit.
///
/// Carries the new rope and a precise description of what changed, so views
/// can update incrementally without diffing. The semantics:
///
/// * `range` is the byte range *in the OLD rope* that was affected.
/// * `inserted_len` is the number of bytes inserted at `range.start`
///   *in the NEW rope*.
///
/// A pure insert has `range.start == range.end` and `inserted_len > 0`.
/// A pure delete has `range.start < range.end` and `inserted_len == 0`.
/// A replace has both nonzero.
/// A **version-only** edit has `range.start == range.end` and
/// `inserted_len == 0` — no bytes changed at all. CRDT-mode `undo` and
/// `redo` produce this shape when the operation being inverted was
/// itself a textual no-op (replacing bytes with identical bytes), and
/// it still carries a [`CrdtOp`]: a CRDT VERSION delta is a separate
/// dimension from a TEXT delta. Forward `apply_edit` never produces it,
/// because the three syntactically empty `EditOp` forms short-circuit
/// before the CRDT path exists. Its `range` sits at the buffer end,
/// which is where `derive_replacement_edit` reports a no-difference
/// diff; see `docs/crdt-identity-undo-framing.md` for the consumer
/// census that ruled that location harmless.
#[derive(Clone, Debug)]
pub struct Edit {
    /// The rope after the edit. `Send + Sync`; safe to hand to a worker.
    pub new_rope: Rope,
    /// The byte range in the *old* rope that was affected.
    pub range: Range,
    /// Number of bytes inserted at `range.start` in the *new* rope.
    pub inserted_len: u64,
    /// T M10.2 Day 3: optional CRDT-op metadata.
    ///
    /// `Some` when this Edit was produced by a CRDT-backed Buffer's
    /// edit path (`apply_edit` / `undo` / `redo`); `None` otherwise —
    /// in v0.1 mode (no CRDT), and in CRDT mode for the three
    /// syntactically empty `EditOp` forms, which `is_no_op_edit`
    /// short-circuits before the CRDT path runs.
    ///
    /// A **version-only** edit is NOT one of those: an empty range with
    /// zero insertion coming out of `undo`/`redo` carries `Some`, and
    /// must, or the version advance the replicas need is lost. See the
    /// shape list on [`Edit`] above.
    ///
    /// `Box` indirection: keeps Edit's None-case cost to 8 bytes
    /// (Box has a niche-optimized None) rather than the ~32 bytes
    /// inline `Option<CrdtOp>` would take. Edit is constructed in
    /// hot paths (every rope edit), so the size matters; CRDT mode
    /// pays one allocation per edit, v0.1 mode pays nothing extra.
    ///
    /// Always present (not `#[cfg]`-gated) to avoid feature-flag
    /// proliferation through every Edit consumer (views, hooks,
    /// intercepts, undo stack — dozens of touch points). Consumers
    /// that don't care ignore the field; M10.5 (wire protocol) and
    /// M10.4 (per-frontend undo) consume it.
    pub crdt_op: Option<Box<CrdtOp>>,
}

// `CrdtOp` moved to `pmacs-protocol::crdt` (session 1 of the
// `pmacs-gpu` arc — see `docs/pmacs-gpu-design.md`). Re-exported here
// so existing `crate::rope::CrdtOp` import paths continue to resolve.
pub use pmacs_protocol::CrdtOp;

/// A half-open byte range `[start, end)` into a rope.
///
/// A range is *valid* for a rope iff `start <= end <= rope.len()`.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Range {
    /// Inclusive start offset, in bytes.
    pub start: Position,
    /// Exclusive end offset, in bytes.
    pub end: Position,
}

impl Range {
    /// Construct a range. Does not validate; validation happens at use sites.
    #[must_use]
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    /// Length of the range in bytes (`end - start`).
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// True iff `start == end`.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Errors produced by the rope.
#[derive(Debug, thiserror::Error)]
pub enum RopeError {
    /// A position or range fell outside the rope.
    ///
    /// Carries the offending position and the rope's length at the time of
    /// the error so the message is useful at the point of display (R12).
    #[error("rope position {pos} out of bounds (len = {len})")]
    OutOfBounds {
        /// The offending position. For ranges, `start` if `start > end`,
        /// otherwise `end`.
        pos: u64,
        /// Length of the rope when the error was raised.
        len: u64,
    },
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// B-tree node. Private; the `Rope` struct is the only exposed handle.
#[derive(Debug)]
enum Node {
    /// Up to [`MAX_LEAF_BYTES`] bytes of immutable storage.
    Leaf(Arc<[u8]>),
    /// 1..=[`MAX_CHILDREN`] children of identical depth, plus the cumulative
    /// byte length of those children.
    Internal {
        /// Children of this node. Empty only inside transient build steps.
        children: Vec<Arc<Node>>,
        /// Cumulative byte length, cached so reads don't re-sum.
        len: u64,
    },
}

impl Node {
    fn len(&self) -> u64 {
        match self {
            Node::Leaf(b) => b.len() as u64,
            Node::Internal { len, .. } => *len,
        }
    }

    fn make_internal(children: Vec<Arc<Node>>) -> Self {
        let len: u64 = children.iter().map(|c| c.len()).sum();
        Node::Internal { children, len }
    }

    fn empty_leaf() -> Arc<Self> {
        Arc::new(Node::Leaf(Arc::from(&[][..])))
    }

    fn leaf_from(bytes: &[u8]) -> Arc<Self> {
        Arc::new(Node::Leaf(Arc::from(bytes)))
    }

    #[cfg(test)]
    fn depth(&self) -> u32 {
        match self {
            Node::Leaf(_) => 0,
            Node::Internal { children, .. } => 1 + children[0].depth(),
        }
    }
}

/// Result of an insert step at a given level: either replace the source slot
/// with one node (no growth) or with two (split). Both same depth as input.
enum NodeOut {
    One(Arc<Node>),
    Two(Arc<Node>, Arc<Node>),
}

/// Insert at most [`MAX_LEAF_BYTES`] bytes into `node` at byte position `pos`.
/// Returns 1 or 2 replacement nodes of identical depth.
fn insert_small(node: &Arc<Node>, pos: u64, bytes: &[u8]) -> NodeOut {
    debug_assert!(bytes.len() <= MAX_LEAF_BYTES);
    debug_assert!(pos <= node.len());

    match &**node {
        Node::Leaf(chunk) => {
            let p = pos as usize;
            let total = chunk.len() + bytes.len();
            let mut buf = Vec::with_capacity(total);
            buf.extend_from_slice(&chunk[..p]);
            buf.extend_from_slice(bytes);
            buf.extend_from_slice(&chunk[p..]);

            if total <= MAX_LEAF_BYTES {
                NodeOut::One(Node::leaf_from(&buf))
            } else {
                // Total ≤ 2 * MAX_LEAF_BYTES (chunk and bytes each ≤ MAX_LEAF).
                // Splitting at total / 2 keeps both halves ≤ MAX_LEAF_BYTES.
                let mid = total / 2;
                NodeOut::Two(Node::leaf_from(&buf[..mid]), Node::leaf_from(&buf[mid..]))
            }
        }
        Node::Internal { children, .. } => {
            // Find which child contains `pos`. Inclusive on the trailing edge
            // so an insert at `len` lands in the last child.
            let mut offset = 0u64;
            for (i, child) in children.iter().enumerate() {
                let child_len = child.len();
                if pos <= offset + child_len {
                    let local = pos - offset;
                    let result = insert_small(child, local, bytes);
                    return splice_internal(children, i, result);
                }
                offset += child_len;
            }
            unreachable!("insert_small: pos > node.len() (caller invariant violated)");
        }
    }
}

/// Replace `children[idx]` with the 1-or-2 nodes from `result`, then either
/// produce a single internal (≤ `MAX_CHILDREN`) or split into two.
fn splice_internal(children: &[Arc<Node>], idx: usize, result: NodeOut) -> NodeOut {
    let new_children: Vec<Arc<Node>> = match result {
        NodeOut::One(n) => {
            let mut v = Vec::with_capacity(children.len());
            for (j, c) in children.iter().enumerate() {
                v.push(if j == idx {
                    Arc::clone(&n)
                } else {
                    Arc::clone(c)
                });
            }
            v
        }
        NodeOut::Two(a, b) => {
            let mut v = Vec::with_capacity(children.len() + 1);
            for (j, c) in children.iter().enumerate() {
                if j == idx {
                    v.push(a.clone());
                    v.push(b.clone());
                } else {
                    v.push(Arc::clone(c));
                }
            }
            v
        }
    };

    if new_children.len() <= MAX_CHILDREN {
        NodeOut::One(Arc::new(Node::make_internal(new_children)))
    } else {
        let mid = new_children.len() / 2;
        let left = new_children[..mid].to_vec();
        let right = new_children[mid..].to_vec();
        NodeOut::Two(
            Arc::new(Node::make_internal(left)),
            Arc::new(Node::make_internal(right)),
        )
    }
}

/// Delete `[start, end)` from `node`. Caller guarantees the range is strictly
/// less than the entire node. The result has the same depth as `node`.
fn delete_in(node: &Arc<Node>, start: u64, end: u64) -> Arc<Node> {
    debug_assert!(start < end);
    debug_assert!(end <= node.len());
    debug_assert!(!(start == 0 && end == node.len()));

    match &**node {
        Node::Leaf(chunk) => {
            let s = start as usize;
            let e = end as usize;
            let mut buf = Vec::with_capacity(chunk.len() - (e - s));
            buf.extend_from_slice(&chunk[..s]);
            buf.extend_from_slice(&chunk[e..]);
            Node::leaf_from(&buf)
        }
        Node::Internal { children, .. } => {
            let mut new_children: Vec<Arc<Node>> = Vec::with_capacity(children.len());
            let mut offset = 0u64;
            for child in children {
                let child_len = child.len();
                let child_end = offset + child_len;

                if end <= offset || start >= child_end {
                    // No overlap: keep child unchanged.
                    new_children.push(Arc::clone(child));
                } else if start <= offset && end >= child_end {
                    // Fully consumed: drop child.
                } else {
                    // Partial overlap: recurse.
                    let local_start = start.saturating_sub(offset);
                    let local_end = (end - offset).min(child_len);
                    let new_child = delete_in(child, local_start, local_end);
                    new_children.push(new_child);
                }
                offset = child_end;
            }

            // We cannot have zero children: the caller guaranteed the
            // delete is strictly less than the whole node, so at least one
            // child either survives unchanged or in modified form.
            debug_assert!(!new_children.is_empty());
            Arc::new(Node::make_internal(new_children))
        }
    }
}

/// After a delete, the root may be a single-child internal whose only child
/// has the same content. Collapse such chains so depth tracks size.
fn collapse_root(node: Arc<Node>) -> Arc<Node> {
    let mut cur = node;
    loop {
        match &*cur {
            Node::Internal { children, .. } if children.len() == 1 => {
                let only = Arc::clone(&children[0]);
                cur = only;
            }
            _ => break,
        }
    }
    cur
}

fn byte_at_in(node: &Node, pos: u64) -> u8 {
    match node {
        Node::Leaf(chunk) => chunk[pos as usize],
        Node::Internal { children, .. } => {
            let mut offset = 0u64;
            for child in children {
                let cl = child.len();
                if pos < offset + cl {
                    return byte_at_in(child, pos - offset);
                }
                offset += cl;
            }
            unreachable!("byte_at_in: pos >= node.len()");
        }
    }
}

fn slice_in(node: &Node, start: u64, end: u64, node_offset: u64, out: &mut [u8]) {
    match node {
        Node::Leaf(chunk) => {
            let local_start = (start - node_offset) as usize;
            let local_end = (end - node_offset) as usize;
            out.copy_from_slice(&chunk[local_start..local_end]);
        }
        Node::Internal { children, .. } => {
            let mut offset = node_offset;
            let mut written = 0usize;
            for child in children {
                let cl = child.len();
                let child_end = offset + cl;
                if end <= offset {
                    break;
                }
                if start < child_end && start.max(offset) < end.min(child_end) {
                    let take_start = start.max(offset);
                    let take_end = end.min(child_end);
                    let take = (take_end - take_start) as usize;
                    slice_in(
                        child,
                        take_start,
                        take_end,
                        offset,
                        &mut out[written..written + take],
                    );
                    written += take;
                }
                offset = child_end;
            }
        }
    }
}

fn collect_leaves<'a>(
    node: &'a Node,
    start: u64,
    end: u64,
    node_offset: u64,
    out: &mut Vec<&'a [u8]>,
) {
    match node {
        Node::Leaf(chunk) => {
            let node_end = node_offset + chunk.len() as u64;
            if end <= node_offset || start >= node_end {
                return;
            }
            let local_start = start.saturating_sub(node_offset) as usize;
            let local_end = ((end - node_offset).min(chunk.len() as u64)) as usize;
            if local_start < local_end {
                out.push(&chunk[local_start..local_end]);
            }
        }
        Node::Internal { children, .. } => {
            let mut offset = node_offset;
            for child in children {
                let cl = child.len();
                let child_end = offset + cl;
                if end <= offset {
                    break;
                }
                if start < child_end {
                    collect_leaves(child, start, end, offset, out);
                }
                offset = child_end;
            }
        }
    }
}

/// Build a balanced internal-tree from a flat list of same-depth nodes.
/// Bottom-up: pack groups of up to [`MAX_CHILDREN`], recurse until one node
/// remains. A trailing group of size 1 is folded into the previous group to
/// avoid pathological single-child internals (cheap rebalance for free).
fn build_balanced_from_nodes(nodes: Vec<Arc<Node>>) -> Arc<Node> {
    if nodes.is_empty() {
        return Node::empty_leaf();
    }
    if nodes.len() == 1 {
        return nodes.into_iter().next().expect("len 1");
    }
    let mut current = nodes;
    while current.len() > 1 {
        let mut next: Vec<Arc<Node>> = Vec::with_capacity(current.len().div_ceil(MAX_CHILDREN));
        let mut i = 0;
        while i < current.len() {
            let remaining = current.len() - i;
            // If we'd leave a single straggler at the end, take fewer now so
            // the next group has at least 2 elements.
            let take = if remaining > MAX_CHILDREN && remaining - MAX_CHILDREN == 1 {
                MAX_CHILDREN - 1
            } else {
                remaining.min(MAX_CHILDREN)
            };
            let group: Vec<Arc<Node>> = current[i..i + take].to_vec();
            next.push(Arc::new(Node::make_internal(group)));
            i += take;
        }
        current = next;
    }
    current.into_iter().next().expect("non-empty after loop")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync_static<T: Send + Sync + 'static>() {}

    fn collect(rope: &Rope) -> Vec<u8> {
        let mut out = vec![0u8; rope.len() as usize];
        rope.slice(0, rope.len(), &mut out);
        out
    }

    fn check_invariants(node: &Node) {
        match node {
            Node::Leaf(b) => {
                assert!(
                    b.len() <= MAX_LEAF_BYTES,
                    "leaf {} > MAX_LEAF_BYTES",
                    b.len()
                );
            }
            Node::Internal { children, len } => {
                assert!(!children.is_empty(), "internal node with zero children");
                assert!(
                    children.len() <= MAX_CHILDREN,
                    "internal {} > MAX_CHILDREN",
                    children.len()
                );
                let depth = children[0].depth();
                let sum: u64 = children.iter().map(|c| c.len()).sum();
                assert_eq!(sum, *len, "internal len out of sync");
                for c in children {
                    assert_eq!(c.depth(), depth, "depth mismatch among siblings");
                    check_invariants(c);
                }
            }
        }
    }

    // ----- type-level / smoke -----

    #[test]
    fn rope_is_send_sync() {
        assert_send_sync_static::<Rope>();
        assert_send_sync_static::<Edit>();
    }

    #[test]
    fn empty_rope_basics() {
        let r = Rope::new();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert_eq!(r.byte_at(0), None);
        assert_eq!(r.chunks(0, 0).count(), 0);
        check_invariants(&r.root);
    }

    #[test]
    fn range_basics() {
        let r = Range::new(3, 8);
        assert_eq!(r.len(), 5);
        assert!(!r.is_empty());

        let empty = Range::new(5, 5);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    // ----- from_bytes / read paths -----

    #[test]
    fn from_bytes_small_roundtrip() {
        let src = b"hello world";
        let r = Rope::from_bytes(src);
        assert_eq!(r.len(), src.len() as u64);
        assert_eq!(collect(&r), src);
        check_invariants(&r.root);
    }

    #[test]
    fn from_bytes_multi_chunk_roundtrip() {
        // Force multiple leaves: 4096 bytes / 1024 leaf cap -> 4 leaves.
        let src: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        let r = Rope::from_bytes(&src);
        assert_eq!(r.len(), src.len() as u64);
        assert_eq!(collect(&r), src);
        check_invariants(&r.root);
    }

    #[test]
    fn from_bytes_large_roundtrip() {
        // 100 KB exercises a multi-level internal tree.
        let src: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
        let r = Rope::from_bytes(&src);
        assert_eq!(r.len(), src.len() as u64);
        assert_eq!(collect(&r), src);
        check_invariants(&r.root);
    }

    #[test]
    fn byte_at_walks_tree() {
        let src: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        let r = Rope::from_bytes(&src);
        for i in [0u64, 1, 100, 1023, 1024, 1025, 4095] {
            assert_eq!(r.byte_at(i), Some(src[i as usize]));
        }
        assert_eq!(r.byte_at(4096), None);
        assert_eq!(r.byte_at(u64::MAX), None);
    }

    #[test]
    fn chunks_match_slice() {
        let src: Vec<u8> = (0..3000).map(|i| (i % 251) as u8).collect();
        let r = Rope::from_bytes(&src);

        // Whole rope.
        let joined: Vec<u8> = r.chunks(0, r.len()).flatten().copied().collect();
        assert_eq!(joined, src);

        // Mid-chunk window crossing leaf boundaries.
        let joined: Vec<u8> = r.chunks(500, 2500).flatten().copied().collect();
        assert_eq!(joined, src[500..2500]);

        // Empty range yields zero items.
        assert_eq!(r.chunks(100, 100).count(), 0);
    }

    // ----- insert -----

    #[test]
    fn insert_into_empty() {
        let r = Rope::new();
        let edit = r.insert(0, b"abc").unwrap();
        assert_eq!(edit.range, Range::new(0, 0));
        assert_eq!(edit.inserted_len, 3);
        assert_eq!(collect(&edit.new_rope), b"abc");
        check_invariants(&edit.new_rope.root);
    }

    #[test]
    fn insert_at_start_middle_end() {
        let r = Rope::from_bytes(b"hello");
        let r = r.insert(5, b"!").unwrap().new_rope;
        assert_eq!(collect(&r), b"hello!");
        let r = r.insert(0, b"[").unwrap().new_rope;
        assert_eq!(collect(&r), b"[hello!");
        let r = r.insert(3, b"-").unwrap().new_rope;
        assert_eq!(collect(&r), b"[he-llo!");
        check_invariants(&r.root);
    }

    #[test]
    fn insert_past_end_is_error() {
        let r = Rope::from_bytes(b"abc");
        let err = r.insert(4, b"x").unwrap_err();
        match err {
            RopeError::OutOfBounds { pos, len } => {
                assert_eq!(pos, 4);
                assert_eq!(len, 3);
            }
        }
    }

    #[test]
    fn insert_triggers_leaf_split() {
        // Start with a near-full leaf, then insert into the middle.
        let mut chunk = vec![b'a'; MAX_LEAF_BYTES - 10];
        let r = Rope::from_bytes(&chunk);
        // Insert 100 bytes in the middle: total 1014 + 100 = 1114 > 1024 → split.
        let inserted = vec![b'b'; 100];
        let edited = r
            .insert((MAX_LEAF_BYTES / 2) as u64, &inserted)
            .unwrap()
            .new_rope;

        let mut expected = chunk.clone();
        expected.splice(
            (MAX_LEAF_BYTES / 2)..(MAX_LEAF_BYTES / 2),
            inserted.iter().copied(),
        );
        assert_eq!(collect(&edited), expected);
        check_invariants(&edited.root);

        // Confirm we actually split: the resulting root is now an Internal.
        chunk.clear();
        match &*edited.root {
            Node::Leaf(_) => panic!("expected internal after split"),
            Node::Internal { .. } => {}
        }
    }

    #[test]
    fn insert_large_block() {
        let r = Rope::from_bytes(b"prefix-suffix");
        let middle: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();
        let edited = r.insert(7, &middle).unwrap().new_rope;

        let mut expected = Vec::with_capacity(13 + middle.len());
        expected.extend_from_slice(b"prefix-");
        expected.extend_from_slice(&middle);
        expected.extend_from_slice(b"suffix");
        assert_eq!(edited.len(), expected.len() as u64);
        assert_eq!(collect(&edited), expected);
        check_invariants(&edited.root);
    }

    // ----- delete -----

    #[test]
    fn delete_in_single_leaf() {
        let r = Rope::from_bytes(b"hello world");
        let r = r.delete(5, 11).unwrap().new_rope;
        assert_eq!(collect(&r), b"hello");
        check_invariants(&r.root);
    }

    #[test]
    fn delete_full_rope() {
        let r = Rope::from_bytes(b"hello");
        let edit = r.delete(0, 5).unwrap();
        assert_eq!(edit.new_rope.len(), 0);
        assert_eq!(edit.inserted_len, 0);
        assert_eq!(edit.range, Range::new(0, 5));
        check_invariants(&edit.new_rope.root);
    }

    #[test]
    fn delete_across_chunks() {
        let src: Vec<u8> = (0..10_000).map(|i| (i % 251) as u8).collect();
        let r = Rope::from_bytes(&src);
        let edited = r.delete(500, 9500).unwrap().new_rope;

        let mut expected = src[..500].to_vec();
        expected.extend_from_slice(&src[9500..]);
        assert_eq!(collect(&edited), expected);
        check_invariants(&edited.root);
    }

    #[test]
    fn delete_invalid_ranges() {
        let r = Rope::from_bytes(b"abc");
        assert!(r.delete(2, 1).is_err());
        assert!(r.delete(0, 4).is_err());
    }

    // ----- replace -----

    #[test]
    fn replace_simple() {
        let r = Rope::from_bytes(b"hello world");
        let edit = r.replace(6, 11, b"there").unwrap();
        assert_eq!(collect(&edit.new_rope), b"hello there");
        assert_eq!(edit.range, Range::new(6, 11));
        assert_eq!(edit.inserted_len, 5);
        check_invariants(&edit.new_rope.root);
    }

    #[test]
    fn replace_with_empty_is_delete() {
        let r = Rope::from_bytes(b"abcdef");
        let edit = r.replace(2, 5, b"").unwrap();
        assert_eq!(collect(&edit.new_rope), b"abf");
        assert_eq!(edit.inserted_len, 0);
    }

    #[test]
    fn replace_empty_range_is_insert() {
        let r = Rope::from_bytes(b"abcdef");
        let edit = r.replace(3, 3, b"XYZ").unwrap();
        assert_eq!(collect(&edit.new_rope), b"abcXYZdef");
        assert_eq!(edit.range, Range::new(3, 3));
        assert_eq!(edit.inserted_len, 3);
    }

    // ----- snapshot independence -----

    #[test]
    fn snapshot_is_independent() {
        let r = Rope::from_bytes(b"original");
        let snap = r.snapshot();
        let edited = r.insert(0, b"X").unwrap().new_rope;
        // The snapshot is unaffected by the new edit.
        assert_eq!(collect(&snap), b"original");
        assert_eq!(collect(&edited), b"Xoriginal");
    }

    // ----- structural sharing -----

    #[test]
    fn unaffected_leaves_are_shared_by_arc() {
        // A 32 KB rope, edit at the end, observe that the front portion's
        // first leaf is still pointer-equal to the original's first leaf.
        let src: Vec<u8> = (0..32 * 1024).map(|i| (i % 251) as u8).collect();
        let original = Rope::from_bytes(&src);
        let original_first_leaf = first_leaf_arc(&original.root);

        let edited = original.insert(original.len(), b"Z").unwrap().new_rope;
        let edited_first_leaf = first_leaf_arc(&edited.root);

        // The first leaf is untouched by an end-of-rope insert -> structural
        // sharing means the Arc points to the same allocation.
        assert!(Arc::ptr_eq(&original_first_leaf, &edited_first_leaf));
    }

    fn first_leaf_arc(node: &Arc<Node>) -> Arc<[u8]> {
        match &**node {
            Node::Leaf(b) => Arc::clone(b),
            Node::Internal { children, .. } => first_leaf_arc(&children[0]),
        }
    }

    // ----- random fuzz -----

    #[test]
    fn random_edit_sequence_matches_reference() {
        // Pseudorandom sequence with a fixed seed: deterministic and
        // dense enough to exercise both leaf and internal splits / collapses.
        let mut rng_state: u64 = 0xDEAD_BEEF;
        let mut rng = || {
            rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (rng_state >> 33) as u32
        };

        let mut reference: Vec<u8> = (0..2048).map(|i| (i % 251) as u8).collect();
        let mut rope = Rope::from_bytes(&reference);

        for _ in 0..1000 {
            let len = reference.len();
            let op = rng() % 3;
            match op {
                0 => {
                    // Insert
                    let pos = (rng() as usize) % (len + 1);
                    let n = (rng() % 64 + 1) as usize;
                    let bytes: Vec<u8> = (0..n)
                        .map(|i| (rng() as u8).wrapping_add(i as u8))
                        .collect();
                    rope = rope.insert(pos as u64, &bytes).unwrap().new_rope;
                    reference.splice(pos..pos, bytes);
                }
                1 => {
                    // Delete
                    if len == 0 {
                        continue;
                    }
                    let s = (rng() as usize) % len;
                    let e = s + (rng() as usize) % (len - s + 1).max(1);
                    let e = e.min(len);
                    if s == e {
                        continue;
                    }
                    rope = rope.delete(s as u64, e as u64).unwrap().new_rope;
                    reference.drain(s..e);
                }
                _ => {
                    // Replace
                    if len == 0 {
                        continue;
                    }
                    let s = (rng() as usize) % len;
                    let e = s + (rng() as usize) % (len - s + 1).max(1);
                    let e = e.min(len);
                    let n = (rng() % 32) as usize;
                    let bytes: Vec<u8> = (0..n)
                        .map(|i| (rng() as u8).wrapping_add(i as u8))
                        .collect();
                    rope = rope.replace(s as u64, e as u64, &bytes).unwrap().new_rope;
                    reference.splice(s..e, bytes);
                }
            }

            assert_eq!(rope.len(), reference.len() as u64);
            check_invariants(&rope.root);
        }

        assert_eq!(collect(&rope), reference);
    }

    // ----- perf smoke -----
    //
    // Coarse timing checks. The acceptance suite (T M1.10) holds the
    // proper benchmarks; these exist so a regression here shows up
    // immediately rather than waiting for the formal harness.

    #[test]
    #[ignore = "perf smoke; run with --release --ignored"]
    fn perf_smoke_load_100mb() {
        let src: Vec<u8> = (0..100 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let started = std::time::Instant::now();
        let rope = Rope::from_bytes(&src);
        let elapsed = started.elapsed();
        eprintln!("from_bytes(100MB): {elapsed:?}");
        assert_eq!(rope.len(), src.len() as u64);
    }

    #[test]
    #[ignore = "perf smoke; run with --release --ignored"]
    fn perf_smoke_snapshot() {
        let src: Vec<u8> = (0..10 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let rope = Rope::from_bytes(&src);

        let started = std::time::Instant::now();
        let mut snaps = Vec::with_capacity(10_000);
        for _ in 0..10_000 {
            snaps.push(rope.snapshot());
        }
        let elapsed = started.elapsed();
        let per_op = elapsed / 10_000;
        eprintln!("snapshot p~avg over 10k: {per_op:?}");
        // Sanity: keep snaps alive past the timing window.
        std::hint::black_box(snaps);
    }

    #[test]
    #[ignore = "perf smoke; run with --release --ignored"]
    fn perf_smoke_edit_latency() {
        let src: Vec<u8> = (0..10 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut rope = Rope::from_bytes(&src);
        let mut times = Vec::with_capacity(1000);
        for i in 0..1000 {
            let pos = (i * 1024) % rope.len();
            let started = std::time::Instant::now();
            rope = rope.insert(pos, b"x").unwrap().new_rope;
            times.push(started.elapsed());
        }
        times.sort_unstable();
        eprintln!(
            "edit p50 = {:?}, p99 = {:?}, max = {:?}",
            times[500], times[990], times[999],
        );
    }
}
