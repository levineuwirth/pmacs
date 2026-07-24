//! Themes Arc 4 stage 3 acceptance
//! (`docs/statusline-segments-framing.md` Revision 3, Acceptance 1-27).
//!
//! Cross-surface matrix:
//! 1-11: this suite drives the real Lua registry/evaluator and full TUI frame;
//! terminal grapheme/clipping bite details are additionally pinned by the six
//! `editor::tests::statusline_*` focused tests.
//! 12: this suite drives builtin `lsp.lua` against `pmacs_fake_lsp`; the complete
//! bounded label table is pinned by `lsp_status::tests::label_*`.
//! 13-21,26: this suite drives protocol values and real `SemanticRenderState`
//! frame/baseline paths; daemon write-gate coverage is CRDT-gated below.
//! 22-25: GPU-only pixel, wrap, precedence, atomic-wire-validation, and cache
//! bites live beside the private renderer in `pmacs-gpu`'s headless suite.
//! 27: code/API documentation is compile-checked here; project docs are owned
//! by the later handoff pass and deliberately excluded from this code pass.

use std::time::{Duration, Instant};

use pmacs::buffer::BufferId;
use pmacs::cell::{Cell, CellGrid, CellSize, Color, Glyph, Style};
use pmacs::editor::EditorState;
use pmacs::protocol::{
    ByteRange, FrontendId, InstanceMessage, MAX_STATUSLINE_FACE_BYTES,
    MAX_STATUSLINE_PROVIDER_NAME_BYTES, MAX_STATUSLINE_PROVIDERS, MAX_STATUSLINE_SEGMENT_BYTES,
    PROTOCOL_VERSION, StatuslineSegment, is_modeline_face_name, is_supported_protocol_version,
};
use pmacs::semantic_render::SemanticRenderState;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};

#[cfg(feature = "crdt")]
mod common;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn editor() -> EditorState {
    let state = EditorState::new();
    exec(&state, "pmacs.lsp.config = {}");
    state
}

fn active_buffer(state: &EditorState) -> BufferId {
    state.core.borrow().active_buffer_id()
}

fn paint(state: &EditorState, rows: u32, cols: u32) -> Vec<Cell> {
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    let _ = pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        CellSize::new(rows, cols),
    );
    cells
}

fn row_text(cells: &[Cell], cols: u32, row: u32) -> String {
    (0..cols)
        .map(
            |column| match &cells[(row * cols + column) as usize].glyph {
                Glyph::Char(ch) => *ch,
                Glyph::Cluster(bytes) => std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|text| text.chars().next())
                    .unwrap_or(' '),
                Glyph::Continuation => ' ',
            },
        )
        .collect()
}

fn semantic(state: &EditorState, version: u32) -> SemanticRenderState {
    let mut semantic = SemanticRenderState::for_peer(FrontendId::LOCAL, version);
    semantic.set_viewport(
        active_buffer(state),
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    semantic
}

fn segments_of(
    messages: &[InstanceMessage],
) -> Option<(BufferId, Vec<StatuslineSegment>, Vec<StatuslineSegment>)> {
    messages.iter().find_map(|message| match message {
        InstanceMessage::StatuslineSegments {
            buffer_id,
            left,
            right,
        } => Some((*buffer_id, left.clone(), right.clone())),
        _ => None,
    })
}

fn theme_faces_of(messages: &[InstanceMessage]) -> Option<Vec<pmacs::protocol::ThemeFace>> {
    messages.iter().find_map(|message| match message {
        InstanceMessage::ThemeFacts { faces } => Some(faces.clone()),
        _ => None,
    })
}

// Acceptance 1-4: default preservation, strict Lua contract, lifecycle/epochs,
// and callback return sanitation/validation.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one transactional registry scenario pins validation, epochs, introspection, and structural capacity without rebuilding expensive editor state"
)]
fn a01_04_registry_contract_limits_epochs_and_results() {
    let state = editor();
    let baseline = paint(&state, 24, 80);
    let baseline_mode = row_text(&baseline, 80, 22);
    assert!(baseline_mode.starts_with(" +  *scratch* "));
    assert!(baseline_mode.ends_with(" L1:C1 All "));

    let initial = state.statusline_registry.borrow().providers();
    assert_eq!(
        initial
            .iter()
            .map(|provider| provider.name.as_str())
            .collect::<Vec<_>>(),
        ["mode", "terminal", "lsp"],
        "built-in providers are discoverable in registration order"
    );
    let before_epochs = {
        let registry = state.statusline_registry.borrow();
        (registry.layout_epoch(), registry.face_set_epoch())
    };

    // Raw traversal/reads must not invoke either metamethod.
    exec(
        &state,
        r"
        _G.SL_META_TOUCHED = false
        local spec = setmetatable({
          name='strict', side='left', priority=7,
          face='ui.modeline.strict', fn=function() return 'ok' end,
        }, {
          __index=function() SL_META_TOUCHED=true; error('__index invoked') end,
          __pairs=function() SL_META_TOUCHED=true; error('__pairs invoked') end,
        })
        _G.SL_HANDLE = pmacs.statusline.register(spec)
        assert(not SL_META_TOUCHED)
        local providers = pmacs.statusline.providers()
        assert(providers[#providers].handle == SL_HANDLE)
        assert(providers[#providers].name == 'strict')
        assert(providers[#providers].side == 'left')
        assert(providers[#providers].priority == 7)
        assert(providers[#providers].face == 'ui.modeline.strict')
        assert(providers[#providers].enabled == true)
        ",
    );
    let after_register = {
        let registry = state.statusline_registry.borrow();
        (registry.layout_epoch(), registry.face_set_epoch())
    };
    assert_eq!(after_register, (before_epochs.0 + 1, before_epochs.1 + 1));

    let invalid = [
        "pmacs.statusline.register{name='x',side='left',fn=function()end,unknown=1}",
        "pmacs.statusline.register{name='',side='left',fn=function()end}",
        "pmacs.statusline.register{name='x',side='middle',fn=function()end}",
        "pmacs.statusline.register{name='x',side='left',priority=0.5,fn=function()end}",
        "pmacs.statusline.register{name='x',side='left',priority=2147483648,fn=function()end}",
        "pmacs.statusline.register{name='x',side='left',face='ui.statusline',fn=function()end}",
        "pmacs.statusline.register{name='x',side='left',fn=true}",
        "pmacs.statusline.set_priority(SL_HANDLE, '9')",
        "pmacs.statusline.set_enabled(SL_HANDLE, 1)",
    ];
    for source in invalid {
        let epochs = {
            let registry = state.statusline_registry.borrow();
            (
                registry.layout_epoch(),
                registry.face_set_epoch(),
                registry.len(),
            )
        };
        assert!(
            state.lua_host.lua().load(source).exec().is_err(),
            "accepted {source}"
        );
        let registry = state.statusline_registry.borrow();
        assert_eq!(
            (
                registry.layout_epoch(),
                registry.face_set_epoch(),
                registry.len()
            ),
            epochs,
            "failed validation partially mutated registry"
        );
    }

    let over_name = "n".repeat(MAX_STATUSLINE_PROVIDER_NAME_BYTES + 1);
    let over_face = format!("ui.modeline.{}", "f".repeat(MAX_STATUSLINE_FACE_BYTES + 1));
    state
        .lua_host
        .lua()
        .globals()
        .set("SL_OVER_NAME", over_name)
        .unwrap();
    state
        .lua_host
        .lua()
        .globals()
        .set("SL_OVER_FACE", over_face)
        .unwrap();
    for source in [
        "pmacs.statusline.register{name=SL_OVER_NAME,side='left',fn=function()end}",
        "pmacs.statusline.register{name='x',side='left',face=SL_OVER_FACE,fn=function()end}",
        "pmacs.statusline.register{name='bad\\nname',side='left',fn=function()end}",
    ] {
        assert!(state.lua_host.lua().load(source).exec().is_err());
    }

    // Fill every remaining structural slot; disabled entries still count.
    let to_add = MAX_STATUSLINE_PROVIDERS - state.statusline_registry.borrow().len();
    state
        .lua_host
        .lua()
        .globals()
        .set("SL_TO_ADD", i64::try_from(to_add).unwrap())
        .unwrap();
    exec(
        &state,
        r"
        for i=1,SL_TO_ADD do
          pmacs.statusline.register{name='cap-'..i,side='right',fn=function() return nil end}
        end
        ",
    );
    let epochs_at_cap = {
        let registry = state.statusline_registry.borrow();
        assert_eq!(registry.len(), MAX_STATUSLINE_PROVIDERS);
        (registry.layout_epoch(), registry.face_set_epoch())
    };
    assert!(
        state
            .lua_host
            .lua()
            .load("pmacs.statusline.register{name='65',side='left',fn=function()end}")
            .exec()
            .is_err()
    );
    assert_eq!(
        {
            let registry = state.statusline_registry.borrow();
            (registry.layout_epoch(), registry.face_set_epoch())
        },
        epochs_at_cap
    );

    // The focused evaluator suite additionally bites invalid UTF-8 and the
    // post-sanitation 1024-byte limit. Pin the public shared limits here.
    assert_eq!(MAX_STATUSLINE_PROVIDERS, 64);
    assert_eq!(MAX_STATUSLINE_SEGMENT_BYTES, 1024);
    assert!(is_modeline_face_name("ui.modeline.child"));
    assert!(!is_modeline_face_name("ui.statusline"));
}

// Acceptance 5-8: isolation, full-context latch, re-entrant mutation,
// context-change guard, per-window/per-frontend context.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one shared editor scenario pins latch continuity and every layout/context mutation without losing cross-step state"
)]
fn a05_08_evaluator_latches_reentrancy_contexts_and_mutation_guards() {
    let state = editor();
    exec(
        &state,
        r"
        _G.SL_FAIL = true
        _G.SL_FAIL_HANDLE = pmacs.statusline.register {
          name='failing', side='left', priority=5,
          fn=function() if SL_FAIL then error('boom') end return nil end,
        }
        _G.SL_GOOD_A = pmacs.statusline.register {
          name='good-a', side='left', priority=10, fn=function() return 'A' end,
        }
        _G.SL_GOOD_B = pmacs.statusline.register {
          name='good-b', side='left', priority=0, fn=function() return 'B' end,
        }
        ",
    );
    let target = StatuslineEvaluationTarget::Grid {
        frontend_id: FrontendId::LOCAL,
    };
    let first = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    assert_eq!(first.new_failures.len(), 1);
    let StatuslineEvaluationOutcome::Ready(windows) = first.outcome else {
        panic!("isolated provider failure invalidated unrelated output")
    };
    assert_eq!(
        windows[0]
            .left
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    let repeated = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    assert!(repeated.new_failures.is_empty());
    exec(&state, "SL_FAIL=false");
    let _ = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    exec(&state, "SL_FAIL=true");
    assert_eq!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .new_failures
        .len(),
        1,
        "a success must re-arm exactly that context"
    );
    state
        .statusline_registry
        .borrow_mut()
        .detach_frontend(FrontendId::LOCAL);
    assert_eq!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .new_failures
        .len(),
        1,
        "detach must release the frontend's latches"
    );
    exec(
        &state,
        "pmacs.statusline.set_enabled(SL_FAIL_HANDLE,false); pmacs.statusline.set_enabled(SL_FAIL_HANDLE,true)",
    );
    assert_eq!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .new_failures
        .len(),
        1,
        "disable/re-enable begins a new failure run"
    );

    // Two split contexts carry distinct ids/buffers and exactly one active flag.
    exec(
        &state,
        r"
        _G.SL_OTHER = pmacs.buffer.create('other')
        pmacs.window.split_vertical()
        pmacs.window.switch_buffer(SL_OTHER)
        _G.SL_SEEN = {}
        _G.SL_CTX = pmacs.statusline.register {
          name='contexts', side='right',
          fn=function(ctx)
            SL_SEEN[#SL_SEEN+1] = {window=ctx.window,buffer=tostring(ctx.buffer),active=ctx.active}
            return ctx.active and 'ACTIVE' or 'PASSIVE'
          end,
        }
        ",
    );
    let split = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    let StatuslineEvaluationOutcome::Ready(windows) = split.outcome else {
        panic!("split evaluation was invalid")
    };
    assert_eq!(windows.len(), 2);
    assert_ne!(windows[0].context.window_id, windows[1].context.window_id);
    assert_ne!(windows[0].context.buffer_id, windows[1].context.buffer_id);
    assert_ne!(windows[0].context.active, windows[1].context.active);
    exec(
        &state,
        r"
        pmacs.statusline.set_enabled(SL_FAIL_HANDLE,false)
        _G.SL_SPLIT_FAIL=pmacs.statusline.register{
          name='split-failure',side='left',priority=20,
          fn=function(ctx) if not ctx.active then error('passive boom') end return nil end,
        }
        ",
    );
    let split_failure = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    assert_eq!(split_failure.new_failures.len(), 1);
    assert!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .new_failures
        .is_empty(),
        "success in the active split must not re-arm the passive split latch"
    );
    exec(&state, "pmacs.window.focus_next()");
    assert_eq!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .new_failures
        .len(),
        1,
        "focus role is part of the full failure context"
    );

    // A second frontend targets only its own registered view and receives its
    // own frontend id in every callback context.
    let foreign = FrontendId(77);
    {
        let mut core = state.core.borrow_mut();
        let window_id = core.active_window_id();
        core.register_frontend_view(
            foreign,
            pmacs::window::FrontendView {
                layout: pmacs::window::Layout::single(window_id),
                active: window_id,
                fold_projection: true,
            },
        );
    }
    let foreign_evaluation = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        StatuslineEvaluationTarget::Grid {
            frontend_id: foreign,
        },
    );
    let StatuslineEvaluationOutcome::Ready(foreign_windows) = foreign_evaluation.outcome else {
        panic!("foreign frontend evaluation was invalid")
    };
    assert_eq!(foreign_windows.len(), 1);
    assert_eq!(foreign_windows[0].context.frontend_id, foreign);
    state.core.borrow_mut().unregister_frontend_view(foreign);
    state
        .statusline_registry
        .borrow_mut()
        .detach_frontend(foreign);

    // Registry mutation during a snapshot is authoritative-empty invalidation.
    exec(
        &state,
        r"
        _G.SL_SELF = pmacs.statusline.register {
          name='self-remove',side='left',priority=100,
          fn=function() pmacs.statusline.unregister(SL_SELF); return 'STALE' end,
        }
        ",
    );
    let invalid = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    assert!(matches!(
        invalid.outcome,
        StatuslineEvaluationOutcome::Invalidated { .. }
    ));
    let next = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        target,
    );
    assert!(matches!(
        next.outcome,
        StatuslineEvaluationOutcome::Ready(_)
    ));

    // Closing a split and killing a source buffer are independently guarded.
    exec(
        &state,
        r"
        _G.SL_CLOSE_ONCE=true
        _G.SL_CLOSE=pmacs.statusline.register {
          name='close',side='left',priority=200,
          fn=function()
            if SL_CLOSE_ONCE then SL_CLOSE_ONCE=false; pmacs.window.close() end
            return 'CLOSE-STALE'
          end,
        }
        ",
    );
    assert!(matches!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .outcome,
        StatuslineEvaluationOutcome::Invalidated { .. }
    ));
    exec(&state, "pmacs.statusline.unregister(SL_CLOSE)");

    exec(
        &state,
        r"
        _G.SL_KILL_ONCE=true
        _G.SL_KILL=pmacs.statusline.register {
          name='kill',side='left',priority=200,
          fn=function(ctx)
            if SL_KILL_ONCE then SL_KILL_ONCE=false; pmacs.buffer.remove(ctx.buffer) end
            return 'KILL-STALE'
          end,
        }
        ",
    );
    assert!(matches!(
        evaluate_statusline(
            state.lua_host.lua(),
            &state.core,
            &state.statusline_registry,
            target,
        )
        .outcome,
        StatuslineEvaluationOutcome::Invalidated { .. }
    ));
}

// Acceptance 6 producer bite: invalid callback mutation clears the old wire
// payload in the same frame after shrinking its dynamic face inventory.
#[test]
fn a06_semantic_invalid_evaluation_is_authoritative_empty_then_survivor_returns() {
    let state = editor();
    exec(
        &state,
        r"
        _G.SL_MUTATE=false
        pmacs.theme.merge{['ui.modeline.mutator']={fg=6}}
        _G.SL_MUTATOR=pmacs.statusline.register{
          name='mutator',side='left',priority=10,face='ui.modeline.mutator',
          fn=function()
            if SL_MUTATE then pmacs.statusline.unregister(SL_MUTATOR) end
            return SL_MUTATE and 'STALE' or 'OLD'
          end,
        }
        pmacs.statusline.register{
          name='survivor',side='left',priority=0,fn=function() return 'GOOD' end,
        }
        ",
    );
    let buffer_id = active_buffer(&state);
    let mut semantic = semantic(&state, 18);
    let initial = semantic.render_frame(&state);
    assert_eq!(
        segments_of(&initial).unwrap().1,
        vec![
            StatuslineSegment {
                text: "OLD".into(),
                face: "ui.modeline.mutator".into(),
            },
            StatuslineSegment {
                text: "GOOD".into(),
                face: "ui.modeline".into(),
            },
        ]
    );

    exec(&state, "SL_MUTATE=true");
    let invalidated = semantic.render_frame(&state);
    let theme_index = invalidated
        .iter()
        .position(|message| matches!(message, InstanceMessage::ThemeFacts { .. }))
        .expect("dynamic face shrink");
    let segment_index = invalidated
        .iter()
        .position(|message| matches!(message, InstanceMessage::StatuslineSegments { .. }))
        .expect("authoritative empty replacement");
    assert!(theme_index < segment_index);
    assert_eq!(
        segments_of(&invalidated),
        Some((buffer_id, Vec::new(), Vec::new()))
    );
    assert!(
        theme_faces_of(&invalidated)
            .unwrap()
            .iter()
            .all(|face| face.name != "ui.modeline.mutator")
    );

    let surviving = semantic.render_frame(&state);
    assert_eq!(
        segments_of(&surviving),
        Some((
            buffer_id,
            vec![StatuslineSegment {
                text: "GOOD".into(),
                face: "ui.modeline".into(),
            }],
            Vec::new(),
        ))
    );
    assert_eq!(segments_of(&semantic.render_frame(&state)), None);
}

// Acceptance 9-11: exact order/separators, protected placement, Unicode and
// clipping through the real frame. Detailed face-cell and wide-grapheme edge
// bites remain colocated with the private painter.
#[test]
fn a09_11_full_tui_frame_composes_unicode_clips_and_preserves_echo() {
    let state = editor();
    state.core.borrow_mut().status = "echo".into();
    exec(
        &state,
        r"
        pmacs.statusline.register{name='left-low',side='left',priority=0,fn=function() return 'LOW' end}
        pmacs.statusline.register{name='left-high',side='left',priority=10,fn=function() return '界e\204\129' end}
        pmacs.statusline.register{name='right-high',side='right',priority=10,fn=function() return 'RH' end}
        pmacs.statusline.register{name='right-low',side='right',priority=0,fn=function() return 'RL' end}
        pmacs.statusline.register{name='nil',side='left',priority=99,fn=function() return nil end}
        ",
    );
    let wide = paint(&state, 24, 100);
    let mode = row_text(&wide, 100, 22);
    assert!(mode.starts_with(" +  *scratch*  界 e LOW"), "{mode:?}");
    assert!(mode.ends_with("RL RH  L1:C1 All "), "{mode:?}");
    assert_eq!(row_text(&wide, 100, 23).trim_end(), "echo");
    assert!(wide.iter().any(|cell| cell.glyph == Glyph::Continuation));
    assert!(wide.iter().any(|cell| {
        matches!(&cell.glyph, Glyph::Cluster(bytes) if bytes.as_ref() == "e\u{301}".as_bytes())
    }));

    let narrow = paint(&state, 6, 14);
    let narrow_mode = row_text(&narrow, 14, 4);
    assert!(narrow_mode.ends_with(" L1:C1 All "), "{narrow_mode:?}");
    assert!(narrow_mode.contains("RH"));
    assert!(!narrow_mode.contains("RL"));
    let too_narrow = paint(&state, 6, 10);
    let too_narrow_mode = row_text(&too_narrow, 10, 4);
    assert!(!too_narrow_mode.contains("L1:C1"));
}

// Acceptance 12: the shipped provider indexes lsp.lua's private attachment
// map and polls the real tracker without a buffer edit.
#[test]
fn a12_builtin_lsp_provider_tracks_real_attachment_and_unknown_label() {
    let mut state = EditorState::new();
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("statusline.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let path = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config = {{}}
            pmacs.lsp.config.rust = {{ command = "{fake}", restart = "never" }}
            _G.SL_LSP_BUFFER = pmacs.buffer.find_or_open("{path}")
            "#
        ),
    );
    let buffer_id = active_buffer(&state);
    let mut render = SemanticRenderState::for_peer(FrontendId::LOCAL, 18);
    render.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 4096,
        },
        0,
    );
    let first = render.render_frame(&state);
    let (_, _, right) = segments_of(&first).expect("first LSP statusline payload");
    assert_eq!(right[0].text, "LSP:init");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ready = false;
    while Instant::now() < deadline {
        state.tick_processes();
        state.tick_lsp();
        let frame = render.render_frame(&state);
        if segments_of(&frame).is_some_and(|(_, _, right)| right[0].text == "LSP:ready") {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready,
        "fake LSP never reached ready in the builtin provider"
    );

    // Forgetting the tracker while lsp.lua retains its attachment record is
    // the specified unknown-server `?` projection.
    exec(
        &state,
        "local rec=pmacs.lsp.active_attachment(); assert(rec); pmacs.lsp.stop(rec.server)",
    );
    let stop_deadline = Instant::now() + Duration::from_secs(5);
    let mut stopped = false;
    while Instant::now() < stop_deadline {
        state.tick_processes();
        state.tick_lsp();
        stopped = eval(
            &state,
            "local rec=pmacs.lsp.active_attachment(); return rec and pmacs.lsp.modeline_label(rec.server)=='stopped'",
        );
        if stopped {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(stopped, "stopped tracker label was never observable");
    let forget_deadline = Instant::now() + Duration::from_secs(5);
    let mut forgotten = false;
    while Instant::now() < forget_deadline {
        state.tick_processes();
        state.tick_lsp();
        forgotten = eval(
            &state,
            "local rec=pmacs.lsp.active_attachment(); return rec and pcall(pmacs.lsp.forget, rec.server)",
        );
        if forgotten {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(forgotten, "stopped fake server never became forgettable");
    let unknown = render.render_frame(&state);
    let (_, _, right) = segments_of(&unknown).expect("forgotten server update");
    assert_eq!(right[0].text, "LSP:?");
}

// Acceptance 13-17 and 26: protocol placement/version, authoritative first
// frame/live output, init + late join, old-peer callback cost, and TUI drop arm
// (the drop arm itself is pinned beside Frontend::apply_message).
#[test]
fn a13_17_26_protocol_semantic_init_late_join_and_version_cost() {
    // Vterm Stage 3 appended the terminal family as v19; GPU initial targets
    // appended the semantic bootstrap family as v20. This acceptance owns the
    // STATUSLINE variant's placement and gate, so it tracks the current wire
    // version rather than pinning 18: the v18 floor it actually cares about is
    // asserted below and in `peer_accepts_statusline_message`.
    assert_eq!(PROTOCOL_VERSION, 20);
    for version in 6..=20 {
        assert!(is_supported_protocol_version(version));
    }
    assert!(!is_supported_protocol_version(21));
    let sample = InstanceMessage::StatuslineSegments {
        buffer_id: BufferId::from_raw(9),
        left: vec![StatuslineSegment {
            text: "left".into(),
            face: "ui.modeline.left".into(),
        }],
        right: Vec::new(),
    };
    let bytes = postcard::to_stdvec(&sample).unwrap();
    assert_eq!(
        postcard::from_bytes::<InstanceMessage>(&bytes).unwrap(),
        sample
    );

    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("init.lua"),
        r#"
        _G.SL_INIT_CALLS = 0
        pmacs.theme.merge { ["ui.modeline.init"] = { fg = 6 } }
        pmacs.statusline.register {
          name='init-provider',side='left',face='ui.modeline.init',
          fn=function() SL_INIT_CALLS=SL_INIT_CALLS+1; return 'INIT' end,
        }
        "#,
    )
    .unwrap();
    let mut state = editor();
    pmacs::config::load_user_config_at(&mut state.lua_host, temp.path());
    let buffer_id = active_buffer(&state);

    let mut v17 = semantic(&state, 17);
    let old = v17.render_frame(&state);
    assert_eq!(segments_of(&old), None);
    assert_eq!(eval::<i64>(&state, "return SL_INIT_CALLS"), 0);
    assert!(
        theme_faces_of(&old)
            .unwrap()
            .iter()
            .all(|face| face.name != "ui.modeline.init")
    );

    let mut first_join = semantic(&state, 18);
    let first = first_join.render_frame(&state);
    assert_eq!(
        segments_of(&first),
        Some((
            buffer_id,
            vec![StatuslineSegment {
                text: "INIT".into(),
                face: "ui.modeline.init".into(),
            }],
            Vec::new(),
        ))
    );
    assert_eq!(segments_of(&first_join.render_frame(&state)), None);

    let mut late_join = semantic(&state, 18);
    let late = late_join.render_frame(&state);
    assert_eq!(segments_of(&late).unwrap().1[0].text, "INIT");

    // Grid invocation cost is one callback per visible context per frame.
    let before = eval::<i64>(&state, "return SL_INIT_CALLS");
    let _ = paint(&state, 24, 80);
    assert_eq!(eval::<i64>(&state, "return SL_INIT_CALLS"), before + 1);
}

// Acceptance 18-21: snapshot/baseline symmetry, dynamic face inventory and
// ordering, theme-only recolor, and fg-only/base-relative mask parity.
#[test]
fn a18_21_snapshot_dynamic_faces_order_recolor_and_mask() {
    let state = editor();
    exec(
        &state,
        r"
        _G.SL_TEXT='SEG'
        pmacs.theme.merge {
          ['ui.modeline']={fg=7,bg=4,reverse=true},
          ['ui.modeline.parent']={fg=2,bg=3,bold=true,reverse=true},
          ['ui.modeline.parent.child']={fg=6,bg=1,bold=true,reverse=true},
        }
        _G.SL_FACE_HANDLE=pmacs.statusline.register {
          name='face',side='left',face='ui.modeline.parent.child',
          fn=function() return SL_TEXT end,
        }
        ",
    );
    let buffer_id = active_buffer(&state);
    let mut semantic = semantic(&state, 18);
    let first = semantic.render_frame(&state);
    let theme_index = first
        .iter()
        .position(|message| matches!(message, InstanceMessage::ThemeFacts { .. }))
        .unwrap();
    let segment_index = first
        .iter()
        .position(|message| matches!(message, InstanceMessage::StatuslineSegments { .. }))
        .unwrap();
    assert!(theme_index < segment_index);
    let face = theme_faces_of(&first)
        .unwrap()
        .into_iter()
        .find(|face| face.name == "ui.modeline.parent.child")
        .expect("dynamic exact face");
    assert_eq!(
        face.style,
        Style {
            fg: Color::Indexed(6),
            ..Style::default()
        }
    );

    // Constant segment text + theme mutation sends ThemeFacts only.
    exec(
        &state,
        "pmacs.theme.merge{['ui.modeline.parent.child']={fg=5,bg=2,reverse=true,bold=true}}",
    );
    let recolor = semantic.render_frame(&state);
    assert!(theme_faces_of(&recolor).is_some());
    assert_eq!(segments_of(&recolor), None);

    semantic.on_buffer_snapshot_sent(buffer_id);
    assert_eq!(
        segments_of(&semantic.render_frame(&state)).unwrap().1[0].text,
        "SEG"
    );

    // Priority-only change has no face traffic. Disabling the final reference
    // shrinks the authoritative inventory and emits an empty replacement.
    exec(&state, "pmacs.statusline.set_priority(SL_FACE_HANDLE,99)");
    assert!(theme_faces_of(&semantic.render_frame(&state)).is_none());
    exec(&state, "pmacs.statusline.set_enabled(SL_FACE_HANDLE,false)");
    let disabled = semantic.render_frame(&state);
    assert!(
        theme_faces_of(&disabled)
            .unwrap()
            .iter()
            .all(|face| face.name != "ui.modeline.parent.child")
    );
    assert_eq!(
        segments_of(&disabled),
        Some((buffer_id, Vec::new(), Vec::new()))
    );

    // Base-relative semantics: an exact default child blocks a colored parent.
    exec(
        &state,
        r"
        pmacs.theme.merge{['ui.modeline.parent.child']={}}
        pmacs.statusline.set_enabled(SL_FACE_HANDLE,true)
        ",
    );
    let blocked = semantic.render_frame(&state);
    if let Some(faces) = theme_faces_of(&blocked) {
        assert!(
            faces
                .iter()
                .all(|face| face.name != "ui.modeline.parent.child")
        );
    }
}

// Acceptance 16 and 26 at the actual daemon producer + independent write loop.
#[cfg(feature = "crdt")]
#[test]
fn a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join() {
    use common::daemon::{TestDaemon, build_default_caps};
    use pmacs::protocol::{AttachRequest, FrontendCapabilities, FrontendEvent, Hello};
    use pmacs::transport::{read_message, write_message};

    fn caps() -> FrontendCapabilities {
        FrontendCapabilities {
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
            ..build_default_caps()
        }
    }

    fn probe(daemon: &TestDaemon, version: u32) -> (bool, bool) {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let hello: Hello = read_message(&mut stream).unwrap();
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: version,
                frontend_capabilities: caps(),
                initial_size: CellSize::new(24, 80),
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let buffer_id = loop {
            assert!(Instant::now() < deadline, "no bootstrap snapshot");
            if let Ok(InstanceMessage::BufferSnapshot { buffer_id, .. }) =
                read_message::<InstanceMessage>(&mut stream)
            {
                break buffer_id;
            }
        };
        write_message(
            &mut stream,
            &FrontendEvent::Viewport {
                frontend_id: hello.assigned_frontend_id,
                buffer_id,
                visible: ByteRange {
                    start: 0,
                    end: 4096,
                },
                generation: 0,
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut saw_segments, mut saw_dynamic_face) = (false, false);
        while Instant::now() < deadline {
            match read_message::<InstanceMessage>(&mut stream) {
                Ok(InstanceMessage::StatuslineSegments { left, .. }) => {
                    saw_segments |= left.iter().any(|segment| segment.text == "DAEMON");
                }
                Ok(InstanceMessage::ThemeFacts { faces }) => {
                    saw_dynamic_face |= faces.iter().any(|face| face.name == "ui.modeline.daemon");
                }
                Ok(_) | Err(_) => {}
            }
            if saw_segments && saw_dynamic_face {
                break;
            }
        }
        (saw_segments, saw_dynamic_face)
    }

    let daemon = TestDaemon::spawn_with_config(
        r"
        pmacs.theme.merge{['ui.modeline.daemon']={fg=6}}
        pmacs.statusline.register{
          name='daemon',side='left',face='ui.modeline.daemon',
          fn=function() return 'DAEMON' end,
        }
        ",
    );
    assert_eq!(probe(&daemon, 17), (false, false));
    assert_eq!(probe(&daemon, 18), (true, true));
    assert_eq!(
        probe(&daemon, 18),
        (true, true),
        "late join sees established state"
    );
}
