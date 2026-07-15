// gpu_font_acceptance.rs --- gpu-set-font Arc 4 stage 2 acceptance
// (docs/gpu-set-font-framing.md, acceptance items 2–7; item 1's pins
// live in src/protocol.rs, item 8's TUI drop arm is a unit in
// src/frontend.rs, items 9–14 and 16–19 are GPU routes in pmacs-gpu's
// headless suite, and item 15 is the docs themselves).

//! The `pmacs.gpu.set_font` preference + the `FontFacts` wire channel
//! (protocol v17).
//!
//! Wire claims drive a `SemanticRenderState` frame by frame (the
//! `ThemeFacts` discipline: authoritative per attachment, epoch-gated,
//! silent when unchanged); the Lua contract is exercised against the
//! real `pmacs.gpu` module installed by `EditorState::new`; the
//! version gate exercises a real daemon; init.lua reachability goes
//! through the real `load_user_config_at`.

use pmacs::editor::EditorState;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;

#[cfg(feature = "crdt")]
mod common;

// ---------------------------------------------------------------------------
// Harness (theme_faces_acceptance conventions)
// ---------------------------------------------------------------------------

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn exec_err(s: &EditorState, src: &str) -> mlua::Error {
    s.lua_host
        .lua()
        .load(src.to_string())
        .exec()
        .expect_err("chunk must error")
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Fresh editor with LSP spawning disabled.
fn editor() -> EditorState {
    let s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn active_buffer(state: &EditorState) -> pmacs::buffer::BufferId {
    state.core.borrow().active_window().buffer_id
}

fn semantic(state: &EditorState) -> SemanticRenderState {
    let buffer_id = active_buffer(state);
    let mut s = SemanticRenderState::new(FrontendId::LOCAL);
    s.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    s
}

fn font_facts_of(msgs: &[InstanceMessage]) -> Option<(Option<String>, Option<u32>)> {
    msgs.iter().find_map(|m| match m {
        InstanceMessage::FontFacts {
            family,
            size_centi_px,
        } => Some((family.clone(), *size_centi_px)),
        _ => None,
    })
}

fn font_facts_count(msgs: &[InstanceMessage]) -> usize {
    msgs.iter()
        .filter(|m| matches!(m, InstanceMessage::FontFacts { .. }))
        .count()
}

/// The daemon-side preference triple `(family, size_centi_px, epoch)`.
fn pref_of(state: &EditorState) -> (Option<String>, Option<u32>, u64) {
    let pref = state.font_pref.lock().expect("font pref lock");
    (pref.family.clone(), pref.size_centi_px, pref.epoch)
}

// ---------------------------------------------------------------------------
// 2 — authoritative default per attachment; unchanged ticks silent;
//     late joiner receives the current preference
// ---------------------------------------------------------------------------

#[test]
fn first_frame_ships_the_authoritative_default_and_then_stays_silent() {
    let state = editor();
    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    assert_eq!(
        font_facts_of(&first),
        Some((None, None)),
        "a fresh attachment's first frame carries the REAL (None, None) default — \
         never inferred from silence"
    );
    for tick in 0..3 {
        let next = sem.render_frame(&state);
        assert_eq!(
            font_facts_count(&next),
            0,
            "unchanged tick {tick} must be FontFacts-silent"
        );
    }
    // `state` untouched since the set below — a late-joining second
    // session receives the current preference without any mutation
    // post-attach.
    exec(
        &state,
        r#"pmacs.gpu.set_font { family = "Iosevka", size = 18 }"#,
    );
    let mut late = semantic(&state);
    let joined = late.render_frame(&state);
    assert_eq!(
        font_facts_of(&joined),
        Some((Some("Iosevka".to_owned()), Some(1800))),
        "a late joiner's first frame carries the current preference"
    );
}

// ---------------------------------------------------------------------------
// 3 — live re-ship: one FontFacts on the next frame; an identical
//     re-set advances the epoch without emitting
// ---------------------------------------------------------------------------

#[test]
fn set_font_reships_once_and_identical_resets_advance_the_epoch_silently() {
    let state = editor();
    let mut sem = semantic(&state);
    let _ = sem.render_frame(&state);
    exec(&state, "pmacs.gpu.set_font { size = 18 }");
    let next = sem.render_frame(&state);
    assert_eq!(
        font_facts_count(&next),
        1,
        "a mid-session set emits exactly one FontFacts on the next frame"
    );
    assert_eq!(font_facts_of(&next), Some((None, Some(1800))));
    let (_, _, epoch_before) = pref_of(&state);
    exec(&state, "pmacs.gpu.set_font { size = 18 }");
    let (_, _, epoch_after) = pref_of(&state);
    assert_eq!(
        epoch_after,
        epoch_before + 1,
        "an identical re-set still advances the epoch (caches advance on \
         computation, Q#TH6)"
    );
    let silent = sem.render_frame(&state);
    assert_eq!(
        font_facts_count(&silent),
        0,
        "an identical payload does not re-emit"
    );
}

// ---------------------------------------------------------------------------
// 4 — snapshot survival, producer side: the buffer-baseline reset
//     re-ships buffer facts but never the bufferless FontFacts
// ---------------------------------------------------------------------------

#[test]
fn buffer_snapshot_reset_never_reships_font_facts() {
    let state = editor();
    exec(&state, "pmacs.gpu.set_font { size = 20 }");
    let buffer_id = active_buffer(&state);
    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    assert_eq!(
        font_facts_of(&first),
        Some((None, Some(2000))),
        "precondition: the preference shipped on the first frame"
    );
    let has_status = |msgs: &[InstanceMessage]| {
        msgs.iter()
            .any(|m| matches!(m, InstanceMessage::StatusFacts { .. }))
    };
    assert!(has_status(&first), "precondition: buffer facts shipped too");
    // The daemon wrote a BufferSnapshot for this buffer (an A → B → A
    // revisit): every BUFFER-scoped baseline resets…
    sem.on_buffer_snapshot_sent(buffer_id);
    let after = sem.render_frame(&state);
    assert!(
        has_status(&after),
        "the reset re-ships the buffer's facts on the next frame"
    );
    // …but the global font baseline survives: the new buffer shapes
    // under the same preference without a redundant global fact.
    assert_eq!(
        font_facts_count(&after),
        0,
        "FontFacts is bufferless — the snapshot reset must not touch it"
    );
}

// ---------------------------------------------------------------------------
// 5 — the daemon version gate (v16 peer never receives FontFacts)
// ---------------------------------------------------------------------------

#[cfg(feature = "crdt")]
#[test]
fn v16_peer_never_receives_font_facts_and_v17_does() {
    use common::daemon::{TestDaemon, build_default_caps};
    use pmacs::cell::CellSize;
    use pmacs::protocol::{AttachRequest, FrontendCapabilities, FrontendEvent, Hello};
    use pmacs::transport::{read_message, write_message};
    use std::time::{Duration, Instant};

    fn semantic_caps() -> FrontendCapabilities {
        FrontendCapabilities {
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
            ..build_default_caps()
        }
    }

    /// Attach a semantic session at `version`, declare a viewport,
    /// and report `(saw_font_facts, saw_style_spans)` within the
    /// deadline.
    fn probe(daemon: &TestDaemon, version: u32) -> (bool, bool) {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let hello: Hello = read_message(&mut stream).expect("read Hello");
        let fid = hello.assigned_frontend_id;
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: version,
                frontend_capabilities: semantic_caps(),
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("write AttachRequest");
        let mut buf = None;
        let learn_by = Instant::now() + Duration::from_secs(2);
        while Instant::now() < learn_by && buf.is_none() {
            if let Ok(InstanceMessage::BufferSnapshot { buffer_id, .. }) =
                read_message::<InstanceMessage>(&mut stream)
            {
                buf = Some(buffer_id);
            }
        }
        let buffer_id = buf.expect("received a BufferSnapshot");
        write_message(
            &mut stream,
            &FrontendEvent::Viewport {
                frontend_id: fid,
                buffer_id,
                visible: ByteRange {
                    start: 0,
                    end: 4096,
                },
                generation: 0,
            },
        )
        .expect("write Viewport");
        let deadline = Instant::now() + Duration::from_secs(3);
        let (mut saw_facts, mut saw_spans) = (false, false);
        while Instant::now() < deadline && !(saw_facts && saw_spans) {
            match read_message::<InstanceMessage>(&mut stream) {
                Ok(InstanceMessage::FontFacts { .. }) => saw_facts = true,
                Ok(InstanceMessage::StyleSpans { .. }) => saw_spans = true,
                Ok(_) | Err(_) => {}
            }
        }
        (saw_facts, saw_spans)
    }

    let daemon = TestDaemon::spawn();
    let (v17_facts, v17_spans) = probe(&daemon, 17);
    assert!(v17_spans, "a v17 semantic session receives StyleSpans");
    assert!(
        v17_facts,
        "a v17 semantic session receives the authoritative FontFacts"
    );
    let (v16_facts, v16_spans) = probe(&daemon, 16);
    assert!(v16_spans, "a v16 peer still receives StyleSpans");
    assert!(
        !v16_facts,
        "the daemon skip arm must keep FontFacts off a v16 wire"
    );
}

// ---------------------------------------------------------------------------
// 6 — the Lua contract: strict plain data, all-or-nothing
// ---------------------------------------------------------------------------

#[test]
fn set_font_rejects_bad_sizes_naming_the_field_and_nothing_lands() {
    let state = editor();
    let mut sem = semantic(&state);
    let _ = sem.render_frame(&state);
    let before = pref_of(&state);
    for bad in [
        "pmacs.gpu.set_font { size = 5.999 }", // must error, not round into range
        "pmacs.gpu.set_font { size = 72.01 }",
        "pmacs.gpu.set_font { size = 0 }",
        "pmacs.gpu.set_font { size = -16 }",
        "pmacs.gpu.set_font { size = 0/0 }",  // NaN
        "pmacs.gpu.set_font { size = 1/0 }",  // +inf
        "pmacs.gpu.set_font { size = '16' }", // non-number
    ] {
        let err = exec_err(&state, bad);
        assert!(
            err.to_string().contains("`size`"),
            "{bad}: the error names the offending field, got: {err}"
        );
    }
    assert_eq!(pref_of(&state), before, "no failed set may land");
    let silent = sem.render_frame(&state);
    assert_eq!(
        font_facts_count(&silent),
        0,
        "no failed set may emit on the wire"
    );
}

#[test]
fn set_font_rejects_bad_families_and_unknown_keys_by_name() {
    let state = editor();
    let err = exec_err(&state, "pmacs.gpu.set_font { family = '' }");
    assert!(
        err.to_string().contains("`family`"),
        "empty family names the field: {err}"
    );
    let err = exec_err(&state, "pmacs.gpu.set_font { family = 12 }");
    assert!(
        err.to_string().contains("`family`"),
        "non-string family names the field: {err}"
    );
    let err = exec_err(&state, "pmacs.gpu.set_font { size = 18, sise = 20 }");
    assert!(
        err.to_string().contains("`sise`"),
        "an unknown key is rejected by NAME: {err}"
    );
    assert_eq!(
        pref_of(&state),
        (None, None, 0),
        "every rejected shape leaves the preference untouched"
    );
}

#[test]
fn set_font_never_consults_metatables() {
    let state = editor();
    // A hostile `__index` that answers every lookup: raw reads must
    // never see its values, and raw iteration must never invoke it.
    exec(
        &state,
        r#"
        _G.__mt_hits = 0
        local spec = setmetatable({ size = 18 }, {
            __index = function(_, _)
                _G.__mt_hits = _G.__mt_hits + 1
                return "Injected Family"
            end,
            __pairs = function()
                _G.__mt_hits = _G.__mt_hits + 1
                return function() return nil end
            end,
        })
        pmacs.gpu.set_font(spec)
        "#,
    );
    let hits: i64 = eval(&state, "return _G.__mt_hits");
    assert_eq!(hits, 0, "metatables are never invoked");
    assert_eq!(
        pref_of(&state),
        (None, Some(1800), 1),
        "the metatable's `family` answer must NOT be injected — only the \
         raw `size` landed"
    );
}

#[test]
fn set_font_quantizes_both_sides_of_a_hundredth_and_empty_resets() {
    let state = editor();
    exec(&state, "pmacs.gpu.set_font { size = 15.994 }");
    assert_eq!(pref_of(&state).1, Some(1599), "15.994 rounds DOWN");
    exec(&state, "pmacs.gpu.set_font { size = 15.996 }");
    assert_eq!(pref_of(&state).1, Some(1600), "15.996 rounds UP");
    // Boundary values are in range and quantize exactly.
    exec(&state, "pmacs.gpu.set_font { size = 6 }");
    assert_eq!(pref_of(&state).1, Some(600));
    exec(&state, "pmacs.gpu.set_font { size = 72 }");
    assert_eq!(pref_of(&state).1, Some(7200));
    exec(
        &state,
        r#"pmacs.gpu.set_font { family = "Iosevka", size = 18 }"#,
    );
    exec(&state, "pmacs.gpu.set_font {}");
    let (family, size, _) = pref_of(&state);
    assert_eq!(
        (family, size),
        (None, None),
        "set_font {{}} resets BOTH axes to the frontend default"
    );
}

#[test]
fn font_getter_returns_a_fresh_quantized_plain_table() {
    let state = editor();
    exec(&state, "pmacs.gpu.set_font { size = 15.996 }");
    let (size, fresh, no_family): (f64, bool, bool) = eval(
        &state,
        r"
        local a = pmacs.gpu.font()
        local b = pmacs.gpu.font()
        a.size = 999  -- scribbling on the returned table…
        local c = pmacs.gpu.font()
        return c.size, rawequal(a, b) == false, c.family == nil
        ",
    );
    assert!(
        (size - 16.0).abs() < f64::EPSILON,
        "the getter reports the QUANTIZED value (1600 → 16.0), got {size}"
    );
    assert!(
        fresh,
        "each call returns a fresh table, never a stored handle"
    );
    assert!(
        no_family,
        "an unset axis is absent, and scribbles don't stick"
    );
}

// ---------------------------------------------------------------------------
// 7 — init.lua reachability: the module installs BEFORE user config
// ---------------------------------------------------------------------------

#[test]
fn init_lua_set_font_lands_in_the_preference_the_first_frame_reads() {
    use pmacs::config::load_user_config_at;
    use pmacs::lua::LuaHost;

    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.gpu.set_font { family = "Iosevka", size = 18 }"#,
    )
    .expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");
    // Mirror `EditorState::new`'s ordering: the pmacs.gpu module
    // installs BEFORE user config runs (src/editor.rs), so an
    // init.lua set_font lands in the same state the first
    // attachment's producer reads.
    let handle = pmacs::lua_bindings::make_font_pref(host.lua()).expect("install pmacs.gpu");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();
    assert!(
        host.errors().is_empty(),
        "init.lua produced errors: {:?}",
        host.errors()
    );
    let pref = handle.lock().expect("font pref lock");
    assert_eq!(pref.family.as_deref(), Some("Iosevka"));
    assert_eq!(pref.size_centi_px, Some(1800));
    assert_eq!(pref.epoch, 1, "exactly the init.lua set landed");
}

/// The producer half of item 7: a preference already in place before
/// the first attachment (the init.lua timing) ships on that
/// attachment's FIRST frame.
#[test]
fn preference_set_before_attach_ships_on_the_first_frame() {
    let state = editor();
    exec(
        &state,
        r#"pmacs.gpu.set_font { family = "Iosevka", size = 18 }"#,
    );
    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    assert_eq!(
        font_facts_of(&first),
        Some((Some("Iosevka".to_owned()), Some(1800))),
        "the first frame ships the pre-attach preference"
    );
}
