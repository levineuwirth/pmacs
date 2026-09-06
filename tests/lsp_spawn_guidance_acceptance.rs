// tests/lsp_spawn_guidance_acceptance.rs --- journey Stage 1b-2.

//! `COHERENCE.md` §1.2's canonical silence: a preconfigured language
//! server that is not installed used to fail with no status message, no
//! record, and no modeline marker, while tree-sitter highlighting kept
//! working and masked it.
//!
//! Pins are labelled **N** (new behaviour, must fail on full revert) or
//! **P** (preservation, falsified by a named targeted mutation), per
//! the archived journey-stage1b2-lsp-guidance framing §4.
//!
//! **Every fixture points the config at a path that does not exist**,
//! rather than relying on a real server's absence — a developer with
//! `rust-analyzer` installed must get the same result as CI. Each
//! fixture asserts that precondition, because a fixture that
//! accidentally named a real binary would make every absence assertion
//! here vacuous.

use std::path::{Path, PathBuf};

use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};
use tempfile::TempDir;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn status(state: &EditorState) -> String {
    state.core.borrow().status.clone()
}

fn lua_str(s: &str) -> String {
    format!("{s:?}")
}

/// A command path that cannot exist. Asserted, not assumed: if this ever
/// resolved, every "did not start" assertion below would be vacuous.
fn absent_command(dir: &Path, name: &str) -> String {
    let p = dir.join("no-such-bin").join(name);
    assert!(
        !p.exists(),
        "fixture precondition: {} must not exist",
        p.display()
    );
    p.display().to_string()
}

/// Point the default `rust` server at `command`.
fn configure_rust(state: &EditorState, command: &str) {
    exec(
        state,
        &format!(
            "pmacs.lsp.config.rust = {{ command = {} }}",
            lua_str(command)
        ),
    );
}

/// A Cargo project, so `key_uri` is non-nil (root detected by marker).
fn cargo_project() -> TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    td
}

fn write_rs(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"fn main() {}\n").expect("write rs");
    p
}

/// Open a file the way `buffer.after-load` sees it.
fn open(state: &EditorState, path: &Path) {
    exec(
        state,
        &format!(
            "pmacs.buffer.find_or_open({})",
            lua_str(&path.display().to_string())
        ),
    );
}

fn editor_for(dir: &Path) -> EditorState {
    let state = EditorState::new_with_roots(&crate::iso::roots());
    // Clamp detection so a stray marker above the tempdir cannot leak in.
    exec(
        &state,
        &format!(
            "pmacs.project.set_search_boundary({})",
            lua_str(&dir.display().to_string())
        ),
    );
    state
}

/// The `lsp` modeline segment's text for the active buffer, or `None`.
fn lsp_segment(state: &EditorState) -> Option<String> {
    let outcome = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        },
    );
    let StatuslineEvaluationOutcome::Ready(windows) = outcome.outcome else {
        return None;
    };
    // Found by face rather than by name: `EvaluatedStatuslineSegment`
    // carries `provider_id`, not the registration's name, and the face
    // is the stable public identity of this segment.
    windows
        .into_iter()
        .flat_map(|w| w.right)
        .find(|s| s.face == "ui.modeline.lsp")
        .map(|s| s.text)
}

fn clear_status(state: &EditorState) {
    exec(state, "pmacs.editor.set_status('')");
}

fn failure_count(state: &EditorState) -> i64 {
    eval(state, "return #pmacs.lsp.spawn_failures()")
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

/// **N** (framing acceptance 2) — the failure is reported, through the
/// real `buffer.after-load` path, naming the command, the language and
/// the underlying error.
#[test]
fn j1b2_a_missing_server_is_reported_with_guidance() {
    let td = cargo_project();
    let state = editor_for(td.path());
    let cmd = absent_command(td.path(), "rust-analyzer");
    configure_rust(&state, &cmd);
    open(&state, &write_rs(td.path(), "a.rs"));

    let msg = status(&state);
    assert!(msg.contains(&cmd), "names the command; got {msg:?}");
    assert!(msg.contains("rust"), "names the language; got {msg:?}");
    assert!(
        msg.contains("No such file") || msg.contains("os error 2"),
        "passes the underlying error through; got {msg:?}"
    );
    assert!(
        msg.contains("init.lua"),
        "says what the user can do; got {msg:?}"
    );
}

/// **N** (3) — reported once per `(language, key_uri, command)`. The
/// spawn is still attempted on the second open; only the message is
/// suppressed.
#[test]
fn j1b2_a_repeat_failure_in_the_same_project_is_not_reannounced() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    open(&state, &write_rs(td.path(), "a.rs"));
    assert!(!status(&state).is_empty(), "first open reports");

    clear_status(&state);
    open(&state, &write_rs(td.path(), "b.rs"));
    assert_eq!(
        status(&state),
        "",
        "a second file in the same project must not re-announce"
    );
    // The failure is still current — the memo is on the report, not the
    // failure — so the surface still knows about it.
    assert_eq!(failure_count(&state), 1);
}

/// **N** (4) — the markerless case shares one memo, because it shares
/// one server. Two loose files in *different* directories both resolve
/// `key_uri = nil`.
///
/// Falsified by keying the memo on the resolved root, which reports
/// twice. This is the pin where the root and the affinity key differ.
#[test]
fn j1b2_markerless_files_in_different_directories_report_once() {
    let td = tempfile::tempdir().expect("tempdir");
    let one = td.path().join("one");
    let two = td.path().join("two");
    std::fs::create_dir_all(&one).expect("mkdir one");
    std::fs::create_dir_all(&two).expect("mkdir two");
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));

    open(&state, &write_rs(&one, "a.rs"));
    assert!(!status(&state).is_empty(), "first markerless open reports");

    clear_status(&state);
    open(&state, &write_rs(&two, "b.rs"));
    assert_eq!(
        status(&state),
        "",
        "a markerless file elsewhere shares the same (language, nil) key"
    );
    assert_eq!(
        failure_count(&state),
        1,
        "and therefore one failure, not two"
    );
}

/// **N** (5) — a genuinely different root reports again.
#[test]
fn j1b2_a_different_project_root_reports_again() {
    let outer = tempfile::tempdir().expect("tempdir");
    let a = outer.path().join("a");
    let b = outer.path().join("b");
    std::fs::create_dir_all(&a).expect("mkdir a");
    std::fs::create_dir_all(&b).expect("mkdir b");
    std::fs::write(a.join("Cargo.toml"), b"[package]\nname=\"a\"\n").expect("w");
    std::fs::write(b.join("Cargo.toml"), b"[package]\nname=\"b\"\n").expect("w");
    let state = editor_for(outer.path());
    configure_rust(&state, &absent_command(outer.path(), "rust-analyzer"));

    open(&state, &write_rs(&a, "a.rs"));
    clear_status(&state);
    open(&state, &write_rs(&b, "b.rs"));
    assert!(
        !status(&state).is_empty(),
        "a different detected root is a different affinity"
    );
    assert_eq!(failure_count(&state), 2);
}

/// **N** (6) — a changed command reports again, because the reported
/// identity includes it.
#[test]
fn j1b2_repointing_at_another_missing_command_reports_again() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    open(&state, &write_rs(td.path(), "a.rs"));

    clear_status(&state);
    let second = absent_command(td.path(), "rust-analyzer-2");
    configure_rust(&state, &second);
    open(&state, &write_rs(td.path(), "b.rs"));
    let msg = status(&state);
    assert!(
        msg.contains(&second),
        "a different missing executable is a new failure; got {msg:?}"
    );
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

/// **N** (7, 8) — the memo is on the report, not the failure: the spawn
/// is retried, so a resolvable command recovers with nothing to
/// invalidate, and both surfaces go quiet.
#[test]
fn j1b2_recovery_clears_the_failure_surface() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    open(&state, &write_rs(td.path(), "a.rs"));
    assert_eq!(failure_count(&state), 1, "precondition: a failure exists");

    // `/bin/sh` exists and is spawnable; it is not an LSP server, but
    // this pin is about the spawn succeeding, not about initialize.
    configure_rust(&state, "/bin/sh");
    open(&state, &write_rs(td.path(), "b.rs"));
    assert_eq!(
        failure_count(&state),
        0,
        "a successful spawn for the same affinity clears the failure"
    );
}

/// **N** (9) — recovery reaches **every** buffer sharing the affinity,
/// not just the one that succeeded.
///
/// Asserted on A, deliberately: a version of this pin that checked B
/// passes on the broken implementation, where only the succeeding
/// buffer's projection is cleared and A keeps rendering `LSP:!` while
/// `lsp.status` reports nothing wrong.
#[test]
fn j1b2_recovery_reaches_every_buffer_sharing_the_affinity() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    let a = write_rs(td.path(), "a.rs");
    open(&state, &a);
    assert_eq!(
        lsp_segment(&state).as_deref(),
        Some("LSP:!"),
        "precondition: A is marked failed"
    );

    configure_rust(&state, "/bin/sh");
    open(&state, &write_rs(td.path(), "b.rs"));

    // Back to A — the buffer that never succeeded itself.
    open(&state, &a);
    assert_ne!(
        lsp_segment(&state).as_deref(),
        Some("LSP:!"),
        "A must stop claiming a failure that the shared affinity has resolved"
    );
}

// ---------------------------------------------------------------------------
// Modeline
// ---------------------------------------------------------------------------

/// **N** (13) — the modeline distinguishes "failed" from "not
/// applicable". Both halves asserted: a pin checking only the failing
/// case passes if the segment renders `!` unconditionally.
#[test]
fn j1b2_the_modeline_distinguishes_failed_from_unsupported() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));

    open(&state, &write_rs(td.path(), "a.rs"));
    assert_eq!(
        lsp_segment(&state).as_deref(),
        Some("LSP:!"),
        "a source file whose server failed says so"
    );

    let txt = td.path().join("notes.txt");
    std::fs::write(&txt, b"plain\n").expect("write txt");
    open(&state, &txt);
    assert_eq!(
        lsp_segment(&state),
        None,
        "a file with no configured server renders nothing at all"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// **N** (14) — a killed buffer's projection is removed.
///
/// Without the `pmacs.buffer.on_removed` registration the entry outlives
/// its buffer for the session, and nothing else can reach it: the LSP
/// resource reconciliation iterates `attachments`, and a failed buffer
/// has none by construction.
#[test]
fn j1b2_a_killed_buffer_drops_its_failure_projection() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    let a = write_rs(td.path(), "a.rs");
    open(&state, &a);
    assert_eq!(lsp_segment(&state).as_deref(), Some("LSP:!"));

    let before: i64 = eval(&state, "return #pmacs.buffer.list()");
    exec(&state, "pmacs.buffer.remove(pmacs.window.buffer())");
    let after: i64 = eval(&state, "return #pmacs.buffer.list()");
    assert!(
        after < before,
        "precondition: the buffer really was removed"
    );

    // Re-open the same path: a fresh buffer that has never failed must
    // not inherit a marker, and the stale projection must not be what
    // answers for it.
    open(&state, &a);
    configure_rust(&state, "/bin/sh");
    open(&state, &write_rs(td.path(), "b.rs"));
    assert_eq!(
        failure_count(&state),
        0,
        "the sweep still terminates and clears with a killed buffer in play"
    );
}

/// **N** (15) — a rename clears the projection rather than leaving an
/// old-path failure attached to a changed buffer.
#[test]
fn j1b2_a_rename_clears_the_failure_projection() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    let a = write_rs(td.path(), "a.rs");
    open(&state, &a);
    assert_eq!(lsp_segment(&state).as_deref(), Some("LSP:!"));

    let renamed = td.path().join("renamed.rs");
    exec(
        &state,
        &format!(
            "pmacs.hook.run('resource.renamed', {}, {})",
            lua_str(&a.display().to_string()),
            lua_str(&renamed.display().to_string())
        ),
    );
    assert_ne!(
        lsp_segment(&state).as_deref(),
        Some("LSP:!"),
        "after a rename the projection no longer describes this buffer"
    );
}

/// **N** (16) — a delete clears it too.
#[test]
fn j1b2_a_delete_clears_the_failure_projection() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    let a = write_rs(td.path(), "a.rs");
    open(&state, &a);
    assert_eq!(lsp_segment(&state).as_deref(), Some("LSP:!"));

    exec(
        &state,
        &format!(
            "pmacs.hook.run('resource.deleted', {})",
            lua_str(&a.display().to_string())
        ),
    );
    assert_ne!(
        lsp_segment(&state).as_deref(),
        Some("LSP:!"),
        "a deleted path leaves no failure to project"
    );
}

// ---------------------------------------------------------------------------
// The *lsp* panel
// ---------------------------------------------------------------------------

fn named_text(state: &EditorState, name: &str) -> String {
    eval(
        state,
        &format!(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == {name:?} then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#
        ),
    )
}

/// **N** (10) — `M-x lsp.status` renders both sections. Asserts content
/// produced, not that a buffer exists.
#[test]
fn j1b2_lsp_status_renders_failures_and_servers() {
    let td = cargo_project();
    let state = editor_for(td.path());
    let cmd = absent_command(td.path(), "rust-analyzer");
    configure_rust(&state, &cmd);
    open(&state, &write_rs(td.path(), "a.rs"));

    exec(&state, "pmacs.command.invoke('lsp.status')");
    let text = named_text(&state, "*lsp*");
    assert!(
        text.contains(&cmd),
        "the failure section names the command; got:\n{text}"
    );
    assert!(
        text.contains("Servers:"),
        "and `status_buffer_text` still renders beneath it; got:\n{text}"
    );
}

/// **N** (11) — `g` refreshes. The **reattach is load-bearing**: making
/// the command resolvable changes no state on its own, since `failures`
/// is cleared by a successful spawn.
#[test]
fn j1b2_g_refreshes_the_lsp_panel_after_recovery() {
    let td = cargo_project();
    let state = editor_for(td.path());
    let cmd = absent_command(td.path(), "rust-analyzer");
    configure_rust(&state, &cmd);
    open(&state, &write_rs(td.path(), "a.rs"));
    exec(&state, "pmacs.command.invoke('lsp.status')");
    assert!(named_text(&state, "*lsp*").contains(&cmd));

    // Resolve AND reattach, then refresh in place.
    configure_rust(&state, "/bin/sh");
    open(&state, &write_rs(td.path(), "b.rs"));
    exec(&state, "pmacs.command.invoke('lsp.status')");
    exec(&state, "pmacs.command.invoke('listview.refresh')");
    assert!(
        !named_text(&state, "*lsp*").contains(&cmd),
        "g must re-render, not leave the panel stale"
    );
}

/// **N** (12) — a foreign `*lsp*` buffer is never adopted. This is
/// `listview.open`'s guarantee, pinned rather than assumed because it is
/// exactly what a hand-rolled panel loses.
#[test]
fn j1b2_a_foreign_lsp_buffer_is_not_adopted() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    exec(
        &state,
        "local b = pmacs.buffer.create('*lsp*') b:insert(0, 'user bytes')",
    );

    exec(&state, "pmacs.command.invoke('lsp.status')");
    assert_eq!(
        named_text(&state, "*lsp*"),
        "user bytes",
        "the user's buffer is untouched"
    );
    assert!(
        named_text(&state, "*lsp*<2>").contains("Servers:"),
        "and the panel opens beside it"
    );
}

// ---------------------------------------------------------------------------
// Preservation
// ---------------------------------------------------------------------------

/// **P** (20) — a failed spawn never fabricates an attachment record.
/// Targeted mutation: recording the failure in `attachments`, which
/// would route requests at a server that does not exist.
#[test]
fn j1b2_preservation_a_failed_spawn_creates_no_attachment() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, &absent_command(td.path(), "rust-analyzer"));
    open(&state, &write_rs(td.path(), "a.rs"));

    assert!(
        eval::<bool>(&state, "return pmacs.lsp.attachment_for_request() == nil"),
        "no request may be issued against a server that failed to start"
    );
}

/// **P** (18) — a spawnable server is unaffected: it attaches, and the
/// modeline reports the server rather than a failure.
#[test]
fn j1b2_preservation_a_spawnable_server_still_attaches() {
    let td = cargo_project();
    let state = editor_for(td.path());
    configure_rust(&state, "/bin/sh");
    open(&state, &write_rs(td.path(), "a.rs"));

    let seg = lsp_segment(&state);
    assert!(
        seg.is_some() && seg.as_deref() != Some("LSP:!"),
        "a started server keeps its own label; got {seg:?}"
    );
    assert_eq!(failure_count(&state), 0);
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
