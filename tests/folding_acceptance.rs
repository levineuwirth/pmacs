//! Folding acceptance (Arc 6, Stage 1 — docs/folding-framing.md).
//!
//! Headless coverage of the fold engine over real grammars: the
//! structural source (derived head line + closer-aware tail across brace
//! and indentation grammars, wrapped signatures, and injection layers),
//! the state-aware operations, the data-API validation, and the
//! dispatch-layer command-path pre-edit unfold. The `FoldState` producer
//! transitions are pinned in `src/semantic_render.rs`
//! (`fold_state_producer_transitions`).

use std::sync::Arc;

use pmacs::buffer::{Buffer, BufferId, EditOp};
use pmacs::editor::EditorState;
use pmacs::fold::{
    self, CycleOutcome, FoldStore, close_at, cycle_at, fold_target_at, open_at,
    top_level_fold_targets,
};
use pmacs::protocol::ByteRange;
use pmacs::syntax::{ParseTreeBundle, ParseView, SyntaxRegistry, run_parse};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Parse `src` under `lang` synchronously into a resolved bundle (mirrors
/// the crate-internal `parse_layered` with public APIs).
fn parse(reg: &SyntaxRegistry, lang: &str, src: &[u8]) -> Arc<ParseTreeBundle> {
    let language = reg.language(lang).expect("grammar loads");
    let mut buf = Buffer::from_bytes(BufferId::next(), "doc", src);
    let view = ParseView::new(&buf, language, lang.to_owned());
    let handle = view.handle();
    let _ = buf.attach_view(Box::new(view));
    let mut req = handle.make_request();
    req.injection_aliases = reg.injection_alias_snapshot();
    let bundle = run_parse(req).expect("parse succeeds");
    reg.resolve_layer_queries(&bundle)
}

/// The byte offset of `needle`'s first occurrence in `src`.
fn byte_of(src: &str, needle: &str) -> u64 {
    src.find(needle).expect("needle present") as u64
}

/// The content-end byte (position of the terminating `\n`, or EOF) of the
/// line containing `needle`'s first occurrence.
fn line_content_end_of(src: &str, needle: &str) -> u64 {
    let idx = src.find(needle).expect("needle present");
    let bytes = src.as_bytes();
    let mut e = idx;
    while e < bytes.len() && bytes[e] != b'\n' {
        e += 1;
    }
    e as u64
}

/// The text of the line containing byte `b`.
fn line_text_at(src: &str, b: u64) -> &str {
    let bytes = src.as_bytes();
    let b = (b as usize).min(bytes.len());
    let start = bytes[..b]
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1);
    let end = bytes[b..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(bytes.len(), |i| b + i);
    &src[start..end]
}

// ---------------------------------------------------------------------------
// 1. Head line — both grammar shapes, wrapped headers (R2-1, R3-1).
// ---------------------------------------------------------------------------

#[test]
fn head_line_rust_single_line_signature() {
    let reg = SyntaxRegistry::new();
    let src = "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n";
    let bundle = parse(&reg, "rust", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "let x")).expect("foldable");
    assert_eq!(r.start, line_content_end_of(src, "fn foo() {"));
    assert_eq!(
        line_text_at(src, r.start),
        "fn foo() {",
        "head line is the fn line"
    );
    assert_eq!(
        line_text_at(src, r.start + 1),
        "    let x = 1;",
        "first hidden line"
    );
}

#[test]
fn head_line_rust_wrapped_signature_keeps_signature_visible() {
    // R3-1: rustfmt puts `{` on the `) -> bool {` line; the head must be
    // that line, NOT `fn foo(` — the wrapped signature stays visible.
    let reg = SyntaxRegistry::new();
    let src = "fn foo(\n    a: u32,\n) -> bool {\n    true\n}\n";
    let bundle = parse(&reg, "rust", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "true")).expect("foldable");
    assert_eq!(r.start, line_content_end_of(src, ") -> bool {"));
    assert_eq!(line_text_at(src, r.start), ") -> bool {");
    // The two wrapped signature lines are before the fold → visible.
    assert!(r.start > line_content_end_of(src, "a: u32,"));
}

#[test]
fn head_line_python_uses_def_not_a_body_line() {
    // R2-1: tree-sitter-python's `block` starts on the first statement
    // line, so the head must ascend to `def foo():`, not `x = 1`.
    let reg = SyntaxRegistry::new();
    let src = "def foo():\n    x = 1\n    y = 2\n";
    let bundle = parse(&reg, "python", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "x = 1")).expect("foldable");
    assert_eq!(r.start, line_content_end_of(src, "def foo():"));
    assert_eq!(line_text_at(src, r.start), "def foo():");
    assert_eq!(line_text_at(src, r.start + 1), "    x = 1");
}

#[test]
fn head_line_python_wrapped_signature_keeps_signature_visible() {
    let reg = SyntaxRegistry::new();
    let src = "def foo(\n    a,\n):\n    x = 1\n";
    let bundle = parse(&reg, "python", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "x = 1")).expect("foldable");
    assert_eq!(r.start, line_content_end_of(src, "):"));
    assert_eq!(line_text_at(src, r.start), "):");
}

// ---------------------------------------------------------------------------
// 4. Range semantics — closer-aware tail (R2-5).
// ---------------------------------------------------------------------------

#[test]
fn brace_closer_line_stays_visible() {
    let reg = SyntaxRegistry::new();
    let src = "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n";
    let bundle = parse(&reg, "rust", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "let x")).expect("foldable");
    // Last hidden line is the last body line; the `}` line is outside.
    assert_eq!(r.end, line_content_end_of(src, "let y = 2;"));
    assert_eq!(
        line_text_at(src, r.end + 1),
        "}",
        "closer line stays visible"
    );
}

#[test]
fn shared_closer_line_else_stays_visible() {
    // R2-5: `} else {` keeps its trailing sibling on screen.
    let reg = SyntaxRegistry::new();
    let src = "fn f() {\n    if a {\n        one();\n    } else {\n        two();\n    }\n}\n";
    let bundle = parse(&reg, "rust", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "one()")).expect("foldable");
    // The consequent block folds; its `} else {` line stays visible.
    assert_eq!(line_text_at(src, r.end + 1).trim(), "} else {");
}

#[test]
fn python_hides_through_last_body_line() {
    let reg = SyntaxRegistry::new();
    let src = "def foo():\n    x = 1\n    y = 2\n";
    let bundle = parse(&reg, "python", src.as_bytes());
    let r = fold_target_at(&bundle, byte_of(src, "x = 1")).expect("foldable");
    // Delimiter-less: the last body line is hidden (inside the range).
    assert_eq!(r.end, line_content_end_of(src, "y = 2"));
}

// ---------------------------------------------------------------------------
// 3. close-all folds top-level regions only; 9. nested + state-aware order.
// ---------------------------------------------------------------------------

#[test]
fn close_all_is_top_level_only() {
    let reg = SyntaxRegistry::new();
    let src = "fn a() {\n    if c {\n        work();\n        more();\n    }\n}\n\nfn b() {\n    x();\n    y();\n}\n";
    let bundle = parse(&reg, "rust", src.as_bytes());
    let top = top_level_fold_targets(&bundle);
    assert_eq!(
        top.len(),
        2,
        "two top-level fns, the nested `if` is not auto-folded"
    );
    // Neither top-level range is the inner `if` block.
    let inner = fold_target_at(&bundle, byte_of(src, "work()")).expect("inner foldable");
    assert!(
        !top.contains(&inner),
        "close-all does not fold the nested region"
    );
}

#[test]
fn nested_state_aware_ordering() {
    // 9 / R3-2: close walks innermost→outer, open walks outer→inner, and
    // toggle cycles close-inner → close-outer → open-all so every command
    // reaches the outer fold.
    let reg = SyntaxRegistry::new();
    let src = "fn outer() {\n    if cond {\n        work();\n        more();\n    }\n}\n";
    let bundle = parse(&reg, "rust", src.as_bytes());
    let p = byte_of(src, "work()");

    let mut store = FoldStore::new();
    let inner = close_at(&mut store, &bundle, p).expect("close inner");
    let outer = close_at(&mut store, &bundle, p).expect("close outer");
    assert!(
        inner.start > outer.start,
        "inner fold is more deeply nested"
    );
    assert!(
        close_at(&mut store, &bundle, p).is_none(),
        "nothing left to close"
    );
    assert_eq!(store.folds().len(), 2);

    assert_eq!(open_at(&mut store, p), Some(outer), "open outermost first");
    assert_eq!(open_at(&mut store, p), Some(inner), "then the inner");
    assert!(store.is_empty());

    assert!(matches!(
        cycle_at(&mut store, &bundle, p),
        CycleOutcome::Closed(_)
    ));
    assert!(matches!(
        cycle_at(&mut store, &bundle, p),
        CycleOutcome::Closed(_)
    ));
    assert_eq!(store.folds().len(), 2, "cycle closed both");
    assert!(matches!(
        cycle_at(&mut store, &bundle, p),
        CycleOutcome::OpenedAll(2)
    ));
    assert!(store.is_empty(), "one more cycle opened them all");
}

// ---------------------------------------------------------------------------
// 10. Injected layer (a fenced rust block inside markdown).
// ---------------------------------------------------------------------------

#[test]
fn fold_sourced_inside_injected_layer() {
    let reg = SyntaxRegistry::new();
    let src = "# Title\n\n```rust\nfn demo() {\n    let x = 1;\n    let y = 2;\n}\n```\n\nText.\n";
    let bundle = parse(&reg, "markdown", src.as_bytes());
    assert!(
        bundle.layers.len() >= 2,
        "markdown fence produced an injected rust layer"
    );
    let r = fold_target_at(&bundle, byte_of(src, "let x")).expect("foldable inside the fence");
    assert_eq!(
        line_text_at(src, r.start),
        "fn demo() {",
        "resolved the inner block"
    );
}

// ---------------------------------------------------------------------------
// 2. Stale / absent parse tree refuses (the precondition the binding keys on).
// ---------------------------------------------------------------------------

#[test]
fn absent_tree_has_no_current_bundle() {
    let reg = SyntaxRegistry::new();
    let language = reg.language("rust").expect("grammar");
    let src = b"fn foo() {\n    let x = 1;\n}\n";
    let buf = Buffer::from_bytes(BufferId::next(), "doc", src);
    let view = ParseView::new(&buf, language, "rust".to_owned());
    let handle = view.handle();
    // Before any parse installs, `current()` is None → the binding refuses.
    assert!(handle.current().is_none());
    let mut req = handle.make_request();
    req.injection_aliases = reg.injection_alias_snapshot();
    let bundle = run_parse(req).expect("parse");
    handle.install(reg.resolve_layer_queries(&bundle));
    assert!(
        handle.current().is_some(),
        "after install, a target is derivable"
    );
}

// ---------------------------------------------------------------------------
// 6. Command-path pre-edit unfold (Q#FD5).
// ---------------------------------------------------------------------------

fn active_id(s: &EditorState) -> BufferId {
    s.core.borrow().active_buffer_id()
}

fn insert_into(s: &EditorState, id: BufferId, text: &str) {
    let core = s.core.borrow();
    let mut reg = core.registry.borrow_mut();
    reg.get_mut(id)
        .unwrap()
        .apply_edit(EditOp::Insert {
            pos: 0,
            bytes: text.as_bytes(),
        })
        .unwrap();
}

#[test]
fn command_path_self_insert_unfolds_at_point() {
    let s = EditorState::new();
    let id = active_id(&s);
    insert_into(&s, id, "line0\nline1\nline2\nline3\n");
    // Fold the interior of lines 1..2: [end of line0, end of line2].
    let store = {
        let core = s.core.borrow();
        let mut reg = core.registry.borrow_mut();
        s.fold_registry.store_or_attach(reg.get_mut(id).unwrap())
    };
    store
        .lock()
        .unwrap()
        .insert(ByteRange { start: 5, end: 17 });
    // Cursor strictly inside the fold (start of "line2").
    s.core.borrow_mut().set_cursor_byte(12);

    // A command-path self-insert unfolds before the edit lands.
    s.core.borrow_mut().insert_char('x');
    assert!(
        store.lock().unwrap().is_empty(),
        "typing inside a fold unfolds it (Q#FD5)"
    );
}

#[test]
fn self_insert_at_head_line_end_does_not_unfold() {
    // `(start, end]` containment: a self-insert exactly at the end of the
    // head line (== range.start) is outside the fold — it must NOT unfold,
    // and the translator shifts the fold right so the char lands visible.
    let s = EditorState::new();
    let id = active_id(&s);
    insert_into(&s, id, "line0\nline1\nline2\nline3\n");
    let store = {
        let core = s.core.borrow();
        let mut reg = core.registry.borrow_mut();
        s.fold_registry.store_or_attach(reg.get_mut(id).unwrap())
    };
    store
        .lock()
        .unwrap()
        .insert(ByteRange { start: 5, end: 17 });
    s.core.borrow_mut().set_cursor_byte(5); // end of head line "line0"

    s.core.borrow_mut().insert_char('x');
    let folds = store.lock().unwrap().folds();
    assert_eq!(
        folds,
        vec![ByteRange { start: 6, end: 18 }],
        "fold shifts right; the character lands on the head line"
    );
}

// ---------------------------------------------------------------------------
// 5. Point moves to the head line when a fold is created around it (Q#FD3).
// 11. Data-API validation (Q#FD11) — driven through the Lua surface.
// ---------------------------------------------------------------------------

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Install a settled rust parse over the active scratch buffer so the
/// `pmacs.fold` surface can drive it end to end.
fn install_rust_parse(s: &EditorState, id: BufferId) {
    let reg = &s.syntax_registry;
    let language = reg.language("rust").expect("grammar");
    let handle = {
        let core = s.core.borrow();
        let mut breg = core.registry.borrow_mut();
        let buf = breg.get_mut(id).unwrap();
        let view = ParseView::new(buf, language, "rust".to_owned());
        let handle = view.handle();
        buf.attach_view(Box::new(view));
        handle
    };
    let mut req = handle.make_request();
    req.injection_aliases = reg.injection_alias_snapshot();
    let bundle = run_parse(req).expect("parse");
    handle.install(reg.resolve_layer_queries(&bundle));
    reg.attach_view(id, handle);
}

#[test]
fn folding_moves_point_to_head_line() {
    let s = EditorState::new();
    let id = active_id(&s);
    let src = "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n";
    insert_into(&s, id, src);
    install_rust_parse(&s, id);
    // Cursor inside the body; `let x` starts on line 1.
    let p = byte_of(src, "let x");
    s.core.borrow_mut().set_cursor_byte(p);

    exec(&s, "pmacs.command.invoke('fold.close')");

    let head = line_content_end_of(src, "fn foo() {");
    assert_eq!(
        s.core.borrow().active_window().cursor,
        head,
        "the invoking point moved to the head line"
    );
    // And the fold exists.
    let n: i64 = eval(&s, "return #pmacs.fold.folds(pmacs.window.buffer())");
    assert_eq!(n, 1);
}

#[test]
fn data_api_validation() {
    let s = EditorState::new();
    // A plain document buffer with four lines.
    exec(
        &s,
        "b = pmacs.buffer.from_bytes('doc.rs', 'aaa\\nbbb\\nccc\\nddd\\n')",
    );

    // A valid multi-line range folds.
    let ok: bool = eval(&s, "return pmacs.fold.fold(b, { start = 3, ['end'] = 11 })");
    assert!(ok, "a >=1-hidden-line range is accepted");
    let n: i64 = eval(&s, "return #pmacs.fold.folds(b)");
    assert_eq!(n, 1);

    // A sub-one-line range (both endpoints on the same line) is rejected.
    let same_line: bool = eval(&s, "return pmacs.fold.fold(b, { start = 0, ['end'] = 2 })");
    assert!(
        !same_line,
        "a range hiding no full line is rejected (Q#FD11)"
    );

    // An out-of-bounds range is rejected.
    let oob: bool = eval(
        &s,
        "return pmacs.fold.fold(b, { start = 0, ['end'] = 99999 })",
    );
    assert!(!oob, "an out-of-bounds range is rejected");

    // Q#FD9 via the >=1-hidden-line rule: a fold at (0,0) on an empty
    // buffer normalizes to zero hidden lines and is rejected.
    exec(&s, "e = pmacs.buffer.from_bytes('empty.rs', '')");
    let empty: bool = eval(&s, "return pmacs.fold.fold(e, { start = 0, ['end'] = 0 })");
    assert!(
        !empty,
        "a zero-length range is rejected (terminals never fold)"
    );

    // Round-trip: unfold the stored range clears it.
    let unfolded: bool = eval(
        &s,
        "local f = pmacs.fold.folds(b)[1]; return pmacs.fold.unfold(b, f)",
    );
    assert!(unfolded);
    let n2: i64 = eval(&s, "return #pmacs.fold.folds(b)");
    assert_eq!(n2, 0);
}

// ---------------------------------------------------------------------------
// 8. Buffer content replacement drops the store.
// ---------------------------------------------------------------------------

#[test]
fn forget_drops_store_and_detaches_view() {
    let s = EditorState::new();
    let id = active_id(&s);
    insert_into(&s, id, "line0\nline1\nline2\n");
    let store = {
        let core = s.core.borrow();
        let mut reg = core.registry.borrow_mut();
        s.fold_registry.store_or_attach(reg.get_mut(id).unwrap())
    };
    store
        .lock()
        .unwrap()
        .insert(ByteRange { start: 5, end: 11 });
    assert!(s.fold_registry.store(id).is_some());

    // Content replacement (revert/reload) drops the store.
    {
        let core = s.core.borrow();
        let mut reg = core.registry.borrow_mut();
        s.fold_registry.forget(reg.get_mut(id).unwrap());
    }
    assert!(s.fold_registry.store(id).is_none(), "the store is dropped");
    assert!(fold::make_shared_fold_registry().folds(id).is_empty());
}
