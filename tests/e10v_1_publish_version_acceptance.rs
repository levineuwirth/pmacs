// tests/e10v_1_publish_version_acceptance.rs --- E10v.1 (#479): a
// publish for an older version does not clear a current error.

//! `absorb_publish_diagnostics` (`src/lsp.rs`) never read a publish's
//! `version`, so the last publish to arrive stood, whatever text it was
//! computed for. HLS publishes the error for the text the user typed and
//! then, 37 to 177 ms later, an empty list computed for a text a few
//! keys older (`v24:error v3:0`); HLS 2.15.0.0 sends nothing after it,
//! so the buffer showed no error for as long as the user looked (19 of
//! 40 runs, `~/build/e10m/hls-215-fixed`).
//!
//! The rule is the owner's: the client keeps the highest version each
//! server has published for a document and drops a publish below it; a
//! publish at the same version replaces (a server's own correction); one
//! with no version takes effect and leaves the highest as it was; an
//! opened document starts over. Having honored it, the client advertises
//! `publishDiagnostics.versionSupport`.
//!
//! Each row scripts a server's publishes through the fake's publish plan
//! (`PMACS_FAKE_LSP_PUBLISH_PLAN`), puts the caret on the error, and
//! reads what a user reads: the painted status row naming the error at
//! point (E7d), beside the store.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use serde_json::{Value, json};

mod common;
#[path = "support/mod.rs"]
mod support;

use common::iso;

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

const ROWS: u32 = 24;
const COLS: u32 = 100;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_until(s: &mut EditorState, secs: u64, mut done: impl FnMut(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if done(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Tick on for `ms`, so anything the server sent after the publishes a
/// row waited for is absorbed too.
fn settle(s: &mut EditorState, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        tick(s);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn status_row(s: &EditorState) -> String {
    let size = CellSize::new(ROWS, COLS);
    let mut cells = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: COLS,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    let last = cells.chunks(COLS as usize).last().unwrap();
    last.iter()
        .map(|c| match &c.glyph {
            Glyph::Char(ch) => *ch,
            _ => ' ',
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The messages the store holds for the window's document.
fn stored(s: &EditorState) -> Vec<String> {
    let rows: Vec<mlua::Table> = eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         if not rec then return {} end
         return pmacs.diag.list(rec.uri)",
    );
    rows.into_iter()
        .map(|d| {
            let m: String = d.get("message").unwrap();
            m.lines().next().unwrap_or("").trim().to_owned()
        })
        .collect()
}

/// The publishes the editor has received for the window's document, as
/// `v<version>:<count>` (`v-` for none), in the order they came.
fn received(s: &EditorState) -> Vec<String> {
    let rows: Vec<String> = eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, p in ipairs(e10v_publishes) do
           if rec and p.uri == rec.uri then out[#out + 1] = p.label end
         end
         return out",
    );
    rows
}

const RECORDER: &str = r"
e10v_publishes = {}
pmacs.lsp.on_notification('textDocument/publishDiagnostics', function(_sid, p)
  local v = p.version
  local label = 'v' .. (type(v) == 'number' and tostring(math.floor(v)) or '-')
    .. ':' .. tostring(#(p.diagnostics or {}))
  e10v_publishes[#e10v_publishes + 1] = { uri = p.uri, label = label }
end)
";

fn error(line: u32, start: u32, end: u32, message: &str) -> Value {
    json!({
        "range": {
            "start": { "line": line, "character": start },
            "end": { "line": line, "character": end }
        },
        "severity": 1,
        "source": "e10v",
        "message": message
    })
}

fn publish(version: Option<i64>, diagnostics: Vec<Value>) -> Value {
    match version {
        Some(v) => json!({ "version": v, "diagnostics": Value::Array(diagnostics) }),
        None => json!({ "diagnostics": Value::Array(diagnostics) }),
    }
}

struct Fixture {
    dir: PathBuf,
    file: PathBuf,
    plan: PathBuf,
    caps: PathBuf,
}

fn fixture(tag: &str, name: &str, text: &str, steps: &Value) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    let plan = dir.join("plan.json");
    std::fs::write(&plan, steps.to_string()).unwrap();
    Fixture {
        caps: dir.join("caps.json"),
        dir,
        file,
        plan,
    }
}

impl Fixture {
    /// An editor with the fake server for `language`, the file open and
    /// the server initialized.
    fn editor(&self, language: &str) -> EditorState {
        let s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(self.dir.clone()));
        exec(&s, RECORDER);
        exec(
            &s,
            &format!(
                "pmacs.lsp.config = {{}}\n\
                 pmacs.project.set_search_boundary({dir:?})\n\
                 pmacs.lsp.config.{language} = {{ command = '{cmd}', env = {{ \
                   PMACS_FAKE_LSP_PUBLISH_PLAN = '{plan}', \
                   PMACS_FAKE_LSP_CAPABILITIES_SINK = '{caps}' }} }}\n",
                dir = self.dir.display().to_string(),
                cmd = fake_lsp_path(),
                plan = self.plan.display(),
                caps = self.caps.display(),
            ),
        );
        self.open(&s);
        let mut s = s;
        assert!(
            pump_until(&mut s, 10, |s| eval::<bool>(
                s,
                "return pmacs.lsp.active_attachment() ~= nil"
            )),
            "premise: the fake server attached"
        );
        s
    }

    fn open(&self, s: &EditorState) {
        exec(
            s,
            &format!(
                "pmacs.buffer.find_or_open({:?})",
                self.file.display().to_string()
            ),
        );
    }
}

/// Wait for `n` publishes for the window's document, then read on a
/// little, so a row judges the store after the last of them.
fn wait_for_publishes(s: &mut EditorState, n: usize) {
    assert!(
        pump_until(s, 10, |s| received(s).len() >= n),
        "premise: the server's {n} publishes reached the editor; received {:?}",
        received(s)
    );
    settle(s, 200);
}

/// The caret on byte `at`, then the status row as painted.
fn status_at(s: &mut EditorState, at: usize) -> String {
    exec(s, &format!("pmacs.editor.goto_byte({at})"));
    settle(s, 50);
    status_row(s)
}

const TEXT: &str = "let first = 1;\nlet second = 2;\n";

/// A one-step plan: on the `didOpen`, these publishes in this order.
fn on_open(publishes: Vec<Value>) -> Value {
    json!([{ "method": "textDocument/didOpen", "publish": Value::Array(publishes) }])
}

/// `v24:error v3:0`: the error for the newer text, then an empty list
/// for a text older than it. The error stands. At the base the empty
/// list, arriving last, clears it.
#[test]
fn e10v_1_an_older_publish_after_the_error_leaves_the_error_shown() {
    let fx = fixture(
        "older",
        "a.rs",
        TEXT,
        &on_open(vec![
            publish(Some(24), vec![error(1, 4, 10, "e10v: the current error")]),
            publish(Some(3), vec![]),
        ]),
    );
    let mut s = fx.editor("rust");
    wait_for_publishes(&mut s, 2);
    assert_eq!(received(&s), ["v24:1", "v3:0"], "premise: the order sent");
    let row = status_at(&mut s, TEXT.find("second").unwrap() + 1);
    assert!(
        row.contains("error:") && row.contains("e10v: the current error"),
        "the error published at version 24 is still shown after an empty list for version 3; \
         status row {row:?}; store {:?}",
        stored(&s)
    );
    assert_eq!(stored(&s), ["e10v: the current error"]);
}

/// `v24:error v24:0`: a publish at the version it replaces is the
/// server's own correction, and clears the error.
#[test]
fn e10v_1_a_publish_at_the_same_version_replaces() {
    let fx = fixture(
        "equal",
        "a.rs",
        TEXT,
        &on_open(vec![
            publish(Some(24), vec![error(1, 4, 10, "e10v: corrected away")]),
            publish(Some(24), vec![]),
        ]),
    );
    let mut s = fx.editor("rust");
    wait_for_publishes(&mut s, 2);
    let row = status_at(&mut s, TEXT.find("second").unwrap() + 1);
    assert!(
        !row.contains("e10v: corrected away"),
        "an empty list at the same version clears the error; status row {row:?}"
    );
    assert!(stored(&s).is_empty(), "store {:?}", stored(&s));
}

/// `v24:error v25:0`: a newer publish clears the error.
#[test]
fn e10v_1_a_publish_at_a_higher_version_replaces() {
    let fx = fixture(
        "higher",
        "a.rs",
        TEXT,
        &on_open(vec![
            publish(Some(24), vec![error(1, 4, 10, "e10v: fixed since")]),
            publish(Some(25), vec![]),
        ]),
    );
    let mut s = fx.editor("rust");
    wait_for_publishes(&mut s, 2);
    let row = status_at(&mut s, TEXT.find("second").unwrap() + 1);
    assert!(
        !row.contains("e10v: fixed since"),
        "an empty list at a higher version clears the error; status row {row:?}"
    );
    assert!(stored(&s).is_empty(), "store {:?}", stored(&s));
}

/// `v24:error`, then an unversioned empty list, then `v3:error`. The
/// unversioned list takes effect and clears the error; it leaves the
/// highest version seen at 24, so the error for version 3 that follows
/// is still dropped. At the base the last, version 3's, stands.
#[test]
fn e10v_1_an_unversioned_publish_clears_and_keeps_the_highest_version() {
    let fx = fixture(
        "unversioned",
        "a.rs",
        TEXT,
        &on_open(vec![
            publish(Some(24), vec![error(1, 4, 10, "e10v: at 24")]),
            publish(None, vec![]),
            publish(
                Some(3),
                vec![error(0, 4, 9, "e10v: computed for version 3")],
            ),
        ]),
    );
    let mut s = fx.editor("rust");
    wait_for_publishes(&mut s, 3);
    assert_eq!(
        received(&s),
        ["v24:1", "v-:0", "v3:1"],
        "premise: the order sent"
    );
    let first = status_at(&mut s, TEXT.find("first").unwrap() + 1);
    let second = status_at(&mut s, TEXT.find("second").unwrap() + 1);
    assert!(
        stored(&s).is_empty() && !first.contains("e10v:") && !second.contains("e10v:"),
        "the unversioned list cleared version 24's error and version 3's was dropped; \
         store {:?}; status rows {first:?} and {second:?}",
        stored(&s)
    );
}

/// A document opened again starts over: after `v24:error`, the buffer is
/// killed and the file opened again, and the server's first publish for
/// the new document, at version 1, stands.
#[test]
fn e10v_1_a_reopened_document_starts_over() {
    let fx = fixture(
        "reopen",
        "a.rs",
        TEXT,
        &json!([
            { "method": "textDocument/didOpen",
              "publish": [publish(Some(24), vec![error(1, 4, 10, "e10v: before the reopen")])] },
            { "method": "textDocument/didOpen",
              "publish": [publish(Some(1), vec![error(0, 4, 9, "e10v: after the reopen")])] },
        ]),
    );
    let mut s = fx.editor("rust");
    wait_for_publishes(&mut s, 1);
    assert_eq!(stored(&s), ["e10v: before the reopen"], "premise");
    exec(&s, "local b = pmacs.window.buffer(); pmacs.buffer.kill(b)");
    fx.open(&s);
    assert!(
        pump_until(&mut s, 10, |s| received(s).iter().any(|p| p == "v1:1")),
        "premise: the reopened document's publish arrived; received {:?}",
        received(&s)
    );
    settle(&mut s, 200);
    let row = status_at(&mut s, TEXT.find("first").unwrap() + 1);
    assert!(
        row.contains("e10v: after the reopen"),
        "the reopened document's version-1 publish stands; status row {row:?}; store {:?}",
        stored(&s)
    );
}

/// The client says it honors versions: `initialize` carries
/// `textDocument.publishDiagnostics.versionSupport: true`.
#[test]
fn e10v_1_the_client_advertises_version_support() {
    let fx = fixture("caps", "a.rs", TEXT, &json!([]));
    let mut s = fx.editor("rust");
    assert!(
        pump_until(&mut s, 10, |_| fx.caps.exists()),
        "premise: the fake recorded the capabilities"
    );
    let caps: Value = serde_json::from_str(&std::fs::read_to_string(&fx.caps).unwrap()).unwrap();
    assert_eq!(
        caps.pointer("/textDocument/publishDiagnostics/versionSupport"),
        Some(&Value::Bool(true)),
        "publishDiagnostics capabilities sent: {}",
        caps.pointer("/textDocument/publishDiagnostics")
            .cloned()
            .unwrap_or(Value::Null)
    );
}

/// HLS 2.15.0.0's lasting failure, replayed: the fixture is what the
/// real server published in `~/build/e10m/hls-215-fixed/run-2` after
/// the user typed `oops :: Int` and `oops = True` into `cabal init`'s
/// `app/Main.hs`: the type error at version 24, then 37 ms later an
/// empty list at version 3, and nothing after it. The keys are typed
/// here as there; the step fires on the `didChange` that holds the
/// whole text, and sends the two publishes with their versions as the
/// server sent them (the editor's own version count in-process need not
/// be 24: the rule compares a server's publishes with each other). The
/// error stays on the status row; at the base it is gone for good.
#[test]
fn e10v_1_hls_2_15s_lasting_failure_replayed_keeps_the_error() {
    let replay: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/e10v/hls-215-run-2-publishes.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let publishes = replay["publishes"].as_array().unwrap().clone();
    assert_eq!(
        publishes
            .iter()
            .map(|p| (
                p["version"].as_i64().unwrap(),
                p["diagnostics"].as_array().unwrap().len()
            ))
            .collect::<Vec<_>>(),
        [(24, 1), (3, 0)],
        "premise: the trace's two publishes"
    );
    let before = "module Main (main) where\n\ngreeting :: String\ngreeting = \"hello\" -- said once\n\nmain :: IO ()\nmain = putStrLn greeting\n";
    let fx = fixture(
        "hls215",
        "Main.hs",
        before,
        &json!([{ "method": "textDocument/didChange", "contains": "oops = True", "publish": publishes }]),
    );
    let mut s = fx.editor("haskell");
    exec(&s, &format!("pmacs.editor.goto_byte({})", before.len()));
    for ch in "oops :: Int".chars() {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(ch)));
        tick(&mut s);
    }
    // The fake answers completion; `C-g` closes its popup so Enter
    // breaks the line rather than accepting an item.
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..key(KeyCode::Char('g'))
        },
    );
    tick(&mut s);
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Enter));
    tick(&mut s);
    for ch in "oops = True".chars() {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(ch)));
        tick(&mut s);
    }
    wait_for_publishes(&mut s, 2);
    assert_eq!(
        received(&s),
        ["v24:1", "v3:0"],
        "premise: the trace's order"
    );
    // Nothing corrects it, as on 2.15.0.0: read on for a second.
    settle(&mut s, 1000);
    let text: mlua::String = eval(
        &s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    let text = String::from_utf8_lossy(&text.as_bytes()).into_owned();
    assert!(
        text.ends_with("putStrLn greeting\noops :: Int\noops = True"),
        "premise: the typed text; {text:?}"
    );
    let at = text.find("oops = True").unwrap() + "oops = ".len() + 1;
    let row = status_at(&mut s, at);
    assert!(
        row.contains("error:") && row.contains("Couldn't match expected type"),
        "HLS 2.15.0.0's type error is still shown after its older empty list; \
         status row {row:?}; store {:?}; received {:?}; text {text:?}; caret {at}; spans {}",
        stored(&s),
        received(&s),
        eval::<String>(
            &s,
            "local rec = pmacs.lsp.active_attachment(); local out = {} for _, d in ipairs(pmacs.diag.list(rec.uri)) do for k, v in pairs(d) do out[#out+1] = k .. '=' .. tostring(v) end end return table.concat(out, ' ')"
        )
    );
}
