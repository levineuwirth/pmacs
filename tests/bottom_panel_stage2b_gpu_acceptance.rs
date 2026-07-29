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

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason, Hello, InstanceMessage, Key,
    KeyEvent, Modifiers, SessionBootstrapRequest,
};
use pmacs_protocol::panel::{PANEL_MIN_VERSION, PanelFramePayload};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{
    ADVERTISED_PROTOCOL_VERSION, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
    is_supported_protocol_version, negotiated_session_version, requested_protocol_version,
};

use common::daemon::{TestDaemon, build_default_caps};

/// Opens a bottom panel in whichever frontend pressed the key.
///
/// A real adopter path rather than a Lua eval hook: `pmacs.window.display`
/// with an explicit `side` is exactly what a Stage 1 adopter does, and the
/// acting frontend is the one that sent the key — so whether this produces
/// a side window is precisely the `panel_capable` question.
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

fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

/// One attached session, with the version it actually offered.
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
    let document_still_driven = drain_until(&mut pre_panel.stream, "fallback", |message| {
        assert!(
            !matches!(message, InstanceMessage::PanelFrame(_)),
            "a v20 semantic session must never be sent a panel frame: {message:?}"
        );
        match message {
            InstanceMessage::StyleSpans { buffer_id, .. }
            | InstanceMessage::Decorations { buffer_id, .. }
                if *buffer_id == document =>
            {
                Some(())
            }
            _ => None,
        }
    });
    assert!(
        document_still_driven.is_some(),
        "the pre-panel session must keep being driven as a DOCUMENT — a \
         frontend placed in a side window it cannot render would have an \
         invisible window, so the daemon must take the Stage 1 fallback"
    );
}

// ---------------------------------------------------------------------------
// The negotiation rules themselves
// ---------------------------------------------------------------------------

/// The advertised baseline is a compatibility floor that does NOT move,
/// and the counter-offer is what reaches the current wire.
#[test]
fn the_baseline_stays_and_the_counter_offer_activates() {
    assert_eq!(PROTOCOL_VERSION, 21);
    assert_eq!(
        ADVERTISED_PROTOCOL_VERSION, 20,
        "moving this is the incompatible act the mechanism exists to avoid"
    );
    assert!(PROTOCOL_VERSION > ADVERTISED_PROTOCOL_VERSION);
    assert_eq!(PANEL_MIN_VERSION, PROTOCOL_VERSION);

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
            assert_eq!(server, ADVERTISED_PROTOCOL_VERSION);
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
