// injection_acceptance.rs --- multi-language injection acceptance gates.

//! Multi-language injection acceptance gates that need the public API
//! surface (the Lua async path and a settle-time perf budget). The
//! layer-structure, alias-resolution, producer, and GPU gates live next to
//! the code they exercise (`syntax.rs` / `semantic_render.rs` /
//! `highlight.rs` / `pmacs-gpu`), where `run_parse`, `scoped_style_spans`,
//! and `source_color_at` are reachable. See the framing acceptance list in
//! `docs/multi-language-injections-framing.md`.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pmacs::buffer::{Buffer, BufferId, EditOp};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::BufferIdLua;
use pmacs::rope::Range;
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
    let mut state = EditorState::new_with_roots(&crate::iso::roots());

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

/// Round-2 finding: the synchronous parse path (`_parse_now`) must snapshot
/// injection aliases too — otherwise a `py` fence (or a Lua-added alias)
/// injects asynchronously but not synchronously. The default `py`→python
/// alias discriminates the fix: with the empty map it would not resolve.
#[test]
fn sync_parse_now_resolves_alias() {
    let state = EditorState::new_with_roots(&crate::iso::roots());
    let src = b"# Doc\n\n```py\nx = 1\n```\n";
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
    state
        .lua_host
        .lua()
        .load("pmacs.parse._parse_now(BUF, 'markdown')")
        .exec()
        .expect("synchronous parse");

    let bundle = state
        .syntax_registry
        .view(buf_id)
        .and_then(|h| h.current())
        .expect("installed bundle");
    assert!(
        bundle.layers.iter().any(|l| l.language_name == "python"),
        "the `py` fence resolved to python on the synchronous `_parse_now` path"
    );
}

/// Round-2 finding: the injection layer cap must be *observably* surfaced,
/// not merely flagged. Drives the real Lua settle path (`syntax.lua`'s tick
/// → `_injection_capped` → `pmacs.error`) and asserts the three behaviors:
/// surfaced once, suppressed on an unchanged re-parse, and re-armed after
/// the file drops below the cap and exceeds it again.
#[test]
fn injection_cap_surfaced_once_and_rearms_via_lua() {
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    // Capture pmacs.error messages into a Lua global.
    state
        .lua_host
        .lua()
        .load("_CAP = {}\npmacs.error = function(msg) _CAP[#_CAP + 1] = tostring(msg) end")
        .exec()
        .expect("install error capture");

    let capping: String = "```rust\nx\n```\n\n".repeat(4096 + 8);
    let buf_id = state
        .lua_host
        .registry()
        .borrow_mut()
        .create_from_bytes("big.md".to_owned(), capping.as_bytes());
    state
        .lua_host
        .lua()
        .globals()
        .set("BUF", BufferIdLua(buf_id))
        .expect("bind BUF");

    let dispatch = |state: &EditorState| {
        state
            .lua_host
            .lua()
            .load("pmacs.parse._dispatch(BUF, 'markdown')")
            .exec()
            .expect("dispatch");
    };
    let cap_count = |state: &EditorState| -> usize {
        state.lua_host.lua().load("return #_CAP").eval().unwrap()
    };
    let current =
        |state: &EditorState| state.syntax_registry.view(buf_id).and_then(|h| h.current());
    let replace_all = |state: &EditorState, bytes: &[u8]| {
        let core = state.core.borrow();
        let mut reg = core.registry.borrow_mut();
        let buf = reg.get_mut(buf_id).expect("buffer");
        let len = buf.len();
        buf.apply_edit(EditOp::Replace {
            range: Range::new(0, len),
            bytes,
        })
        .expect("replace");
    };

    // 1) First settle → surfaced exactly once, message names the cap.
    dispatch(&state);
    pump_async(&mut state, |s| current(s).is_some());
    assert_eq!(cap_count(&state), 1, "cap surfaced once on first settle");
    let msg: String = state.lua_host.lua().load("return _CAP[1]").eval().unwrap();
    assert!(
        msg.contains("injection layer cap"),
        "message names the cap: {msg}"
    );

    // 2) Re-dispatch with no change → suppressed (still once).
    let b1 = current(&state).unwrap();
    dispatch(&state);
    pump_async(&mut state, |s| {
        current(s).is_some_and(|b| !Arc::ptr_eq(&b1, &b))
    });
    assert_eq!(
        cap_count(&state),
        1,
        "once-per-buffer: no re-warn without change"
    );

    // 3) Shrink below the cap → warned flag clears, no new error.
    replace_all(&state, b"# small\n\n```rust\nx\n```\n");
    let b2 = current(&state).unwrap();
    dispatch(&state);
    pump_async(&mut state, |s| {
        current(s).is_some_and(|b| !Arc::ptr_eq(&b2, &b))
    });
    assert_eq!(
        cap_count(&state),
        1,
        "dropping below the cap surfaces no new error"
    );

    // 4) Grow back above the cap → re-armed, warns once more.
    replace_all(&state, capping.as_bytes());
    let b3 = current(&state).unwrap();
    dispatch(&state);
    pump_async(&mut state, |s| {
        current(s).is_some_and(|b| !Arc::ptr_eq(&b3, &b))
    });
    assert_eq!(
        cap_count(&state),
        2,
        "re-armed: exceeding the cap again warns once more"
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

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
