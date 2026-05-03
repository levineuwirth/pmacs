// m5_6_acceptance.rs --- Acceptance suite for M5.6 (AttachTarget +
// pmacs.attach Lua surface + describe-instance + post-init dispatcher).

//! End-to-end acceptance tests for T M5.6 (`AttachTarget` enum, the
//! `pmacs.attach{...}` / `pmacs.current_attachment` / `pmacs.instance.*`
//! Lua surface, the describe-instance commands, and the post-init
//! attach dispatcher).
//!
//! The seven acceptance scenarios from the M5.6 plan:
//!
//! 1. **CLI attach** — `pmacs --attach --socket NAME` connects a local
//!    frontend to a running daemon. **Covered by M5.5 acceptance**;
//!    see `tests/m5_5_acceptance.rs::attach_send_key_receive_cell_response`
//!    and `tests/m5_5_acceptance.rs::clean_detach_then_reattach`.
//! 2. **init.lua attach (`LocalSocket`) hands off** — an `init.lua`
//!    containing `pmacs.attach{ target = "local:..." }` flows through
//!    [`pmacs::config::load_user_config_at`], populates
//!    [`pmacs::lua_bindings::RequestedAttach`], and the post-init
//!    dispatcher returns `RunAttachLocalSocket(p)`. Tested here as
//!    [`init_lua_local_socket_target_dispatches_to_attach_run`].
//! 3. **Mid-session attach error** — calling `pmacs.attach` after the
//!    init-complete flag flips raises [`pmacs::lua_bindings::BindingError::InitOnlyApi`]
//!    with a message pointing at the CLI workaround. Tested here as
//!    [`attach_called_after_init_complete_yields_init_only_error`]
//!    (in addition to the lib-level test
//!    `lua_bindings::tests::attach_after_init_complete_errors_with_workaround_pointer`).
//! 4. **Echo describe** — `M-x editor.describe-instance` populates the
//!    status row with `format_echo_line`. **Covered in lib tests:**
//!    `editor::tests::editor_describe_instance_echoes_status_line` and
//!    `instance_buffer::tests::echo_*`.
//! 5. **Buffer describe** — `M-x editor.describe-instance-buffer`
//!    switches the active window to `*pmacs-instance*` and binds
//!    buffer-local `q` to `buffer.kill-this`. **Covered in lib tests:**
//!    `editor::tests::editor_describe_instance_buffer_switches_and_binds_q`
//!    and `editor::tests::editor_describe_instance_buffer_q_kills_the_buffer`.
//! 6. **SSH dispatch (post-M5.7e)** — an `init.lua` containing
//!    `pmacs.attach{ target = "ssh:host" }` parses + validates locally
//!    and the post-init dispatcher routes it to
//!    `AttachDispatch::RunAttachSsh(target)`. Tested here as
//!    [`init_lua_ssh_target_dispatches_to_run_attach_ssh`]. (Pre-M5.7e
//!    this scenario produced `DeferredInV01 { milestone = "M5.7" }`.)
//! 7. **`AlreadyAttached`** — a daemon with one frontend rejects a
//!    second connection with `Goodbye(AlreadyAttached)`. **Covered by
//!    M5.5 acceptance**; see
//!    `tests/m5_5_acceptance.rs::attach_send_key_receive_cell_response`
//!    (the second-attach branch).
//!
//! # Why some scenarios are cited and not duplicated
//!
//! Scenarios 1 and 7 require a real subprocess + socket and are
//! exercised end-to-end by `tests/m5_5_acceptance.rs`. Re-running
//! them under the M5.6 banner would be wasteful.
//!
//! Scenarios 4 and 5 require an [`pmacs::editor::EditorState`] with
//! the full builtin command surface loaded. The lib-level tests live
//! inside the `editor` and `instance_buffer` modules, where
//! `cfg(test)` short-circuits the user-config load so the test
//! environment doesn't pick up the developer's real `init.lua`.
//! Reproducing that environment from an integration test would
//! require mutating `XDG_CONFIG_HOME` (which pmacs forbids — `set_var`
//! is `unsafe` in edition 2024 and would race in parallel `cargo
//! test`); the lib-level coverage is the right place.
//!
//! Scenarios 2, 3, and 6 *are* end-to-end here: they drive a real
//! `init.lua` file through the real `load_user_config_at` →
//! `RequestedAttach` → `dispatch_attach` chain, which is the unique
//! integration story M5.6 introduces.

use std::path::PathBuf;

use pmacs::attach_dispatch::{AttachDispatch, dispatch_attach};
use pmacs::config::load_user_config_at;
use pmacs::lua::LuaHost;

// ---------------------------------------------------------------------------
// Scenario 2: init.lua attach (LocalSocket) hands off to attach mode.
// ---------------------------------------------------------------------------

#[test]
fn init_lua_local_socket_target_dispatches_to_attach_run() {
    // Stage an init.lua that requests a local-socket attach. The
    // path is fictitious — the dispatcher decides what to do based
    // on the *kind* of target, not on whether the socket exists.
    // Connection-attempt-time errors are M5.7's territory.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.attach { target = "local:/run/pmacs/work.sock" }"#,
    )
    .expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");

    // Mirror what `EditorState::new` does in production: run the
    // config file, then flip the init-complete gate. Tests can't go
    // through `EditorState::new` here because the integration-test
    // build doesn't have `cfg(test)` set on the lib; the lib's own
    // gate against picking up the developer's real init.lua only
    // applies inside the lib's test target.
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    assert!(
        host.errors().is_empty(),
        "init.lua produced errors: {:?}",
        host.errors()
    );

    let requested = host
        .take_requested_attach()
        .expect("init.lua recorded an attach request");

    match dispatch_attach(Some(requested)) {
        AttachDispatch::RunAttachLocalSocket(path) => {
            assert_eq!(path, PathBuf::from("/run/pmacs/work.sock"));
        }
        other => panic!("expected RunAttachLocalSocket, got {other:?}"),
    }
}

#[test]
fn no_init_lua_attach_call_dispatches_to_run_local() {
    // Symmetric to the previous test: when init.lua does *not* call
    // `pmacs.attach`, the dispatcher falls through to RunLocal.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(dir.path().join("init.lua"), "-- no attach here\n").expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    assert!(
        host.take_requested_attach().is_none(),
        "no attach call → empty slot"
    );
    assert_eq!(dispatch_attach(None), AttachDispatch::RunLocal);
}

// ---------------------------------------------------------------------------
// Scenario 3: mid-session attach error.
// ---------------------------------------------------------------------------

#[test]
fn attach_called_after_init_complete_yields_init_only_error() {
    // A user invoking `pmacs.attach` from a hook or M-x command (i.e.
    // after init has finished) must hit the InitOnlyApi gate. The
    // error message must name a workaround the user can act on
    // ("restart with the equivalent CLI flag").
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.set_init_complete();

    let err = host
        .eval(Some("test"), r#"pmacs.attach { target = "local:/x.sock" }"#)
        .expect_err("post-init pmacs.attach must error");
    let msg = err.to_string();
    assert!(msg.contains("pmacs.attach"), "{msg}");
    assert!(
        msg.contains("init.lua"),
        "error must name the right phase: {msg}"
    );
    assert!(
        msg.contains("CLI flag"),
        "error must point at the workaround: {msg}"
    );
    assert!(
        host.take_requested_attach().is_none(),
        "no request should have been recorded after a gated call"
    );
}

#[test]
fn attach_called_during_init_then_dispatch_succeeds() {
    // Companion test: confirm that the gate is *not* punitive — an
    // attach call from init.lua (i.e. before set_init_complete) does
    // populate the slot, and the post-init dispatch picks it up.
    // Together with the previous test this pins both halves of the
    // gate: closed after init, open during it.
    let mut host = LuaHost::new().expect("LuaHost::new");
    host.eval(
        Some("test:init.lua"),
        r#"pmacs.attach { target = "local:/run/pmacs/early.sock" }"#,
    )
    .expect("during-init pmacs.attach must succeed");
    host.set_init_complete();
    let target = host.take_requested_attach().expect("slot populated");
    match dispatch_attach(Some(target)) {
        AttachDispatch::RunAttachLocalSocket(p) => {
            assert_eq!(p, PathBuf::from("/run/pmacs/early.sock"));
        }
        other => panic!("expected RunAttachLocalSocket, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Scenario 6: SSH stub error at dispatch.
// ---------------------------------------------------------------------------

#[test]
fn init_lua_ssh_target_dispatches_to_run_attach_ssh() {
    // M5.7e activates SSH: an init.lua-recorded SSH target now
    // produces `RunAttachSsh(target)` from the post-init dispatcher,
    // not the deferral that M5.6 shipped. The target round-trips
    // through the dispatcher unchanged so `attach::run_attach_ssh`
    // can build the SSH command from the same parsed fields.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.attach { target = "ssh:lev@example.com" }"#,
    )
    .expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    assert!(
        host.errors().is_empty(),
        "ssh target should parse cleanly during init; errors: {:?}",
        host.errors()
    );

    let requested = host
        .take_requested_attach()
        .expect("init.lua recorded the ssh attach");

    match dispatch_attach(Some(requested)) {
        AttachDispatch::RunAttachSsh(pmacs::protocol::AttachTarget::Ssh {
            host,
            user,
            instance_name,
        }) => {
            assert_eq!(host, "example.com");
            assert_eq!(user, Some("lev".into()));
            assert!(instance_name.is_none());
        }
        other => panic!("expected RunAttachSsh(Ssh), got {other:?}"),
    }
}

#[test]
fn init_lua_kwargs_form_ssh_dispatches_to_run_attach_ssh() {
    // The kwargs form (`{ kind = "ssh", host = ..., user = ... }`)
    // routes through the same dispatch as the target-string form
    // and lands at `RunAttachSsh` post-M5.7e.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.attach { kind = "ssh", host = "mac-studio", user = "lev" }"#,
    )
    .expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    let target = host.take_requested_attach().expect("slot populated");
    match dispatch_attach(Some(target)) {
        AttachDispatch::RunAttachSsh(pmacs::protocol::AttachTarget::Ssh { host, user, .. }) => {
            assert_eq!(host, "mac-studio");
            assert_eq!(user, Some("lev".into()));
        }
        other => panic!("expected RunAttachSsh(Ssh), got {other:?}"),
    }
}

#[test]
fn init_lua_tls_target_defers_to_v0_2() {
    // TLS shares the deferral pathway but with milestone "v0.2"
    // rather than "M5.7" — that's where TLS lands per the project
    // roadmap. The exact message text is part of the user contract.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.attach { target = "tls:example.com:9999#/etc/p.crt" }"#,
    )
    .expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    let target = host.take_requested_attach().expect("slot populated");
    let dispatch = dispatch_attach(Some(target));
    match &dispatch {
        AttachDispatch::DeferredInV01 { kind, milestone } => {
            assert_eq!(*kind, "tls");
            assert_eq!(*milestone, "v0.2");
        }
        other => panic!("expected DeferredInV01, got {other:?}"),
    }
    assert!(dispatch.deferred_message().unwrap().contains("v0.2"));
}

#[test]
fn init_lua_custom_target_defers_to_v0_2() {
    // Symmetric to the TLS case: custom transports also defer to
    // v0.2. Pinning all three deferred kinds (ssh, tls, custom)
    // makes a future regression in any of the three loud, not silent.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.attach { kind = "custom", command = { "ssh", "host", "pmacs", "--daemon" } }"#,
    )
    .expect("write init.lua");

    let mut host = LuaHost::new().expect("LuaHost::new");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    let target = host.take_requested_attach().expect("slot populated");
    match dispatch_attach(Some(target)) {
        AttachDispatch::DeferredInV01 { kind, milestone } => {
            assert_eq!(kind, "custom");
            assert_eq!(milestone, "v0.2");
        }
        other => panic!("expected DeferredInV01, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Defensive: the slot is consumed exactly once.
// ---------------------------------------------------------------------------

#[test]
fn take_requested_attach_consumes_slot_exactly_once() {
    // The dispatcher's contract is "consume the slot." Verify a
    // second take() yields None — the dispatcher is single-shot, not
    // a peek.
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("init.lua"),
        r#"pmacs.attach { target = "local:/x.sock" }"#,
    )
    .unwrap();

    let mut host = LuaHost::new().expect("LuaHost::new");
    load_user_config_at(&mut host, dir.path());
    host.set_init_complete();

    let first = host.take_requested_attach();
    let second = host.take_requested_attach();
    assert!(first.is_some(), "first take should observe the request");
    assert!(
        second.is_none(),
        "second take must be empty (single-shot semantics)"
    );
}
