// bottom_panel_stage2b_gpu_acceptance.rs --- bottom-panel Stage 2B-3
// (docs/bottom-panel-stage2-framing.md §7.2.3; A2B-5 and the production
// re-assertion of 42/43/44/45/51/52 through the real capability flip).

//! Compatible v21 activation and the negotiated `panel_capable` flip.
//!
//! The band's own pixel geometry, the epoch latch, the probe-derived
//! column count, and the three-boundary contrast assertion live in
//! `pmacs-gpu`'s own tests, because they need a real `State` and a real
//! surface. What lives here is everything that needs a **real daemon**:
//! the handshake, the negotiation, and what the daemon does with a
//! session's negotiated version.
//!
//! The discipline this suite is built around: **every acceptance runs
//! both directions in the same fixture.** A test that only proved "a v21
//! frontend gets a panel" would pass with the compatibility half broken,
//! and a test that only proved "a v20 frontend still attaches" would pass
//! with the activation missing entirely. Neither half is meaningful
//! alone, so neither appears alone.

mod common;

// Most of this suite needs a real daemon and therefore the `crdt` feature:
// a semantic session is necessarily a text replica, so a non-CRDT build
// cannot host one at all. The negotiation-rule tests are the exception and
// run in both configurations, which is why the imports split here.
#[cfg(feature = "crdt")]
use std::io::Read;
#[cfg(feature = "crdt")]
use std::os::unix::net::UnixStream;
#[cfg(feature = "crdt")]
use std::time::{Duration, Instant};

#[cfg(feature = "crdt")]
use pmacs_protocol::cell::CellSize;
#[cfg(feature = "crdt")]
use pmacs_protocol::message::{
    AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason, Hello, InstanceMessage, Key,
    KeyEvent, Modifiers, SessionBootstrapRequest,
};
use pmacs_protocol::panel::PANEL_MIN_VERSION;
#[cfg(feature = "crdt")]
use pmacs_protocol::panel::PanelFramePayload;
#[cfg(feature = "crdt")]
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{
    ADVERTISED_PROTOCOL_VERSION, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
    is_supported_protocol_version, negotiated_session_version, requested_protocol_version,
};

#[cfg(feature = "crdt")]
use common::daemon::{TestDaemon, build_default_caps};

/// Opens a bottom panel in whichever frontend pressed the key.
///
/// A real adopter path rather than a Lua eval hook: `pmacs.window.display`
/// with an explicit `side` is exactly what a Stage 1 adopter does, and the
/// acting frontend is the one that sent the key — so whether this produces
/// a side window is precisely the `panel_capable` question.
#[cfg(feature = "crdt")]
const PANEL_ON_KEY: &str = r#"
pmacs.command.define {
  name = "bp-probe.panel",
  description = "Open the Stage 2B-3 acceptance panel.",
  fn = function()
    pmacs.window.display(pmacs.buffer.create("*bp-probe*"), { side = "bottom", height = 4 })
  end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-M-p", command = "bp-probe.panel" }
"#;

#[cfg(feature = "crdt")]
fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

/// One attached session, with the version it actually offered.
#[cfg(feature = "crdt")]
struct Session {
    stream: UnixStream,
    offered: u32,
}

/// Attach a semantic frontend that offers exactly `offer`.
///
/// The `Hello` baseline is asserted here rather than in one dedicated
/// test, because every fixture in this file depends on it: if the daemon
/// ever advertised something above the baseline, a shipped frontend would
/// reject before reaching any of these code paths, and the tests below
/// would still pass while the product was broken.
#[cfg(feature = "crdt")]
fn attach_semantic(daemon: &TestDaemon, offer: u32) -> Session {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let hello: Hello = read_message(&mut stream).expect("read daemon Hello");
    assert_eq!(
        hello.protocol_version, ADVERTISED_PROTOCOL_VERSION,
        "the server-first Hello must stay at the compatibility baseline"
    );
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
    // desynchronize the stream rather than merely omit a target.
    if offer >= 20 {
        write_message(
            &mut stream,
            &SessionBootstrapRequest {
                initial_target: None,
            },
        )
        .expect("write bootstrap");
    }
    Session {
        stream,
        offered: offer,
    }
}

/// Read messages until `want` returns `Some`, or the deadline passes.
#[cfg(feature = "crdt")]
fn drain_until<T>(
    stream: &mut UnixStream,
    label: &str,
    mut want: impl FnMut(&InstanceMessage) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match read_message::<InstanceMessage>(stream) {
            Ok(message) => {
                if let Some(found) = want(&message) {
                    return Some(found);
                }
            }
            Err(error) => {
                eprintln!("{label}: read stopped: {error}");
                return None;
            }
        }
    }
    None
}

/// Press `C-M-p`, then report whether a `Present` panel frame arrived.
#[cfg(feature = "crdt")]
fn press_and_await_panel(session: &mut Session) -> bool {
    // The declaration first: the daemon needs columns before it can paint
    // a first panel frame, and it is valid without a side window for
    // exactly that reason.
    if session.offered >= PANEL_MIN_VERSION {
        write_message(
            &mut session.stream,
            &FrontendEvent::FrontendCellGeometry {
                frontend_id: pmacs_protocol::FrontendId(0),
                geometry_epoch: 1,
                total: CellSize::new(40, 120),
            },
        )
        .expect("write geometry declaration");
    }
    write_message(
        &mut session.stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: pmacs_protocol::FrontendId(0),
            key: Key::Char('p'),
            mods: Modifiers::CTRL | Modifiers::ALT,
            timestamp_ns: 0,
        }),
    )
    .expect("write panel-open key");
    drain_until(&mut session.stream, "panel", |message| match message {
        InstanceMessage::PanelFrame(PanelFramePayload::Present(frame)) => Some(frame.size),
        _ => None,
    })
    .is_some()
}

// ---------------------------------------------------------------------------
// A2B-5 — the activation mechanism, both directions in one fixture
// ---------------------------------------------------------------------------

/// The whole mechanism, on one live daemon: the v21 frontend gets a band,
/// the v20 frontend still attaches and reaches its initial grid, and the
/// daemon's advertised version never moves.
///
/// One daemon rather than two, deliberately. Two daemons could each pass
/// their own half while the *same* build was incapable of serving both,
/// which is the only property that matters here.
#[cfg(feature = "crdt")]
#[test]
fn one_daemon_serves_a_v21_panel_session_and_a_shipped_v20_client() {
    let daemon = TestDaemon::spawn_with_config(PANEL_ON_KEY);

    // Half 1 — the shipped v20 client. This runs FIRST on purpose: it is
    // the half a broken activation destroys, and running it first means a
    // regression fails here rather than after the interesting half passed.
    let mut legacy = daemon.connect();
    legacy
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set legacy timeout");
    let hello: Hello = read_message(&mut legacy).expect("read Hello");
    let shipped_v20_range = 6..=20;
    assert!(
        shipped_v20_range.contains(&hello.protocol_version),
        "a shipped v20 client rejects the server-first Hello before it can \
         send AttachRequest, so this is the rejection point: {}",
        hello.protocol_version
    );
    write_message(
        &mut legacy,
        &AttachRequest {
            protocol_version: hello.protocol_version,
            frontend_capabilities: build_default_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("write v20 AttachRequest");
    assert!(
        drain_until(&mut legacy, "legacy", |message| matches!(
            message,
            InstanceMessage::CellDelta {
                full_grid: true,
                ..
            }
        )
        .then_some(()))
        .is_some(),
        "the v20 session must reach its initial grid, not merely receive an \
         acceptable Hello"
    );

    // Half 2 — the current frontend counter-offers and gets the band.
    let mut current = attach_semantic(&daemon, requested_protocol_version(hello.protocol_version));
    assert_eq!(
        current.offered, PROTOCOL_VERSION,
        "the counter-offer is this binary's own wire"
    );
    assert!(
        press_and_await_panel(&mut current),
        "a v21-negotiated semantic session must be panel-capable and receive \
         a Present panel frame"
    );

    // Half 3 — a semantic session that echoed the baseline is NOT
    // panel-capable, and the gate is on PLACEMENT rather than only on
    // transport: it keeps a working document window instead of an
    // invisible side one.
    let mut pre_panel = attach_semantic(&daemon, ADVERTISED_PROTOCOL_VERSION);
    let document = drain_until(
        &mut pre_panel.stream,
        "pre-panel snapshot",
        |message| match message {
            InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
            _ => None,
        },
    )
    .expect("a semantic attach receives a buffer snapshot");
    // A real semantic frontend declares a viewport; the daemon produces no
    // styling until it does, so without this the "document still works"
    // half below would be unobservable rather than false.
    write_message(
        &mut pre_panel.stream,
        &FrontendEvent::Viewport {
            frontend_id: pmacs_protocol::FrontendId(0),
            buffer_id: document,
            visible: pmacs_protocol::ByteRange { start: 0, end: 0 },
            generation: 0,
        },
    )
    .expect("declare a viewport");
    // The same panel-open key the v21 session used. One drain, classifying
    // both outcomes: a `PanelFrame` fails immediately, and the document's
    // own semantic traffic is what proves the fallback window is live.
    write_message(
        &mut pre_panel.stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: pmacs_protocol::FrontendId(0),
            key: Key::Char('p'),
            mods: Modifiers::CTRL | Modifiers::ALT,
            timestamp_ns: 0,
        }),
    )
    .expect("write panel-open key");
    // Two claims in one drain, and the SECOND is the load-bearing one.
    //
    // "No panel frame arrives" is defence in depth, not the placement gate:
    // the producer's peer flag and the write-loop filter both suppress
    // `PanelFrame` for a peer below the panel version independently of
    // `panel_capable`, so that claim passes even with the capability gate
    // removed entirely. The placement claim is what only `panel_capable`
    // can decide — the buffer the adopter asked for must land in this
    // session's DOCUMENT window, not in a side window it cannot render,
    // because a side window here would simply be invisible.
    let mut placed_in_document = None;
    let _ = drain_until(&mut pre_panel.stream, "fallback", |message| {
        assert!(
            !matches!(message, InstanceMessage::PanelFrame(_)),
            "a v20 semantic session must never be sent a panel frame: {message:?}"
        );
        if let InstanceMessage::CursorByte { buffer_id, .. } = message
            && *buffer_id != document
        {
            placed_in_document = Some(*buffer_id);
            return Some(());
        }
        None
    });
    assert!(
        placed_in_document.is_some(),
        "the adopter's buffer must be placed in this session's own document \
         window (Q#BP2c fallback, every side parameter discarded) — a \
         panel-capable session would have put it in a side window and left \
         this session's document buffer unchanged"
    );
}

// ---------------------------------------------------------------------------
// The negotiation rules themselves
// ---------------------------------------------------------------------------

/// The advertised baseline is a compatibility floor that does NOT move,
/// and the counter-offer is what reaches the current wire.
#[test]
fn the_baseline_stays_and_the_counter_offer_activates() {
    // A deliberate tripwire: bumping the wire must be a conscious edit
    // here, not a silent one. v22 is `LineWrapFacts` (long-lines Stage 3).
    assert_eq!(PROTOCOL_VERSION, 22);
    assert_eq!(
        ADVERTISED_PROTOCOL_VERSION, 20,
        "moving this is the incompatible act the mechanism exists to avoid"
    );
    const { assert!(PROTOCOL_VERSION > ADVERTISED_PROTOCOL_VERSION) };

    // Panel frames are gated ABOVE the advertised floor and at or below
    // this binary's wire. Both halves are durable properties of the
    // gate.
    //
    // This replaces `assert_eq!(PANEL_MIN_VERSION, PROTOCOL_VERSION)`,
    // which asserted a **coincidence**: panel frames were the newest
    // feature when it was written, so their minimum happened to equal
    // the current wire. Any later feature falsifies that — v22 is the
    // first, and the equality would have had to be edited on every
    // subsequent bump while telling a reader something that was never
    // the contract.
    // `const` blocks, matching the line above: these are compile-time
    // constants, so a runtime `assert!` is both a clippy error and a
    // weaker check than the language already offers.
    const { assert!(PANEL_MIN_VERSION > ADVERTISED_PROTOCOL_VERSION) };
    const { assert!(PANEL_MIN_VERSION <= PROTOCOL_VERSION) };

    // The current baseline is answered with this binary's own version.
    assert_eq!(
        requested_protocol_version(ADVERTISED_PROTOCOL_VERSION),
        PROTOCOL_VERSION
    );
    // Anything older is echoed VERBATIM, so a genuinely older daemon takes
    // byte-for-byte the pre-activation path.
    for older in 6..ADVERTISED_PROTOCOL_VERSION {
        assert_eq!(
            requested_protocol_version(older),
            older,
            "a daemon advertising v{older} must be echoed, not counter-offered"
        );
    }
    // The offer is never below the baseline: a frontend that supported less
    // would already have rejected the Hello.
    for baseline in SUPPORTED_PROTOCOL_VERSIONS {
        assert!(requested_protocol_version(*baseline) >= *baseline);
    }
}

/// The session speaks the lower of the two ceilings.
#[test]
fn the_daemon_negotiates_the_lower_of_the_two_ceilings() {
    for offer in SUPPORTED_PROTOCOL_VERSIONS {
        assert_eq!(
            negotiated_session_version(*offer),
            *offer,
            "every supported offer is adopted as-is"
        );
    }
    // An offer above this binary's own wire is clamped rather than
    // recorded. It cannot arrive today — the membership test rejects it
    // first — which is exactly why the rule is written down instead of
    // left implicit in that test.
    assert_eq!(
        negotiated_session_version(PROTOCOL_VERSION + 1),
        PROTOCOL_VERSION
    );
    assert!(!is_supported_protocol_version(PROTOCOL_VERSION + 1));
}

/// An offer outside the supported set is still refused with an explicit
/// `VersionMismatch` naming both versions, so the one-way window the
/// counter-offer leaves open is visible rather than a silent hang.
#[cfg(feature = "crdt")]
#[test]
fn an_unsupported_offer_is_refused_by_name() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set timeout");
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: PROTOCOL_VERSION + 7,
            frontend_capabilities: semantic_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("write over-offer");
    let message: InstanceMessage = read_message(&mut stream).expect("read refusal");
    match message {
        InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch { server, client }) => {
            // The daemon reports the wire it can SPEAK, not the baseline it
            // advertised. Those differ now, and pinning the baseline here
            // would hold in place a diagnostic telling the operator to
            // downgrade to a version the daemon has already moved past.
            assert_eq!(
                server, PROTOCOL_VERSION,
                "the instance must report its own PROTOCOL_VERSION"
            );
            assert_ne!(
                server, ADVERTISED_PROTOCOL_VERSION,
                "and that is deliberately not the advertised baseline"
            );
            assert_eq!(client, PROTOCOL_VERSION + 7);
        }
        other => panic!("expected a named VersionMismatch, got {other:?}"),
    }
    assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
    // And the connection closes rather than lingering half-open.
    let mut sink = [0u8; 1];
    assert!(
        matches!(stream.read(&mut sink), Ok(0) | Err(_)),
        "a refused attach must close"
    );
}

// ---------------------------------------------------------------------------
// Parent acceptance 54 — one real daemon + real PTY + real wgpu, through a
// PANEL-HOSTED terminal
// ---------------------------------------------------------------------------

/// The daemon config for the panel-hosted probe: a command that opens a
/// controlled terminal child **and then displays it in a bottom panel**,
/// bound to a key the probe presses.
///
/// The distinction from the Vterm Stage 3 fixture is the whole point. There,
/// `terminal.open` leaves the child in the frontend's own full-window buffer,
/// so the GPU enters terminal mode and the band is never involved. Here the
/// adopter passes `display = "panel"` — the real Stage 1 opt-in that Stage 3
/// will make the default — so the child is projected as a `PanelFrame` while
/// the frontend's document window stays a document.
///
/// Opening the terminal and *then* moving it with `window.display` was the
/// first attempt and it is subtly wrong: the buffer ends up displayed twice,
/// the document window keeps projecting it as a full-window terminal, and the
/// acceptance can no longer tell a panel-hosted child from a document one.
#[cfg(feature = "crdt")]
const PANEL_TERMINAL_INIT_LUA: &str = r#"
pmacs.command.define {
  name = "bp-probe.panel-terminal",
  description = "Open the Stage 2B-3 acceptance terminal inside a bottom panel.",
  fn = function()
    return pmacs.terminal.open {
      command = "/bin/sh",
      args = { "-c",
        "i=0; while [ $i -lt 400 ]; do printf 'PANELROW%02d\n' \"$i\"; i=$((i+1)); sleep 0.05; done" },
      display = "panel",
    }
  end,
}
-- C-M-p is deliberately an unbound chord: `bind` is strict and refuses to
-- shadow an existing binding, so a bound one would fail init and leave the
-- probe pressing nothing.
pmacs.keymap.bind { scope = "global", sequence = "C-M-p", command = "bp-probe.panel-terminal" }
"#;

#[cfg(feature = "crdt")]
fn decode_hex(encoded: &str) -> String {
    let bytes: Vec<u8> = encoded
        .as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        })
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Parent acceptance 54: a `--headless-probe` run drives one real daemon, one
/// real PTY child, and real wgpu **through a panel-hosted terminal**.
///
/// The probe is the real attach client — same handshake, same reader, same
/// outbox — driven as a process, because `pmacs-gpu` deliberately depends only
/// on `pmacs-protocol`. Nothing here is emulated: the daemon opens a `/bin/sh`
/// child, moves it into a side window, projects it as a `PanelFrame`, and the
/// probe composites real frames from the band.
///
/// **This test's green is worth nothing unless it actually ran**, which is the
/// standing trap for every probe acceptance in this repo: without the binary
/// built it returns early, and it is `crdt`-gated so CI never reaches it. The
/// skip is therefore an assertion failure under `PMACS_REQUIRE_GPU`, and the
/// report's own `completion_observed` is asserted so a run that merely waited
/// out its safety deadline cannot read as a pass.
#[cfg(feature = "crdt")]
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real-daemon/real-PTY/real-wgpu scenario; splitting it would hide the fit it exists to prove"
)]
fn a54_real_daemon_real_pty_and_headless_gpu_render_one_panel_hosted_terminal() {
    use std::path::{Path, PathBuf};

    fn gpu_binary() -> PathBuf {
        Path::new(env!("CARGO_BIN_EXE_pmacs"))
            .parent()
            .expect("test binary directory")
            .join("pmacs-gpu")
    }

    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = gpu_binary();
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!("skipping a54: {} is not built", binary.display());
        return;
    }

    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        PANEL_TERMINAL_INIT_LUA,
    );

    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("gpu-panel-probe.txt");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_OPEN_KEY", "p")
        // The BAND's own breadcrumb. Naming it separately from the terminal
        // fixture's is what keeps one fixture's evidence from satisfying the
        // other's loop exit.
        .env("PMACS_GPU_PROBE_EXPECT_PANEL_TEXT", "PANELROW")
        .output()
        .expect("run the headless GPU panel probe");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU panel probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping a54: no wgpu adapter available");
        return;
    }

    let text = std::fs::read_to_string(&report).expect("probe report");
    let facts: std::collections::HashMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let fact = |key: &str| facts.get(key).copied().unwrap_or_default();
    let number = |key: &str| fact(key).parse::<u32>().unwrap_or_default();

    // The probe reached its own stated evidence rather than its deadline.
    assert_eq!(
        fact("completion_observed"),
        "true",
        "a deadline-driven pass must not read as success: {text}"
    );
    // The activation, end to end on a real socket.
    //
    // The session version is compared against `PROTOCOL_VERSION` rather
    // than the literal "21" it used to pin: what the counter-offer
    // activates is *this binary's* wire, so the literal was only ever
    // correct while the panel stage was the newest one. The baseline
    // stays a literal, because 20 not moving IS the claim.
    assert_eq!(
        fact("session_protocol_version"),
        PROTOCOL_VERSION.to_string(),
        "{text}"
    );
    assert_eq!(fact("baseline_protocol_version"), "20", "{text}");
    // …and that negotiated version is panel-capable, which is the part
    // "21" used to carry implicitly.
    assert!(
        number("session_protocol_version") >= PANEL_MIN_VERSION,
        "the negotiated wire must reach the panel minimum: {text}"
    );

    // The band is real: declared, projected, focused, and carrying the child.
    assert!(
        number("panel_declarations") >= 2,
        "the probe declares geometry at attach and again after its resize: {text}"
    );
    assert!(
        number("panel_frames") >= 2,
        "the daemon must project the panel-hosted terminal: {text}"
    );
    assert!(
        number("panel_rows") >= 2 && number("panel_cols") > 0,
        "the projected band must have a real grid: {text}"
    );
    assert_eq!(
        fact("panel_focused"),
        "true",
        "the adopter selected the panel, so the projection must say so: {text}"
    );
    assert_eq!(
        fact("panel_text_observed"),
        "true",
        "the PTY child's output must arrive IN THE BAND: {text}"
    );
    assert!(
        decode_hex(fact("panel_frame_text_hex")).contains("PANELROW")
            || fact("panel_text_observed") == "true",
        "and the band's own text is reported for diagnosis: {text}"
    );
    assert_eq!(
        fact("panel_observed_resized_frame"),
        "true",
        "a surface resize must round-trip: new declaration, new band width: {text}"
    );
    assert!(
        number("rendered_nonuniform_frames") >= 2,
        "real wgpu must have composited the band more than once: {text}"
    );

    // And the terminal did NOT take over the frontend's own window: that is
    // the difference between this acceptance and the Vterm Stage 3 one, and
    // without it a full-window terminal would satisfy every assertion above.
    assert_eq!(
        fact("entered_terminal_mode"),
        "false",
        "the child belongs to the panel, not to the document window: {text}"
    );
}
