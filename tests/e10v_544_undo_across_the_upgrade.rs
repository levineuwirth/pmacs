// tests/e10v_544_undo_across_the_upgrade.rs --- #544, E10v.7's case "a
// rename across two buffers, one undo in each" in the GPU.

//! A buffer the daemon holds without CRDT backing keeps the v0.1 undo
//! stack, and is converted when a replica first shows it, or when a
//! replica attaches (`Buffer::upgrade_to_crdt`, called from
//! `src/daemon.rs`). Until E10v's fix round 2 the conversion cleared the
//! stack, so a language server's rename that edited a second file no
//! window showed could not be undone there once a GPU window showed it:
//! `C-x u` said "nothing to undo", and the daemon logged
//! `upgrade_to_crdt clearing 1 undo + 0 redo entries` (the owner's pass,
//! 2026-11-24). Now the conversion writes the history into the buffer's
//! document as ops and records them as the arbiter records any edit.
//!
//! Every row drives the GPU's production `App` through the headless
//! probe against a daemon of its own, and reads the window's own replica
//! (`place:`'s `text`), its caret and its view, and the daemon's buffer
//! by a save.

#![cfg(feature = "crdt")]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

mod common;
#[path = "support/mod.rs"]
mod support;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v544-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// E10a.3's source, as `r1_p10` has it: forty comment lines, then the
/// function whose `total`s the rename edits on lines 40 to 42.
fn source() -> String {
    let mut text = String::new();
    for i in 0..40 {
        let _ = writeln!(text, "// {i}");
    }
    text.push_str("fn count(total: u32) -> u32 {\n    let x = total + total;\n    total * x + total\n}\n// tail\n");
    text
}

/// The second file the rename edits, four lines so a caret can be moved
/// off the place the rename found it at before the undo.
const OTHER: &str = "fn other(total: u32) -> u32 {\n    total\n}\n// other tail\n";
const OTHER_RENAMED: &str = "fn other(n: u32) -> u32 {\n    n\n}\n// other tail\n";

fn rename_edit(line: u32, start: u32, end: u32) -> Value {
    json!({
        "range": { "start": { "line": line, "character": start },
                   "end": { "line": line, "character": end } },
        "newText": "n"
    })
}

/// The steps that put the caret on `total` in `a.rs` and rename it to `n`
/// by `C-c r`, as `r1_p10` takes them.
fn rename_steps() -> String {
    let right = "tap:C-f ".repeat(22);
    format!(
        "shows:a.rs quiet:1500 key:M-x type:goto-line key:ret type:42 key:ret quiet:500 \
         {right}quiet:800 key:C-c key:r quiet:500 type:n key:ret band:LSP: quiet:800 \
         place:renamed"
    )
}

/// `C-x b b.rs RET`, waiting for the window to show it: its first show,
/// at which the daemon converts it.
const SHOW_OTHER: &str = "key:C-x key:b type:b.rs key:ret shows:b.rs quiet:800 place:shown";

/// A daemon whose rust server is the fake answering a rename in `a.rs`
/// with five edits there and two in `b.rs`, `a.rs` visited and `b.rs`
/// not open: the rename opens it.
fn rename_daemon(tag: &str) -> (common::daemon::TestDaemon, PathBuf) {
    let d = dir(tag);
    let a = d.join("a.rs");
    let b = d.join("b.rs");
    std::fs::write(&a, source()).unwrap();
    std::fs::write(&b, OTHER).unwrap();
    let mut changes = serde_json::Map::new();
    changes.insert(
        uri(&a),
        json!([
            rename_edit(40, 9, 14),
            rename_edit(41, 12, 17),
            rename_edit(41, 20, 25),
            rename_edit(42, 4, 9),
            rename_edit(42, 16, 21)
        ]),
    );
    changes.insert(
        uri(&b),
        json!([rename_edit(0, 9, 14), rename_edit(1, 4, 9)]),
    );
    let plan = d.join("plan.json");
    std::fs::write(&plan, json!({ "changes": changes }).to_string()).unwrap();
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.lsp.config.rust = {{ command = '{}', env = {{ \
           PMACS_FAKE_LSP_MODE = 'applyeditplan', PMACS_FAKE_LSP_EDIT_PLAN = '{}', \
           PMACS_FAKE_LSP_APPLYEDIT_SINK = '{}' }} }}\n\
         pmacs.buffer.find_or_open({:?})\n",
        fake_lsp_path(),
        plan.display(),
        d.join("sink.json").display(),
        a.display().to_string()
    );
    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(&[], &init);
    (daemon, b)
}

/// The probe's report, or `None` when it was skipped (said).
fn probe(
    daemon: &common::daemon::TestDaemon,
    label: &str,
    steps: &str,
) -> Option<std::collections::HashMap<String, String>> {
    let facts = common::gpu_probe::run_script_probe(daemon.socket_path(), label, steps);
    if facts.is_none() {
        support::record_skip("the GPU probe did not run");
    }
    facts
}

/// The premise every row stands on: the daemon converted `b.rs` while the
/// row ran, whatever it did with the history (the line names the buffer).
fn assert_b_was_converted(daemon: &common::daemon::TestDaemon) {
    let log = daemon.stderr_log();
    assert!(
        log.lines()
            .any(|l| l.starts_with("Buffer b.rs ") && l.contains("upgrade_to_crdt")),
        "premise: the daemon converted b.rs; its log:\n{log}"
    );
}

/// The window's own text at a `place:` step, as the probe wrote it.
fn text_fact(facts: &std::collections::HashMap<String, String>, label: &str) -> String {
    common::gpu_probe::fact(facts, &format!("{label}.text")).to_owned()
}

/// The window's caret line and column and its view's first line at a
/// `place:` step.
fn place_fact(
    facts: &std::collections::HashMap<String, String>,
    label: &str,
) -> (String, String, String) {
    let fact = |key: &str| common::gpu_probe::fact(facts, &format!("{label}.{key}")).to_owned();
    (fact("line"), fact("col"), fact("scroll_top"))
}

/// The pass's case. A rename from `a.rs` edits `b.rs`, which no window
/// shows; `b.rs` is then shown in the GPU, where the daemon converts it;
/// one `C-x u` there restores its text exactly, in the window's replica
/// and in the daemon's buffer, and puts the caret and the view where the
/// rename found the buffer: the top, where the workspace edit's
/// `find_or_open` put them (E10v.7's place for a buffer the edit opened;
/// its in-process rows state none for the second buffer). The caret is
/// moved off it first, so the place is read and not left. At `3a92af5`
/// the conversion cleared the rename's entry and `C-x u` said "nothing
/// to undo", the text still renamed.
#[test]
fn e10v_544_a_rename_into_a_buffer_no_window_shows_is_one_undo_there_in_the_gpu() {
    let (daemon, b) = rename_daemon("undo");
    let steps = format!(
        "{} {SHOW_OTHER} key:C-n key:C-n quiet:500 place:moved \
         key:C-x key:u quiet:800 place:undone key:C-x key:C-s quiet:800",
        rename_steps()
    );
    let Some(facts) = probe(&daemon, "e544undo", &steps) else {
        return;
    };
    eprintln!("E544 undo facts {facts:?}");
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert!(
        fact("renamed.status").contains("LSP: renamed"),
        "premise: the rename was applied; facts {facts:?}"
    );
    assert_eq!(
        text_fact(&facts, "shown"),
        format!("{OTHER_RENAMED:?}"),
        "premise: b.rs, first shown after the rename, shows it"
    );
    let top = ("1".to_owned(), "1".to_owned(), "0".to_owned());
    assert_ne!(
        place_fact(&facts, "moved"),
        top,
        "premise: the caret moved off the top before the undo"
    );
    assert_b_was_converted(&daemon);
    assert_eq!(
        text_fact(&facts, "undone"),
        format!("{OTHER:?}"),
        "one C-x u in b.rs takes the rename back in the window's replica; status {:?}",
        fact("undone.status")
    );
    assert_eq!(
        place_fact(&facts, "undone"),
        top,
        "the caret and the view where the rename found b.rs"
    );
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        OTHER,
        "and in the daemon's buffer (the save wrote what it held)"
    );
}

/// The pass's case, then `C-x r`: the redo puts the rename back in `b.rs`
/// whole, in the window's replica and in the daemon's buffer. The undo
/// is this row's first assertion, since a redo after an undo that did
/// nothing leaves the renamed text standing; at `3a92af5` it fails there.
#[test]
fn e10v_544_a_redo_there_puts_the_rename_back_whole() {
    let (daemon, b) = rename_daemon("redo");
    let steps = format!(
        "{} {SHOW_OTHER} key:C-x key:u quiet:800 place:undone \
         key:C-x key:r quiet:800 place:redone key:C-x key:C-s quiet:800",
        rename_steps()
    );
    let Some(facts) = probe(&daemon, "e544redo", &steps) else {
        return;
    };
    eprintln!("E544 redo facts {facts:?}");
    assert!(
        common::gpu_probe::fact(&facts, "renamed.status").contains("LSP: renamed"),
        "premise: the rename was applied; facts {facts:?}"
    );
    assert_b_was_converted(&daemon);
    assert_eq!(
        text_fact(&facts, "undone"),
        format!("{OTHER:?}"),
        "one C-x u in b.rs takes the rename back"
    );
    assert_eq!(
        text_fact(&facts, "redone"),
        format!("{OTHER_RENAMED:?}"),
        "one C-x r puts it back whole in the window's replica"
    );
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        OTHER_RENAMED,
        "and in the daemon's buffer"
    );
}

/// The attach case. `b.rs` is edited while no window is attached (the
/// daemon's `init.lua`, one edit group of two edits with the place it was
/// asked from, as a workspace edit makes one); a GPU window then attaches,
/// whose bootstrap converts every buffer the daemon holds, `b.rs` among
/// them (`send_buffer_snapshots`); one `C-x u` there restores it, in the
/// window's replica and the daemon's buffer, with the caret and the view
/// at the group's place. At `3a92af5` the attach cleared the entry.
#[test]
fn e10v_544_a_buffer_edited_before_a_window_attaches_is_one_undo_in_it() {
    let d = dir("attach");
    let a = d.join("a.rs");
    let b = d.join("b.rs");
    std::fs::write(&a, source()).unwrap();
    std::fs::write(&b, OTHER).unwrap();
    // The two `total`s of `OTHER`, by byte, the later first; the place is
    // line 2, column 5.
    let second = OTHER.find("    total").unwrap() + 4;
    let first = OTHER.find("total").unwrap();
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         local b = pmacs.buffer.find_or_open({b:?})\n\
         b:edit_group(function()\n\
           b:replace({second}, {second_end}, 'n')\n\
           b:replace({first}, {first_end}, 'n')\n\
         end, {second}, 0)\n\
         pmacs.buffer.find_or_open({a:?})\n",
        b = b.display().to_string(),
        a = a.display().to_string(),
        second_end = second + 5,
        first_end = first + 5,
    );
    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(&[], &init);
    let steps = format!(
        "shows:a.rs quiet:500 {SHOW_OTHER} key:C-n key:C-n quiet:500 place:moved \
         key:C-x key:u quiet:800 place:undone key:C-x key:C-s quiet:800"
    );
    let Some(facts) = probe(&daemon, "e544attach", &steps) else {
        return;
    };
    eprintln!("E544 attach facts {facts:?}");
    assert_eq!(
        text_fact(&facts, "shown"),
        format!("{OTHER_RENAMED:?}"),
        "premise: init.lua's edit stands when the window attaches"
    );
    let place = ("2".to_owned(), "5".to_owned(), "0".to_owned());
    assert_ne!(
        place_fact(&facts, "moved"),
        place,
        "premise: the caret moved off the group's place before the undo"
    );
    assert_b_was_converted(&daemon);
    assert_eq!(
        text_fact(&facts, "undone"),
        format!("{OTHER:?}"),
        "one C-x u in b.rs takes the group back in the window's replica; status {:?}",
        common::gpu_probe::fact(&facts, "undone.status")
    );
    assert_eq!(
        place_fact(&facts, "undone"),
        place,
        "the caret and the view at the group's place"
    );
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        OTHER,
        "and in the daemon's buffer"
    );
}
