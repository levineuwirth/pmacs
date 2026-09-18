// tests/e7b_status_acceptance.rs --- E7b.1, busy is not a state.

//! The modeline's `LSP:` segment through the real drain: a fake server
//! echoes whatever `$/progress` cycle the test asks for
//! (`pmacs/progress`, every mode), so each witness chooses when a
//! begin and an end arrive and reads the segment between them.
//!
//! * A flycheck cycle --- rust-analyzer's `rust-analyzer/flycheck/<n>`
//!   token titled after the check command --- leaves the label `ready`
//!   throughout, with the work as a suffix (`LSP:ready·check`).
//! * An indexing cycle --- `rustAnalyzer/cachePriming`, title
//!   `Indexing` --- flips the label to `idx` and flips it back.
//!
//! The `Degraded` case is witnessed at the unit level in
//! `src/lsp_status.rs` with the same token names.
//!
//! Two `#[ignore]`d measurements run against the real rust-analyzer
//! on this repository's `src/editor.rs`, by hand, and print what they
//! saw: the owner's own sequence (E7b.1's last clause: five saves in a
//! row with the server warm, the label sampled every few milliseconds
//! and never leaving `ready`), and E7b.0's `walk_tree` question (what
//! dispatches `pmacs.fs.walk_tree` when nothing asked for the finder,
//! and how often the activity indicator shows it).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};

#[path = "common/iso.rs"]
mod iso;
#[path = "support/mod.rs"]
mod support;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
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

/// One keystroke through the production dispatch, as both frontends'
/// keys arrive.
fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

/// Flush the coalesced `didChange` and tick for a moment so the server
/// holds the edit before the round measures anything; returns the
/// buffer's first sixty bytes.
fn settle_did_change(s: &mut EditorState) -> String {
    exec(s, "pmacs.lsp._flush_did_changes()");
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        tick(s);
        std::thread::sleep(Duration::from_millis(5));
    }
    eval(s, "local b = pmacs.window.buffer() return b:slice(0, 60)")
}

/// The open popup's candidates as `label (kind) detail`, or none. The
/// kind and detail tell a server's item from the word source's.
fn popup_labels(s: &EditorState) -> Vec<String> {
    let core = s.core.borrow();
    let popup = core.completion_popup.lock().unwrap();
    popup
        .as_ref()
        .map(|p| {
            p.candidates
                .iter()
                .map(|c| format!("{} ({:?}) {:?}", c.label, c.kind, c.detail))
                .collect()
        })
        .unwrap_or_default()
}

fn pump_lua_flag(s: &mut EditorState, flag: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        let done: bool = s
            .lua_host
            .lua()
            .load(format!("return ({flag}) == true"))
            .eval()
            .unwrap_or(false);
        if done {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// A segment's text on the active window's modeline, by face.
fn segment(s: &EditorState, face: &str) -> Option<String> {
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
        .find(|seg| seg.face == face)
        .map(|seg| seg.text)
}

fn lsp_segment(s: &EditorState) -> Option<String> {
    segment(s, "ui.modeline.lsp")
}

/// An editor visiting `dir/a.rs`, the rust server pointed at the fake,
/// returning once the fake has initialized and the segment says so.
fn fake_editor(dir: &Path) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = fake_lsp_path();
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{ PMACS_FAKE_LSP_MODE = 'e7b' }},
             }}"
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    assert!(
        wait_segment(&mut s, "LSP:ready", 5),
        "the segment reads ready after the handshake; it reads {:?}",
        lsp_segment(&s)
    );
    s
}

/// Send one `$/progress` through the fake's echo. `value` is the Lua
/// table literal of the notification's `value`.
fn progress(s: &EditorState, token: &str, value: &str) {
    exec(
        s,
        &format!(
            "local rec = pmacs.lsp.active_attachment()
             pmacs.lsp.send_notification(rec.server, 'pmacs/progress',
               {{ token = {token:?}, value = {value} }})"
        ),
    );
}

/// Tick until the `LSP:` segment reads exactly `want`, recording every
/// distinct text seen on the way; `true` when it arrived.
fn wait_segment(s: &mut EditorState, want: &str, secs: u64) -> bool {
    wait_segment_seeing(s, want, secs).0
}

fn wait_segment_seeing(s: &mut EditorState, want: &str, secs: u64) -> (bool, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut seen: Vec<String> = Vec::new();
    loop {
        tick(s);
        let text = lsp_segment(s).unwrap_or_default();
        if seen.last() != Some(&text) {
            seen.push(text.clone());
        }
        if text == want {
            return (true, seen);
        }
        if Instant::now() >= deadline {
            return (false, seen);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn busy(s: &EditorState) -> Option<String> {
    eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local st = pmacs.lsp.status_summary(rec.server)
         return st and st.busy or nil",
    )
}

// ---------------------------------------------------------------------------
// The witnesses
// ---------------------------------------------------------------------------

/// A flycheck cycle leaves the label `ready` throughout: the begin adds
/// the suffix, the report keeps it, the end removes it, and `idx`
/// never appears. Bitten by putting `rust-analyzer/flycheck/0` in
/// `INDEXING_TOKENS`, or by rendering busy as the label.
#[test]
fn e7b_1_a_flycheck_cycle_leaves_the_label_ready_with_a_suffix() {
    let dir = temp_dir("flycheck");
    let mut s = fake_editor(&dir);

    progress(
        &s,
        "rust-analyzer/flycheck/0",
        "{ kind = 'begin', title = 'cargo check', cancellable = true }",
    );
    let (arrived, seen) = wait_segment_seeing(&mut s, "LSP:ready·check", 5);
    assert!(arrived, "the suffix arrives; the segment went {seen:?}");
    assert!(
        seen.iter().all(|t| t.starts_with("LSP:ready")),
        "the label never left ready on the way: {seen:?}"
    );
    assert_eq!(busy(&s).as_deref(), Some("cargo check"));

    progress(
        &s,
        "rust-analyzer/flycheck/0",
        "{ kind = 'report', message = 'pmacs' }",
    );
    // A report changes nothing visible: pump a little and look.
    for _ in 0..50 {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(1));
        assert_eq!(lsp_segment(&s).as_deref(), Some("LSP:ready·check"));
    }

    progress(&s, "rust-analyzer/flycheck/0", "{ kind = 'end' }");
    let (arrived, seen) = wait_segment_seeing(&mut s, "LSP:ready", 5);
    assert!(
        arrived,
        "the suffix goes at the end; the segment went {seen:?}"
    );
    assert!(
        seen.iter().all(|t| t.starts_with("LSP:ready")),
        "and the label never left ready: {seen:?}"
    );
    assert_eq!(busy(&s), None);
}

/// An indexing cycle flips the label to `idx` and flips it back, and
/// the `*lsp*` surface names the work while it runs. Bitten by
/// removing `rustAnalyzer/cachePriming` from `INDEXING_TOKENS`.
#[test]
fn e7b_1_an_indexing_cycle_flips_the_label_and_flips_it_back() {
    let dir = temp_dir("indexing");
    let mut s = fake_editor(&dir);

    progress(
        &s,
        "rustAnalyzer/cachePriming",
        "{ kind = 'begin', title = 'Indexing', percentage = 0, cancellable = true }",
    );
    assert!(
        wait_segment(&mut s, "LSP:idx", 5),
        "indexing flips the label; it reads {:?}",
        lsp_segment(&s)
    );
    assert_eq!(busy(&s), None, "indexing is the kind, not busy");
    let summary: String = eval(
        &s,
        "local rec = pmacs.lsp.active_attachment()
         local st = pmacs.lsp.status_summary(rec.server)
         return st.kind .. ':' .. tostring(st.indexing_title)",
    );
    assert_eq!(summary, "indexing:Indexing");

    progress(
        &s,
        "rustAnalyzer/cachePriming",
        "{ kind = 'report', percentage = 50, message = '3/7 (pmacs)' }",
    );
    for _ in 0..50 {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(1));
        assert_eq!(lsp_segment(&s).as_deref(), Some("LSP:idx"));
    }

    progress(&s, "rustAnalyzer/cachePriming", "{ kind = 'end' }");
    assert!(
        wait_segment(&mut s, "LSP:ready", 5),
        "the end flips it back; it reads {:?}",
        lsp_segment(&s)
    );
}

/// A flycheck that begins while indexing runs, and outlives it: the
/// label is `idx` until the priming ends, then `ready·check` until the
/// check ends. The two cycles settle by token, never by order.
#[test]
fn e7b_1_overlapping_cycles_settle_by_token() {
    let dir = temp_dir("overlap");
    let mut s = fake_editor(&dir);

    progress(
        &s,
        "rustAnalyzer/cachePriming",
        "{ kind = 'begin', title = 'Indexing' }",
    );
    assert!(wait_segment(&mut s, "LSP:idx", 5));
    progress(
        &s,
        "rust-analyzer/flycheck/0",
        "{ kind = 'begin', title = 'cargo check' }",
    );
    for _ in 0..50 {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(1));
        assert_eq!(lsp_segment(&s).as_deref(), Some("LSP:idx"));
    }
    progress(&s, "rustAnalyzer/cachePriming", "{ kind = 'end' }");
    let (arrived, seen) = wait_segment_seeing(&mut s, "LSP:ready·check", 5);
    assert!(arrived, "the check outlives the priming: {seen:?}");
    assert!(
        !seen.iter().any(|t| t == "LSP:ready"),
        "and the label never said plain ready while the check ran: {seen:?}"
    );
    progress(&s, "rust-analyzer/flycheck/0", "{ kind = 'end' }");
    assert!(wait_segment(&mut s, "LSP:ready", 5));
}

// ---------------------------------------------------------------------------
// Measurements against rust-analyzer, by hand
// ---------------------------------------------------------------------------

/// An editor visiting this repository's `src/editor.rs` with the real
/// rust-analyzer, returned once the handshake is answered.
fn editor_rs_with_rust_analyzer(tag: &str) -> EditorState {
    // `PMACS_E7B_MEASURE_ROOT` points the measurement at a copy of the
    // repository, so a round that edits the file edits the copy.
    let manifest = std::env::var_os("PMACS_E7B_MEASURE_ROOT")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let target = manifest.join("src").join("editor.rs");
    let dir = temp_dir(tag);
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir));
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            target.display().to_string()
        ),
    );
    let mut s = s;
    let t0 = Instant::now();
    assert!(
        pump_lua_flag(&mut s, INITIALIZED, 120),
        "rust-analyzer init"
    );
    eprintln!(
        "MEASURE handshake answered {} ms after the open",
        t0.elapsed().as_millis()
    );
    s
}

/// Tick until the server has no cycle in flight at all --- the label is
/// exactly `LSP:ready` --- and has stayed so for `quiet`. That is
/// "warm": priming done, the initial flycheck done.
fn wait_warm(s: &mut EditorState, secs: u64, quiet: Duration) -> Vec<(u128, String)> {
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(secs);
    let mut trace: Vec<(u128, String)> = Vec::new();
    let mut ready_since: Option<Instant> = None;
    loop {
        tick(s);
        let text = lsp_segment(s).unwrap_or_default();
        if trace.last().map(|(_, t)| t) != Some(&text) {
            trace.push((t0.elapsed().as_millis(), text.clone()));
        }
        if text == "LSP:ready" {
            let since = *ready_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= quiet {
                return trace;
            }
        } else {
            ready_since = None;
        }
        assert!(
            Instant::now() < deadline,
            "the server never went warm: {trace:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// E7b.1's last clause, the owner's own sequence: with rust-analyzer
/// warm on `src/editor.rs`, save five times in a row and sample the
/// label every five milliseconds until the server is quiet again. The
/// label must never leave `ready`; the transitions are printed so the
/// record can say what the suffix showed and for how long. Then the
/// same with the five saves back to back.
#[test]
#[ignore = "a measurement against rust-analyzer on src/editor.rs; run by hand and record"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear measurement session, printed as it goes"
)]
fn e7b_1_measure_five_saves_keep_the_label_ready() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let mut s = editor_rs_with_rust_analyzer("saves");
    let warm = wait_warm(&mut s, 600, Duration::from_secs(3));
    eprintln!("MEASURE warm after {warm:?}");

    let dump = |s: &EditorState, when: &str| {
        let lines: Vec<String> = eval(
            s,
            "local out = {}
             local rec = pmacs.lsp.active_attachment()
             local st = pmacs.lsp.status_summary(rec.server)
             out[#out + 1] = 'kind=' .. tostring(st.kind) .. ' busy=' .. tostring(st.busy)
               .. ' retry_responses=' .. tostring(st.retry_responses)
             if st.last_error then
               out[#out + 1] = 'last_error=' .. tostring(st.last_error.source) .. ': '
                 .. tostring(st.last_error.message)
             end
             for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
               out[#out + 1] = '[' .. m.channel .. '] ' .. m.summary
                 .. (m.detail and (' -- ' .. m.detail) or '')
             end
             return out",
        );
        eprintln!("MEASURE *lsp* {when}:");
        for line in lines {
            eprintln!("MEASURE   {line}");
        }
    };
    dump(&s, "warm");
    for (round, gap) in [
        ("spaced", Duration::from_millis(300)),
        ("burst", Duration::ZERO),
        ("formatting", Duration::from_millis(300)),
        ("broken", Duration::from_millis(300)),
        ("completion", Duration::from_millis(300)),
    ] {
        if round == "formatting" {
            // The owner's E7.3 setting: the save waits on
            // `textDocument/formatting` first. The file is
            // rustfmt-clean, so the answer is zero edits and nothing
            // on disk changes.
            exec(&s, "pmacs.config.set('lsp.format-on-save', true)");
        }
        if round == "broken" {
            // A stray character at the top makes the file unparsable,
            // as a file mid-edit is: rustfmt fails, the formatter
            // answers nothing, the save proceeds. Only under
            // `PMACS_E7B_MEASURE_ROOT`, since it writes the file.
            // Typed through `dispatch_key`, as a user's edit is (C7b
            // fix round 1): a Lua `insert` outside a command fires no
            // `buffer.after-edit` and queues no `didChange`, so the
            // first version of this round broke the file on disk
            // while rust-analyzer went on holding the clean document.
            if std::env::var_os("PMACS_E7B_MEASURE_ROOT").is_none() {
                eprintln!("MEASURE broken round skipped: not on a copy");
                continue;
            }
            exec(&s, "pmacs.editor.goto_byte(0)");
            press(&mut s, KeyCode::Char('X'));
            let text = settle_did_change(&mut s);
            eprintln!("MEASURE broken: typed and flushed; the top now reads {text:?}");
        }
        if round == "completion" {
            if std::env::var_os("PMACS_E7B_MEASURE_ROOT").is_none() {
                eprintln!("MEASURE completion round skipped: not on a copy");
                continue;
            }
            // Take the stray character back, then a completion at
            // point on a fresh line: type the prefix, ask, accept the
            // first candidate with TAB, and save. Every edit is a
            // keystroke, so the server holds what the buffer holds.
            exec(&s, "pmacs.editor.goto_byte(1)");
            press(&mut s, KeyCode::Backspace);
            for ch in "use std::collections::HashMa".chars() {
                press(&mut s, KeyCode::Char(ch));
                tick(&mut s);
            }
            // The auto-completion driver may have opened on `:`; an
            // open popup would take RET as its accept.
            if eval::<bool>(&s, "return pmacs.completion.popup_visible()") {
                press(&mut s, KeyCode::Esc);
            }
            press(&mut s, KeyCode::Enter);
            let text = settle_did_change(&mut s);
            eprintln!("MEASURE completion: typed and flushed; the top now reads {text:?}");
            exec(&s, "pmacs.editor.goto_byte(28)");
            exec(&s, "pmacs.command.invoke('completion.at-point')");
            let popup = pump_lua_flag(&mut s, "pmacs.completion.popup_visible()", 20);
            let labels = popup_labels(&s);
            eprintln!("MEASURE completion popup visible: {popup}; candidates {labels:?}");
            if popup {
                press(&mut s, KeyCode::Tab);
                let line: String =
                    eval(&s, "local b = pmacs.window.buffer() return b:slice(0, 60)");
                eprintln!("MEASURE completion accepted; the top now reads {line:?}");
            }
        }
        let t0 = Instant::now();
        let mut trace: Vec<(u128, String)> = Vec::new();
        let mut left_ready: Vec<(u128, String)> = Vec::new();
        let mut sample = |s: &mut EditorState, trace: &mut Vec<(u128, String)>| {
            tick(s);
            let text = lsp_segment(s).unwrap_or_default();
            if !text.starts_with("LSP:ready") {
                left_ready.push((t0.elapsed().as_millis(), text.clone()));
            }
            if trace.last().map(|(_, t)| t) != Some(&text) {
                trace.push((t0.elapsed().as_millis(), text));
            }
        };
        for i in 0..5 {
            exec(&s, "pmacs.command.invoke('buffer.save')");
            eprintln!(
                "MEASURE {round} save {} at {} ms: status {:?}",
                i + 1,
                t0.elapsed().as_millis(),
                s.core.borrow().status
            );
            let until = Instant::now() + gap;
            while Instant::now() < until {
                sample(&mut s, &mut trace);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        // Sample until the server has been quiet for three seconds.
        let mut ready_since: Option<Instant> = None;
        let deadline = Instant::now() + Duration::from_mins(2);
        loop {
            sample(&mut s, &mut trace);
            let text = lsp_segment(&s).unwrap_or_default();
            if text == "LSP:ready" {
                let since = *ready_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(3) {
                    break;
                }
            } else {
                ready_since = None;
            }
            assert!(
                Instant::now() < deadline,
                "the server never settled: {trace:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        eprintln!("MEASURE {round} transitions (ms since the first save): {trace:?}");
        dump(&s, &format!("after the {round} saves"));
        assert!(
            left_ready.is_empty(),
            "the label left ready during the {round} saves: {left_ready:?}"
        );
    }
}

/// E7b.0's question, reproduced: open `src/editor.rs` with
/// rust-analyzer, invoke nothing, and watch the runtime's job table and
/// the activity indicator for twenty seconds after the handshake.
/// Prints every `walk_tree` job seen with its start and length, and
/// every text the `activity` segment showed; asserts nothing.
#[test]
#[ignore = "a reproduction against rust-analyzer on src/editor.rs; run by hand and record"]
fn e7b_0_watch_for_walk_tree_jobs_nobody_asked_for() {
    if !on_path("rust-analyzer") {
        support::skip_or_fail("rust-analyzer", "PMACS_REQUIRE_LSP");
        return;
    }
    let mut s = editor_rs_with_rust_analyzer("walk");
    let t0 = Instant::now();
    let mut walks: Vec<(u64, u128, u128)> = Vec::new(); // id, first seen, last seen
    let mut indicator: Vec<(u128, String)> = Vec::new();
    let mut lsp_log_hits = 0usize;
    while t0.elapsed() < Duration::from_secs(20) {
        tick(&mut s);
        let now = t0.elapsed().as_millis();
        let active: Vec<String> = eval(
            &s,
            "local out = {}
             for _, job in ipairs(pmacs.workers.snapshot().active) do
               out[#out + 1] = tostring(job.id) .. '\t' .. job.purpose
             end
             return out",
        );
        for row in active {
            let (id, purpose) = row.split_once('\t').expect("id\tpurpose");
            let id: u64 = id.parse().expect("job id");
            if purpose.starts_with("walk_tree") {
                match walks.iter_mut().find(|(wid, _, _)| *wid == id) {
                    Some(w) => w.2 = now,
                    None => walks.push((id, now, now)),
                }
            }
        }
        let text = segment(&s, "ui.modeline.activity").unwrap_or_default();
        if indicator.last().map(|(_, t)| t) != Some(&text) {
            indicator.push((now, text));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let recent: Vec<String> = eval(
        &s,
        "local out = {}
         local rec = pmacs.lsp.active_attachment()
         for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
           out[#out + 1] = m.summary
         end
         return out",
    );
    for m in &recent {
        if m.contains("walk_tree") {
            lsp_log_hits += 1;
        }
    }
    eprintln!(
        "MEASURE walk_tree jobs in 20 s: {} --- {walks:?}",
        walks.len()
    );
    eprintln!("MEASURE activity indicator texts: {indicator:?}");
    eprintln!(
        "MEASURE *lsp* recent messages mentioning walk_tree: {lsp_log_hits} of {}",
        recent.len()
    );
}
