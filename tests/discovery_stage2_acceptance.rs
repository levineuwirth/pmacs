// discovery_stage2_acceptance.rs --- Discovery Stage 2
// (docs/discovery-stage2-framing.md §6).

//! `M-x` rows stop being bare names.
//!
//! `Command.description` already existed and was already rendered by
//! `help.list-commands`; it was missing at the one moment it would
//! change a decision. Carrying it to the row is two independent halves,
//! and this suite keeps them separate because they fail separately:
//!
//! - **The wire half** is a protocol bump, v22 → v23, and it is
//!   *additive*. `MinibufferPrompt` is FROZEN and still sent to every
//!   `12..=22` peer, because postcard encodes fields positionally — a
//!   widened `candidates` would make those peers mis-decode rather than
//!   ignore, and gating the widened form would have left them with no
//!   minibuffer message at all. The rich shape lives in an appended
//!   `MinibufferPromptRows`, and **exactly one of the two reaches any
//!   peer, ever**.
//! - **The TUI half involves no wire at all.** `src/editor.rs` contains
//!   zero references to `MinibufferPrompt`: `paint_minibuffer` reads
//!   `core.minibuffer` directly and renders the selected candidate as an
//!   inline suffix. So it reads `Command.description` from the registry
//!   in-process, which is why this half is independent of the bump.
//!
//! The daemon fixtures are `crdt`-gated because a semantic session is
//! necessarily a text replica: a non-CRDT build advertises no
//! `semantic_render` and cannot host one. They run in the
//! `--features crdt` sweep that `scripts/gate --protocol` adds.

mod common;

use std::path::Path;

use pmacs::bootstrap::BootstrapRoots;
use pmacs::editor::EditorState;
use pmacs_protocol::{
    ADVERTISED_PROTOCOL_VERSION, ByteRange, InstanceMessage, MinibufferRow, PROTOCOL_VERSION,
    is_supported_protocol_version,
};

#[cfg(feature = "crdt")]
use std::os::unix::net::UnixStream;
#[cfg(feature = "crdt")]
use std::time::{Duration, Instant};

#[cfg(feature = "crdt")]
use pmacs_protocol::cell::CellSize;
#[cfg(feature = "crdt")]
use pmacs_protocol::message::{
    AttachRequest, FrontendCapabilities, FrontendEvent, Hello, Key, KeyEvent, Modifiers,
    SessionBootstrapRequest,
};
#[cfg(feature = "crdt")]
use pmacs_protocol::transport::{read_message, write_message};

#[cfg(feature = "crdt")]
use common::daemon::{TestDaemon, build_default_caps};

// ---------------------------------------------------------------------------
// Version-bump discipline (§6, last bullet)
// ---------------------------------------------------------------------------

/// The bump is deliberate, and the advertised baseline does NOT move.
///
/// `ADVERTISED_PROTOCOL_VERSION` is pinned at 20 and is the one constant
/// that must never be edited (handoff §3/§5): the handshake is
/// server-first, so moving it locks out every already-shipped frontend
/// before it can counter-offer. An additive family never needs it.
#[test]
fn the_wire_is_v23_and_the_advertised_baseline_is_unmoved() {
    assert_eq!(
        PROTOCOL_VERSION, 23,
        "v23 is MinibufferPromptRows (Discovery Stage 2)"
    );
    assert_eq!(
        ADVERTISED_PROTOCOL_VERSION, 20,
        "moving this is the incompatible act the counter-offer mechanism exists to avoid"
    );
    // The whole v12..=22 population this lane is compatible with is
    // still supported, and the set ends at the new wire — a widened set
    // is a failure rather than a silent pass.
    for version in 6..=23 {
        assert!(
            is_supported_protocol_version(version),
            "v{version} must still be supported"
        );
    }
    assert!(!is_supported_protocol_version(24));
}

// ---------------------------------------------------------------------------
// The TUI half: no wire involvement (§3.4, §6)
// ---------------------------------------------------------------------------

fn session(name: &str) -> EditorState {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("discovery-stage2")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let roots = BootstrapRoots::isolated_under(&base);
    for (_, dir) in roots.child_env() {
        std::fs::create_dir_all(&dir).expect("create controlled root");
    }
    let state = EditorState::new_with_roots(&roots);
    state.install_state_dirs();
    state
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Render one frame at `cols` columns and return the bottom row's text.
///
/// Through `RenderState` and the wire rather than by calling the painter
/// directly: the spans are what the TUI actually consumes, so this
/// asserts on the cells that reach a screen.
fn bottom_row(s: &EditorState, rows: u32, cols: u32) -> String {
    use std::collections::HashMap;

    let size = pmacs::cell::CellSize::new(rows, cols);
    let mut rs = pmacs::instance_render::RenderState::new(size);
    let msgs = rs.render_frame(s, pmacs::protocol::FrontendId::LOCAL, &HashMap::new(), &[]);

    let mut row = vec![' '; cols as usize];
    for msg in &msgs {
        if let pmacs_protocol::InstanceMessage::CellDelta { spans, .. } = msg {
            for span in spans {
                if span.start.row != rows - 1 {
                    continue;
                }
                for (i, cell) in span.cells.iter().enumerate() {
                    let c = span.start.col as usize + i;
                    if c < cols as usize
                        && let pmacs::cell::Glyph::Char(ch) = cell.glyph
                    {
                        row[c] = ch;
                    }
                }
            }
        }
    }
    row.into_iter().collect::<String>().trim_end().to_owned()
}

/// Open `M-x` narrowed to `zzprobe` and return the candidate rows the
/// semantic producer ships to a current-wire peer.
///
/// Through `SemanticRenderState` and the real minibuffer session rather
/// than by constructing a message: the clip lives in the producer, so a
/// hand-built row would skip the thing under test.
fn mx_rows(s: &EditorState) -> Vec<MinibufferRow> {
    let bid = s.core.borrow().active_buffer_id();
    let mut render = pmacs::semantic_render::SemanticRenderState::for_peer(
        pmacs::protocol::FrontendId::LOCAL,
        PROTOCOL_VERSION,
    );
    render.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);
    let _ = render.render_frame(s);

    exec(
        s,
        "pmacs.minibuffer.read{ prompt = 'M-x ', source = 'commands', on_accept = function() end }",
    );
    exec(s, "pmacs.minibuffer.set_contents('zzprobe')");
    render
        .render_frame(s)
        .into_iter()
        .find_map(|msg| match msg {
            InstanceMessage::MinibufferPromptRows { rows, .. } => Some(rows),
            _ => None,
        })
        .expect("the producer ships a rows prompt")
}

/// Open `M-x`, narrowed to exactly one command with a known
/// description, and report the bottom row at `cols` columns.
fn mx_bottom_row(s: &EditorState, cols: u32) -> String {
    exec(
        s,
        "pmacs.minibuffer.read{ prompt = 'M-x ', source = 'commands', on_accept = function() end }",
    );
    exec(s, "pmacs.minibuffer.set_contents('zzprobe')");
    bottom_row(s, 24, cols)
}

const PROBE_DESCRIPTION: &str = "Probe the description row.";

fn define_probe(s: &EditorState) {
    exec(
        s,
        &format!(
            "pmacs.command.define{{ name = 'zzprobe', description = '{PROBE_DESCRIPTION}', \
             fn = function() end }}"
        ),
    );
}

#[test]
fn the_tui_renders_the_description_beside_the_selected_name() {
    let s = session("tui-wide");
    define_probe(&s);
    let row = mx_bottom_row(&s, 120);
    assert!(
        row.contains(&format!("[zzprobe — {PROBE_DESCRIPTION}]")),
        "the selected candidate carries its description: {row:?}"
    );
}

#[test]
fn the_tui_drops_the_description_then_the_whole_suffix_as_width_shrinks() {
    // §3.4's three ORDERED steps, at the three widths that separate
    // them. The guarantee is "never a PARTIAL name", which is
    // achievable; "the name always survives" is not, because the prompt
    // and the typed input consume the budget first.
    let s = session("tui-clip");
    define_probe(&s);

    // 1. Wide: name + description.
    let wide = mx_bottom_row(&s, 120);
    assert!(
        wide.contains(&format!("[zzprobe — {PROBE_DESCRIPTION}]")),
        "wide: {wide:?}"
    );

    // 2. Room for the whole name but not the whole description: the
    //    description is dropped, leaving exactly today's `[name]`. No
    //    ellipsis stub, and no prefix of the description either.
    let medium = mx_bottom_row(&s, 30);
    assert!(medium.contains("[zzprobe]"), "medium: {medium:?}");
    assert!(
        !medium.contains('—'),
        "a description that does not fit whole is dropped entirely: {medium:?}"
    );

    // 3. Too narrow for even the whole name: the suffix vanishes. The
    //    assertion is that no PREFIX of the name is emitted — `[zzpr`
    //    would read as a different command, which is worse than nothing.
    let narrow = mx_bottom_row(&s, 18);
    assert!(
        !narrow.contains('['),
        "a suffix that cannot hold the whole name is omitted entirely: {narrow:?}"
    );
    assert!(
        narrow.starts_with("M-x zzprobe"),
        "the prompt and the typed input still own the row: {narrow:?}"
    );
    for cut in 1.."zzprobe".len() {
        assert!(
            !narrow.contains(&format!("[{}", &"zzprobe"[..cut])),
            "no prefix of the name may be emitted: {narrow:?}"
        );
    }
}

#[test]
fn a_source_with_no_detail_renders_exactly_as_before_in_the_tui() {
    // Q#D2-2: the file-path prompt is the witness. It has no detail, so
    // its suffix is the pre-v23 `[name]` and nothing else.
    let s = session("tui-files");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("discovery-stage2-files");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create file-prompt dir");
    std::fs::write(dir.join("zznotes.txt"), b"x").expect("seed a file");
    exec(
        &s,
        &format!(
            "pmacs.minibuffer.read{{ prompt = 'File: ', source = 'files', \
             source_root = '{}', on_accept = function() end }}",
            dir.display()
        ),
    );
    exec(&s, "pmacs.minibuffer.set_contents('zznotes.txt')");
    let row = bottom_row(&s, 24, 120);
    assert!(row.contains("[zznotes.txt]"), "file prompt row: {row:?}");
    assert!(
        !row.contains('—'),
        "a source with no detail gains no separator: {row:?}"
    );
}

// ---------------------------------------------------------------------------
// Multi-line descriptions reach single-row surfaces as ONE line
// ---------------------------------------------------------------------------

/// An MCP-shaped description: tool text, blank line, `Arguments:`, then
/// one line per argument.
///
/// This is the real shape, not an invented one —
/// `tests/fixtures/pmacs-mcp-tools/init.lua:272` builds it with
/// `table.concat(lines, "\n")` and `m9_6_acceptance.rs:583-598` asserts
/// four of its lines, which is why registration accepts it and the
/// SURFACES clip instead.
const MCP_SHAPED: &str = "Greet someone.\\n\\nArguments:\\n  name (string, required)";

fn define_multiline_probe(s: &EditorState, name: &str, description: &str) {
    exec(
        s,
        &format!(
            "pmacs.command.define{{ name = '{name}', description = \"{description}\", \
             fn = function() end }}"
        ),
    );
}

#[test]
fn a_multi_line_description_reaches_the_tui_band_as_one_line() {
    let s = session("tui-multiline");
    define_multiline_probe(&s, "zzprobe", MCP_SHAPED);
    let row = mx_bottom_row(&s, 200);
    assert!(
        row.contains("[zzprobe — Greet someone.]"),
        "the band shows the first line only: {row:?}"
    );
    assert!(
        !row.contains("Arguments:"),
        "the schema block must not reach a single-row band: {row:?}"
    );
    // `bottom_row` reads one grid row, so anything below would be lost
    // rather than visibly wrong — assert on the registry-side clip too,
    // which is what the painter consumed.
    let clipped: String = eval(&s, "return pmacs.describe.command('zzprobe').description");
    assert!(
        clipped.contains("Arguments:"),
        "describe-command must still see the WHOLE description, or the clip \
         silently deleted the schema block everywhere: {clipped:?}"
    );
}

#[test]
fn a_multi_line_description_reaches_the_gpu_row_as_one_physical_line() {
    // The geometry hazard, through the real prompt path: the dropdown
    // sizes itself from `rows.len()` — one logical row per candidate —
    // so a detail carrying a break would shape into more physical lines
    // than the geometry accounts for.
    //
    // All three break forms, since a clip handling only LF would pass a
    // bare CR through to the same surface.
    for (label, description, tail) in [
        ("LF", MCP_SHAPED, "Arguments:"),
        (
            "CR",
            "Greet someone.\\r\\rArguments:\\r  name (string, required)",
            "Arguments:",
        ),
        (
            "CRLF",
            "Greet someone.\\r\\n\\r\\nArguments:\\r\\n  name (string, required)",
            "Arguments:",
        ),
    ] {
        let s = session(&format!("gpu-multiline-{label}"));
        define_multiline_probe(&s, "zzprobe", description);
        let rows = mx_rows(&s);
        let probe = rows
            .iter()
            .find(|row| row.label == "zzprobe")
            .unwrap_or_else(|| panic!("{label}: the probe command is a candidate"));
        let detail = probe
            .detail
            .as_deref()
            .unwrap_or_else(|| panic!("{label}: the row carries a detail"));
        assert_eq!(
            detail, "Greet someone.",
            "{label}: the wire row carries the first line only"
        );
        assert!(
            !detail.contains(['\n', '\r']),
            "{label}: a row detail must carry no line break: {detail:?}"
        );
        assert!(
            !detail.contains(tail),
            "{label}: the schema block must not reach the dropdown"
        );

        // And the full text is still there for the discoverability
        // path, which is what makes this a rendering decision.
        let full: String = eval(&s, "return pmacs.describe.command('zzprobe').description");
        assert!(
            full.contains("name (string, required)"),
            "{label}: describe-command must still report every line: {full:?}"
        );
    }
}

#[test]
fn a_single_line_description_is_unchanged_on_the_wire() {
    // The clip did not tighten past its purpose: a description with no
    // break reaches the row byte-identical, with no truncation marker.
    let s = session("wire-single-line");
    define_probe(&s);
    let rows = mx_rows(&s);
    let probe = rows
        .iter()
        .find(|row| row.label == "zzprobe")
        .expect("the probe command is a candidate");
    assert_eq!(probe.detail.as_deref(), Some(PROBE_DESCRIPTION));
}

#[test]
fn typed_but_unmatched_input_is_still_accepted() {
    // Q#D2-5, the trap this lane arrives with: richer rows make `M-x`
    // LOOK like a closed set, which invites making acceptance reject
    // unmatched input. That would be a behaviour change, and it is out
    // of scope. `resolve_accepted_value` still returns the literal typed
    // text when nothing is selected.
    let s = session("open-set");
    exec(
        &s,
        "_G.ACCEPTED = nil
         pmacs.minibuffer.read{ prompt = 'M-x ', source = 'commands',
           on_accept = function(v) _G.ACCEPTED = v end }",
    );
    exec(
        &s,
        "pmacs.minibuffer.set_contents('no-such-command-at-all')",
    );
    assert_eq!(
        eval::<usize>(&s, "return #pmacs.minibuffer.candidates()"),
        0,
        "the probe input must match nothing, or this asserts the wrong thing"
    );
    exec(&s, "pmacs.minibuffer.accept()");
    assert_eq!(
        eval::<String>(&s, "return _G.ACCEPTED"),
        "no-such-command-at-all",
        "completion is assistance, not validation"
    );
}

// ---------------------------------------------------------------------------
// The wire half: one real daemon, two negotiated versions (§6)
// ---------------------------------------------------------------------------

/// An `init.lua` that registers the probe command whose description the
/// wire must carry.
#[cfg(feature = "crdt")]
const PROBE_INIT: &str = r#"
pmacs.command.define {
  name = "zzprobe",
  description = "Probe the description row.",
  fn = function() end,
}
"#;

/// A minibuffer message, in whichever family it arrived.
#[cfg(feature = "crdt")]
#[derive(Debug)]
enum Mb {
    Legacy {
        prompt: Option<String>,
        candidates: Vec<String>,
    },
    Rows {
        prompt: Option<String>,
        rows: Vec<MinibufferRow>,
    },
}

#[cfg(feature = "crdt")]
fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

/// Attach a semantic session offering exactly `offer`, declare a
/// viewport so the projection producer is live, and hand back the
/// stream plus this session's frontend id.
#[cfg(feature = "crdt")]
fn attach_semantic(daemon: &TestDaemon, offer: u32) -> (UnixStream, pmacs_protocol::FrontendId) {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let hello: Hello = read_message(&mut stream).expect("read daemon Hello");
    assert_eq!(
        hello.protocol_version, ADVERTISED_PROTOCOL_VERSION,
        "the server-first Hello must stay at the compatibility baseline"
    );
    let fid = hello.assigned_frontend_id;
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: offer,
            frontend_capabilities: semantic_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("write AttachRequest");
    // A v20-or-later semantic session sends the bootstrap envelope; the
    // daemon reads it unconditionally for those, so skipping it would
    // desynchronize the stream.
    if offer >= 20 {
        write_message(
            &mut stream,
            &SessionBootstrapRequest {
                initial_target: None,
            },
        )
        .expect("write bootstrap");
    }
    let document = pump(&mut stream, "first BufferSnapshot", |msg| match msg {
        InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
        _ => None,
    });
    write_message(
        &mut stream,
        &FrontendEvent::Viewport {
            frontend_id: fid,
            buffer_id: document,
            visible: ByteRange { start: 0, end: 0 },
            generation: 0,
        },
    )
    .expect("declare a viewport");
    (stream, fid)
}

#[cfg(feature = "crdt")]
fn pump<T>(
    stream: &mut UnixStream,
    what: &str,
    mut want: impl FnMut(&InstanceMessage) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match read_message::<InstanceMessage>(stream) {
            Ok(msg) => {
                if let Some(found) = want(&msg) {
                    return found;
                }
            }
            Err(error) => panic!("{what}: read stopped: {error}"),
        }
    }
    panic!("timed out waiting for {what}");
}

/// Collect every minibuffer message this session receives, up to and
/// including the first one `done` accepts.
///
/// Collecting rather than filtering is the point: "a v23 peer receives
/// the rows form" is only half the guarantee, and the other half — that
/// it never receives the legacy form — can only be checked against
/// everything that arrived.
#[cfg(feature = "crdt")]
fn collect_minibuffer(
    stream: &mut UnixStream,
    what: &str,
    mut done: impl FnMut(&Mb) -> bool,
) -> Vec<Mb> {
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match read_message::<InstanceMessage>(stream) {
            Ok(InstanceMessage::MinibufferPrompt {
                prompt, candidates, ..
            }) => {
                seen.push(Mb::Legacy { prompt, candidates });
            }
            Ok(InstanceMessage::MinibufferPromptRows { prompt, rows, .. }) => {
                seen.push(Mb::Rows { prompt, rows });
            }
            Ok(_) => continue,
            Err(error) => panic!("{what}: read stopped: {error}"),
        }
        if done(seen.last().expect("just pushed")) {
            return seen;
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

#[cfg(feature = "crdt")]
fn send_key(stream: &mut UnixStream, fid: pmacs_protocol::FrontendId, key: Key, mods: Modifiers) {
    write_message(
        stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: fid,
            key,
            mods,
            timestamp_ns: 0,
        }),
    )
    .expect("write key");
}

/// The whole exclusivity guarantee, on one live daemon: a v22 peer and a
/// v23 peer attached **simultaneously** each receive their own variant
/// and only their own — open and close alike.
///
/// One daemon rather than two, and both directions in one fixture. Two
/// daemons could each pass their own half while the same build was
/// incapable of serving both, which is the only property that matters;
/// and a test that only proved "v23 gets rows" would pass with the
/// compatibility half broken.
#[cfg(feature = "crdt")]
#[test]
fn one_daemon_serves_a_v23_rows_session_and_a_frozen_v22_session() {
    let daemon = TestDaemon::spawn_with_config(PROBE_INIT);

    // The compatibility half attaches FIRST, deliberately: it is the
    // half an over-eager bump destroys, so a regression fails here
    // rather than after the interesting half has already passed.
    let (mut legacy, _legacy_fid) = attach_semantic(&daemon, 22);
    let (mut current, current_fid) = attach_semantic(&daemon, PROTOCOL_VERSION);
    assert_eq!(PROTOCOL_VERSION, 23);

    // Open the real `M-x` through the real key path, then narrow to the
    // probe command by typing it — the candidate window is ten rows out
    // of well over a hundred commands, so an unnarrowed prompt would
    // assert nothing about the probe.
    send_key(&mut current, current_fid, Key::Char('x'), Modifiers::ALT);
    for ch in "zzprobe".chars() {
        send_key(&mut current, current_fid, Key::Char(ch), Modifiers::NONE);
    }

    let on_current = collect_minibuffer(&mut current, "v23 open", |mb| match mb {
        Mb::Rows { prompt, rows } => {
            prompt.is_some() && rows.iter().any(|row| row.label == "zzprobe")
        }
        Mb::Legacy { .. } => false,
    });
    assert!(
        on_current.iter().all(|mb| matches!(mb, Mb::Rows { .. })),
        "a v23 peer must never receive the frozen legacy variant: {on_current:?}"
    );
    let Some(Mb::Rows { rows, .. }) = on_current.last() else {
        unreachable!("collect_minibuffer returns on a Rows match")
    };
    let probe = rows
        .iter()
        .find(|row| row.label == "zzprobe")
        .expect("the probe command is a candidate");
    assert_eq!(
        probe.detail.as_deref(),
        Some(PROBE_DESCRIPTION),
        "the description reaches the row through the real prompt path"
    );

    // The same session state, seen by the v22 peer, in the frozen shape.
    let on_legacy = collect_minibuffer(&mut legacy, "v22 open", |mb| match mb {
        Mb::Legacy { prompt, candidates } => {
            prompt.is_some() && candidates.iter().any(|c| c == "zzprobe")
        }
        Mb::Rows { .. } => false,
    });
    assert!(
        on_legacy.iter().all(|mb| matches!(mb, Mb::Legacy { .. })),
        "a v22 peer must never receive the v23 rows variant: {on_legacy:?}"
    );

    // The close must arrive in the SAME family as the open. A rows
    // session closed by a legacy clear leaves the dropdown on screen
    // forever, and the witness for "it actually cleared" is a `prompt:
    // None` in the family the frontend is mirroring.
    send_key(&mut current, current_fid, Key::Escape, Modifiers::NONE);
    let closed_current = collect_minibuffer(&mut current, "v23 close", |mb| {
        matches!(mb, Mb::Rows { prompt: None, .. })
    });
    assert!(
        closed_current
            .iter()
            .all(|mb| matches!(mb, Mb::Rows { .. })),
        "the v23 close must not arrive as a legacy clear: {closed_current:?}"
    );
    let closed_legacy = collect_minibuffer(&mut legacy, "v22 close", |mb| {
        matches!(mb, Mb::Legacy { prompt: None, .. })
    });
    assert!(
        closed_legacy
            .iter()
            .all(|mb| matches!(mb, Mb::Legacy { .. })),
        "the v22 close must stay in the frozen family: {closed_legacy:?}"
    );
}
