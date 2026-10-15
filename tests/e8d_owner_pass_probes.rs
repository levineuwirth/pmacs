// tests/e8d_owner_pass_probes.rs --- the owner's pass over E8d, its probes.

//! The owner's pass over C8d after fix round 1 (PR #372 at `22de659`)
//! found two Mediums in E8d.3, each with a reproduction left for the fix
//! round to commit as a witness. Every row here fails at `22de659` for
//! the pass's reason and passes with its fix.
//!
//! - **Round-trip input.** The three writers of `*help*`, `*workers*`
//!   and `*pmacs-instance*` left their buffers read-only at the daemon
//!   without `set_round_trip_input`, the second layer the invariant
//!   names (`docs/invariants.md`, Generated buffers): a semantic frontend
//!   still applied a typed character to its mirror optimistically and
//!   sent the CRDT op, which the daemon refused as `ReadOnly` --- the
//!   mirror diverged. The first three rows read the seam the daemon
//!   publishes as `DispatchIdle` (`EditorState::dispatch_idle_for`)
//!   after each shipped command, with the existing mechanism as the
//!   positive control. The `gpu_rows` drive the production frontend
//!   itself (`pmacs-gpu --headless-probe`, every key through
//!   `App::apply_keyboard`) against a real daemon: `C-c z` shows the
//!   buffer, `X` is typed, and the band must carry the daemon's
//!   refusal, which only a round-tripped key can reach.
//! - **Edit notification.** The Rust renders' one `Replace` was queued
//!   to replicas and the windows' text views rebuilt, but no window
//!   overlay heard it and the search state kept its accepted ranges
//!   current. The last two rows carry their own overlay and an accepted
//!   search over the first render; the normal generated notification
//!   (`pmacs.buffer.set_generated_contents`, through
//!   `notify_buffer_edit_to_windows`) is the positive control.

#![cfg(unix)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pmacs::buffer::{Buffer, BufferError};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::rope::Edit;
use pmacs::view::View;

mod common;

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn active_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    )
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

// ---------------------------------------------------------------------------
// Round-trip input: the daemon's own `DispatchIdle` seam
// ---------------------------------------------------------------------------

/// After `command` shows its generated buffer, the daemon must tell a
/// semantic frontend that input round-trips (`dispatch_idle_for` false
/// for the frontend showing it), as the dired, git, listview and
/// compile writers arrange through `pmacs.buffer.set_round_trip_input`.
/// The row's positive control is that mechanism, applied by hand.
fn read_only_writer_disables_optimistic_input(command: &str) {
    let s = editor();
    assert!(
        s.dispatch_idle_for(FrontendId::LOCAL),
        "control: *scratch* permits optimistic input"
    );
    exec(&s, &format!("pmacs.command.invoke({command:?})"));
    let name = active_name(&s);
    let id = s.core.borrow().active_buffer_id();
    let read_only = s
        .core
        .borrow()
        .registry
        .borrow()
        .get(id)
        .unwrap()
        .is_read_only();
    assert!(
        read_only,
        "premise: {command} leaves {name} read-only at the daemon"
    );
    let idle_after_write = s.dispatch_idle_for(FrontendId::LOCAL);
    s.core.borrow_mut().set_round_trip_input(id, true);
    assert!(
        !s.dispatch_idle_for(FrontendId::LOCAL),
        "positive control: the existing round-trip mechanism closes the gap"
    );
    assert!(
        !idle_after_write,
        "{command}: {name} is read-only at the daemon, but a semantic frontend is still \
         told it may apply a key to its mirror optimistically; the writer must register \
         round-trip input before a replica edits a mirror the daemon will refuse"
    );
}

/// The Lua writer of `*help*` (`help.list-commands` through
/// `pmacs.editor._show_help`).
#[test]
fn help_disables_optimistic_input() {
    read_only_writer_disables_optimistic_input("help.list-commands");
}

/// The Rust writer of `*workers*` (`editor.list-workers`).
#[test]
fn workers_disable_optimistic_input() {
    read_only_writer_disables_optimistic_input("editor.list-workers");
}

/// The Rust writer of `*pmacs-instance*` (`editor.describe-instance-buffer`).
#[test]
fn instance_disables_optimistic_input() {
    read_only_writer_disables_optimistic_input("editor.describe-instance-buffer");
}

// ---------------------------------------------------------------------------
// Round-trip input: the production GPU frontend against a real daemon
// ---------------------------------------------------------------------------

#[cfg(feature = "crdt")]
mod gpu_rows {
    use std::collections::HashMap;
    use std::path::Path;

    use super::common::daemon::TestDaemon;

    /// Run `pmacs-gpu --headless-probe` under the popup action with
    /// `steps`: every `key:` step through `App::apply_keyboard`, the
    /// production key path, `band:<prefix>` waiting up to five seconds
    /// for the status band to begin with `prefix`, and every
    /// `report:<label>` writing the band as `<label>.status`. Returns
    /// the report and the input-path lines of the frontend's stderr,
    /// where `PMACS_GPU_DEBUG_INPUT=1` logs each key's path (`send_crdt`
    /// for an optimistic op, `send_key` for a round trip). `None` when
    /// skipped (no `pmacs-gpu` binary or no wgpu adapter), which panics
    /// under `PMACS_REQUIRE_GPU` so the gate's GPU leg never passes
    /// vacuously.
    fn run_gpu_probe(
        daemon: &TestDaemon,
        steps: &str,
    ) -> Option<(HashMap<String, String>, String)> {
        let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
        let binary = Path::new(env!("CARGO_BIN_EXE_pmacs"))
            .parent()
            .expect("test binary directory")
            .join("pmacs-gpu");
        if !binary.exists() {
            assert!(
                !required,
                "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
                binary.display()
            );
            eprintln!("skipping the GPU probe: {} is not built", binary.display());
            return None;
        }
        let report = daemon
            .socket_path()
            .parent()
            .expect("socket parent")
            .join("owner-pass.report");
        let output = std::process::Command::new(&binary)
            .arg("--headless-probe")
            .arg(daemon.socket_path())
            .arg(&report)
            .env("PMACS_GPU_PROBE_TYPE_TEXT", steps)
            .env("PMACS_GPU_PROBE_ACTION", "popup")
            .env("PMACS_GPU_PROBE_DEADLINE_MS", "60000")
            .env("PMACS_GPU_DEBUG_INPUT", "1")
            .output()
            .expect("run the headless GPU probe");
        // Keep the input path's lines alone: the rest of the
        // frontend's debug stream is a cursor trace, hundreds of lines.
        let stderr = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|l| l.contains("send_") || l.contains("defer_key") || l.contains("probe"))
            .collect::<Vec<_>>()
            .join("\n");
        if !output.status.success() {
            let no_adapter = output.status.code() == Some(3);
            assert!(
                no_adapter && !required,
                "headless GPU probe failed (status {:?}):\n{stderr}",
                output.status.code()
            );
            eprintln!("skipping the GPU probe: no wgpu adapter available");
            return None;
        }
        let text = std::fs::read_to_string(&report).expect("read the probe report");
        let report = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        Some((report, stderr))
    }

    /// A daemon whose config binds `C-c z` to `command`; the GPU
    /// frontend presses it, then types `X` into the buffer it showed.
    /// With round-trip input registered the key reaches the daemon,
    /// which refuses the insert on the read-only buffer and says so in
    /// the band; without it the frontend applies `X` to its mirror and
    /// sends a CRDT op the daemon drops, and the band says nothing.
    fn gpu_round_trips_a_key_into(command: &str, name: &str) {
        let init = format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.command.define {{ name = 'owner.open', \
             description = 'Show the generated buffer under review', \
             fn = function() pmacs.command.invoke({command:?}) end }}\n\
             pmacs.keymap.bind {{ scope = 'global', sequence = 'C-c z', command = 'owner.open' }}\n"
        );
        let daemon =
            TestDaemon::spawn_with_env_and_init(&[("PMACS_INSTANCE_SEMANTIC_RENDER", "1")], &init);
        let steps = "key:C-c key:z quiet:500 report:opened key:X band:insert report:typed";
        let Some((report, stderr)) = run_gpu_probe(&daemon, steps) else {
            return;
        };
        assert_eq!(
            report.get("ready").map(String::as_str),
            Some("true"),
            "premise: the probe attached and settled; report {report:?}"
        );
        let typed = report.get("typed.status").cloned().unwrap_or_default();
        let refusal = format!("insert failed: buffer `{name}`");
        assert!(
            typed.contains(&refusal),
            "{command}: after `C-c z` showed {name}, the typed X must round-trip to the daemon \
             and be refused there, which the band says; the band read {typed}\n\
             frontend stderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("send_crdt:"),
            "{command}: the frontend applied X to its mirror and sent a CrdtOp the daemon \
             refuses as ReadOnly, so the mirror diverged; frontend stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("send_key (intercepted): Char('X')"),
            "{command}: X was expected to round-trip as a key; frontend stderr:\n{stderr}"
        );
    }

    #[test]
    fn gpu_help_round_trips_a_typed_key() {
        gpu_round_trips_a_key_into("help.list-commands", "*help*");
    }

    #[test]
    fn gpu_workers_round_trips_a_typed_key() {
        gpu_round_trips_a_key_into("editor.list-workers", "*workers*");
    }

    #[test]
    fn gpu_instance_round_trips_a_typed_key() {
        gpu_round_trips_a_key_into("editor.describe-instance-buffer", "*pmacs-instance*");
    }
}

// ---------------------------------------------------------------------------
// Edit notification: overlays and search after a Rust render
// ---------------------------------------------------------------------------

/// A window overlay counting the edits it is told of, as
/// `BufferStyleSpanTranslator` carries spans: the object the Rust
/// renders' fan-out must reach.
struct EditWitness(Arc<AtomicUsize>);

impl View for EditWitness {
    fn on_edit(&mut self, _: &Buffer, _: &Edit) -> Result<(), BufferError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "owner-pass-edit-witness"
    }
}

/// The Rust help renderer's second render replaces `*help*` whole: the
/// window's overlay must hear the one `Replace`, and the search ranges
/// accepted over the first render must go stale, as they do for every
/// edit through `notify_buffer_edit`.
#[test]
fn rust_help_refresh_notifies_windows_and_invalidates_search() {
    let s = editor();
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.help.show_command('cursor.left'))",
    );
    let id = s.core.borrow().active_buffer_id();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let mut core = s.core.borrow_mut();
        core.active_window_mut()
            .push_overlay(Box::new(EditWitness(calls.clone())));
        core.search_begin(true, false);
        for c in "cursor.left".chars() {
            core.search_input_char(c);
        }
        assert!(
            core.search_match_summary().1 > 0,
            "premise: the search matched the first render"
        );
        core.search_finish(true);
        assert!(
            !core.search_store.lock().unwrap().is_stale(id),
            "premise: the accepted ranges are current"
        );
    }
    exec(&s, "pmacs.help.show_command('cursor.right')");
    let stale = s.core.borrow().search_store.lock().unwrap().is_stale(id);
    let count = calls.load(Ordering::SeqCst);
    eprintln!("Rust help refresh: overlay calls={count}, search stale={stale}");
    assert!(
        active_text(&s).contains("cursor.right"),
        "premise: the second render replaced the text"
    );
    exec(
        &s,
        "local b = pmacs.window.buffer(); pmacs.buffer.set_generated_contents(b, b:slice(0, b:len()))",
    );
    assert!(
        s.core.borrow().search_store.lock().unwrap().is_stale(id),
        "positive control: the normal generated-buffer notification marks the search stale"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        count + 1,
        "positive control: the normal notification reaches the overlay"
    );
    assert!(
        stale,
        "a whole-buffer replacement must invalidate the accepted search ranges"
    );
    assert_eq!(
        count, 1,
        "the render's one Replace must reach the window's overlays"
    );
}

/// The shipped `editor.list-workers`, rendered twice: a search accepted
/// over the first listing, whose text the second removes, must be
/// marked stale by the refresh.
#[test]
fn shipped_workers_refresh_invalidates_search() {
    let s = editor();
    exec(&s, "pmacs.command.invoke('editor.list-workers')");
    let id = s.core.borrow().active_buffer_id();
    {
        let mut core = s.core.borrow_mut();
        core.search_begin(true, false);
        for c in "(no active jobs)".chars() {
            core.search_input_char(c);
        }
        assert_eq!(
            core.search_match_summary().1,
            1,
            "premise: one literal match in the first render"
        );
        core.search_finish(true);
    }
    s.async_runtime.dispatch_sleep(10_000, None);
    exec(&s, "pmacs.command.invoke('editor.list-workers')");
    let text = active_text(&s);
    assert!(
        !text.contains("(no active jobs)"),
        "premise: the new render removed the searched text; got {text:?}"
    );
    let stale = s.core.borrow().search_store.lock().unwrap().is_stale(id);
    exec(
        &s,
        "local b = pmacs.window.buffer(); pmacs.buffer.set_generated_contents(b, b:slice(0, b:len()))",
    );
    assert!(
        s.core.borrow().search_store.lock().unwrap().is_stale(id),
        "positive control: the normal notification invalidates the stale matches"
    );
    assert!(
        stale,
        "a shipped workers refresh must invalidate search matches computed for its old contents"
    );
}
