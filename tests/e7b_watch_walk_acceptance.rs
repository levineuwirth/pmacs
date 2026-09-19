// tests/e7b_watch_walk_acceptance.rs --- C7b fix round 1, #279: the
// file-watch poll's walk prunes `.git` and `target` and is quiet on
// the activity indicator.

//! The file-watch poll `lsp.lua` runs for a server's registered
//! `workspace/didChangeWatchedFiles` watchers is one whole-tree walk
//! per scan group, every 250 ms after a change and every 4 s at rest,
//! for as long as the server lives. At E7b.0 it was filed as #279 on
//! two counts: the activity indicator announced each walk as if the
//! user had asked for it, and the walk visited everything under the
//! root --- on this repository 1578 entries, 1012 of them under
//! `.git`. These rows pin the two cheap fixes through the seams a user
//! meets:
//!
//! * with the `filewatch` fake's `**/*.txt` watcher registered, a
//!   `.txt` created at the root is reported to the server, while one
//!   created under `.git/` and one under `target/` never are --- the
//!   walk does not enter either, so the server is not told about a
//!   change no glob of a language server asks about;
//! * while the poll runs, the walk's jobs list in `*workers*` flagged
//!   quiet and the statusline `activity` segment never names
//!   `walk_tree`, sampled every 2 ms across the first second of the
//!   backoff curve (250, 500 ms; on a tree large enough for a walk to
//!   outlast a tick).
//!
//! Bitten by `scripts/bite HEAD^ builtin/runtime/lsp.lua` (the walk
//! enters `.git`, the file under it is reported; the jobs are not
//! quiet) and by hand, `|| job.quiet` dropped from
//! `AsyncRuntime::activity_summary` (the indicator shows the walk).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};

#[path = "common/iso.rs"]
mod iso;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_until(s: &mut EditorState, ms: u64, done: impl Fn(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        tick(s);
        if done(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// The `activity` segment's text on the active window's modeline.
fn activity_segment(s: &EditorState) -> Option<String> {
    let outcome = evaluate_statusline(
        s.lua_host.lua(),
        &s.core,
        &s.statusline_registry,
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
        .find(|seg| seg.face == "ui.modeline.activity")
        .map(|seg| seg.text)
}

/// Every active `fs_walk_tree` row in the workers snapshot, as
/// `(purpose, quiet)`.
fn active_walks(s: &EditorState) -> Vec<(String, bool)> {
    let (purposes, quiets): (Vec<String>, Vec<bool>) = eval(
        s,
        "local purposes, quiets = {}, {}
         for _, r in ipairs(pmacs.workers.snapshot().active) do
           if r.kind == 'fs_walk_tree' then
             purposes[#purposes + 1] = r.purpose
             quiets[#quiets + 1] = r.quiet == true
           end
         end
         return purposes, quiets",
    );
    purposes.into_iter().zip(quiets).collect()
}

/// How many `fs_walk_tree` jobs have settled.
fn completed_walks(s: &EditorState) -> usize {
    eval(
        s,
        "local n = 0
         for _, r in ipairs(pmacs.workers.snapshot().completed) do
           if r.kind == 'fs_walk_tree' then n = n + 1 end
         end
         return n",
    )
}

/// A project directory with `.git/` and `target/` subtrees, `a.rs`
/// visited with the `filewatch` fake --- whose `**/*.txt` watcher is
/// rooted at the directory --- and the handshake answered. `bulk`
/// files under `src/` make the walk long enough to be seen in flight.
fn editor_watching(base: &Path, bulk: usize) -> EditorState {
    std::fs::create_dir_all(base.join(".git/objects/aa")).unwrap();
    std::fs::write(base.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(base.join("target/debug")).unwrap();
    std::fs::write(base.join("target/debug/built"), b"").unwrap();
    std::fs::create_dir_all(base.join("src")).unwrap();
    for i in 0..bulk {
        std::fs::write(base.join(format!("src/f{i}.rs")), b"").unwrap();
    }
    let a_path = base.join("a.rs");
    std::fs::write(&a_path, b"fn a() {}\n").unwrap();
    let s = EditorState::new_with_roots(&iso::roots());
    let fake = fake_lsp_path();
    exec(
        &s,
        &format!(
            "pmacs.lsp.config = {{}}
             pmacs.lsp.config.rust = {{ command = '{fake}',
               env = {{ PMACS_FAKE_LSP_MODE = 'filewatch',
                        PMACS_FAKE_LSP_WATCH_BASE = {:?} }} }}",
            base.display().to_string()
        ),
    );
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            a_path.display().to_string()
        ),
    );
    let mut s = s;
    assert!(
        pump_until(&mut s, 10_000, |s| eval::<bool>(
            s,
            &format!("return {INITIALIZED}")
        )),
        "fake server init"
    );
    s
}

fn received(base: &Path) -> String {
    std::fs::read_to_string(base.join(".received")).unwrap_or_default()
}

/// The walk does not enter `.git` or `target`: a matching file created
/// under either is never reported, while the same file at the root is
/// --- the positive control that the watcher runs and reports.
#[test]
fn files_under_git_and_target_are_never_reported_while_the_root_is() {
    let td = tempfile::tempdir().expect("tempdir");
    let base: PathBuf = td.path().to_path_buf();
    let mut s = editor_watching(&base, 0);
    // The baseline: let the watcher establish its first snapshot.
    assert!(
        pump_until(&mut s, 5_000, |s| completed_walks(s) >= 1),
        "the first walk settles"
    );
    std::fs::write(base.join(".git/inner.txt"), b"one\n").unwrap();
    std::fs::write(base.join("target/built.txt"), b"one\n").unwrap();
    std::fs::write(base.join("top.txt"), b"one\n").unwrap();
    let top_uri = format!("file://{}", base.join("top.txt").display());
    assert!(
        pump_until(&mut s, 6_000, |_| received(&base)
            .contains(&format!("1 {top_uri}"))),
        "CREATED for top.txt reported; .received = {:?}",
        received(&base)
    );
    // Two more scans after the report, at the reset 250 ms cadence,
    // so a pruned file would have had its chance.
    let walks = completed_walks(&s);
    assert!(
        pump_until(&mut s, 6_000, |s| completed_walks(s) >= walks + 2),
        "two more walks settle"
    );
    let got = received(&base);
    eprintln!("WATCH .received:\n{got}");
    assert!(
        !got.contains(".git/inner.txt"),
        "a file under .git is never reported: {got:?}"
    );
    assert!(
        !got.contains("target/built.txt"),
        "a file under target is never reported: {got:?}"
    );
}

/// The walk's jobs are quiet: listed in `*workers*` with the flag, and
/// never named by the `activity` segment. Sampled every 2 ms for the
/// first second after the handshake, over a tree of 1500 files so a
/// walk spans more than one tick; the walks seen in flight are the
/// positive control that there was something to hide.
#[test]
fn the_walk_is_quiet_on_the_activity_indicator_and_flagged_in_workers() {
    let td = tempfile::tempdir().expect("tempdir");
    let base: PathBuf = td.path().to_path_buf();
    let mut s = editor_watching(&base, 1500);
    let deadline = Instant::now() + Duration::from_millis(1_200);
    let mut in_flight_seen = 0usize;
    let mut shown: Vec<String> = Vec::new();
    while Instant::now() < deadline {
        tick(&mut s);
        for (purpose, quiet) in active_walks(&s) {
            in_flight_seen += 1;
            assert!(quiet, "the walk {purpose:?} is dispatched quiet");
        }
        if let Some(text) = activity_segment(&s)
            && text.contains("walk_tree")
        {
            shown.push(text);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let settled = completed_walks(&s);
    eprintln!(
        "WATCH walks seen in flight at {in_flight_seen} samples, {settled} settled, shown {shown:?}"
    );
    assert!(settled >= 2, "the poll ran: {settled} walks settled");
    assert!(
        in_flight_seen >= 1,
        "a walk was caught in flight, so the indicator had something to hide"
    );
    assert!(
        shown.is_empty(),
        "the activity segment never named the walk: {shown:?}"
    );
}
