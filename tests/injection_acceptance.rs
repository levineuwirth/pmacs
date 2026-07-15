// injection_acceptance.rs --- multi-language injection acceptance gates.

//! Multi-language injection acceptance gates that need the public API
//! surface (the Lua async path and a settle-time perf budget). The
//! layer-structure, alias-resolution, producer, and GPU gates live next to
//! the code they exercise (`syntax.rs` / `semantic_render.rs` /
//! `highlight.rs` / `pmacs-gpu`), where `run_parse`, `scoped_style_spans`,
//! and `source_color_at` are reachable. See the framing acceptance list in
//! `docs/multi-language-injections-framing.md`.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use pmacs::buffer::{Buffer, BufferId, EditOp};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::BufferIdLua;
use pmacs::syntax::{self, ParseView, SyntaxRegistry};

/// Drive `tick_async` until `predicate` holds or a deadline passes.
fn pump_async<F: Fn(&EditorState) -> bool>(state: &mut EditorState, predicate: F) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate(state) {
        assert!(Instant::now() < deadline, "async pump deadline exceeded");
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Framing acceptance #5 (full Lua async path): an alias added from Lua via
/// `pmacs.parse.injection_aliases` must reach the parse worker through the
/// dispatch snapshot, so an *asynchronously* dispatched injected parse
/// resolves a fence named by the new alias. Testing only the static
/// resolver would leave the Lua write-through + snapshot bridge unproven.
#[test]
fn lua_alias_override_resolves_on_async_parse() {
    let mut state = EditorState::new();

    // Add a bespoke fence alias from Lua (write-through to the registry).
    state
        .lua_host
        .lua()
        .load(r#"pmacs.parse.injection_aliases.mydsl = "rust""#)
        .exec()
        .expect("set injection alias from Lua");

    // A markdown buffer whose fence uses the new alias.
    let src = b"# Doc\n\n```mydsl\nfn injected() { let x = 1; }\n```\n";
    let buf_id = state
        .lua_host
        .registry()
        .borrow_mut()
        .create_from_bytes("doc.md".to_owned(), src);
    state
        .lua_host
        .lua()
        .globals()
        .set("BUF", BufferIdLua(buf_id))
        .expect("bind BUF");

    // Dispatch asynchronously (the wrapped `_dispatch` records the job; the
    // per-tick settle path installs the resolved bundle).
    state
        .lua_host
        .lua()
        .load("pmacs.parse._dispatch(BUF, 'markdown')")
        .exec()
        .expect("async dispatch");

    pump_async(&mut state, |s| {
        s.syntax_registry
            .view(buf_id)
            .and_then(|h| h.current())
            .is_some()
    });

    let bundle = state
        .syntax_registry
        .view(buf_id)
        .and_then(|h| h.current())
        .expect("settled bundle");
    assert!(
        bundle.layers.iter().any(|l| l.language_name == "rust"),
        "the Lua-set `mydsl` alias resolved the fence to a rust child layer; \
         layers: {:?}",
        bundle
            .layers
            .iter()
            .map(|l| l.language_name.as_str())
            .collect::<Vec<_>>()
    );
}

/// Framing acceptance #12: a large all-inline markdown buffer settles
/// (root + one cold inline layer per paragraph) within a comfortable
/// budget, and the FINAL paragraph still receives an inline layer — the
/// tail is not silently dropped. This is the measured guard that keeps
/// child-incrementality (Q#IJ8) out of v1.
#[test]
fn many_paragraph_settle_under_budget_with_tail_covered() {
    let reg = SyntaxRegistry::new();
    let n = 200usize;
    let mut src = String::new();
    for i in 0..n {
        // A blank line separates paragraphs; each carries emphasis + a link
        // so the block grammar injects a markdown_inline layer for it.
        writeln!(
            src,
            "Paragraph {i} with *emphasis* and a [link](http://x).\n"
        )
        .expect("write");
    }
    // A distinctly-marked final paragraph.
    let marker = "FINALPARAGRAPH";
    writeln!(src, "{marker} with *stress*.\n").expect("write");

    let language = reg.language("markdown").expect("markdown grammar");
    let mut buf = Buffer::new(BufferId::next(), "big.md");
    buf.apply_edit(EditOp::Insert {
        pos: 0,
        bytes: src.as_bytes(),
    })
    .expect("seed markdown");
    let view = ParseView::new(&buf, language, "markdown".to_owned());
    let handle = view.handle();
    let _vid = buf.attach_view(Box::new(view));
    let mut req = handle.make_request();
    req.injection_aliases = reg.injection_alias_snapshot();

    let start = Instant::now();
    let bundle = syntax::run_parse(req).expect("layered markdown parse");
    let elapsed = start.elapsed();

    // Catastrophic-regression guard: cold-parsing ~200 tiny inline layers
    // is milliseconds of work; a generous ceiling avoids debug/CI flakiness
    // while still catching a blow-up (e.g. accidental O(n^2) layer work).
    assert!(
        elapsed < Duration::from_secs(2),
        "many-paragraph settle took {elapsed:?}, exceeds the budget"
    );

    let inline_count = bundle
        .layers
        .iter()
        .filter(|l| l.language_name == "markdown_inline")
        .count();
    assert!(
        inline_count >= 100,
        "most paragraphs produce an inline layer; got {inline_count}"
    );

    // Tail coverage: an inline layer's tree spans the final paragraph.
    let marker_off = src.find(marker).expect("marker present");
    let tail_covered = bundle
        .layers
        .iter()
        .filter(|l| l.language_name == "markdown_inline")
        .any(|l| {
            let r = l.tree.root_node();
            (r.start_byte()..r.end_byte()).contains(&marker_off)
        });
    assert!(
        tail_covered,
        "the final paragraph still receives an inline layer (no tail loss)"
    );
}
