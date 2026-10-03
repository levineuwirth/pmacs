// fold.rs --- the structural fold source (Arc 6), where the tree is.

//! Which regions of a parse fold, read from its tree: the nearest enclosing
//! block-like node of at least two source lines, its introducer resolved to
//! its body, the head line kept visible above the first hidden line and a
//! closing-delimiter line kept visible below the last (hideshow and LSP
//! `foldingRange` parity; the framing is
//! `docs/archive/framings/folding-framing.md`).
//!
//! It lives here, beside the parse, because since E7i the tree lives in a
//! parse unit: the unit answers a fold request by running these functions
//! over its installed bundle, and an in-process bundle is read the same
//! way. The editor keeps the fold store and the state-aware operations
//! (`pmacs::fold`), which need ranges, not a tree. Ranges are
//! `(start, end)` byte offsets, stored as `[end of head line, end of last
//! hidden line]`.

use tree_sitter::Node;

use crate::{Layer, ParseTreeBundle};

const OPEN_DELIMS: &[u8] = b"{[(";
const CLOSE_DELIMS: &[u8] = b"}])";

/// `out[n]` = start byte of line `n`; `out` always begins with `0`. The
/// number of lines is `out.len()` (a trailing entry past the final `\n`
/// is included).
#[must_use]
pub fn line_offsets(source: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(source.len() / 32 + 1);
    out.push(0);
    for (i, b) in source.iter().enumerate() {
        if *b == b'\n' {
            out.push(i as u32 + 1);
        }
    }
    out
}

/// Index of the line containing byte `offset`.
#[must_use]
pub fn line_at_offset(line_offsets: &[u32], offset: u32) -> usize {
    match line_offsets.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// The byte offset just past line `row`'s last *visible* character — i.e.
/// the position of the row's terminating `\n`, or `source.len()` for the
/// final unterminated line. This is the "end of line" the stored range
/// uses for both its head and tail.
#[must_use]
pub fn line_content_end(source: &[u8], line_offsets: &[u32], row: usize) -> u64 {
    let start = line_offsets
        .get(row)
        .copied()
        .unwrap_or(source.len() as u32) as usize;
    let next = line_offsets
        .get(row + 1)
        .copied()
        .unwrap_or(source.len() as u32) as usize;
    let mut end = next.min(source.len());
    if end > start && source[end - 1] == b'\n' {
        end -= 1;
    }
    end as u64
}

/// True iff line `row`'s first non-whitespace byte is a closing delimiter
/// (`}`, `)`, `]`) — the closer-aware tail test.
fn line_starts_with_closer(source: &[u8], line_offsets: &[u32], row: usize) -> bool {
    let Some(&ls) = line_offsets.get(row) else {
        return false;
    };
    let mut i = ls as usize;
    while i < source.len() && (source[i] == b' ' || source[i] == b'\t') {
        i += 1;
    }
    i < source.len() && CLOSE_DELIMS.contains(&source[i])
}

/// Every foldable region enclosing `pos`, **innermost first**. The
/// state-aware commands walk this list against the store to decide what to
/// close (innermost open) or open (outermost closed). Empty for a bundle
/// whose trees live in a parse unit, which answers this itself.
#[must_use]
pub fn candidates_at(bundle: &ParseTreeBundle, pos: u64) -> Vec<(u64, u64)> {
    let source: &[u8] = &bundle.source;
    let offsets = line_offsets(source);
    let Some(node) = innermost_named_node(&bundle.layers, pos) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(r) = fold_from_node(n, source, &offsets)
            && !out.contains(&r)
        {
            out.push(r);
        }
        cur = n.parent();
    }
    out
}

/// The top-level foldable regions in the buffer — what `fold.close-all`
/// collapses (Emacs `hs-hide-all`: top level only, nested not auto-folded).
/// Empty for a bundle whose trees live in a parse unit.
#[must_use]
pub fn top_level_targets(bundle: &ParseTreeBundle) -> Vec<(u64, u64)> {
    let Some(root_layer) = bundle.layers.first() else {
        return Vec::new();
    };
    let source: &[u8] = &bundle.source;
    let offsets = line_offsets(source);
    let root = root_layer.tree.root_node();
    let mut out = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if let Some(r) = fold_from_node(child, source, &offsets)
            && !out.contains(&r)
        {
            out.push(r);
        }
    }
    out
}

/// The innermost named node at `pos`, resolved through injection layers:
/// the deepest layer whose root span covers `pos` wins (a fenced code block
/// inside markdown resolves to the inner block, not the markdown node).
fn innermost_named_node(layers: &[Layer], pos: u64) -> Option<Node<'_>> {
    let p = pos as usize;
    let mut best: Option<&Layer> = None;
    for layer in layers {
        let root = layer.tree.root_node();
        if root.start_byte() <= p && p <= root.end_byte() {
            best = match best {
                Some(b) if b.depth >= layer.depth => Some(b),
                _ => Some(layer),
            };
        }
    }
    best?.tree.root_node().named_descendant_for_byte_range(p, p)
}

/// Compute the fold range a single node yields, or `None` if it is not a
/// foldable structure (< 2 source lines, no block-like body, or a
/// normalized interior with < 1 hidden line).
fn fold_from_node(n: Node<'_>, source: &[u8], line_offsets: &[u32]) -> Option<(u64, u64)> {
    // Match condition: the node spans >= 2 source lines.
    if n.end_position().row <= n.start_position().row {
        return None;
    }
    let (b, introduced) = resolve_body(n, source)?;

    let b_start_row = b.start_position().row;
    let b_end_row = b.end_position().row;
    let b_start_byte = b.start_byte();
    if b_start_byte >= source.len() {
        return None;
    }
    let is_brace = OPEN_DELIMS.contains(&source[b_start_byte]);

    // Head line = the line immediately above the first hidden line. For a
    // brace body that is the `{` line (the introducer's own line, or the
    // `) -> bool {` line when a signature wraps). For an *introduced*
    // delimiter-less body (a Python `block`) the introducer's header ends
    // on the line above, so it is `b_start_row - 1`.
    let head_row = if is_brace {
        b_start_row
    } else if introduced && b_start_row > 0 {
        b_start_row - 1
    } else {
        b_start_row
    };

    // Tail: a closing-delimiter line stays visible (`} else {`); a
    // delimiter-less body hides through its last line.
    let last_hidden_row = if line_starts_with_closer(source, line_offsets, b_end_row) {
        if b_end_row == 0 {
            return None;
        }
        b_end_row - 1
    } else {
        b_end_row
    };

    // Foldability = the normalized interior has >= 1 hidden line.
    if last_hidden_row < head_row + 1 {
        return None;
    }
    let start = line_content_end(source, line_offsets, head_row);
    let end = line_content_end(source, line_offsets, last_hidden_row);
    if end <= start {
        return None;
    }
    Some((start, end))
}

/// Resolve the interior-defining body `B` and whether it is *introduced*
/// (its parent is an introducer whose body field is `B`). If `n` is itself
/// a body, use it; if it is an introducer with a block-like body child,
/// descend to that child (Q#FD1 step 2 — matching/`close-all` association).
fn resolve_body<'tree>(n: Node<'tree>, source: &[u8]) -> Option<(Node<'tree>, bool)> {
    if is_body_kind(n, source) {
        return Some((n, is_introduced(n)));
    }
    if let Some(b) = body_child(n)
        && is_body_kind(b, source)
    {
        return Some((b, true));
    }
    None
}

fn body_child(n: Node) -> Option<Node> {
    n.child_by_field_name("body")
        .or_else(|| n.child_by_field_name("consequence"))
}

fn is_introduced(n: Node<'_>) -> bool {
    if let Some(p) = n.parent()
        && let Some(b) = body_child(p)
    {
        return b.id() == n.id();
    }
    false
}

/// A node is a fold *body* if it opens with a bracket delimiter (a brace
/// body) or is a grammar block node (an indentation body). The delimiter
/// probe generalizes across grammars without a per-language kind list.
fn is_body_kind(n: Node<'_>, source: &[u8]) -> bool {
    let sb = n.start_byte();
    if sb < source.len() && OPEN_DELIMS.contains(&source[sb]) {
        return true;
    }
    matches!(
        n.kind(),
        "block"
            | "statement_block"
            | "declaration_list"
            | "field_declaration_list"
            | "enum_variant_list"
            | "block_mapping"
            | "block_sequence"
    ) || n.kind().ends_with("_body")
}
