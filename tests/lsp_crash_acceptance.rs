//! E5.3 --- a crashed language server is announced.
//!
//! Before this phase a `crashed` event was drained and dropped in
//! `lsp.lua` (the arm list had no case for it), the modeline showed the
//! state's own label, and `*lsp*` listed only start failures. Now the
//! crash reaches `pmacs.error` (so `*errors*`, the status line and the
//! unread mark), the buffer's modeline segment reads `LSP:!`, and
//! `*lsp*` opens with a "Crashed" section naming the server and the
//! reason. A stale attachment found on a later attach reports through
//! the same note.
//!
//! The fake server in `crash` mode answers `initialize` and exits 7;
//! `restart = "never"` keeps it crashed so the modeline and panel can
//! be read at rest.
//!
//! Bite: drop the `crashed` arm in the drain and the first row fails
//! at the `*errors*` wait; drop the modeline's crashed check and the
//! segment row fails; drop the "Crashed" section and the panel row
//! fails.

use std::path::{Path, PathBuf};

use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};
use tempfile::TempDir;

#[path = "common/iso.rs"]
mod iso;
#[path = "common/ready.rs"]
mod ready;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn lua_str(s: &str) -> String {
    format!("{s:?}")
}

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

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

fn editor_for(dir: &Path) -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.project.set_search_boundary({})",
            lua_str(&dir.display().to_string())
        ),
    );
    exec(
        &state,
        &format!(
            r#"pmacs.lsp.config = {{ rust = {{ command = {}, env = {{ PMACS_FAKE_LSP_MODE = "crash" }}, restart = "never" }} }}"#,
            lua_str(&fake_lsp_path())
        ),
    );
    state
}

fn open(state: &EditorState, path: &Path) {
    exec(
        state,
        &format!(
            "pmacs.buffer.find_or_open({})",
            lua_str(&path.display().to_string())
        ),
    );
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
    windows
        .into_iter()
        .flat_map(|w| w.right)
        .find(|s| s.face == "ui.modeline.lsp")
        .map(|s| s.text)
}

fn wait_errors_contain(state: &mut EditorState, needle: &str) -> String {
    ready::tick_until(
        state,
        &format!("*errors* containing {needle:?}"),
        ready::DEADLINE,
        |s| {
            let text = s.lua_host.errors_buffer_text();
            if text.contains(needle) {
                ready::Probe::Ready(text)
            } else {
                ready::Probe::Pending(text)
            }
        },
    )
}

#[test]
fn a_server_crash_is_reported_marks_the_modeline_and_lists_in_lsp() {
    let td = cargo_project();
    let mut state = editor_for(td.path());
    open(&state, &write_rs(td.path(), "a.rs"));

    // The crash reaches the error channel with the server named.
    let text = wait_errors_contain(&mut state, "crashed");
    assert!(
        text.contains("LSP: default-rust crashed"),
        "the report names the server: {text:?}"
    );
    assert!(state.lua_host.unread_errors() >= 1);

    // The modeline shows `!`, not the state's label.
    let segment = lsp_segment(&state);
    assert_eq!(
        segment.as_deref(),
        Some("LSP:!"),
        "a crashed server is marked on the buffer's modeline"
    );

    // `*lsp*` opens with the crash listed, reason included.
    exec(&state, "pmacs.command.invoke('lsp.status')");
    let panel: String = eval(
        &state,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
          if pmacs.describe.buffer(id).name == "*lsp*" then
            return id:slice(0, id:len())
          end
        end
        return ""
        "#,
    );
    assert!(
        panel.contains("Crashed (1):") && panel.contains("default-rust (crashed, attempt"),
        "the panel lists the crash: {panel:?}"
    );
}

#[test]
fn a_stale_attachment_found_on_reattach_notes_it_once_and_replaces_the_server() {
    let td = cargo_project();
    let mut state = editor_for(td.path());
    let file = write_rs(td.path(), "b.rs");
    open(&state, &file);
    wait_errors_contain(&mut state, "crashed");
    let sid_before: String = eval(
        &state,
        "return tostring(pmacs.lsp.list()[1] and pmacs.lsp.list()[1].id)",
    );

    // Attachments heal at the point of use: the next interactive LSP
    // command resolves the buffer's attachment, finds its server
    // crashed, notes it through the same latch the drain used (so no
    // second line for the same server), forgets it and spawns a
    // replacement, which crashes in its turn --- the second line.
    // `pmacs.lsp._attach_buffer()` is that resolution without a
    // command around it.
    exec(&state, "pmacs.lsp._attach_buffer()");
    // Wait for the REPLACEMENT's crash, identified by its id and not by
    // a line count: a count of two is what a duplicated note for the
    // first server would also produce, before the replacement had
    // crashed at all, which made a first draft of this row pass with
    // the latch removed.
    let sid_after = ready::tick_until(
        &mut state,
        "a replacement server, crashed in its turn",
        ready::DEADLINE,
        |s| {
            let listed: String = eval(
                s,
                r#"
                local out = {}
                for _, info in ipairs(pmacs.lsp.list()) do
                  out[#out + 1] = tostring(info.id) .. ":" .. tostring(info.state and info.state.kind)
                end
                table.sort(out)
                return table.concat(out, ",")
                "#,
            );
            let replacement_crashed = listed
                .split(',')
                .any(|entry| entry.ends_with(":crashed") && !entry.starts_with(&sid_before));
            if replacement_crashed {
                ready::Probe::Ready(listed)
            } else {
                ready::Probe::Pending(listed)
            }
        },
    );
    assert!(
        !sid_after
            .split(',')
            .any(|entry| entry.starts_with(&sid_before)),
        "the crashed server was forgotten and replaced: before {sid_before}, after {sid_after}"
    );
    // The replacement's own crash is drained on the next tick.
    let _ = ready::tick_until(
        &mut state,
        "the replacement's crash report",
        ready::DEADLINE,
        |s| {
            let text = s.lua_host.errors_buffer_text();
            if text.matches("LSP: default-rust crashed").count() >= 2 {
                ready::Probe::Ready(text)
            } else {
                ready::Probe::Pending(text)
            }
        },
    );
    // One further tick, asserted quiet. A count of two is also what the
    // drain's line plus a duplicated note for the first server produce
    // if the replacement's own line lags the listing change by a tick,
    // so the count is taken again after one more drain and must still
    // be two: with the latch gone it is three here.
    state.tick_processes();
    state.tick_lsp();
    state.tick_async();
    let text = state.lua_host.errors_buffer_text();
    assert_eq!(
        text.matches("LSP: default-rust crashed").count(),
        2,
        "one line per server, the stale-attachment note deduplicated against the drain's: {text:?}"
    );
}

#[test]
fn review_replacement_clears_the_current_crashed_section() {
    let td = cargo_project();
    let mut state = editor_for(td.path());
    let file = write_rs(td.path(), "b.rs");
    open(&state, &file);
    wait_errors_contain(&mut state, "crashed");
    exec(
        &state,
        "pmacs.lsp.config.rust.env = {}; pmacs.lsp._attach_buffer()",
    );
    ready::tick_until(
        &mut state,
        "replacement initialized",
        ready::DEADLINE,
        |s| {
            let states: String = eval(
                s,
                "local out = {}; for _, i in ipairs(pmacs.lsp.list()) do out[#out+1] = i.state.kind end; return table.concat(out, ',')",
            );
            if states == "initialized" {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(states)
            }
        },
    );
    state.tick_async();
    assert_eq!(eval::<usize>(&state, "return #pmacs.lsp.list()"), 1);
    assert_ne!(lsp_segment(&state).as_deref(), Some("LSP:!"));
    exec(&state, "pmacs.command.invoke('lsp.status')");
    let panel: String = eval(
        &state,
        r"for _, id in ipairs(pmacs.buffer.list()) do if id:name() == '*lsp*' then return id:slice(0,id:len()) end end; return ''",
    );
    assert!(
        !panel.contains("Crashed ("),
        "only an initialized replacement exists, but status retains a current crash: {panel}"
    );
}
